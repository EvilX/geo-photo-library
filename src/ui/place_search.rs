//! Поиск местности (Nominatim) и «Моё местоположение» — над картой вкладки и в окне
//! редактирования координат.
use crate::{
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

/// Nominatim разрешает не больше одного запроса в секунду — на всю программу.
const SEARCH_INTERVAL: Duration = Duration::from_secs(1);
/// Зум для найденной точки без границ.
const PLACE_ZOOM: f64 = 15.0;
/// Предельный зум для найденного места: мелкий объект показывается с окрестностями.
const PLACE_MAX_ZOOM: f64 = 17.0;
/// Предельный зум для текущего местоположения: точнее улицы не нужно.
const LOCATION_MAX_ZOOM: f64 = 16.0;

thread_local! {
    /// Время последнего запроса к Nominatim (общее для всех строк поиска).
    static LAST_REQUEST: Cell<Option<Instant>> = const { Cell::new(None) };
}

type PlaceHandler = Box<dyn Fn(&Place)>;

/// Строка поиска и всплывающий список найденного. Запрос — по Enter, без автодополнения.
pub struct PlaceSearch {
    pub entry: gtk::SearchEntry,
    popover: gtk::Popover,
    list: gtk::ListBox,
    places: RefCell<Vec<Place>>,
    session: soup::Session,
    /// Номер последнего запроса — ответы на устаревшие запросы отбрасываются.
    generation: Cell<u64>,
    on_place: RefCell<Option<PlaceHandler>>,
}

impl PlaceSearch {
    pub fn new() -> Rc<Self> {
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
        // Привязанный вручную popover нужно и отвязать вручную, иначе GTK при
        // разрушении строки поиска предупреждает об оставшихся дочерних виджетах.
        let pop = popover.clone();
        entry.connect_destroy(move |_| pop.unparent());

        let this = Rc::new(Self {
            entry,
            popover,
            list,
            places: RefCell::default(),
            session: geocode::session(),
            generation: Cell::new(0),
            on_place: RefCell::default(),
        });
        let weak = Rc::downgrade(&this);
        this.entry.connect_activate(move |e| {
            if let Some(t) = weak.upgrade() {
                t.search(e.text().to_string());
            }
        });
        let weak = Rc::downgrade(&this);
        this.entry.connect_stop_search(move |_| {
            if let Some(t) = weak.upgrade() {
                t.popover.popdown();
            }
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_row_activated(move |_, row| {
            let Some(t) = weak.upgrade() else { return };
            // Строки списка — найденные места по порядку (сообщение показывается без мест).
            let place = usize::try_from(row.index()).ok().and_then(|i| t.places.borrow().get(i).cloned());
            if let Some(place) = place {
                t.popover.popdown();
                t.picked(&place);
            }
        });
        this
    }

    /// Вызывать `f` для выбранного места (единственное найденное выбирается сразу).
    pub fn connect_place(&self, f: impl Fn(&Place) + 'static) {
        *self.on_place.borrow_mut() = Some(Box::new(f));
    }

    fn picked(&self, place: &Place) {
        if let Some(f) = self.on_place.borrow().as_ref() {
            f(place);
        }
    }

    /// Найти место через геокодер; одно найденное — сразу выбрать его.
    pub fn search(self: &Rc<Self>, query: String) {
        if query.trim().is_empty() {
            return;
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let wait = LAST_REQUEST
            .get()
            .map(|t| SEARCH_INTERVAL.saturating_sub(t.elapsed()))
            .unwrap_or_default();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            if !wait.is_zero() {
                glib::timeout_future(wait).await;
            }
            let Some(t) = weak.upgrade() else { return };
            if t.generation.get() != generation {
                return;
            }
            // Запрос идёт секунды — в списке под строкой видно, что поиск начат.
            t.show(Vec::new(), Some((i18n::searching_place(), true)));
            LAST_REQUEST.set(Some(Instant::now()));
            let session = t.session.clone();
            drop(t);
            let result = {
                let _t = trace::span(format!("поиск местности «{query}»"));
                geocode::search(&session, &query, i18n::lang_code()).await
            };
            let Some(t) = weak.upgrade() else { return };
            if t.generation.get() != generation {
                return;
            }
            match result {
                Ok(places) if places.len() == 1 => {
                    t.popover.popdown();
                    t.picked(&places[0]);
                }
                Ok(places) if places.is_empty() => t.show(places, Some((i18n::place_not_found(), false))),
                Ok(places) => t.show(places, None),
                Err(e) => {
                    eprintln!("Поиск местности: {e:#}");
                    t.show(Vec::new(), Some((i18n::place_search_failed(), false)));
                }
            }
        });
    }

    /// Показать в списке найденные места либо (без мест) сообщение; `true` при нём —
    /// с индикатором: запрос ещё идёт.
    fn show(&self, places: Vec<Place>, message: Option<(&str, bool)>) {
        self.list.remove_all();
        if let Some((text, busy)) = message {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("dim-label");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.set_margin_top(8);
            line.set_margin_bottom(8);
            line.set_margin_start(8);
            line.set_margin_end(8);
            if busy {
                let spinner = gtk::Spinner::new();
                spinner.start();
                line.append(&spinner);
            }
            line.append(&label);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&line));
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

/// Показать найденное место целиком (по его границам) или точку крупным планом.
pub fn go_to_place(map: &shumate::SimpleMap, place: &Place) {
    let (Some(m), Some(viewport)) = (map.map(), map.viewport()) else { return };
    let (w, h) = (map.width() as f64, map.height() as f64);
    let pts = place.corners();
    let max = viewport.max_zoom_level() as f64;
    let fit = geo::fit_zoom(&pts, w, h, 40.0);
    // Целый зум: на дробном у тайлов видны швы.
    let zoom = if fit.is_finite() { fit.floor() } else { PLACE_ZOOM }.clamp(1.0, PLACE_MAX_ZOOM.min(max));
    let (lat, lon) = geo::center(&pts).unwrap_or((place.lat, place.lon));
    m.go_to_full(lat, lon, zoom);
}

/// Показать текущее местоположение с окрестностями по его точности.
pub fn go_to_location(map: &shumate::SimpleMap, fix: &location::Fix) {
    let (Some(m), Some(viewport)) = (map.map(), map.viewport()) else { return };
    // Круг точности целиком в кадре: 1° широты ≈ 111 км.
    let d = fix.accuracy.max(50.0) / 111_320.0;
    let d_lon = d / fix.lat.to_radians().cos().max(0.01);
    let corners = [(fix.lat - d, fix.lon - d_lon), (fix.lat + d, fix.lon + d_lon)];
    let (w, h) = (map.width() as f64, map.height() as f64);
    let max = viewport.max_zoom_level() as f64;
    let zoom = geo::fit_zoom(&corners, w, h, 40.0);
    let zoom = if zoom.is_finite() { zoom.floor() } else { LOCATION_MAX_ZOOM }.clamp(3.0, LOCATION_MAX_ZOOM.min(max));
    m.go_to_full(fix.lat, fix.lon, zoom);
}

/// Кнопка «Моё местоположение». Местоположение — через портал D-Bus (GeoClue),
/// на Windows его нет, и кнопка скрыта.
pub fn locate_button() -> gtk::Button {
    let button = gtk::Button::from_icon_name("find-location-symbolic");
    button.set_tooltip_text(Some(i18n::my_location()));
    super::a11y_label(&button, i18n::my_location());
    button.add_css_class("map-search");
    button.set_visible(cfg!(not(windows)));
    button
}

/// Спросить у портала геолокации текущее местоположение и отдать его `on_fix`; ошибку
/// показать окном. Пока идёт поиск, кнопка неактивна и вместо значка крутится индикатор.
pub fn locate(button: &gtk::Button, on_fix: impl FnOnce(location::Fix) + 'static) {
    let spinner = gtk::Spinner::new();
    spinner.start();
    button.set_child(Some(&spinner));
    button.set_sensitive(false);
    let weak = button.downgrade();
    glib::spawn_future_local(async move {
        let result = {
            let _t = trace::span("местоположение");
            location::current().await
        };
        let Some(button) = weak.upgrade() else { return };
        button.set_icon_name("find-location-symbolic");
        button.set_sensitive(true);
        match result {
            Ok(fix) => on_fix(fix),
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
                dialog.show(button.root().and_downcast_ref::<gtk::Window>());
            }
        }
    });
}

/// Отметка «я здесь» на слое карты.
pub fn location_marker(fix: &location::Fix) -> shumate::Marker {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("my-location");
    dot.set_tooltip_text(Some(i18n::my_location()));
    let marker = shumate::Marker::new();
    marker.set_child(Some(&dot));
    marker.set_location(fix.lat, fix.lon);
    marker
}
