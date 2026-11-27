//! Инкрементальное сканирование папки библиотеки.
use crate::{
    db::{Db, NewPhoto},
    i18n, metadata, thumbs, video,
};
use anyhow::{bail, Result};
use rayon::prelude::*;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::UNIX_EPOCH,
};
use walkdir::{DirEntry, WalkDir};

pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "heic", "heif", "png", "webp", "tif", "tiff", "dng"];
const CHUNK: usize = 64;
/// Отметка в БД: при прошлом сканировании кадры видео можно было получить («1»).
const META_FRAMES: &str = "video_frames";

/// Сканирование отменено (см. флаг `cancel` в [`scan`]).
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(i18n::scan_cancelled())
    }
}

impl std::error::Error for Cancelled {}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScanStats {
    pub found: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// Фото или видео (`video::EXTENSIONS`).
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        || video::is_video(path)
}

fn is_hidden(e: &DirEntry) -> bool {
    e.depth() > 0 && e.file_name().to_str().is_some_and(|s| s.starts_with('.'))
}

struct Todo {
    path: PathBuf,
    size: i64,
    mtime: i64,
}

/// `progress(done, total)` вызывается из рабочего потока. Если выставить `cancel`,
/// сканирование остановится между пачками файлов с ошибкой [`Cancelled`]; уже
/// обработанные файлы остаются в БД.
pub fn scan(
    db_path: &Path,
    root: &Path,
    thumbs_dir: &Path,
    progress: &(dyn Fn(usize, usize) + Sync),
    cancel: &AtomicBool,
) -> Result<ScanStats> {
    let check = || if cancel.load(Ordering::Relaxed) { Err(Cancelled) } else { Ok(()) };
    // Если папка недоступна (например, не подключён диск), не трогаем БД —
    // иначе все записи были бы удалены как «пропавшие».
    if !root.is_dir() {
        bail!(i18n::folder_unavailable(root));
    }
    // Абсолютный канонический путь: иначе `--scan ./lib` и GUI с полным путём
    // видели бы одну библиотеку как разные файлы и пересканировали её целиком.
    let root = &canonical(root)?;
    std::fs::create_dir_all(thumbs_dir)?;
    let mut db = Db::open(db_path)?;
    let known = db.known_files()?;
    // Пустая папка, из которой в БД есть файлы, — скорее всего точка монтирования без диска
    // (сетевой или внешний том не подключён): каталог существует, но записи удалять нельзя.
    if std::fs::read_dir(root)?.next().is_none() && known.keys().any(|k| Path::new(k).starts_with(root)) {
        bail!(i18n::folder_unavailable(root));
    }
    // Видео без миниатюры повторяются один раз — когда кадры стало можно получить (поставили
    // ffmpeg). Видео, из которого кадр не выходит и с декодером, каждый раз заново не обрабатывается.
    let frames = video::frames_available();
    let frames_before = db.meta(META_FRAMES)?.as_deref() == Some("1");
    let mut retry = if frames && !frames_before { db.videos_without_thumbs()? } else { HashSet::new() };
    // Файлы миниатюр, которых больше нет (кэш очистили), — создать заново.
    let thumbs: HashSet<std::ffi::OsString> =
        std::fs::read_dir(thumbs_dir)?.filter_map(|e| e.ok().map(|e| e.file_name())).collect();
    for (path, thumb) in db.thumb_paths()? {
        if !Path::new(&thumb).file_name().is_some_and(|n| thumbs.contains(n)) {
            retry.insert(path);
        }
    }

    let mut stats = ScanStats::default();
    let mut found = HashSet::new();
    let mut todo = Vec::new();
    // Каталоги, которые не удалось прочитать (нет прав, сбой сети): их файлы не найдены,
    // но не пропали — записи о них остаются.
    let mut unreadable: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(root).follow_links(true).into_iter().filter_entry(|e| !is_hidden(e)) {
        check()?;
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("обход папки: {e}");
                unreadable.push(e.path().unwrap_or(root).to_path_buf());
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_supported(entry.path()) {
            continue;
        }
        let Some(path_str) = entry.path().to_str() else { continue };
        let Ok(md) = entry.metadata() else {
            // Файл есть, но сведения о нём не прочитались — запись не трогаем.
            found.insert(path_str.to_owned());
            continue;
        };
        let size = md.len() as i64;
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        found.insert(path_str.to_owned());
        match known.get(path_str) {
            Some(&(s, m)) if s == size && m == mtime && !retry.contains(path_str) => continue,
            // Изменённый файл или повтор (нет миниатюры) — интерфейс перечитает его миниатюру.
            Some(_) => stats.updated += 1,
            None => stats.added += 1,
        }
        todo.push(Todo { path: entry.into_path(), size, mtime });
    }
    stats.found = found.len();

    let removed: Vec<String> = known
        .into_keys()
        .filter(|k| !found.contains(k) && !unreadable.iter().any(|dir| Path::new(k).starts_with(dir)))
        .collect();
    stats.removed = removed.len();
    for thumb in db.delete(&removed)? {
        let _ = std::fs::remove_file(thumb);
    }

    let total = todo.len();
    progress(0, total);
    let mut done = 0;
    for chunk in todo.chunks(CHUNK) {
        check()?;
        let records: Vec<NewPhoto> = chunk.par_iter().map(|t| process(t, thumbs_dir)).collect();
        db.upsert(&records)?;
        done += chunk.len();
        progress(done, total);
    }
    if frames != frames_before {
        db.set_meta(META_FRAMES, if frames { "1" } else { "0" })?;
    }
    Ok(stats)
}

