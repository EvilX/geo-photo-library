//! Пути приложения: XDG на Linux (через GLib), `%LOCALAPPDATA%` на Windows, `~/Library` на macOS.
use std::path::PathBuf;

const DIR: &str = "geo-photo-library";

#[cfg(all(feature = "gtk-ui", not(windows)))]
fn dirs() -> (PathBuf, PathBuf, PathBuf) {
    use gtk::glib;
    (glib::user_config_dir(), glib::user_data_dir(), glib::user_cache_dir())
}

/// Windows: всё, включая миниатюры, — в `%LOCALAPPDATA%\geo-photo-library` (GLib отдаёт его
/// как каталоги настроек и данных; его каталог кэша на Windows — `INetCache` браузера).
/// Переменные `XDG_*_HOME` GLib учитывает и здесь — для тестов.
#[cfg(all(feature = "gtk-ui", windows))]
fn dirs() -> (PathBuf, PathBuf, PathBuf) {
    use gtk::glib;
    let cache = if std::env::var_os("XDG_CACHE_HOME").is_some() { glib::user_cache_dir() } else { glib::user_data_dir() };
    (glib::user_config_dir(), glib::user_data_dir(), cache)
}

/// Без GLib (нативный интерфейс macOS): настройки и БД — в `Application Support`,
/// миниатюры — в `Caches`. `GEO_PHOTO_LIBRARY_HOME` подменяет оба каталога (для тестов,
/// чтобы не трогать данные пользователя).
#[cfg(not(feature = "gtk-ui"))]
fn dirs() -> (PathBuf, PathBuf, PathBuf) {
    if let Some(dir) = std::env::var_os("GEO_PHOTO_LIBRARY_HOME").map(PathBuf::from) {
        return (dir.join("config"), dir.join("data"), dir.join("cache"));
    }
    let library = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library");
    let support = library.join("Application Support");
    (support.clone(), support, library.join("Caches"))
}

pub fn config_file() -> PathBuf {
    dirs().0.join(DIR).join("config.toml")
}

pub fn db_path() -> PathBuf {
    dirs().1.join(DIR).join("library.db")
}

pub fn thumbs_dir() -> PathBuf {
    dirs().2.join(DIR).join("thumbs")
}
