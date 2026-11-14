//! Главное окно: две вкладки, настройки, фоновое сканирование.
use super::{dates_tab::DatesTab, map_tab::MapTab, settings, textures::TextureCache, CSS};
use crate::{config::Config, db::Db, paths, scanner, trace};
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Как часто подгружать в интерфейс уже обработанные фото во время сканирования.
const PARTIAL_RELOAD: Duration = Duration::from_secs(3);

enum ScanEvent {
    Progress(usize, usize),
    Finished(Result<scanner::ScanStats, String>),
    Cancelled,
}

struct App {
    window: gtk::ApplicationWindow,
    config: RefCell<Config>,
    db: Rc<Db>,
    cache: Rc<TextureCache>,
    dates: Rc<DatesTab>,
    map: Rc<MapTab>,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    rescan: gtk::Button,
    scanning: Cell<bool>,
    /// Флаг отмены текущего сканирования.
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    /// Запустить сканирование заново после окончания текущего (сменилась папка).
    rescan_pending: Cell<bool>,
    last_partial_reload: Cell<Instant>,
}

pub fn build(app: &gtk::Application) {
    if let Some(w) = app.active_window() {
        w.present();
        return;
    }
    load_css();
    gtk::Window::set_default_icon_name(crate::APP_ID);
    // При запуске через cargo иконка ещё не установлена в систему.
    #[cfg(debug_assertions)]
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display)
            .add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons"));
    }
    let started = std::time::Instant::now();
    let build_span = trace::span("window.build");

    let db = match Db::open(&paths::db_path()) {
        Ok(db) => Rc::new(db),
        Err(e) => {
            eprintln!("Не удалось открыть БД {}: {e:#}", paths::db_path().display());
            return;
        }
    };
    let cache = Rc::new(TextureCache::default());
    let dates = DatesTab::new(db.clone(), cache.clone());
    let map = MapTab::new(db.clone(), cache.clone());

    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.add_titled(&dates.root, Some("dates"), "По датам");
    stack.add_titled(&map.root, Some("map"), "Карта");
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));

    let settings_btn = gtk::Button::from_icon_name("emblem-system-symbolic");
    settings_btn.set_tooltip_text(Some("Настройки"));
    let rescan = gtk::Button::from_icon_name("view-refresh-symbolic");
    rescan.set_tooltip_text(Some("Пересканировать библиотеку"));
    let progress = gtk::ProgressBar::new();
    progress.set_valign(gtk::Align::Center);
    progress.set_size_request(120, -1);
    progress.set_visible(false);
    let status = gtk::Label::new(None);
    status.add_css_class("dim-label");

    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    header.pack_end(&settings_btn);
    header.pack_end(&rescan);
    header.pack_start(&progress);
    header.pack_start(&status);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Geo photo library")
        .default_width(1280)
        .default_height(820)
        .child(&stack)
        .build();
    window.set_titlebar(Some(&header));

    let state = Rc::new(App {
        window,
        config: RefCell::new(Config::load()),
        db,
        cache,
        dates,
        map,
        progress,
        status,
        rescan: rescan.clone(),
        scanning: Cell::new(false),
        cancel: RefCell::default(),
        rescan_pending: Cell::new(false),
        last_partial_reload: Cell::new(Instant::now()),
    });

    let s = state.clone();
    settings_btn.connect_clicked(move |_| open_settings(&s));
    let s = state.clone();
    rescan.connect_clicked(move |_| start_scan(&s));

    state.window.present();
    reload(&state, true);
    drop(build_span);
    let first = Cell::new(true);
    state.window.add_tick_callback(move |_, _| {
        if first.replace(false) && trace::enabled() {
            eprintln!("[trace] первый кадр через {:.1} мс", started.elapsed().as_secs_f64() * 1000.0);
        }
        glib::ControlFlow::Break
    });
    if state.config.borrow().library_path.is_some() {
        start_scan(&state);
    } else {
        open_settings(&state);
    }
    debug_screenshot(&state.window, &stack);
    debug_bench(&state, &stack);
    debug_switch(&state);
}

/// Отладка: `GEO_PHOTO_LIBRARY_DEBUG_SWITCH=<папка>` — через 3 с сменить папку
/// библиотеки, как из настроек (проверка смены папки во время сканирования).
fn debug_switch(state: &Rc<App>) {
    let Some(path) = std::env::var_os("GEO_PHOTO_LIBRARY_DEBUG_SWITCH") else { return };
    let state = state.clone();
    glib::timeout_add_seconds_local_once(3, move || {
        eprintln!("[debug] смена папки, сканирование идёт: {}", state.scanning.get());
        set_library(&state, path.into());
    });
}

/// Отладка: `GEO_PHOTO_LIBRARY_BENCH=1` — прогон зумов карты 2…18 вокруг первого фото и выход.
fn debug_bench(state: &Rc<App>, stack: &gtk::Stack) {
    if std::env::var_os("GEO_PHOTO_LIBRARY_BENCH").is_none() {
        return;
    }
    stack.set_visible_child_name("map");
    let steps: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0]));
    let state = state.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
        let next = steps.borrow_mut().pop();
        match (next, state.map.first_photo_coords()) {
            (Some(z), Some((lat, lon))) => {
                eprintln!("[bench] зум {z}");
                state.map.jump_to(lat, lon, z);
                glib::ControlFlow::Continue
            }
            _ => {
                if let Some(app) = state.window.application() {
                    app.quit();
                }
                glib::ControlFlow::Break
            }
        }
    });
}

