//! Ядро GeoPhotoLibrary: сканер, БД, миниатюры, EXIF, геометрия карты, локализация.
//!
//! От интерфейса не зависит: на Linux и Windows его использует приложение GTK (`src/main.rs`,
//! фича `gtk-ui`), на macOS — приложение на Swift через C-интерфейс (`ffi`, см. `macos/`).
pub mod config;
pub mod db;
pub mod dng;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod geo;
pub mod i18n;
pub mod metadata;
pub mod paths;
pub mod scanner;
pub mod thumbs;
pub mod trace;
pub mod update;
#[cfg(windows)]
pub mod win;