/// Заново прочитать один файл (после записи EXIF): метаданные и миниатюра — как при сканировании.
pub fn process_file(path: &Path, size: i64, mtime: i64, thumbs_dir: &Path) -> NewPhoto {
    process(&Todo { path: path.to_path_buf(), size, mtime }, thumbs_dir)
}

/// Время изменения файла как местное — дата съёмки, если в файле её нет.
fn mtime_local(mtime: i64) -> chrono::NaiveDateTime {
    chrono::DateTime::from_timestamp(mtime, 0)
        .map(|d| d.with_timezone(&chrono::Local).naive_local())
        .unwrap_or_default()
}

fn process(t: &Todo, thumbs_dir: &Path) -> NewPhoto {
    if video::is_video(&t.path) {
        return process_video(t, thumbs_dir);
    }
    let meta = metadata::read(&t.path);
    let taken_at = meta.taken_at.unwrap_or_else(|| mtime_local(t.mtime));
    let out = thumbs::thumb_path(thumbs_dir, &t.path);
    let (thumb_path, dims) = match thumbs::generate(&t.path, meta.orientation, &out) {
        Ok(d) => (out.to_str().map(str::to_owned), Some(d)),
        Err(e) => {
            eprintln!("миниатюра {}: {e:#}", t.path.display());
            (None, None)
        }
    };
    NewPhoto {
        path: t.path.to_string_lossy().into_owned(),
        size: t.size,
        mtime: t.mtime,
        taken_at,
        lat: meta.lat,
        lon: meta.lon,
        width: dims.map(|d| d.0),
        height: dims.map(|d| d.1),
        orientation: meta.orientation,
        make: meta.make,
        model: meta.model,
        thumb_path,
        video: false,
        duration: None,
    }
}

/// Видео: дата, длительность и размеры из контейнера, миниатюра — кадр; координат нет.
fn process_video(t: &Todo, thumbs_dir: &Path) -> NewPhoto {
    let meta = video::read(&t.path);
    let out = thumbs::thumb_path(thumbs_dir, &t.path);
    let thumb_path = match video::frame(&t.path, thumbs::THUMB_SIZE).and_then(|f| thumbs::save(f.into(), &out)) {
        Ok(()) => out.to_str().map(str::to_owned),
        Err(e) => {
            eprintln!("миниатюра {}: {e:#}", t.path.display());
            None
        }
    };
    NewPhoto {
        path: t.path.to_string_lossy().into_owned(),
        size: t.size,
        mtime: t.mtime,
        taken_at: meta.taken_at.unwrap_or_else(|| mtime_local(t.mtime)),
        lat: None,
        lon: None,
        width: meta.dims.map(|d| d.0),
        height: meta.dims.map(|d| d.1),
        orientation: 1,
        make: meta.make,
        model: meta.model,
        thumb_path,
        video: true,
        duration: meta.duration,
    }
}