/// Отладка: `GEO_PHOTO_LIBRARY_SCREENSHOT=out.png [GEO_PHOTO_LIBRARY_TAB=map]` — через несколько
/// секунд отрисовать окно в PNG и выйти.
fn debug_screenshot(window: &gtk::ApplicationWindow, stack: &gtk::Stack) {
    let Ok(path) = std::env::var("GEO_PHOTO_LIBRARY_SCREENSHOT") else { return };
    if let Ok(tab) = std::env::var("GEO_PHOTO_LIBRARY_TAB") {
        stack.set_visible_child_name(&tab);
    }
    let delay = std::env::var("GEO_PHOTO_LIBRARY_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
    let w = window.clone();
    glib::timeout_add_seconds_local_once(delay, move || {
        let paintable = gtk::WidgetPaintable::new(Some(&w));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w.width() as f64, w.height() as f64);
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), w.renderer()) {
            if let Err(e) = renderer.render_texture(&node, None).save_to_png(&path) {
                eprintln!("скриншот: {e}");
            }
        }
        if let Some(app) = w.application() {
            app.quit();
        }
    });
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Перечитать данные из БД. `force` — перерисовать всё (после сканирования, когда
/// миниатюры могли пересоздаться); иначе сохраняются раскрытые узлы, выбор и
/// не перестраивается то, что не изменилось.
fn reload(state: &Rc<App>, force: bool) {
    state.dates.reload(force);
    state.map.reload(force);
    if !state.scanning.get() {
        let n = state.db.count().unwrap_or(0);
        state.status.set_text(&format!("{n} фото"));
    }
}

fn open_settings(state: &Rc<App>) {
    let current = state.config.borrow().library_path.clone();
    let s = state.clone();
    settings::show(&state.window, current.as_deref(), move |path| set_library(&s, path));
}

/// Сменить папку библиотеки: сохранить настройки и пересканировать
/// (текущее сканирование, если идёт, отменяется).
fn set_library(state: &Rc<App>, path: std::path::PathBuf) {
    let changed = state.config.borrow().library_path.as_ref() != Some(&path);
    state.config.borrow_mut().library_path = Some(path);
    if let Err(e) = state.config.borrow().save() {
        eprintln!("Не удалось сохранить настройки: {e:#}");
    }
    if changed {
        start_scan(state);
    }
}

fn start_scan(state: &Rc<App>) {
    if state.scanning.get() {
        // Например, во время стартового сканирования сменили папку: останавливаем
        // текущее и запускаем заново, когда оно завершится.
        state.rescan_pending.set(true);
        if let Some(cancel) = state.cancel.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
        state.status.set_text("Остановка текущего сканирования…");
        return;
    }
    let Some(root) = state.config.borrow().library_path.clone() else {
        open_settings(state);
        return;
    };
    state.scanning.set(true);
    state.rescan.set_sensitive(false);
    state.progress.set_fraction(0.0);
    state.progress.set_visible(true);
    state.status.set_text("Поиск фотографий…");
    state.last_partial_reload.set(Instant::now());
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.borrow_mut() = Some(cancel.clone());

    let (tx, rx) = async_channel::unbounded::<ScanEvent>();
    std::thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress = move |done: usize, total: usize| {
            let _ = progress_tx.send_blocking(ScanEvent::Progress(done, total));
        };
        let ev = match scanner::scan(&paths::db_path(), &root, &paths::thumbs_dir(), &progress, &cancel) {
            Err(e) if e.is::<scanner::Cancelled>() => ScanEvent::Cancelled,
            res => ScanEvent::Finished(res.map_err(|e| format!("{e:#}"))),
        };
        let _ = tx.send_blocking(ev);
    });

    let state = state.clone();
    glib::spawn_future_local(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                ScanEvent::Progress(done, total) => {
                    if total > 0 && !state.rescan_pending.get() {
                        state.progress.set_fraction(done as f64 / total as f64);
                        state.status.set_text(&format!("Обработка {done} / {total}"));
                    }
                    // Длинное сканирование: показываем уже обработанные фото.
                    if state.last_partial_reload.get().elapsed() >= PARTIAL_RELOAD {
                        state.last_partial_reload.set(Instant::now());
                        reload(&state, false);
                    }
                }
                ScanEvent::Cancelled => {
                    finish_scan(&state);
                    break;
                }
                ScanEvent::Finished(res) => {
                    finish_scan(&state);
                    if state.scanning.get() {
                        // Уже запущено новое сканирование (сменилась папка).
                        break;
                    }
                    match res {
                        Ok(s) => {
                            if s.added + s.updated > 0 {
                                // Миниатюры обновлённых файлов перезаписаны по тем же путям.
                                state.cache.clear();
                            }
                            reload(&state, true);
                            if s.added + s.updated + s.removed > 0 {
                                state.status.set_text(&format!(
                                    "{} фото (новых {}, изменено {}, удалено {})",
                                    s.found, s.added, s.updated, s.removed
                                ));
                            }
                        }
                        Err(e) => {
                            reload(&state, true);
                            state.status.set_text(&format!("Ошибка сканирования: {e}"));
                        }
                    }
                    break;
                }
            }
        }
    });
}

/// Общая часть завершения/отмены; перезапускает сканирование, если оно было запрошено.
fn finish_scan(state: &Rc<App>) {
    state.scanning.set(false);
    state.cancel.borrow_mut().take();
    state.rescan.set_sensitive(true);
    state.progress.set_visible(false);
    if state.rescan_pending.replace(false) {
        start_scan(state);
    }
}
