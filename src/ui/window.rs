//! Главное окно: две вкладки, настройки, фоновое сканирование.
use super::{
    about, coords_dialog,
    dates_tab::DatesTab,
    info_panel::InfoPanel,
    map_tab::MapTab,
    menu::{icon_menu, Item},
    settings,
    textures::{self, TextureCache},
    update_banner::UpdateBanner,
    CSS,
};
use crate::{
    config::{self, Config, MapView, SortOrder},
    db::{Db, Photo},
    geotag, i18n, paths, scanner, trace,
};
use gtk::{gdk, gio, glib, prelude::*};
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
/// Во сколько раз пауза между подгрузками должна быть длиннее самой подгрузки:
/// чтение БД идёт в главном потоке и растёт с библиотекой, а так на него уходит
/// не больше 5% времени, какой бы большой она ни была.
const PARTIAL_RELOAD_RATIO: u32 = 20;

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
    stack: gtk::Stack,
    /// Панель «Информация» справа от вкладок.
    info: Rc<InfoPanel>,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    rescan: gtk::Button,
    scanning: Cell<bool>,
    /// Флаг отмены текущего сканирования.
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    /// Запустить сканирование заново после окончания текущего (сменилась папка).
    rescan_pending: Cell<bool>,
    last_partial_reload: Cell<Instant>,
    /// Пауза до следующей подгрузки во время сканирования (см. `PARTIAL_RELOAD_RATIO`).
    partial_reload_interval: Cell<Duration>,
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
    // Первая вкладка открывается при запуске.
    stack.add_titled(&map.root, Some("map"), i18n::tab_map());
    stack.add_titled(&dates.root, Some("dates"), i18n::tab_dates());
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));

    let menu = icon_menu(vec![vec![
        Item::new(i18n::settings(), "win.settings", "emblem-system-symbolic").accel("<Control>comma"),
        Item::new(i18n::about(), "win.about", "help-about-symbolic"),
    ]]);
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .popover(&menu)
        .primary(true)
        .tooltip_text(i18n::main_menu())
        .build();
    app.set_accels_for_action("win.settings", &["<Control>comma"]);
    let rescan = gtk::Button::from_icon_name("view-refresh-symbolic");
    rescan.set_tooltip_text(Some(i18n::rescan()));
    let progress = gtk::ProgressBar::new();
    progress.set_valign(gtk::Align::Center);
    progress.set_size_request(120, -1);
    progress.set_visible(false);
    let status = gtk::Label::new(None);
    status.add_css_class("dim-label");
    // Длинные сообщения (ошибки) не должны раздвигать заголовок окна.
    status.set_ellipsize(gtk::pango::EllipsizeMode::End);
    status.set_max_width_chars(50);

    let header = gtk::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    // Переключатель панели «Информация» — слева от кнопки меню.
    let info_btn = gtk::ToggleButton::new();
    // «i» в круге; dialog-information в Adwaita — лампочка.
    info_btn.set_icon_name("help-about-symbolic");
    info_btn.set_tooltip_text(Some(i18n::info()));
    header.pack_end(&menu_btn);
    header.pack_end(&info_btn);
    header.pack_end(&rescan);
    header.pack_start(&progress);
    header.pack_start(&status);

    // Справа от вкладок — панель «Информация», общая для обеих.
    let info = InfoPanel::new();
    stack.set_hexpand(true);
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&stack);
    body.append(&info.root);
    // Над вкладками — уведомление о новой версии (скрыто, пока её нет).
    let banner = UpdateBanner::new();
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.append(&banner.root);
    body.set_vexpand(true);
    main.append(&body);

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(crate::APP_NAME)
        .default_width(1280)
        .default_height(820)
        .child(&main)
        .build();
    window.set_titlebar(Some(&header));

    let state = Rc::new(App {
        window,
        config: RefCell::new(Config::load()),
        db,
        cache,
        dates,
        map,
        stack: stack.clone(),
        info,
        progress,
        status,
        rescan: rescan.clone(),
        scanning: Cell::new(false),
        cancel: RefCell::default(),
        rescan_pending: Cell::new(false),
        last_partial_reload: Cell::new(Instant::now()),
        partial_reload_interval: Cell::new(PARTIAL_RELOAD),
    });

    init_sort(&state);
    init_thumb_scale(&state);
    init_map_view(&state);
    apply_preferred_apps(&state);
    for grid in [state.dates.grid(), state.map.grid()] {
        let s = Rc::downgrade(&state);
        grid.connect_trashed(move |results| {
            if let Some(s) = s.upgrade() {
                photos_trashed(&s, results);
            }
        });
    }

    init_info_panel(&state, &info_btn);
    for grid in [state.dates.grid(), state.map.grid()] {
        let s = Rc::downgrade(&state);
        grid.connect_show_on_map(move |photo| {
            if let Some(s) = s.upgrade() {
                show_on_map(&s, photo);
            }
        });
        let s = Rc::downgrade(&state);
        grid.connect_edit_coords(move |photos| {
            if let Some(s) = s.upgrade() {
                edit_coords(&s, photos);
            }
        });
    }

    // Обработчики на виджетах, которыми владеет `App`, держат его слабой ссылкой:
    // сильная замкнула бы цикл App → виджет → обработчик → App, и при закрытии окна
    // не освободились бы ни состояние, ни соединение с БД.
    let s = Rc::downgrade(&state);
    let settings_action = gio::ActionEntry::builder("settings")
        .activate(move |_, _, _| {
            if let Some(s) = s.upgrade() {
                open_settings(&s);
            }
        })
        .build();
    let about_action = gio::ActionEntry::builder("about")
        .activate(|w: &gtk::ApplicationWindow, _, _| about::show(w))
        .build();
    state.window.add_action_entries([settings_action, about_action]);
    let s = Rc::downgrade(&state);
    rescan.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            start_scan(&s);
        }
    });

    // Состоянием владеет окно: единственная сильная ссылка живёт в обработчике
    // закрытия и отпускается в нём же — вместе с ней закрывается БД. Сигнал `destroy`
    // для этого не годится: пока обработчик держит состояние, а оно — окно, окно
    // не разрушается. Обработчик подключён после `init_map_view`, поэтому положение
    // карты к этому моменту уже сохранено.
    let owner = RefCell::new(Some(state.clone()));
    state.window.connect_close_request(move |_| {
        if let Some(state) = owner.borrow_mut().take() {
            // Незачем досканировать библиотеку для закрытого окна.
            if let Some(cancel) = state.cancel.borrow().as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        glib::Propagation::Proceed
    });

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
    if state.config.borrow().check_updates {
        banner.check();
    }
    debug_action(&state.window);
    debug_screenshot(&state.window, &stack);
    debug_bench(&state, &stack);
    debug_switch(&state);
    debug_search(&state, &stack);
    debug_date_filter(&state, &stack);
    debug_info(&state);
    debug_select(&state);
    debug_edit_coords(&state);
    debug_locate(&state, &stack);
}

