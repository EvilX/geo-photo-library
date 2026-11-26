//! Запись координат GPS в EXIF файла (little_exif; kamadak-exif умеет только читать).
//!
//! Пишем только туда, где запись проверена на снимках телефонов и не трогает остальное:
//! JPEG, HEIC/HEIF и WebP. В PNG little_exif кладёт EXIF в текстовый блок, который не
//! читает сканер, а в DNG/TIFF переносит данные внутри файла — оригиналы RAW не трогаем.
use crate::{i18n, metadata};
use anyhow::{bail, Context, Result};
use little_exif::{exif_tag::ExifTag, filetype::FileExtension, metadata::Metadata, rational::uR64};
use std::{
    collections::HashSet,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Можно ли записать координаты в этот файл (по расширению).
pub fn writable(path: &Path) -> bool {
    file_type(path).is_some()
}

fn file_type(path: &Path) -> Option<FileExtension> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some(FileExtension::JPEG),
        "heic" | "heif" => Some(FileExtension::HEIF),
        "webp" => Some(FileExtension::WEBP),
        _ => None,
    }
}

/// Размер и время изменения файла — как их хранит сканер.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: i64,
    pub mtime: i64,
}

impl Stamp {
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let md = std::fs::metadata(path)?;
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        Ok(Self { size: md.len() as i64, mtime })
    }
}

/// Записать координаты в EXIF файла. Новый файл собирается в памяти и проверяется
/// (координаты читаются обратно, прежние теги на месте), затем атомарно заменяет старый.
/// Возвращает новые размер и время изменения — для записи в БД без пересканирования.
pub fn write_coords(path: &Path, lat: f64, lon: f64) -> Result<Stamp> {
    if !metadata::valid_coords(lat, lon) {
        bail!(i18n::invalid_coordinates());
    }
    let Some(kind) = file_type(path) else {
        bail!(i18n::coords_format_unsupported());
    };
    let original = std::fs::read(path)?;
    let data = with_coords(&original, kind, lat, lon)?;
    replace_file(path, &data)?;
    Ok(Stamp::of(path)?)
}

/// Содержимое файла с новыми координатами (проверенное).
fn with_coords(original: &[u8], kind: FileExtension, lat: f64, lon: f64) -> Result<Vec<u8>> {
    let before = tags(original);
    // Без EXIF — новый блок. Если EXIF есть, а little_exif его не разобрал, файл не трогаем:
    // иначе все прежние теги пропали бы.
    let mut md = match Metadata::new_from_vec(&original.to_vec(), kind) {
        Ok(md) => md,
        Err(_) if before.is_none() => Metadata::new(),
        Err(e) => return Err(e).context("EXIF"),
    };
    md.set_tag(ExifTag::GPSVersionID(vec![2, 3, 0, 0]));
    md.set_tag(ExifTag::GPSLatitudeRef(if lat < 0.0 { "S" } else { "N" }.into()));
    md.set_tag(ExifTag::GPSLatitude(dms(lat)));
    md.set_tag(ExifTag::GPSLongitudeRef(if lon < 0.0 { "W" } else { "E" }.into()));
    md.set_tag(ExifTag::GPSLongitude(dms(lon)));
    let mut data = original.to_vec();
    md.write_to_vec(&mut data, kind).context("EXIF")?;

    // Проверка: координаты читаются так же, как их прочтёт сканер, и ни один прежний тег
    // (кроме GPS) не потерян.
    let meta = metadata::read_from(&mut Cursor::new(&data));
    let ok = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-6);
    if !ok(meta.lat, lat) || !ok(meta.lon, lon) {
        bail!("координаты не записались");
    }
    if let Some(before) = before {
        let after = tags(&data).unwrap_or_default();
        if let Some(lost) = before.iter().find(|t| !t.1.starts_with("GPS") && !after.contains(t)) {
            bail!("после записи пропал тег EXIF {}", lost.1);
        }
    }
    Ok(data)
}

/// Теги EXIF файла: (IFD, имя); `None` — EXIF нет.
fn tags(data: &[u8]) -> Option<HashSet<(u16, String)>> {
    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(data)).ok()?;
    Some(exif.fields().map(|f| (f.ifd_num.index(), f.tag.to_string())).collect())
}

/// Десятичные градусы → градусы, минуты, секунды (до десятитысячной секунды, ~3 мм).
fn dms(v: f64) -> Vec<uR64> {
    let v = v.abs();
    let deg = v.trunc();
    let min_f = (v - deg) * 60.0;
    let min = min_f.trunc();
    let sec = ((min_f - min) * 60.0 * 10_000.0).round();
    // Округление секунд до 60 не переносим в минуты — значение остаётся верным.
    vec![
        uR64 { nominator: deg as u32, denominator: 1 },
        uR64 { nominator: min as u32, denominator: 1 },
        uR64 { nominator: sec as u32, denominator: 10_000 },
    ]
}

/// Записать во временный файл рядом и переименовать: при сбое оригинал остаётся целым.
fn replace_file(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = temp_path(path);
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::set_permissions(&tmp, std::fs::metadata(path)?.permissions())?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(result?)
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Скрытый файл (с точкой) сканер пропускает.
    path.with_file_name(format!(".{name}.gpl-tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::jpeg::JpegEncoder, RgbImage};

    fn jpeg() -> Vec<u8> {
        let mut out = Vec::new();
        JpegEncoder::new(&mut out).encode_image(&RgbImage::new(16, 16)).unwrap();
        out
    }

    #[test]
    fn dms_roundtrip() {
        for v in [55.751244, -33.8568, 151.2153, -0.000123, 179.999999] {
            let d: Vec<f64> = dms(v).iter().map(|r| r.nominator as f64 / r.denominator as f64).collect();
            let back = metadata::dms_to_decimal(&d, v < 0.0).unwrap();
            assert!((back - v).abs() < 1e-6, "{v} → {back}");
        }
    }

    #[test]
    fn write_jpeg_without_and_with_exif() {
        let dir = std::env::temp_dir().join(format!("gpl-geotag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.JPG");
        std::fs::write(&path, jpeg()).unwrap();
        assert_eq!(metadata::read(&path).lat, None);

        let stamp = write_coords(&path, -33.8568, 151.2153).unwrap();
        assert_eq!(stamp, Stamp::of(&path).unwrap());
        let m = metadata::read(&path);
        assert!((m.lat.unwrap() + 33.8568).abs() < 1e-6 && (m.lon.unwrap() - 151.2153).abs() < 1e-6);

        // Повторная запись меняет координаты, другие теги остаются.
        write_coords(&path, 55.75, -37.62).unwrap();
        let m = metadata::read(&path);
        assert!((m.lat.unwrap() - 55.75).abs() < 1e-6 && (m.lon.unwrap() + 37.62).abs() < 1e-6);
        assert!(!dir.join(".a.JPG.gpl-tmp").exists());
        // Картинка по-прежнему читается.
        image::open(&path).unwrap();

        assert!(write_coords(&path, 91.0, 0.0).is_err());
        let png = dir.join("b.png");
        RgbImage::new(4, 4).save(&png).unwrap();
        assert!(!writable(&png));
        assert!(write_coords(&png, 10.0, 10.0).is_err());
        assert!(writable(Path::new("/x/IMG_1.HEIC")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
