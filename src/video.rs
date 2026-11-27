//! Видео: дата съёмки, длительность и размеры из контейнера MP4/QuickTime, кадр для миниатюры.
//!
//! Координат у видео в библиотеке нет. Кадр берёт системный декодер, если он подключён
//! (на macOS — AVFoundation через `ffi`), иначе программа `ffmpeg`; без неё у видео нет
//! миниатюры, и интерфейс рисует значок.
use anyhow::{bail, Context, Result};
use chrono::NaiveDateTime;
use image::RgbImage;
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

pub const EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "3gp", "mkv", "webm", "avi", "mts", "m2ts"];

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Сведения из контейнера; у форматов кроме MP4/QuickTime — пустые.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoMeta {
    /// Местное время съёмки.
    pub taken_at: Option<NaiveDateTime>,
    /// Длительность, с.
    pub duration: Option<f64>,
    /// Размеры кадра с учётом поворота.
    pub dims: Option<(u32, u32)>,
    pub make: Option<String>,
    pub model: Option<String>,
}

pub fn read(path: &Path) -> VideoMeta {
    let Ok(file) = File::open(path) else { return VideoMeta::default() };
    let mut r = BufReader::new(file);
    let Ok(end) = r.seek(SeekFrom::End(0)) else { return VideoMeta::default() };
    let mut meta = VideoMeta::default();
    let mut quicktime_date = None;
    let _ = walk(&mut r, 0, end, &mut |r, kind, start, end| {
        if &kind == b"moov" {
            parse_moov(r, start, end, &mut meta, &mut quicktime_date)?;
        }
        Ok(())
    });
    // Дата из ключей QuickTime записана в местном времени съёмки — она точнее mvhd (UTC).
    if let Some(d) = quicktime_date {
        meta.taken_at = Some(d);
    }
    meta
}

/// Секунд между 1904-01-01 (эпоха MP4) и 1970-01-01.
const MP4_EPOCH: u64 = 2_082_844_800;
/// Маленькие боксы (заголовки, ключи) читаются целиком; больше — пропускаются.
const SMALL_BOX: u64 = 1 << 20;

type Reader = BufReader<File>;
/// Обработчик бокса: `(читатель, тип, начало тела, конец бокса)`.
type BoxFn<'a> = dyn FnMut(&mut Reader, [u8; 4], u64, u64) -> std::io::Result<()> + 'a;

/// Обойти боксы в `[start, end)`: `f(читатель, тип, начало тела, конец бокса)`.
fn walk(
    r: &mut Reader,
    mut pos: u64,
    end: u64,
    f: &mut BoxFn,
) -> std::io::Result<()> {
    while pos + 8 <= end {
        r.seek(SeekFrom::Start(pos))?;
        let mut h = [0u8; 8];
        r.read_exact(&mut h)?;
        let size = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = h[4..].try_into().unwrap();
        let (body, box_end) = match size {
            0 => (pos + 8, end),
            1 => {
                let mut l = [0u8; 8];
                r.read_exact(&mut l)?;
                (pos + 16, pos + u64::from_be_bytes(l))
            }
            s => (pos + 8, pos + s),
        };
        if box_end < body || box_end > end {
            break;
        }
        f(r, kind, body, box_end)?;
        pos = box_end;
    }
    Ok(())
}

