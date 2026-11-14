mod config;
mod db;
mod dng;
mod geo;
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
        eprintln!("Укажите папку: geo-photo-library --scan <папка>");
        return glib::ExitCode::FAILURE;
    };
    let progress = |done: usize, total: usize| {
        if done == total || done.is_multiple_of(256) {
            eprintln!("{done}/{total}");
        }
    };
    match scanner::scan(&paths::db_path(), &root, &paths::thumbs_dir(), &progress) {
        Ok(s) => {
            println!(
                "Найдено: {}, добавлено: {}, обновлено: {}, удалено: {}",
                s.found, s.added, s.updated, s.removed
            );
            glib::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Ошибка: {e:#}");
            glib::ExitCode::FAILURE
        }
    }
}
