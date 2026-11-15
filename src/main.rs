mod config;
mod db;
mod dng;
mod geo;
mod geocode;
mod i18n;
mod location;
mod metadata;
mod paths;
mod scanner;
mod thumbs;
mod trace;
mod ui;

use gtk::glib;
use gtk::prelude::*;
use std::path::PathBuf;

const APP_ID: &str = "io.github.EvilX.GeoPhotoLibrary";
const APP_NAME: &str = "Geo photo library";

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--scan") {
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