fn read_body(r: &mut Reader, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    if end - start > SMALL_BOX {
        return Ok(Vec::new());
    }
    r.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0; (end - start) as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn parse_moov(
    r: &mut Reader,
    start: u64,
    end: u64,
    meta: &mut VideoMeta,
    quicktime_date: &mut Option<NaiveDateTime>,
) -> std::io::Result<()> {
    walk(r, start, end, &mut |r, kind, start, end| {
        match &kind {
            b"mvhd" => {
                let b = read_body(r, start, end)?;
                let (created, timescale, duration) = match b.first() {
                    Some(1) => (be64(&b, 4), be32(&b, 20), be64(&b, 24)),
                    _ => (be32(&b, 4).map(u64::from), be32(&b, 12), be32(&b, 16).map(u64::from)),
                };
                if let (Some(ts), Some(d)) = (timescale.filter(|&t| t > 0), duration) {
                    meta.duration = Some(d as f64 / ts as f64).filter(|d| d.is_finite() && *d > 0.0);
                }
                meta.taken_at = created.and_then(utc_to_local);
            }
            b"trak" => {
                walk(r, start, end, &mut |r, kind, start, end| {
                    if &kind == b"tkhd" && meta.dims.is_none() {
                        meta.dims = tkhd_dims(&read_body(r, start, end)?);
                    }
                    Ok(())
                })?;
            }
            b"meta" => {
                let keys = quicktime_keys(r, start, end)?;
                let value = |name: &str| keys.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
                *quicktime_date = value("com.apple.quicktime.creationdate").and_then(|s| crate::metadata::parse_exif_datetime(&s));
                meta.make = value("com.apple.quicktime.make");
                meta.model = value("com.apple.quicktime.model");
            }
            _ => {}
        }
        Ok(())
    })
}

/// Время создания из mvhd (UTC, секунды с 1904 года) → местное; 0 — не задано.
fn utc_to_local(t: u64) -> Option<NaiveDateTime> {
    let unix = t.checked_sub(MP4_EPOCH).filter(|&u| u > 0)?;
    let d = chrono::DateTime::from_timestamp(i64::try_from(unix).ok()?, 0)?;
    Some(d.with_timezone(&chrono::Local).naive_local())
}

/// Размеры кадра дорожки (у звука нули) с учётом поворота из матрицы.
fn tkhd_dims(b: &[u8]) -> Option<(u32, u32)> {
    // После версии и флагов: v1 — 8+8+4+4+8 байт времён и номера, v0 — 4+4+4+4+4.
    let base = if b.first() == Some(&1) { 4 + 32 } else { 4 + 20 };
    // Дальше: 8 зарезервировано, слой, группа, громкость, 2 зарезервировано — 16 байт; матрица 9×4.
    let matrix = base + 16;
    let (w, h) = (be32(b, matrix + 36)? >> 16, be32(b, matrix + 40)? >> 16);
    if w == 0 || h == 0 {
        return None;
    }
    // Матрица поворота на 90° или 270°: a = 0, b = ±1 (16.16).
    let a = be32(b, matrix)? as i32;
    let rotated = a == 0 && (be32(b, matrix + 4)? as i32).unsigned_abs() == 1 << 16;
    Some(if rotated { (h, w) } else { (w, h) })
}

/// Ключи QuickTime (`meta` → `keys` + `ilst`): имя → текстовое значение.
fn quicktime_keys(r: &mut Reader, start: u64, end: u64) -> std::io::Result<Vec<(String, String)>> {
    // В QuickTime `meta` — обычный бокс, в ISO — с версией и флагами: тогда первые 4 байта нули.
    r.seek(SeekFrom::Start(start))?;
    let mut first = [0u8; 4];
    r.read_exact(&mut first)?;
    let start = if first == [0; 4] { start + 4 } else { start };
    let (mut names, mut values) = (Vec::new(), Vec::new());
    walk(r, start, end, &mut |r, kind, start, end| {
        match &kind {
            b"keys" => {
                let b = read_body(r, start, end)?;
                let mut at = 8;
                while let Some(size) = be32(&b, at).map(|s| s as usize).filter(|&s| s >= 8) {
                    let Some(name) = b.get(at + 8..at + size) else { break };
                    names.push(String::from_utf8_lossy(name).into_owned());
                    at += size;
                }
            }
            b"ilst" => {
                walk(r, start, end, &mut |r, kind, start, end| {
                    let index = u32::from_be_bytes(kind) as usize;
                    walk(r, start, end, &mut |r, kind, start, end| {
                        let b = read_body(r, start, end)?;
                        // data: тип (1 — UTF-8), язык, значение.
                        if &kind == b"data" && be32(&b, 0) == Some(1) {
                            let text = String::from_utf8_lossy(b.get(8..).unwrap_or_default()).trim().to_string();
                            values.push((index, text));
                        }
                        Ok(())
                    })
                })?;
            }
            _ => {}
        }
        Ok(())
    })?;
    // Номер в ilst — с единицы.
    Ok(values
        .into_iter()
        .filter_map(|(i, v)| Some((names.get(i.checked_sub(1)?)?.clone(), v)))
        .filter(|(_, v)| !v.is_empty())
        .collect())
}

/// Длительность для подписи: `0:07`, `12:30`, `1:02:03`.
pub fn format_duration(secs: f64) -> String {
    let s = secs.round().max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Системный декодер кадра: `(файл, минимальная короткая сторона)` → уже повёрнутый кадр.
pub type FrameDecoder = fn(&Path, u32) -> Result<RgbImage>;

static FRAME_DECODER: OnceLock<FrameDecoder> = OnceLock::new();

/// Подключить системный декодер видео (на macOS — AVFoundation) вместо `ffmpeg`.
pub fn set_frame_decoder(decoder: FrameDecoder) -> bool {
    FRAME_DECODER.set(decoder).is_ok()
}

/// Кадр в начале видео (на 1-й секунде, у коротких — первый) с короткой стороной не меньше `min_side`.
pub fn frame(path: &Path, min_side: u32) -> Result<RgbImage> {
    if let Some(decoder) = FRAME_DECODER.get() {
        return decoder(path, min_side);
    }
    ffmpeg_frame(path, min_side)
}

/// Можно ли получить кадр: подключён системный декодер или установлен `ffmpeg` (проверяется один раз).
pub fn frames_available() -> bool {
    static FFMPEG: OnceLock<bool> = OnceLock::new();
    FRAME_DECODER.get().is_some()
        || *FFMPEG.get_or_init(|| {
            let mut cmd = Command::new("ffmpeg");
            cmd.arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000);
            }
            cmd.status().is_ok_and(|s| s.success())
        })
}

