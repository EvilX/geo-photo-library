//! Вкладка «Карта»: OSM-карта с превью-маркерами (кластеризация) и список фото
//! в видимой области, отсортированный по дате.
use super::{
    photo_grid::{self, PhotoGrid},
    textures::{set_thumbnail, TextureCache},
};
use crate::{
    config::SortOrder,
    db::{Db, Photo},
    geo, i18n, trace,
};
use gtk::{glib, prelude::*};
use shumate::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const MARKER_SIZE: i32 = 56;
/// Размер ячейки кластеризации в пикселях.
const CELL: f64 = 72.0;
const DEBOUNCE: Duration = Duration::from_millis(150);

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

        let root = gtk::Paned::new(gtk::Orientation::Horizontal);
        root.set_start_child(Some(&simple_map));
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

    /// Мгновенно (без анимации) перейти к точке — для бенчмарка.
    pub fn jump_to(&self, lat: f64, lon: f64, zoom: f64) {
        self.viewport.set_zoom_level(zoom);
        self.map.center_on(lat, lon);
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