/// Отладка: `GEO_PHOTO_LIBRARY_LOCATE=1` — через 2 с нажать «Моё местоположение».
fn debug_locate(state: &Rc<App>, stack: &gtk::Stack) {
    if std::env::var_os("GEO_PHOTO_LIBRARY_LOCATE").is_none() {
        return;
    }
    stack.set_visible_child_name("map");
    let state = state.clone();
    glib::timeout_add_seconds_local_once(2, move || state.map.locate_me());
}

/// Панель «Информация»: открывается из меню фото или кнопкой в заголовке, показывает выбранное фото
/// активной вкладки, закрывается крестиком или той же кнопкой.
fn init_info_panel(state: &Rc<App>, button: &gtk::ToggleButton) {
    // Кнопка в заголовке и видимость панели всегда согласованы: панель открывают
    // кнопка и пункт меню, закрывают кнопка и крестик.
    let s = Rc::downgrade(state);
    button.connect_toggled(move |b| {
        let Some(s) = s.upgrade() else { return };
        if b.is_active() {
            s.info.open();
            update_info(&s);
        } else {
            s.info.hide();
        }
    });
    let b = button.downgrade();
    state.info.root.connect_visible_notify(move |root| {
        if let Some(b) = b.upgrade() {
            b.set_active(root.is_visible());
        }
    });
    for grid in [state.dates.grid(), state.map.grid()] {
        let s = Rc::downgrade(state);
        grid.connect_info(move |_| {
            if let Some(s) = s.upgrade() {
                // Фото уже выделено щелчком правой кнопки — панель покажет его.
                s.info.open();
                update_info(&s);
            }
        });
        let s = Rc::downgrade(state);
        grid.connect_selection_changed(move || {
            if let Some(s) = s.upgrade() {
                update_info(&s);
            }
        });
    }
    let s = Rc::downgrade(state);
    state.stack.connect_visible_child_notify(move |_| {
        if let Some(s) = s.upgrade() {
            update_info(&s);
        }
    });
    let s = Rc::downgrade(state);
    state.info.close.connect_clicked(move |_| {
        if let Some(s) = s.upgrade() {
            s.info.hide();
        }
    });
}