/// `canonicalize` без префикса `\\?\` на Windows (`\\?\C:\Photos` → `C:\Photos`,
/// `\\?\UNC\srv\share` → `\\srv\share`): с ним пути видны пользователю в панели
/// «Информация» и в буфере обмена, и не все программы открывают такие файлы.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let path = path.canonicalize()?;
    Ok(strip_verbatim(&path.to_string_lossy()).map(PathBuf::from).unwrap_or(path))
}

fn strip_verbatim(path: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{rest}"));
    }
    let rest = path.strip_prefix(r"\\?\")?;
    // Только обычный путь с буквой диска: «X:\…».
    let b = rest.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\').then(|| rest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    #[test]
    fn verbatim_paths() {
        assert_eq!(strip_verbatim(r"\\?\C:\Photos\a.jpg").as_deref(), Some(r"C:\Photos\a.jpg"));
        assert_eq!(strip_verbatim(r"\\?\UNC\nas\photos").as_deref(), Some(r"\\nas\photos"));
        assert_eq!(strip_verbatim(r"\\?\Volume{1234}\x"), None);
        assert_eq!(strip_verbatim("/home/user/Photos"), None);
    }

    #[test]
    fn supported() {
        assert!(is_supported(Path::new("/a/IMG_0001.HEIC")));
        assert!(is_supported(Path::new("/a/20230714_123045.jpg")));
        assert!(is_supported(Path::new("/a/video.MOV")));
        assert!(!is_supported(Path::new("/a/notes.txt")));
        assert!(!is_supported(Path::new("/a/noext")));
    }

    #[test]
    fn incremental_scan() {
        let base = std::env::temp_dir().join(format!("geo-photo-library-scan-{}", std::process::id()));
        let lib = base.join("lib");
        std::fs::create_dir_all(lib.join("sub")).unwrap();
        std::fs::create_dir_all(lib.join(".hidden")).unwrap();
        for p in ["a.png", "sub/b.png", ".hidden/c.png"] {
            RgbImage::new(10, 10).save(lib.join(p)).unwrap();
        }
        std::fs::write(lib.join("notes.txt"), "x").unwrap();
        let db = base.join("db.sqlite");
        let th = base.join("thumbs");
        let noop = |_: usize, _: usize| {};
        let go = AtomicBool::new(false);
        let scan = |db: &Path, lib: &Path, th: &Path, p: &(dyn Fn(usize, usize) + Sync)| super::scan(db, lib, th, p, &go);

        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.found, s.added, s.updated, s.removed), (2, 2, 0, 0));
        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));

        std::fs::remove_file(lib.join("a.png")).unwrap();
        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.found, s.removed), (1, 1));
        assert_eq!(std::fs::read_dir(&th).unwrap().count(), 1);

        assert!(scan(&db, &base.join("missing"), &th, &noop).is_err());

        // Пропавшая миниатюра (кэш очистили) создаётся заново, хотя файл не менялся.
        for f in std::fs::read_dir(&th).unwrap() {
            std::fs::remove_file(f.unwrap().path()).unwrap();
        }
        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.added, s.updated, s.removed), (0, 1, 0));
        assert_eq!(std::fs::read_dir(&th).unwrap().count(), 1);

        // Папка опустела целиком (точка монтирования без диска) — записи остаются.
        let away = base.join("away");
        std::fs::rename(&lib, &away).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        assert!(scan(&db, &lib, &th, &noop).is_err());
        assert_eq!(Db::open(&db).unwrap().count().unwrap(), 1);
        std::fs::remove_dir(&lib).unwrap();
        std::fs::rename(&away, &lib).unwrap();
        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.found, s.added, s.removed), (1, 0, 0));

        let stop = AtomicBool::new(true);
        let err = super::scan(&db, &lib, &th, &noop, &stop).unwrap_err();
        assert!(err.is::<Cancelled>());
        std::fs::remove_dir_all(&base).ok();
    }
}
