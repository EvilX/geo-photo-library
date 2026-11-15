//! Вкладка «Карта»: OSM-карта с превью-маркерами (кластеризация) и список фото
//! в видимой области, отсортированный по дате.
use super::{
    photo_grid::{self, PhotoGrid},
    textures::{set_thumbnail, TextureCache},
};
use crate::{
    config::{MapView, SortOrder},
    db::{Db, Photo},
    geo,
    geocode::{self, Place},
    i18n, location, trace,
};
use gtk::{glib, prelude::*};
use shumate::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

const MARKER_SIZE: i32 = 56;
/// Размер ячейки кластеризации в пикселях.
const CELL: f64 = 72.0;
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Nominatim разрешает не больше одного запроса в секунду.
const SEARCH_INTERVAL: Duration = Duration::from_secs(1);
/// Зум для найденной точки без границ.
const PLACE_ZOOM: f64 = 15.0;
/// Зум для пункта «Найти на карте» — уровень улиц.
const PHOTO_ZOOM: f64 = 16.0;
/// Предельный зум для текущего местоположения: точнее улицы не нужно.
const LOCATION_MAX_ZOOM: f64 = 16.0;
/// Предельный зум для найденного места: мелкий объект показывается с окрестностями.
const PLACE_MAX_ZOOM: f64 = 17.0;

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
    /// Все фото с координатами, по дате.
    photos: RefCell<Vec<Photo>>,
    /// id фото в правом списке — чтобы не перестраивать его без изменений.
    visible_ids: RefCell<Vec<i64>>,
    /// Подпись текущего набора кластеров (cover id, количество).
    cluster_sig: RefCell<Vec<(i64, usize)>>,
    pending: RefCell<Option<glib::SourceId>>,
    /// Не центрировать карту при программном изменении выделения.
    suppress_center: Cell<bool>,
    fitted: Cell<bool>,
    last_size: Cell<(i32, i32)>,
    search: Search,
    /// Кнопка «Моё местоположение» и слой с отметкой найденной точки.
    locate: gtk::Button,
    location_layer: shumate::MarkerLayer,
    /// Фото, которое выделить в списке, когда оно появится в видимой области
    /// (после перехода «Найти на карте»).
    pending_select: Cell<Option<i64>>,
}

/// Поиск местности: строка над картой и всплывающий список найденного.
struct Search {
    entry: gtk::SearchEntry,
    popover: gtk::Popover,
    list: gtk::ListBox,
    places: RefCell<Vec<Place>>,
    session: soup::Session,
    /// Номер последнего запроса — ответы на устаревшие запросы отбрасываются.
    generation: Cell<u64>,
    last_request: Cell<Option<Instant>>,
}

impl Search {
    fn new() -> Self {
        let entry = gtk::SearchEntry::new();
        entry.set_placeholder_text(Some(i18n::search_place()));
        entry.set_width_chars(30);
        entry.add_css_class("map-search");

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(360)
            .child(&list)
            .build();
        let popover = gtk::Popover::new();
        popover.set_child(Some(&scroll));
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_parent(&entry);

        Self {
            entry,
            popover,
            list,
            places: RefCell::default(),
            session: geocode::session(),
            generation: Cell::new(0),
            last_request: Cell::new(None),
        }
    }

