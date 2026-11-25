// На Windows — без консольного окна; `--scan` подключается к консоли, из которой запущен.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod geocode;
mod location;
mod ui;

// Ядро — в библиотеке (src/lib.rs); здесь оно видно как `crate::db` и т. д.
use geo_photo_library::{config, db, geo, i18n, metadata, paths, scanner, trace};

use gtk::glib;
use gtk::prelude::*;
use std::path::PathBuf;

const APP_ID: &str = "io.github.EvilX.GeoPhotoLibrary";
const APP_NAME: &str = "GeoPhotoLibrary";

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--scan") {
        #[cfg(windows)]
        geo_photo_library::win::attach_parent_console();
        return cli_scan(args.get(2).map(PathBuf::from));
    }

    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(ui::window::build);
    app.run()
}

/// Сканирование без GUI: `geo-photo-library --scan [папка]` (по умолчанию — папка из настроек).
fn cli_scan(dir: Option<PathBuf>) -> glib::ExitCode {
    let Some(root) = dir.or_else(|| config::Config::load().library_path) else {
        eprintln!("{}", i18n::cli_usage());
        return glib::ExitCode::FAILURE;
    };
    let progress = |done: usize, total: usize| {
        if done == total || done.is_multiple_of(256) {
            eprintln!("{done}/{total}");
        }
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    match scanner::scan(&paths::db_path(), &root, &paths::thumbs_dir(), &progress, &cancel) {
        Ok(s) => {
            println!("{}", i18n::cli_summary(s.found, s.added, s.updated, s.removed));
            glib::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}: {e:#}", i18n::error());
            glib::ExitCode::FAILURE
        }
    }
}
