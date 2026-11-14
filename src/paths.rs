//! XDG-пути приложения.
use gtk::glib;
use std::path::PathBuf;

const DIR: &str = "geo-photo-library";

pub fn config_file() -> PathBuf {
    glib::user_config_dir().join(DIR).join("config.toml")
}

pub fn db_path() -> PathBuf {
    glib::user_data_dir().join(DIR).join("library.db")
}

pub fn thumbs_dir() -> PathBuf {
    glib::user_cache_dir().join(DIR).join("thumbs")
}