/// `ffmpeg` не найден — сообщить об этом один раз, а не о каждом файле.
static NO_FFMPEG: AtomicBool = AtomicBool::new(false);

fn ffmpeg_frame(path: &Path, min_side: u32) -> Result<RgbImage> {
    if NO_FFMPEG.load(Ordering::Relaxed) {
        bail!("ffmpeg не найден");
    }
    for at in ["1", "0"] {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-nostdin", "-v", "error", "-ss", at, "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-vf"])
            .arg(format!("scale={min_side}:{min_side}:force_original_aspect_ratio=increase"))
            .args(["-f", "image2pipe", "-c:v", "png", "-"]);
        #[cfg(windows)]
        {
            // Без консольного окна на каждый файл (CREATE_NO_WINDOW).
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = match cmd.output() {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !NO_FFMPEG.swap(true, Ordering::Relaxed) {
                    eprintln!("ffmpeg не найден: у видео не будет миниатюр");
                }
                bail!("ffmpeg не найден");
            }
            Err(e) => return Err(e).context("запуск ffmpeg"),
        };
        // У видео короче секунды кадра на 1-й секунде нет — тогда первый кадр.
        if out.status.success() && !out.stdout.is_empty() {
            let img = image::load_from_memory_with_format(&out.stdout, image::ImageFormat::Png)?;
            return Ok(img.to_rgb8());
        }
        if at == "0" {
            bail!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Минимальный MP4: ftyp + moov (mvhd, trak/tkhd с поворотом на 90°, meta с ключами QuickTime).
    pub(crate) fn sample_mp4(quicktime_date: Option<&str>) -> Vec<u8> {
        let mut mvhd = vec![0u8; 100];
        // 2023-07-14 09:30:45 UTC
        let created = (1_689_327_045u64 + MP4_EPOCH) as u32;
        mvhd[4..8].copy_from_slice(&created.to_be_bytes());
        mvhd[12..16].copy_from_slice(&600u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&(600u32 * 65).to_be_bytes());
        let mut tkhd = vec![0u8; 84];
        let m = 4 + 20 + 16;
        tkhd[m + 4..m + 8].copy_from_slice(&(1u32 << 16).to_be_bytes());
        tkhd[m + 12..m + 16].copy_from_slice(&(-(1i32 << 16)).to_be_bytes());
        tkhd[m + 36..m + 40].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd[m + 40..m + 44].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        let mut moov = [bx(b"mvhd", &mvhd), bx(b"trak", &bx(b"tkhd", &tkhd))].concat();
        if let Some(date) = quicktime_date {
            let key = |name: &str| [&((name.len() + 8) as u32).to_be_bytes()[..], b"mdta", name.as_bytes()].concat();
            let keys = [&[0u8; 4][..], &2u32.to_be_bytes(), &key("com.apple.quicktime.model"), &key("com.apple.quicktime.creationdate")].concat();
            let data = |v: &str| bx(b"data", &[&1u32.to_be_bytes()[..], &[0; 4], v.as_bytes()].concat());
            let ilst = [bx(&1u32.to_be_bytes(), &data("iPhone 15")), bx(&2u32.to_be_bytes(), &data(date))].concat();
            let meta = [bx(b"hdlr", &[0; 24]), bx(b"keys", &keys), bx(b"ilst", &ilst)].concat();
            moov.extend(bx(b"meta", &meta));
        }
        [bx(b"ftyp", b"isom\0\0\0\0isom"), bx(b"mdat", &[0; 32]), bx(b"moov", &moov)].concat()
    }

    fn bx(kind: &[u8], body: &[u8]) -> Vec<u8> {
        [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat()
    }

    fn read_bytes(name: &str, bytes: &[u8]) -> VideoMeta {
        let path = std::env::temp_dir().join(format!("gpl-video-{}-{name}.mp4", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let meta = read(&path);
        std::fs::remove_file(&path).unwrap();
        meta
    }

    #[test]
    fn mp4_metadata() {
        let m = read_bytes("plain", &sample_mp4(None));
        assert_eq!(m.duration, Some(65.0));
        assert_eq!(m.dims, Some((1080, 1920)));
        let utc = chrono::DateTime::from_timestamp(1_689_327_045, 0).unwrap();
        assert_eq!(m.taken_at, Some(utc.with_timezone(&chrono::Local).naive_local()));
        assert_eq!(m.model, None);

        let m = read_bytes("quicktime", &sample_mp4(Some("2023-07-14T12:30:45+0300")));
        assert_eq!(m.taken_at.unwrap().to_string(), "2023-07-14 12:30:45");
        assert_eq!(m.model.as_deref(), Some("iPhone 15"));

        assert_eq!(read_bytes("junk", b"not a video at all"), VideoMeta::default());
    }

    /// Настоящее видео от ffmpeg (без него тест ничего не проверяет): метаданные и кадр.
    #[test]
    fn ffmpeg_video() {
        let path = std::env::temp_dir().join(format!("gpl-video-{}-real.mp4", std::process::id()));
        let made = Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=10:duration=2"])
            .args(["-metadata", "creation_time=2023-07-14T09:30:45Z", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            eprintln!("ffmpeg не найден, тест пропущен");
            return;
        }
        let m = read(&path);
        assert_eq!(m.dims, Some((640, 360)));
        assert!((m.duration.unwrap() - 2.0).abs() < 0.2);
        let utc = chrono::DateTime::from_timestamp(1_689_327_045, 0).unwrap();
        assert_eq!(m.taken_at, Some(utc.with_timezone(&chrono::Local).naive_local()));
        // Панель «Информация»: поля из контейнера вместо EXIF.
        let d = crate::metadata::read_details(&path).unwrap();
        assert_eq!((d.width, d.height, d.taken_at), (Some(640), Some(360), m.taken_at));
        assert!(d.duration.is_some() && d.all.is_empty());
        let f = ffmpeg_frame(&path, 320).unwrap();
        assert_eq!((f.width(), f.height()), (569, 320));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn duration_and_extensions() {
        assert_eq!(format_duration(7.4), "0:07");
        assert_eq!(format_duration(750.0), "12:30");
        assert_eq!(format_duration(3723.0), "1:02:03");
        assert!(is_video(Path::new("/a/IMG_0001.MOV")));
        assert!(!is_video(Path::new("/a/IMG_0001.jpg")));
    }
}