/// «Найти на карте»: открыть карту в месте съёмки, а без координат — предупредить.
fn show_on_map(state: &Rc<App>, photo: &Photo) {
    if photo.coords().is_none() {
        gtk::AlertDialog::builder()
            .modal(true)
            .message(i18n::no_coordinates())
            .detail(i18n::no_coordinates_detail(photo.file_name()))
            .buttons([i18n::ok()])
            .build()
            .show(Some(&state.window));
        return;
    }
    state.stack.set_visible_child_name("map");
    state.map.show_photo(photo);
}

/// Окно «Редактировать координаты» для выделенных фото. Без координат у фото карта
/// открывается там же, где главная.
fn edit_coords(state: &Rc<App>, photos: Vec<Photo>) {
    let s = Rc::downgrade(state);
    coords_dialog::show(state.window.upcast_ref(), photos, state.map.view(), move |lat, lon, photos| {
        if let Some(s) = s.upgrade() {
            save_coords(&s, lat, lon, photos);
        }
    });
}

/// Записать координаты в файлы (в фоне, параллельно), затем в БД; ошибку показать окном.
fn save_coords(state: &Rc<App>, lat: f64, lon: f64, photos: Vec<Photo>) {
    let s = Rc::downgrade(state);
    glib::spawn_future_local(async move {
        let results = gio::spawn_blocking(move || {
            use rayon::prelude::*;
            let _t = trace::span(format!("запись координат в {} файлов", photos.len()));
            photos
                .into_par_iter()
                .map(|p| {
                    let r = geotag::write_coords(std::path::Path::new(&p.path), lat, lon);
                    (p, r)
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let Some(s) = s.upgrade() else { return };
        let mut saved = Vec::new();
        let mut failed = None;
        for (photo, res) in results {
            match res.and_then(|st| s.db.set_coords(photo.id, lat, lon, st.size, st.mtime)) {
                Ok(()) => saved.push(photo),
                Err(e) => {
                    eprintln!("Координаты {}: {e:#}", photo.path);
                    failed.get_or_insert((photo, e));
                }
            }
        }
        if !saved.is_empty() {
            // Маркеры карты хранятся по составу кластера — с новыми координатами их нужно пересоздать.
            reload(&s, true);
        }
        let status = match (&failed, saved.as_slice()) {
            (Some((photo, e)), _) => i18n::coordinates_failed(photo.file_name(), &format!("{e:#}")),
            (None, [photo]) => i18n::coordinates_saved(photo.file_name()),
            (None, many) => i18n::coordinates_saved(&i18n::photos(many.len())),
        };
        s.status.set_text(&status);
        if failed.is_some() {
            gtk::AlertDialog::builder().modal(true).message(status).buttons([i18n::ok()]).build().show(Some(&s.window));
        }
    });
}

/// Сетка фото на открытой вкладке.
fn active_grid(state: &App) -> &super::photo_grid::PhotoGrid {
    match state.stack.visible_child_name().as_deref() {
        Some("dates") => state.dates.grid(),
        _ => state.map.grid(),
    }
}

/// Показать в открытой панели «Информация» выбранное фото активной вкладки;
/// при нескольких выделенных панель пуста.
///
/// Откладывается до простоя: при перестроении списка (сдвиг карты, пересканирование)
/// выделение на миг снимается и восстанавливается — панель видит уже итоговый выбор.
fn update_info(state: &Rc<App>) {
    let s = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        if let Some(s) = s.upgrade().filter(|s| s.info.is_open()) {
            let grid = active_grid(&s);
            if grid.selected_count() > 1 {
                s.info.show_nothing();
            } else {
                s.info.show_photo(grid.selected_photo());
            }
        }
    });
}

/// Отладка: `GEO_PHOTO_LIBRARY_INFO=1` — через 3 с выделить первое фото на открытой
/// вкладке и показать панель «Информация», как из меню фото.
fn debug_info(state: &Rc<App>) {
    if std::env::var_os("GEO_PHOTO_LIBRARY_INFO").is_none() {
        return;
    }
    let state = state.clone();
    glib::timeout_add_seconds_local_once(3, move || {
        active_grid(&state).select_first(1);
        state.info.open();
        update_info(&state);
    });
}

/// Отладка: `GEO_PHOTO_LIBRARY_SELECT=<n>` — через 4 с выделить первые n фото на открытой вкладке.
fn debug_select(state: &Rc<App>) {
    let Some(n) = std::env::var("GEO_PHOTO_LIBRARY_SELECT").ok().and_then(|v| v.parse().ok()) else {
        return;
    };
    let state = state.clone();
    glib::timeout_add_seconds_local_once(4, move || active_grid(&state).select_first(n));
}

/// Отладка: `GEO_PHOTO_LIBRARY_EDIT_COORDS=1` — через 5 с открыть редактор координат
/// для выделенных фото на открытой вкладке (без выделения — для первого фото).
fn debug_edit_coords(state: &Rc<App>) {
    if std::env::var_os("GEO_PHOTO_LIBRARY_EDIT_COORDS").is_none() {
        return;
    }
    let state = state.clone();
    glib::timeout_add_seconds_local_once(5, move || {
        let grid = active_grid(&state);
        let mut photos = grid.selected_photos();
        if photos.is_empty() {
            photos.extend(grid.photo(0));
        }
        if !photos.is_empty() {
            edit_coords(&state, photos);
        }
    });
}

/// Отладка: `GEO_PHOTO_LIBRARY_DATE_FILTER=open|<год>` — через 3 с открыть фильтр карты
/// по датам (и выбрать год).
fn debug_date_filter(state: &Rc<App>, stack: &gtk::Stack) {
    let Ok(value) = std::env::var("GEO_PHOTO_LIBRARY_DATE_FILTER") else { return };
    stack.set_visible_child_name("map");
    let state = state.clone();
    glib::timeout_add_seconds_local_once(3, move || state.map.open_date_filter(value.parse().ok()));
}

/// Отладка: `GEO_PHOTO_LIBRARY_SEARCH=<запрос>` — через 2 с найти место на карте.
fn debug_search(state: &Rc<App>, stack: &gtk::Stack) {
    let Ok(query) = std::env::var("GEO_PHOTO_LIBRARY_SEARCH") else { return };
    stack.set_visible_child_name("map");
    let state = state.clone();
    glib::timeout_add_seconds_local_once(2, move || state.map.search_for(&query));
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

/// Отладка: `GEO_PHOTO_LIBRARY_ACTION=about` — при запуске выполнить действие окна
/// (пункт главного меню: `settings`, `about`).
fn debug_action(window: &gtk::ApplicationWindow) {
    let Ok(name) = std::env::var("GEO_PHOTO_LIBRARY_ACTION") else { return };
    gtk::prelude::ActionGroupExt::activate_action(window, &name, None);
}

/// Отладка: `GEO_PHOTO_LIBRARY_SCREENSHOT=out.png [GEO_PHOTO_LIBRARY_TAB=map]` — через несколько
/// секунд отрисовать окно в PNG и выйти. Если открыт диалог (настройки, «О программе»),
/// снимается он.
fn debug_screenshot(window: &gtk::ApplicationWindow, stack: &gtk::Stack) {
    let Ok(path) = std::env::var("GEO_PHOTO_LIBRARY_SCREENSHOT") else { return };
    if let Ok(tab) = std::env::var("GEO_PHOTO_LIBRARY_TAB") {
        stack.set_visible_child_name(&tab);
    }
    let delay = std::env::var("GEO_PHOTO_LIBRARY_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(6);
    let main = window.clone();
    glib::timeout_add_seconds_local_once(delay, move || {
        let w: gtk::Window = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|o| o.downcast::<gtk::Window>().ok())
            .find(|t| t.is_visible() && t.transient_for().as_ref() == Some(main.upcast_ref()))
            .unwrap_or_else(|| main.clone().upcast());
        let paintable = gtk::WidgetPaintable::new(Some(&w));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w.width() as f64, w.height() as f64);
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), w.renderer()) {
            if let Err(e) = renderer.render_texture(&node, None).save_to_png(&path) {
                eprintln!("скриншот: {e}");
            }
        }
        // Закрыть как пользователь (сохраняется положение карты), затем выйти.
        let app = main.application();
        main.close();
        if let Some(app) = app {
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
        state.status.set_text(&i18n::photos(n as usize));
    }
}

/// Сортировка фото: один выбор на обе вкладки, сохраняется в настройках.
fn init_sort(state: &Rc<App>) {
    let order = state.config.borrow().sort;
    state.dates.set_order(order);
    state.map.set_order(order);
    state.dates.sort.set_selected(order.index());
    state.map.sort.set_selected(order.index());
    state
        .dates
        .sort
        .bind_property("selected", &state.map.sort, "selected")
        .bidirectional()
        .build();
    // Обе вкладки уже синхронизированы привязкой — реагируем на одну.
    let s = Rc::downgrade(state);
    state.dates.sort.connect_selected_notify(move |d| {
        let Some(s) = s.upgrade() else { return };
        let order = SortOrder::from_index(d.selected());
        s.dates.set_order(order);
        s.map.set_order(order);
        if s.config.borrow().sort != order {
            s.config.borrow_mut().sort = order;
            if let Err(e) = s.config.borrow().save() {
                eprintln!("Не удалось сохранить настройки: {e:#}");
            }
        }
    });
}

/// Масштаб миниатюр: один слайдер на обе вкладки (общий `Adjustment`),
/// значение округляется до шага 5% и сохраняется в настройках.
/// Положение карты: восстановить при запуске, запомнить при закрытии окна.
fn init_map_view(state: &Rc<App>) {
    if let Some(view) = state.config.borrow().map_view.and_then(MapView::valid) {
        state.map.restore_view(view);
    }
    let s = Rc::downgrade(state);
    state.window.connect_close_request(move |_| {
        if let Some(s) = s.upgrade() {
            save_map_view(&s);
        }
        glib::Propagation::Proceed
    });
}

fn save_map_view(state: &App) {
    let Some(view) = state.map.view().map(Some) else { return };
    if state.config.borrow().map_view == view {
        return;
    }
    state.config.borrow_mut().map_view = view;
    if let Err(e) = state.config.borrow().save() {
        eprintln!("Не удалось сохранить настройки: {e:#}");
    }
}

fn init_thumb_scale(state: &Rc<App>) {
    let percent = config::snap_thumb_scale(state.config.borrow().thumb_scale as f64);
    let adj = state.dates.zoom.adjustment();
    state.map.zoom.set_adjustment(&adj);
    adj.set_value(percent as f64);
    state.dates.set_thumb_scale(percent);
    state.map.set_thumb_scale(percent);
    let s = Rc::downgrade(state);
    adj.connect_value_changed(move |adj| {
        let Some(s) = s.upgrade() else { return };
        let percent = config::snap_thumb_scale(adj.value());
        if adj.value() != percent as f64 {
            // Перетаскивание даёт дробные значения — прилипаем к шагу.
            adj.set_value(percent as f64);
            return;
        }
        s.dates.set_thumb_scale(percent);
        s.map.set_thumb_scale(percent);
        if s.config.borrow().thumb_scale != percent {
            s.config.borrow_mut().thumb_scale = percent;
            if let Err(e) = s.config.borrow().save() {
                eprintln!("Не удалось сохранить настройки: {e:#}");
            }
        }
    });
}

/// Фото перемещены в корзину из контекстного меню: убрать их из БД и интерфейса.
/// В строке состояния — имя фото (или их число), при ошибке — первая из них.
fn photos_trashed(state: &Rc<App>, results: super::photo_grid::TrashResults) {
    let mut moved = Vec::new();
    let mut failed = None;
    for (photo, res) in results {
        match res {
            Ok(()) => moved.push(photo),
            Err(e) => {
                failed.get_or_insert((photo, e));
            }
        }
    }
    for photo in &moved {
        match state.db.remove(photo.id) {
            Ok(Some(thumb)) => {
                let _ = std::fs::remove_file(thumb);
            }
            Ok(None) => {}
            Err(e) => eprintln!("БД: {e:#}"),
        }
    }
    if !moved.is_empty() {
        reload(state, false);
    }
    let status = match (failed, moved.as_slice()) {
        (Some((photo, e)), _) => i18n::trash_failed(photo.file_name(), e.message()),
        (None, [photo]) => i18n::trashed(photo.file_name()),
        (None, many) => i18n::trashed(&i18n::photos(many.len())),
    };
    state.status.set_text(&status);
}

fn open_settings(state: &Rc<App>) {
    let s = state.clone();
    let config = state.config.borrow().clone();
    settings::show(&state.window, &config, move |new| {
        {
            let mut c = s.config.borrow_mut();
            c.viewer = new.viewer;
            c.editor = new.editor;
            c.check_updates = new.check_updates;
        }
        apply_preferred_apps(&s);
        // Сохраняет настройки и при смене папки пересканирует.
        if let Some(path) = new.library_path {
            set_library(&s, path);
        }
    });
}

/// Передать выбранные в настройках просмотрщик и редактор сеткам фото.
fn apply_preferred_apps(state: &App) {
    let c = state.config.borrow();
    textures::set_preferred_apps(textures::PreferredApps { viewer: c.viewer.clone(), editor: c.editor.clone() });
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
        state.status.set_text(i18n::stopping_scan());
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
    state.status.set_text(i18n::searching());
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
                        state.status.set_text(&i18n::processing(done, total));
                    }
                    // Длинное сканирование: показываем уже обработанные фото.
                    if state.last_partial_reload.get().elapsed() >= state.partial_reload_interval.get() {
                        let started = Instant::now();
                        reload(&state, false);
                        let interval = (started.elapsed() * PARTIAL_RELOAD_RATIO).max(PARTIAL_RELOAD);
                        state.partial_reload_interval.set(interval);
                        state.last_partial_reload.set(Instant::now());
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
                                state.status.set_text(&i18n::scan_summary(
                                    s.found, s.added, s.updated, s.removed,
                                ));
                            }
                        }
                        Err(e) => {
                            reload(&state, true);
                            state.status.set_text(&i18n::scan_error(&e));
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