    /// Показать в списке найденные места либо (без мест) сообщение.
    fn show(&self, places: Vec<Place>, message: Option<&str>) {
        self.list.remove_all();
        if let Some(text) = message {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("dim-label");
            label.set_margin_top(8);
            label.set_margin_bottom(8);
            label.set_margin_start(8);
            label.set_margin_end(8);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&label));
            row.set_activatable(false);
            self.list.append(&row);
        }
        for p in &places {
            let label = gtk::Label::new(Some(&p.name));
            label.set_xalign(0.0);
            label.set_wrap(true);
            // Иначе метка с переносом сжимается до самого длинного слова.
            label.set_width_chars(40);
            label.set_max_width_chars(50);
            label.set_margin_top(6);
            label.set_margin_bottom(6);
            label.set_margin_start(6);
            label.set_margin_end(6);
            self.list.append(&label);
        }
        *self.places.borrow_mut() = places;
        self.popover.popup();
    }
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

        let search = Search::new();
        let locate = gtk::Button::from_icon_name("find-location-symbolic");
        locate.set_tooltip_text(Some(i18n::my_location()));
        locate.add_css_class("map-search");
        // Строка поиска и кнопка местоположения — в левом верхнем углу карты.
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.set_halign(gtk::Align::Start);
        toolbar.set_valign(gtk::Align::Start);
        toolbar.set_margin_top(10);
        toolbar.set_margin_start(10);
        toolbar.append(&search.entry);
        toolbar.append(&locate);
        let map_overlay = gtk::Overlay::new();
        map_overlay.set_child(Some(&simple_map));
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
            visible_ids: RefCell::default(),
            cluster_sig: RefCell::default(),
            pending: RefCell::default(),
            suppress_center: Cell::new(false),
            fitted: Cell::new(false),
            last_size: Cell::new((0, 0)),
            search,
            locate,
            location_layer,
            pending_select: Cell::new(None),
        });

        let weak = Rc::downgrade(&this);
        this.locate.connect_clicked(move |_| {
            if let Some(t) = weak.upgrade() {
                t.locate_me();
            }
        });

        // Поиск местности: запрос по Enter, переход по выбору из списка.
        let weak = Rc::downgrade(&this);
        this.search.entry.connect_activate(move |e| {
            if let Some(t) = weak.upgrade() {
                t.search_place(e.text().to_string());
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.entry.connect_stop_search(move |_| {
            if let Some(t) = weak.upgrade() {
                t.search.popover.popdown();
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.list.connect_row_activated(move |_, row| {
            let Some(t) = weak.upgrade() else { return };
            // Строки списка — найденные места по порядку (сообщение показывается без мест).
            let place = usize::try_from(row.index())
                .ok()
                .and_then(|i| t.search.places.borrow().get(i).cloned());
            if let Some(place) = place {
                t.search.popover.popdown();
                t.go_to_place(&place);
            }
        });

        // Любое изменение зума/центра — пересчитать (с задержкой).
        let weak = Rc::downgrade(&this);
        this.viewport.connect_notify_local(None, move |_, _| {
            if let Some(t) = weak.upgrade() {
                t.schedule_refresh();
            }
        });
        // Изменение размера виджета (переключение вкладки, перетаскивание Paned).
        let weak = Rc::downgrade(&this);
        this.simple_map.add_tick_callback(move |w, _| {
            if let Some(t) = weak.upgrade() {
                let size = (w.width(), w.height());
                if size != t.last_size.get() {
                    t.last_size.set(size);
                    if !t.fitted.get() {
                        t.fit_all();
                    }
                    t.schedule_refresh();
                }
            }
            glib::ControlFlow::Continue
        });
        // Выбор фото в списке — центрировать карту на нём.
        let weak = Rc::downgrade(&this);
        this.grid.selection.connect_selected_notify(move |_| {
            let Some(t) = weak.upgrade() else { return };
            if t.suppress_center.get() {
                return;
            }
            if let Some((lat, lon)) = t.grid.selected_photo().and_then(|p| p.coords()) {
                t.map.go_to(lat, lon);
            }
        });
        this
    }

    /// Ввести запрос в строку поиска и выполнить его (отладочный хук).
    pub fn search_for(self: &Rc<Self>, query: &str) {
        self.search.entry.set_text(query);
        self.search_place(query.to_string());
    }

    /// Найти место через геокодер; одно найденное — сразу перейти к нему.
    fn search_place(self: &Rc<Self>, query: String) {
        if query.trim().is_empty() {
            return;
        }
        let generation = self.search.generation.get() + 1;
        self.search.generation.set(generation);
        let wait = self
            .search
            .last_request
            .get()
            .map(|t| SEARCH_INTERVAL.saturating_sub(t.elapsed()))
            .unwrap_or_default();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            if !wait.is_zero() {
                glib::timeout_future(wait).await;
            }
            let Some(t) = weak.upgrade() else { return };
            if t.search.generation.get() != generation {
                return;
            }
            t.search.last_request.set(Some(Instant::now()));
            let session = t.search.session.clone();
            drop(t);
            let result = {
                let _t = trace::span(format!("поиск местности «{query}»"));
                geocode::search(&session, &query, i18n::lang_code()).await
            };
            let Some(t) = weak.upgrade() else { return };
            if t.search.generation.get() != generation {
                return;
            }
            match result {
                Ok(places) if places.len() == 1 => {
                    t.search.popover.popdown();
                    t.go_to_place(&places[0]);
                }
                Ok(places) if places.is_empty() => t.search.show(places, Some(i18n::place_not_found())),
                Ok(places) => t.search.show(places, None),
                Err(e) => {
                    eprintln!("Поиск местности: {e:#}");
                    t.search.show(Vec::new(), Some(i18n::place_search_failed()));
                }
            }
        });
    }

    /// Показать найденное место целиком (по его границам) или точку крупным планом.
    fn go_to_place(&self, place: &Place) {
        let (w, h) = (self.simple_map.width() as f64, self.simple_map.height() as f64);
        let pts = place.corners();
        let max = self.viewport.max_zoom_level() as f64;
        let fit = geo::fit_zoom(&pts, w, h, 40.0);
        // Целый зум: на дробном у тайлов видны швы.
        let zoom = if fit.is_finite() { fit.floor() } else { PLACE_ZOOM }.clamp(1.0, PLACE_MAX_ZOOM.min(max));
        let (lat, lon) = geo::center(&pts).unwrap_or((place.lat, place.lon));
        self.map.go_to_full(lat, lon, zoom);
    }

    /// Сменить сортировку списка, не сдвигая карту к выбранному фото.
    pub fn set_order(&self, order: SortOrder) {
        self.suppress_center.set(true);
        self.grid.set_order(order);
        self.suppress_center.set(false);
    }

    pub fn grid(&self) -> &PhotoGrid {
        &self.grid
    }

    pub fn set_thumb_scale(&self, percent: u32) {
        self.grid.set_scale(percent);
    }

    /// «Моё местоположение»: спросить у портала геолокации, перейти и отметить точку.
    /// Пока идёт поиск, кнопка неактивна и вместо значка крутится индикатор.
    pub fn locate_me(self: &Rc<Self>) {
        let spinner = gtk::Spinner::new();
        spinner.start();
        self.locate.set_child(Some(&spinner));
        self.locate.set_sensitive(false);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = {
                let _t = trace::span("местоположение");
                location::current().await
            };
            let Some(t) = weak.upgrade() else { return };
            t.locate.set_icon_name("find-location-symbolic");
            t.locate.set_sensitive(true);
            match result {
                Ok(fix) => t.show_location(fix),
                Err(e) => {
                    eprintln!("Местоположение: {e}");
                    let detail = match e {
                        location::Error::Denied => i18n::location_denied(),
                        location::Error::Unavailable(_) => i18n::location_unavailable(),
                    };
                    let dialog = gtk::AlertDialog::builder()
                        .modal(true)
                        .message(i18n::location_failed())
                        .detail(detail)
                        .buttons([i18n::ok()])
                        .build();
                    dialog.show(t.root.root().and_downcast_ref::<gtk::Window>());
                }
            }
        });
    }

    /// Отметить найденное местоположение и показать его с окрестностями по точности.
    fn show_location(&self, fix: location::Fix) {
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("my-location");
        dot.set_tooltip_text(Some(i18n::my_location()));
        let marker = shumate::Marker::new();
        marker.set_child(Some(&dot));
        marker.set_location(fix.lat, fix.lon);
        self.location_layer.remove_all();
        self.location_layer.add_marker(&marker);

        // Круг точности целиком в кадре: 1° широты ≈ 111 км.
        let d = fix.accuracy.max(50.0) / 111_320.0;
        let d_lon = d / fix.lat.to_radians().cos().max(0.01);
        let corners = [(fix.lat - d, fix.lon - d_lon), (fix.lat + d, fix.lon + d_lon)];
        let (w, h) = (self.simple_map.width() as f64, self.simple_map.height() as f64);
        let max = self.viewport.max_zoom_level() as f64;
        let zoom = geo::fit_zoom(&corners, w, h, 40.0);
        let zoom = if zoom.is_finite() { zoom.floor() } else { LOCATION_MAX_ZOOM }.clamp(3.0, LOCATION_MAX_ZOOM.min(max));
        self.fitted.set(true);
        self.map.go_to_full(fix.lat, fix.lon, zoom);
    }

    /// Перейти к месту съёмки фото и выделить его в списке. `false` — у фото нет координат.
    pub fn show_photo(self: &Rc<Self>, photo: &Photo) -> bool {
        let Some((lat, lon)) = photo.coords() else { return false };
        // Сохранённое положение или автоподгонка больше не нужны.
        self.fitted.set(true);
        self.pending_select.set(Some(photo.id));
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
        self.photos.borrow().iter().find_map(Photo::coords)
    }

    /// Перечитать фото с координатами. Без `force` маркеры и список перестраиваются,
    /// только если изменился их состав.
    pub fn reload(self: &Rc<Self>, force: bool) {
        let _t = trace::span("map.reload");
        let photos = self.db.geotagged().unwrap_or_else(|e| {
            eprintln!("БД: {e:#}");
            Vec::new()
        });
        *self.photos.borrow_mut() = photos;
        if force {
            self.cluster_sig.borrow_mut().clear();
            self.visible_ids.borrow_mut().clear();
        }
        if !self.fitted.get() {
            self.fit_all();
        }
        self.schedule_refresh();
    }

    /// Показать все фото с координатами. Работает, когда виджет уже имеет размер.
    fn fit_all(&self) {
        let (w, h) = (self.simple_map.width() as f64, self.simple_map.height() as f64);
        if w < 2.0 || h < 2.0 {
            return;
        }
        let pts: Vec<(f64, f64)> = self.photos.borrow().iter().filter_map(Photo::coords).collect();
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

        // Маркеры: точки с запасом за краем экрана, чтобы кластеры не появлялись рывком.
        let near: Vec<&Photo> = photos
            .iter()
            .filter(|p| p.coords().is_some_and(|(la, lo)| view.contains(la, lo, CELL)))
            .collect();
        let pts: Vec<(f64, f64)> = near.iter().filter_map(|p| p.coords()).collect();
        let clusters = {
            let _t = trace::span(format!("  кластеризация {} точек", pts.len()));
            geo::cluster(&pts, zoom, CELL)
        };
        let sig: Vec<(i64, usize)> = clusters.iter().map(|c| (near[c.members[0]].id, c.members.len())).collect();
        if *self.cluster_sig.borrow() != sig {
            let _t = trace::span(format!("  маркеры: {}", clusters.len()));
            self.layer.remove_all();
            for c in &clusters {
                let members: Vec<&Photo> = c.members.iter().map(|&i| near[i]).collect();
                self.layer.add_marker(&self.make_marker(c.lat, c.lon, &members));
            }
            *self.cluster_sig.borrow_mut() = sig;
        }

        // Правый список: только видимые, по дате.
        let visible: Vec<Photo> = photos
            .iter()
            .filter(|p| p.coords().is_some_and(|(la, lo)| view.contains(la, lo, 0.0)))
            .cloned()
            .collect();
        drop(photos);
        let ids: Vec<i64> = visible.iter().map(|p| p.id).collect();
        if *self.visible_ids.borrow() != ids {
            let _t = trace::span(format!("  список: {} фото", ids.len()));
            let selected = self.grid.selected_photo().map(|p| p.id);
            self.suppress_center.set(true);
            {
                let _t = trace::span("    set_photos");
                self.grid.set_photos(&visible);
            }
            if let Some(id) = selected {
                let _t = trace::span("    select_id");
                self.grid.select_id(id);
            }
            self.suppress_center.set(false);
            *self.visible_ids.borrow_mut() = ids;
        }
        // Фото из «Найти на карте» — выделить, как только оно в видимой области.
        if let Some(id) = self.pending_select.get().filter(|id| self.visible_ids.borrow().contains(id)) {
            self.suppress_center.set(true);
            self.grid.select_id(id);
            self.suppress_center.set(false);
            self.pending_select.set(None);
        }
        let total = self.photos.borrow().len();
        self.info.set_text(&if total == 0 {
            i18n::no_geotagged().to_string()
        } else {
            i18n::in_map_area(visible.len(), total)
        });
    }

    fn make_marker(self: &Rc<Self>, lat: f64, lon: f64, members: &[&Photo]) -> shumate::Marker {
        let cover = members[0];
        let image = gtk::Image::new();
        image.set_pixel_size(MARKER_SIZE);
        set_thumbnail(&self.cache, &image, cover.thumb_path.as_deref(), || true);
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
        // Одиночное фото (или максимальный зум) — выделить в списке.
        self.suppress_center.set(true);
        self.grid.select_id(ids[0]);
        self.suppress_center.set(false);
    }
}
