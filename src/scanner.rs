//! Инкрементальное сканирование папки библиотеки.
use crate::{
    db::{Db, NewPhoto},
    metadata, thumbs,
};
use anyhow::{bail, Result};
use rayon::prelude::*;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use walkdir::{DirEntry, WalkDir};

pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "heic", "heif", "png", "webp", "tif", "tiff", "dng"];
const CHUNK: usize = 64;

#[derive(Debug, Clone, Copy, Default)]
pub struct ScanStats {
    pub found: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn is_hidden(e: &DirEntry) -> bool {
    e.depth() > 0 && e.file_name().to_str().is_some_and(|s| s.starts_with('.'))
}

struct Todo {
    path: PathBuf,
    size: i64,
    mtime: i64,
}

/// `progress(done, total)` вызывается из рабочего потока.
pub fn scan(
    db_path: &Path,
    root: &Path,
    thumbs_dir: &Path,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<ScanStats> {
    // Если папка недоступна (например, не подключён диск), не трогаем БД —
    // иначе все записи были бы удалены как «пропавшие».
    if !root.is_dir() {
        bail!("папка {} недоступна", root.display());
    }
    std::fs::create_dir_all(thumbs_dir)?;
    let mut db = Db::open(db_path)?;
    let known = db.known_files()?;

    let mut stats = ScanStats::default();
    let mut found = HashSet::new();
    let mut todo = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| !is_hidden(e))
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() || !is_supported(entry.path()) {
            continue;
        }
        let Some(path_str) = entry.path().to_str() else { continue };
        let Ok(md) = entry.metadata() else { continue };
        let size = md.len() as i64;
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        found.insert(path_str.to_owned());
        match known.get(path_str) {
            Some(&(s, m)) if s == size && m == mtime => continue,
            Some(_) => stats.updated += 1,
            None => stats.added += 1,
        }
        todo.push(Todo { path: entry.into_path(), size, mtime });
    }
    stats.found = found.len();

    let removed: Vec<String> = known.into_keys().filter(|k| !found.contains(k)).collect();
    stats.removed = removed.len();
    for thumb in db.delete(&removed)? {
        let _ = std::fs::remove_file(thumb);
    }

    let total = todo.len();
    progress(0, total);
    let mut done = 0;
    for chunk in todo.chunks(CHUNK) {
        let records: Vec<NewPhoto> = chunk.par_iter().map(|t| process(t, thumbs_dir)).collect();
        db.upsert(&records)?;
        done += chunk.len();
        progress(done, total);
    }
    Ok(stats)
}

fn process(t: &Todo, thumbs_dir: &Path) -> NewPhoto {
    let meta = metadata::read(&t.path);
    let taken_at = meta.taken_at.unwrap_or_else(|| {
        chrono::DateTime::from_timestamp(t.mtime, 0)
            .map(|d| d.with_timezone(&chrono::Local).naive_local())
            .unwrap_or_default()
    });
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    #[test]
    fn supported() {
        assert!(is_supported(Path::new("/a/IMG_0001.HEIC")));
        assert!(is_supported(Path::new("/a/20230714_123045.jpg")));
        assert!(!is_supported(Path::new("/a/video.mp4")));
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

        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.found, s.added, s.updated, s.removed), (2, 2, 0, 0));
        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.added, s.updated, s.removed), (0, 0, 0));

        std::fs::remove_file(lib.join("a.png")).unwrap();
        let s = scan(&db, &lib, &th, &noop).unwrap();
        assert_eq!((s.found, s.removed), (1, 1));
        assert_eq!(std::fs::read_dir(&th).unwrap().count(), 1);

        assert!(scan(&db, &base.join("missing"), &th, &noop).is_err());
        std::fs::remove_dir_all(&base).ok();
    }
}
