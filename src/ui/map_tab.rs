//! Вкладка «Карта»: OSM-карта с превью-маркерами (кластеризация) и список фото
//! в видимой области, отсортированный по дате.
use super::{
    date_tree::{DateTree, NodeKey},
    photo_grid::{self, PhotoGrid},
    place_search::{self, PlaceSearch},
    textures::{set_thumbnail, TextureCache, PHOTO_ICON},
};
use crate::{
    config::{MapView, SortOrder},
    db::{DateCount, Day, Db, Photo},
    geo, i18n, location, trace,
};
use gtk::{glib, prelude::*};
use shumate::prelude::*;
use std::{
    cell::{Cell, Ref, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

const MARKER_SIZE: i32 = 56;
/// Размер ячейки кластеризации в пикселях.
const CELL: f64 = 72.0;
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Зум для пункта «Найти на карте» — уровень улиц.
const PHOTO_ZOOM: f64 = 16.0;

/// Фото в быстрых проходах: id и мировые координаты на зуме 0.
#[derive(Clone, Copy)]
struct Point {
    id: i64,
    x: f64,
    y: f64,
}

/// Сколько ждать, пока фото из «Найти на карте» или со щелчка по маркеру окажется в
/// видимой области (карта едет к нему с анимацией). Позже выделять его уже некстати:
/// пользователь занят другим.
const PENDING_SELECT_TTL: Duration = Duration::from_secs(8);

/// Ключ кластера: id первого фото, число фото и хеш id всех его фото (состав).
type ClusterKey = (i64, usize, u64);

/// Хеш состава кластера: зависит от каждого id и от порядка (он у одного состава всегда
/// один — по дате). Сумма id, что была раньше, совпадала у разных наборов (1+4 = 2+3),
/// и маркер оставался со старыми фото. Умножение и сдвиг на фото — обновление карты идёт
/// по всем фото на каждое движение.
fn members_hash(ids: impl Iterator<Item = i64>) -> u64 {
    ids.fold(0xcbf2_9ce4_8422_2325, |h, id| {
        let h = (h ^ id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        h ^ (h >> 29)
    })
}

pub struct MapTab {
    pub root: gtk::Paned,
    /// Выбор сортировки в заголовке списка.
    pub sort: gtk::DropDown,
    /// Масштаб миниатюр под списком.
    pub zoom: gtk::Scale,
    simple_map: shumate::SimpleMap,
    map: shumate::Map,
    viewport: shumate::Viewport,
    layer: shumate::MarkerLayer,
    grid: PhotoGrid,
    info: gtk::Label,
    db: Rc<Db>,
    cache: Rc<TextureCache>,
    /// Все фото с координатами, по дате: объекты с [`Photo`] внутри. Создаются один
    /// раз при загрузке и передаются в список как есть, без копирования данных.
    photos: RefCell<Vec<glib::BoxedAnyObject>>,
    /// id и проекция (зум 0) каждого фото из `photos` — для быстрых проходов по всем
    /// фото при каждом движении карты.
    points: RefCell<Vec<Point>>,
    /// id фото в правом списке — чтобы не перестраивать его без изменений.
    visible_ids: RefCell<Vec<i64>>,
    /// Маркеры на карте по ключу кластера — при сдвиге карты пересоздаются только
    /// изменившиеся.
    markers: RefCell<HashMap<ClusterKey, shumate::Marker>>,
    pending: RefCell<Option<glib::SourceId>>,
    fitted: Cell<bool>,
    last_size: Cell<(i32, i32)>,
    search: Rc<PlaceSearch>,
    /// Кнопка «Моё местоположение» и слой с отметкой найденной точки.
    locate: gtk::Button,
    location_layer: shumate::MarkerLayer,
    /// Фото, которое выделить в списке, когда оно появится в видимой области (после перехода
    /// «Найти на карте» или щелчка по маркеру у края карты), и время запроса: ожидание
    /// снимается следующим действием пользователя или по `PENDING_SELECT_TTL`.
    pending_select: Cell<Option<(i64, Instant)>>,
    /// Фильтр по датам: дерево в выдвижной панели слева, кнопка над картой с подписью
    /// выбранного периода и сам период (`None` — всё время).
    filter: Rc<DateTree>,
    filter_button: gtk::ToggleButton,
    filter_label: gtk::Label,
    period: Cell<Option<NodeKey>>,
    /// Все фото с координатами (без фильтра): проекция на зум 0 и день съёмки — по ним
    /// дерево фильтра показывает даты фото в видимой области карты.
    all_days: RefCell<Vec<(f64, f64, Day)>>,
    /// Счётчики, показанные в дереве фильтра, — чтобы не перестраивать его без изменений.
    shown_counts: RefCell<Vec<DateCount>>,
}

impl MapTab {
    pub fn new(db: Rc<Db>, cache: Rc<TextureCache>) -> Rc<Self> {
        let simple_map = shumate::SimpleMap::new();
        let registry = shumate::MapSourceRegistry::with_defaults();
        if let Some(src) = registry.by_id(shumate::MAP_SOURCE_OSM_MAPNIK) {
            simple_map.set_map_source(Some(&src));
        }
        simple_map.set_hexpand(true);
        simple_map.set_vexpand(true);
        let map = simple_map.map().expect("SimpleMap без Map");
        let viewport = simple_map.viewport().expect("SimpleMap без Viewport");
        let layer = shumate::MarkerLayer::new(&viewport);
        simple_map.add_overlay_layer(&layer);
        // Отметка «я здесь» — поверх превью фото.
        let location_layer = shumate::MarkerLayer::new(&viewport);
        simple_map.add_overlay_layer(&location_layer);

        let grid = PhotoGrid::new(cache.clone(), 100, 4);
        grid.set_empty_text(i18n::no_geotagged());
        let info = gtk::Label::new(None);
        let sort = photo_grid::sort_dropdown();
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.append(&photo_grid::pane_header(&info, &sort));
        right.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        right.append(&grid.root);
        let zoom = photo_grid::zoom_scale();
        right.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        right.append(&photo_grid::pane_footer(&zoom));
        right.set_size_request(260, -1);

        let search = PlaceSearch::new();
        // Фильтр по датам: кнопка открывает панель слева, на ней же — выбранный период.
        // Панель плавает над картой под строкой поиска: открытие не сдвигает карту.
        let filter = DateTree::new(true);
        filter.root.set_size_request(240, -1);
        filter.root.add_css_class("map-filter");
        filter.root.set_overflow(gtk::Overflow::Hidden);
        // Под строкой поиска: её отступ 10 + высота ~34 + зазор.
        filter.root.set_margin_top(52);
        filter.root.set_margin_bottom(10);
        filter.root.set_margin_start(10);
        filter.root.set_margin_end(10);
        let filter_panel = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideRight)
            .child(&filter.root)
            .halign(gtk::Align::Start)
            .build();
        let filter_label = gtk::Label::new(None);
        filter_label.set_visible(false);
        let filter_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        filter_content.append(&gtk::Image::from_icon_name("x-office-calendar-symbolic"));
        filter_content.append(&filter_label);
        let filter_button = gtk::ToggleButton::new();
        filter_button.set_child(Some(&filter_content));
        filter_button.set_tooltip_text(Some(i18n::filter_by_date()));
        super::a11y_label(&filter_button, i18n::filter_by_date());
        filter_button.add_css_class("map-search");
        filter_button.bind_property("active", &filter_panel, "reveal-child").sync_create().build();
        let locate = place_search::locate_button();
        // Строка поиска и кнопка местоположения — в левом верхнем углу карты.
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.set_halign(gtk::Align::Start);
        toolbar.set_valign(gtk::Align::Start);
        toolbar.set_margin_top(10);
        toolbar.set_margin_start(10);
        toolbar.append(&filter_button);
        toolbar.append(&search.entry);
        toolbar.append(&locate);
        // Панель — отдельным слоем во всю высоту: скрытая, она нулевой ширины и не
        // перехватывает щелчки по карте.
        let map_overlay = gtk::Overlay::new();
        map_overlay.set_child(Some(&simple_map));
        let size_probe = gtk::DrawingArea::new();
        size_probe.set_can_target(false);
        map_overlay.add_overlay(&size_probe);
        map_overlay.add_overlay(&filter_panel);
        map_overlay.add_overlay(&toolbar);

        let root = gtk::Paned::new(gtk::Orientation::Horizontal);
        root.set_start_child(Some(&map_overlay));
        root.set_end_child(Some(&right));
        root.set_resize_end_child(false);
        root.set_shrink_end_child(false);
        root.set_position(880);

        let this = Rc::new(Self {
            root,
            sort,
            zoom,
            simple_map,
            map,
            viewport,
            layer,
            grid,
            info,
            db,
            cache,
            photos: RefCell::default(),
            points: RefCell::default(),
            visible_ids: RefCell::default(),
            markers: RefCell::default(),
            pending: RefCell::default(),
            fitted: Cell::new(false),
            last_size: Cell::new((0, 0)),
            search,
            locate,
            location_layer,
            pending_select: Cell::new(None),
            filter,
            filter_button,
            filter_label,
            period: Cell::new(None),
            all_days: RefCell::default(),
            shown_counts: RefCell::default(),
        });

        // Смена периода — перечитать фото; положение и масштаб карты не меняются.
        let weak = Rc::downgrade(&this);
        this.filter.connect_changed(move || {
            let Some(t) = weak.upgrade() else { return };
            let node = t.filter.selected().filter(|n| !n.is_all_time());
            t.set_period(node.map(|n| (n.key(), n.title())));
        });

        let weak = Rc::downgrade(&this);
        this.locate.connect_clicked(move |_| {
            if let Some(t) = weak.upgrade() {
                t.locate_me();
            }
        });

        // Поиск местности: переход к выбранному месту.
        let weak = Rc::downgrade(&this);
        this.search.connect_place(move |place| {
            if let Some(t) = weak.upgrade() {
                place_search::go_to_place(&t.simple_map, place);
            }
        });

        // Щелчок по карте закрывает панель фильтра по датам. Фаза захвата — чтобы
        // увидеть щелчок раньше карты и маркеров; событие не поглощается. При сдвиге
        // мыши (перетаскивание карты) щелчок не засчитывается.
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&this);
        click.connect_released(move |_, _, _, _| {
            if let Some(t) = weak.upgrade() {
                t.filter_button.set_active(false);
                // Пользователь занялся картой сам — отложенное выделение больше не нужно
                // (щелчок по маркеру назначит его заново: обработчик маркера идёт позже).
                t.pending_select.set(None);
            }
        });
        this.simple_map.add_controller(click);

        // Любое изменение зума/центра — пересчитать (с задержкой).
        let weak = Rc::downgrade(&this);
        this.viewport.connect_notify_local(None, move |_, _| {
            if let Some(t) = weak.upgrade() {
                t.schedule_refresh();
            }
        });
        // Изменение размера карты (первый показ, размер окна, перетаскивание Paned).
        // У виджетов GTK4 нет сигнала о размере, а покадровый обработчик будил бы
        // главный поток 60 раз в секунду даже в простое — поэтому за размером следит
        // невидимая DrawingArea поверх карты. Обработка — после раскладки, из простоя.
        let weak = Rc::downgrade(&this);
        size_probe.connect_resize(move |_, _, _| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                let Some(t) = weak.upgrade() else { return };
                let size = (t.simple_map.width(), t.simple_map.height());
                if size != t.last_size.get() {
                    t.last_size.set(size);
                    if !t.fitted.get() {
                        t.fit_all();
                    }
                    t.schedule_refresh();
                }
            });
        });
        // Щелчок по фото в списке — центрировать карту на нём (выделение клавишами,
        // рамкой и из программы карту не двигает).
        let weak = Rc::downgrade(&this);
        this.grid.connect_picked(move |photo| {
            let Some(t) = weak.upgrade() else { return };
            // Явный выбор в списке отменяет отложенное выделение.
            t.pending_select.set(None);
            if let Some((lat, lon)) = photo.coords() {
                t.map.go_to(lat, lon);
            }
        });
        this
    }

    /// Ввести запрос в строку поиска и выполнить его (отладочный хук).
    pub fn search_for(self: &Rc<Self>, query: &str) {
        self.search.entry.set_text(query);
        self.search.search(query.to_string());
    }

    pub fn set_order(&self, order: SortOrder) {
        self.grid.set_order(order);
    }

    pub fn grid(&self) -> &PhotoGrid {
        &self.grid
    }

    pub fn set_thumb_scale(&self, percent: u32) {
        self.grid.set_scale(percent);
    }

    /// «Моё местоположение»: спросить у портала геолокации, перейти и отметить точку.
    pub fn locate_me(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        place_search::locate(&self.locate, move |fix| {
            if let Some(t) = weak.upgrade() {
                t.show_location(fix);
            }
        });
    }

    /// Отметить найденное местоположение и показать его с окрестностями по точности.
    fn show_location(&self, fix: location::Fix) {
        self.location_layer.remove_all();
        self.location_layer.add_marker(&place_search::location_marker(&fix));
        self.fitted.set(true);
        place_search::go_to_location(&self.simple_map, &fix);
    }

    /// Перейти к месту съёмки фото и выделить его в списке. `false` — у фото нет координат.
    pub fn show_photo(self: &Rc<Self>, photo: &Photo) -> bool {
        let Some((lat, lon)) = photo.coords() else { return false };
        // Сохранённое положение или автоподгонка больше не нужны.
        self.fitted.set(true);
        // Фото вне выбранного периода на карте нет — фильтр по датам сбрасывается на
        // «Всё время» (и в дереве, и на кнопке), иначе карта приехала бы в пустое место.
        if let Some((year, month, day)) = self.period.get() {
            let inside = photo.day().is_some_and(|(y, m, d)| {
                y == year && month.is_none_or(|x| x == m) && day.is_none_or(|x| x == d)
            });
            if !inside {
                self.reset_period();
            }
        }
        self.pending_select.set(Some((photo.id, Instant::now())));
        let max = self.viewport.max_zoom_level() as f64;
        self.map.go_to_full(lat, lon, PHOTO_ZOOM.min(max));
        // Если карта уже здесь, viewport не изменится — обновить список явно.
        self.schedule_refresh();
        true
    }

    /// Мгновенно (без анимации) перейти к точке — для бенчмарка.
    pub fn jump_to(&self, lat: f64, lon: f64, zoom: f64) {
        self.viewport.set_zoom_level(zoom);
        self.map.center_on(lat, lon);
    }

    /// Текущее положение карты — чтобы запомнить его при закрытии. `None`, пока карта
    /// ещё не показана (окно без размера) — иначе запомнилась бы нулевая точка.
    pub fn view(&self) -> Option<MapView> {
        let v = &self.viewport;
        self.fitted.get().then(|| MapView { lat: v.latitude(), lon: v.longitude(), zoom: v.zoom_level() })
    }

    /// Вернуть карту в запомненное положение вместо подгонки под все фото.
    pub fn restore_view(&self, view: MapView) {
        let (min, max) = (self.viewport.min_zoom_level() as f64, self.viewport.max_zoom_level() as f64);
        // Целый зум: на дробном у тайлов видны швы.
        self.jump_to(view.lat, view.lon, view.zoom.round().clamp(min, max));
        self.fitted.set(true);
    }

    pub fn first_photo_coords(&self) -> Option<(f64, f64)> {
        self.photos.borrow().first().and_then(|o| o.borrow::<Photo>().coords())
    }

    /// Перечитать фото с координатами: все (для дерева фильтра) и за выбранный период
    /// (на карту). Без `force` маркеры и список перестраиваются, только если изменился их состав.
    pub fn reload(self: &Rc<Self>, force: bool) {
        let _t = trace::span("map.reload");
        let all = self.db.geotagged().unwrap_or_else(|e| {
            eprintln!("БД: {e:#}");
            Vec::new()
        });
        *self.all_days.borrow_mut() = all
            .iter()
            .filter_map(|p| {
                let (lat, lon) = p.coords()?;
                let (x, y) = geo::project(lat, lon, 0.0);
                Some((x, y, p.day()?))
            })
            .collect();
        // Без фильтра на карте те же фото — второй запрос не нужен.
        let photos = if self.period.get().is_none() { all } else { self.period_photos() };
        self.set_photos(photos, force);
        if !self.fitted.get() {
            self.fit_all();
        }
        self.schedule_refresh();
    }

    /// Сменить период фильтра (`None` — всё время; иначе узел и его подпись для кнопки):
    /// показать его на кнопке и перечитать фото. Положение и масштаб карты не меняются.
    fn set_period(self: &Rc<Self>, period: Option<(NodeKey, String)>) {
        self.filter_label.set_visible(period.is_some());
        if let Some((_, title)) = &period {
            self.filter_label.set_text(title);
            self.filter_button.add_css_class("suggested-action");
        } else {
            self.filter_button.remove_css_class("suggested-action");
        }
        let period = period.map(|(key, _)| key);
        if period != self.period.replace(period) {
            self.set_photos(self.period_photos(), false);
            self.schedule_refresh();
        }
    }

    /// Сбросить фильтр на «Всё время»: через строку дерева, чтобы выделение в нём совпадало
    /// с периодом; если дерево ещё не заполнено — напрямую.
    fn reset_period(self: &Rc<Self>) {
        if !self.filter.select_all_time() || self.period.get().is_some() {
            self.set_period(None);
        }
    }

    /// Открыть фильтр по датам и выбрать год (отладочный хук).
    pub fn open_date_filter(&self, year: Option<i32>) {
        self.filter_button.set_active(true);
        if let Some(year) = year {
            self.filter.select((year, None, None));
        }
    }

    /// Фото с координатами за выбранный период.
    fn period_photos(&self) -> Vec<Photo> {
        let (year, month, day) = match self.period.get() {
            Some((y, m, d)) => (Some(y), m, d),
            None => (None, None, None),
        };
        self.db.geotagged_by_date(year, month, day).unwrap_or_else(|e| {
            eprintln!("БД: {e:#}");
            Vec::new()
        })
    }

    /// Фото на карте и в списке. Без `force` маркеры и список перестраиваются,
    /// только если изменился их состав.
    fn set_photos(&self, photos: Vec<Photo>, force: bool) {
        *self.points.borrow_mut() = photos
            .iter()
            .filter_map(|p| {
                let (lat, lon) = p.coords()?;
                let (x, y) = geo::project(lat, lon, 0.0);
                Some(Point { id: p.id, x, y })
            })
            .collect();
        *self.photos.borrow_mut() = photos.into_iter().map(glib::BoxedAnyObject::new).collect();
        if force {
            self.layer.remove_all();
            self.markers.borrow_mut().clear();
            self.visible_ids.borrow_mut().clear();
        }
    }

    /// Показать все фото с координатами (за период фильтра). Работает, когда виджет уже имеет размер.
    fn fit_all(&self) {
        let (w, h) = (self.simple_map.width() as f64, self.simple_map.height() as f64);
        if w < 2.0 || h < 2.0 {
            return;
        }
        let pts: Vec<(f64, f64)> =
            self.photos.borrow().iter().filter_map(|o| o.borrow::<Photo>().coords()).collect();
        let Some((lat, lon)) = geo::center(&pts) else { return };
        let max = self.viewport.max_zoom_level() as f64;
        // Целый зум: на дробном у тайлов видны швы.
        let zoom = geo::fit_zoom(&pts, w, h, 60.0).min(max - 2.0).max(1.0).floor();
        self.viewport.set_zoom_level(zoom);
        self.map.center_on(lat, lon);
        self.fitted.set(true);
    }

    fn schedule_refresh(self: &Rc<Self>) {
        if let Some(id) = self.pending.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(DEBOUNCE, move || {
            if let Some(t) = weak.upgrade() {
                // Источник уже завершён — просто забываем его id.
                t.pending.borrow_mut().take();
                t.refresh();
            }
        });
        *self.pending.borrow_mut() = Some(id);
    }

    fn refresh(self: &Rc<Self>) {
        let (w, h) = (self.simple_map.width() as f64, self.simple_map.height() as f64);
        if w < 2.0 || h < 2.0 {
            return;
        }
        let zoom = self.viewport.zoom_level();
        let _t = trace::span(format!("map.refresh (zoom {zoom:.1})"));
        let view = geo::View::new(self.viewport.latitude(), self.viewport.longitude(), zoom, w, h);
        let photos = self.photos.borrow();
        let points = self.points.borrow();

        // Один проход по всем фото: `near` — для маркеров (с запасом за краем экрана,
        // чтобы кластеры не появлялись рывком), `visible` — для списка справа.
        let (mut near, mut visible) = (Vec::new(), Vec::new());
        for (i, p) in points.iter().enumerate() {
            if view.contains_world(p.x, p.y, CELL) {
                near.push(i);
                if view.contains_world(p.x, p.y, 0.0) {
                    visible.push(i);
                }
            }
        }
        let pts: Vec<(f64, f64)> = near.iter().map(|&i| (points[i].x, points[i].y)).collect();
        let clusters = {
            let _t = trace::span(format!("  кластеризация {} точек", pts.len()));
            geo::cluster_world(&pts, zoom, CELL)
        };
        let keys: Vec<ClusterKey> = clusters
            .iter()
            .map(|c| {
                let ids = c.members.iter().map(|&m| points[near[m]].id);
                (points[near[c.members[0]]].id, c.members.len(), members_hash(ids))
            })
            .collect();
        {
            let mut markers = self.markers.borrow_mut();
            if markers.len() != keys.len() || !keys.iter().all(|k| markers.contains_key(k)) {
                let _t = trace::span(format!("  маркеры: {} (было {})", keys.len(), markers.len()));
                let keep: HashSet<ClusterKey> = keys.iter().copied().collect();
                markers.retain(|key, marker| {
                    let stays = keep.contains(key);
                    if !stays {
                        self.layer.remove_marker(marker);
                    }
                    stays
                });
                for (c, key) in clusters.iter().zip(&keys) {
                    if markers.contains_key(key) {
                        continue;
                    }
                    let refs: Vec<Ref<Photo>> = c.members.iter().map(|&m| photos[near[m]].borrow()).collect();
                    let members: Vec<&Photo> = refs.iter().map(|r| &**r).collect();
                    let marker = self.make_marker(c.lat, c.lon, &members);
                    self.layer.add_marker(&marker);
                    markers.insert(*key, marker);
                }
            }
        }

        // Правый список: только видимые, по дате.
        let ids: Vec<i64> = visible.iter().map(|&i| points[i].id).collect();
        let visible_count = ids.len();
        if *self.visible_ids.borrow() != ids {
            let _t = trace::span(format!("  список: {} фото", ids.len()));
            let objects: Vec<glib::BoxedAnyObject> = visible.iter().map(|&i| photos[i].clone()).collect();
            // Выделенные фото, ушедшие из видимой области, из выделения выпадают.
            let selected = self.grid.selected_ids();
            {
                let _t = trace::span("    set_photos");
                self.grid.set_objects(objects);
            }
            if !selected.is_empty() {
                let _t = trace::span("    restore_selection");
                self.grid.restore_selection(&selected);
            }
            *self.visible_ids.borrow_mut() = ids;
        }
        drop(points);
        drop(photos);
        // Фото из «Найти на карте» — выделить, как только оно в видимой области; не дождались
        // (карту увели в сторону) — забыть, чтобы оно не выделилось когда-нибудь потом.
        if let Some((id, since)) = self.pending_select.get() {
            if self.visible_ids.borrow().contains(&id) {
                self.grid.select_id(id);
                self.pending_select.set(None);
            } else if since.elapsed() > PENDING_SELECT_TTL {
                self.pending_select.set(None);
            }
        }
        // Дерево фильтра: даты всех фото с координатами в видимой области.
        let counts = DateCount::tally(
            self.all_days.borrow().iter().filter(|&&(x, y, _)| view.contains_world(x, y, 0.0)).map(|&(_, _, d)| d),
        );
        if *self.shown_counts.borrow() != counts {
            *self.shown_counts.borrow_mut() = counts.clone();
            self.filter.set_counts(counts);
        }
        let total = self.photos.borrow().len();
        self.grid.set_empty_text(if total == 0 { i18n::no_geotagged() } else { i18n::no_photos_in_area() });
        self.info.set_text(&if total == 0 {
            i18n::no_geotagged().to_string()
        } else {
            i18n::in_map_area(visible_count, total)
        });
    }

    fn make_marker(self: &Rc<Self>, lat: f64, lon: f64, members: &[&Photo]) -> shumate::Marker {
        let cover = members[0];
        let image = gtk::Image::new();
        image.set_pixel_size(MARKER_SIZE);
        set_thumbnail(&self.cache, &image, cover.thumb_path.as_deref(), PHOTO_ICON, || true);
        image.add_css_class("map-thumb");
        image.set_overflow(gtk::Overflow::Hidden);
        // Место под бейдж в правом верхнем углу.
        image.set_margin_top(8);
        image.set_margin_end(8);

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&image));
        if members.len() > 1 {
            let badge = gtk::Label::new(Some(&members.len().to_string()));
            badge.add_css_class("map-badge");
            badge.set_halign(gtk::Align::End);
            badge.set_valign(gtk::Align::Start);
            overlay.add_overlay(&badge);
        }
        overlay.set_cursor_from_name(Some("pointer"));
        overlay.set_tooltip_text(Some(&if members.len() == 1 {
            format!("{}\n{}", cover.file_name(), cover.display_date())
        } else {
            format!(
                "{}\n{} — {}",
                i18n::photos(members.len()),
                members[0].display_date(),
                members[members.len() - 1].display_date()
            )
        }));

        let ids: Vec<i64> = members.iter().map(|p| p.id).collect();
        let pts: Vec<(f64, f64)> = members.iter().filter_map(|p| p.coords()).collect();
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(t) = weak.upgrade() {
                t.on_marker_clicked(&ids, &pts);
            }
        });
        overlay.add_controller(click);

        let marker = shumate::Marker::new();
        marker.set_child(Some(&overlay));
        marker.set_location(lat, lon);
        marker
    }

    fn on_marker_clicked(&self, ids: &[i64], pts: &[(f64, f64)]) {
        let zoom = self.viewport.zoom_level();
        let max = self.viewport.max_zoom_level() as f64;
        if ids.len() > 1 && zoom < max - 0.01 {
            // Кластер — приблизить так, чтобы все его фото поместились.
            let (w, h) = (self.simple_map.width() as f64, self.simple_map.height() as f64);
            let fit = geo::fit_zoom(pts, w, h, 80.0);
            let target = if fit.is_finite() && fit.floor() > zoom { fit.floor() } else { zoom.floor() + 2.0 };
            let target = target.min(max);
            if let Some((lat, lon)) = geo::center(pts) {
                self.map.go_to_full(lat, lon, target);
            }
            return;
        }
        // Одиночное фото (или максимальный зум) — выделить в списке. Маркеры рисуются и
        // чуть за краем карты: такого фото в списке ещё нет — карта едет к нему, и оно
        // выделится, когда попадёт в видимую область.
        if self.grid.select_id(ids[0]) {
            self.pending_select.set(None);
        } else if let Some(&(lat, lon)) = pts.first() {
            self.pending_select.set(Some((ids[0], Instant::now())));
            self.map.go_to(lat, lon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cluster_hash() {
        // У суммы id эти наборы совпадали.
        assert_ne!(members_hash([1, 4].into_iter()), members_hash([2, 3].into_iter()));
        assert_ne!(members_hash([1, 2, 6].into_iter()), members_hash([1, 3, 5].into_iter()));
        assert_eq!(members_hash([7, 8, 9].into_iter()), members_hash([7, 8, 9].into_iter()));
        assert_ne!(members_hash([7].into_iter()), members_hash([7, 0].into_iter()));
    }
}
