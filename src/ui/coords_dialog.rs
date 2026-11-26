//! Окно «Редактировать координаты»: метка всегда в центре карты, точку выбирают, двигая
//! карту под ней (перетаскивание, щелчок, поиск местности, «Моё местоположение»), или
//! вводят в поля широты и долготы.
use super::place_search::{self, PlaceSearch};
use crate::{config::MapView, db::Photo, geo, i18n, metadata};
use gtk::{gio, glib, prelude::*};
use shumate::prelude::*;
use std::{cell::Cell, rc::Rc};

/// Зум, с которым показывается точка фото.
const POINT_ZOOM: f64 = 15.0;
/// Сдвиг мыши между нажатием и отпусканием, после которого это уже не щелчок, px.
const CLICK_SLOP: f64 = 6.0;
/// Вид без точки и без сохранённого положения карты.
const WORLD: MapView = MapView { lat: 30.0, lon: 0.0, zoom: 2.0 };

/// Открыть редактор координат для `photos`. Для одного фото поля заполнены его
/// координатами (если есть), для нескольких — пустые. `view` — положение карты, если
/// у фото нет координат. По «Сохранить» (после подтверждения) окно закрывается
/// и вызывается `on_save(широта, долгота, фото)`.
pub fn show(
    parent: &gtk::Window,
    photos: Vec<Photo>,
    view: Option<MapView>,
    on_save: impl Fn(f64, f64, Vec<Photo>) + 'static,
) -> gtk::Window {
    let dialog = gtk::Window::builder()
        .title(i18n::coordinates_title())
        .transient_for(parent)
        .modal(true)
        .default_width(760)
        .default_height(600)
        .build();

    // Карта со слоем «я здесь» и меткой точки поверх центра.
    let map = shumate::SimpleMap::new();
    let registry = shumate::MapSourceRegistry::with_defaults();
    if let Some(src) = registry.by_id(shumate::MAP_SOURCE_OSM_MAPNIK) {
        map.set_map_source(Some(&src));
    }
    map.set_hexpand(true);
    map.set_vexpand(true);
    let viewport = map.viewport().expect("SimpleMap без Viewport");
    let location_layer = shumate::MarkerLayer::new(&viewport);
    map.add_overlay_layer(&location_layer);
    // Метка — красная капля (отличается от синей отметки «я здесь»); щелчки проходят к карте.
    let point = pin();
    let map_overlay = gtk::Overlay::new();
    map_overlay.set_child(Some(&map));
    map_overlay.add_overlay(&point);

    let search = PlaceSearch::new();
    search.entry.set_hexpand(true);
    let locate = place_search::locate_button();
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    toolbar.append(&search.entry);
    toolbar.append(&locate);

    let lat = coord_entry();
    let lon = coord_entry();
    let fields = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    fields.append(&field_label(i18n::latitude()));
    fields.append(&lat);
    fields.append(&field_label(i18n::longitude()));
    fields.append(&lon);
    let hint = gtk::Label::new(Some(i18n::coordinates_hint()));
    hint.add_css_class("dim-label");
    hint.set_xalign(0.0);
    hint.set_wrap(true);

    let cancel = gtk::Button::with_label(i18n::cancel());
    let save = gtk::Button::with_label(&i18n::save_count(photos.len()));
    save.add_css_class("suggested-action");
    save.set_sensitive(false);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::End);
    buttons.set_hexpand(true);
    buttons.append(&cancel);
    buttons.append(&save);
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bottom.append(&fields);
    bottom.append(&buttons);

    let frame = gtk::Frame::new(None);
    frame.set_child(Some(&map_overlay));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&toolbar);
    content.append(&frame);
    content.append(&hint);
    content.append(&bottom);
    dialog.set_child(Some(&content));
    dialog.set_default_widget(Some(&save));

    let state = Rc::new(State {
        map: map.clone(),
        lat: lat.clone(),
        lon: lon.clone(),
        save: save.clone(),
        syncing: Cell::new(false),
        ready: Cell::new(false),
    });

    // Поля — координаты единственного фото; без них пустые, пока карту не сдвинут.
    let initial = match photos.as_slice() {
        [p] => p.coords(),
        _ => None,
    };
    if let Some((la, lo)) = initial {
        state.fill(la, lo);
    }
    // Положение карты — когда у неё появится размер (иначе зум не применится).
    let start = initial.map(|(lat, lon)| MapView { lat, lon, zoom: POINT_ZOOM }).or(view).unwrap_or(WORLD);
    let s = Rc::downgrade(&state);
    map.connect_realize(move |_| {
        let s = s.clone();
        glib::idle_add_local_once(move || {
            let Some(s) = s.upgrade() else { return };
            if let (Some(map), Some(v)) = (s.map.map(), s.map.viewport()) {
                let (min, max) = (v.min_zoom_level() as f64, v.max_zoom_level() as f64);
                v.set_zoom_level(start.zoom.round().clamp(min, max));
                map.center_on(start.lat, start.lon);
            }
            // Дальше любое движение карты — выбор точки.
            s.ready.set(true);
        });
    });
    let s = Rc::downgrade(&state);
    viewport.connect_notify_local(None, move |v, p| {
        if let Some(s) = s.upgrade().filter(|_| matches!(p.name(), "latitude" | "longitude")) {
            s.center_moved(v.latitude(), v.longitude());
        }
    });

    // Щелчок переносит точку щелчка в центр — под метку. Перетаскивание просто двигает карту:
    // при сдвиге мыши щелчок не засчитывается.
    // Фаза захвата — раньше жестов самой карты, которые иначе забирают щелчок.
    let click = gtk::GestureClick::new();
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    // После перетаскивания GTK тоже присылает `released` — щелчок засчитывается, только
    // если кнопку отпустили там же, где нажали.
    let pressed_at = Rc::new(Cell::new(None));
    let p = pressed_at.clone();
    click.connect_pressed(move |_, _, x, y| p.set(Some((x, y))));
    let s = Rc::downgrade(&state);
    click.connect_released(move |g, n, x, y| {
        let (Some(s), Some(w)) = (s.upgrade(), g.widget()) else { return };
        let Some((px, py)) = pressed_at.take() else { return };
        if n != 1 || (x - px).hypot(y - py) > CLICK_SLOP {
            return;
        }
        if let (Some(v), Some(m)) = (s.map.viewport(), s.map.map()) {
            let (la, lo) = v.widget_coords_to_location(&w, x, y);
            m.go_to(la, lo);
        }
    });
    // На самой карте, а не на SimpleMap: щелчки по кнопкам зума карту не сдвигают.
    map.map().expect("SimpleMap без Map").add_controller(click);

    // Ввод в поля двигает карту; пара «широта, долгота», вставленная в одно поле, разбирается.
    for entry in [&lat, &lon] {
        let s = Rc::downgrade(&state);
        entry.connect_changed(move |e| {
            let Some(s) = s.upgrade() else { return };
            if s.syncing.get() {
                return;
            }
            if let Some((la, lo)) = geo::parse_coord_pair(&e.text()) {
                s.fill(la, lo);
            }
            if let Some((la, lo)) = s.value() {
                s.center_on(la, lo);
            }
            s.save.set_sensitive(s.value().is_some());
        });
    }

    // Найденное место и местоположение — переход; точка — новый центр карты.
    let s = Rc::downgrade(&state);
    search.connect_place(move |place| {
        if let Some(s) = s.upgrade() {
            place_search::go_to_place(&s.map, place);
        }
    });
    let (s, layer) = (Rc::downgrade(&state), location_layer.clone());
    locate.connect_clicked(move |b| {
        let (s, layer) = (s.clone(), layer.clone());
        place_search::locate(b, move |fix| {
            let Some(s) = s.upgrade() else { return };
            layer.remove_all();
            layer.add_marker(&place_search::location_marker(&fix));
            place_search::go_to_location(&s.map, &fix);
        });
    });

    let d = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });
    let keys = gtk::EventControllerKey::new();
    let d = dialog.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(d) = d.upgrade() {
                d.close();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(keys);

    let (d, s) = (dialog.downgrade(), Rc::downgrade(&state));
    let on_save = Rc::new(on_save);
    save.connect_clicked(move |_| {
        let (Some(d), Some(s)) = (d.upgrade(), s.upgrade()) else { return };
        let Some((la, lo)) = s.value() else { return };
        let photos = photos.clone();
        let on_save = on_save.clone();
        let (message, detail, button) = match photos.as_slice() {
            [p] if p.coords().is_none() => {
                d.close();
                on_save(la, lo, photos);
                return;
            }
            [p] => (
                i18n::confirm_update_coordinates().to_string(),
                i18n::confirm_update_coordinates_detail(p.file_name()),
                i18n::update().to_string(),
            ),
            many => (
                i18n::confirm_save_coordinates(many.len()),
                i18n::confirm_save_coordinates_detail().to_string(),
                i18n::save().to_string(),
            ),
        };
        let confirm = gtk::AlertDialog::builder()
            .modal(true)
            .message(message)
            .detail(detail)
            .buttons([i18n::cancel(), button.as_str()])
            .cancel_button(0)
            .default_button(1)
            .build();
        let dw = d.downgrade();
        confirm.choose(Some(&d), gio::Cancellable::NONE, move |res| {
            if res == Ok(1) {
                if let Some(d) = dw.upgrade() {
                    d.close();
                }
                on_save(la, lo, photos);
            }
        });
    });

    // Обработчики держат состояние слабо; сильная ссылка — у окна, до его уничтожения
    // (при закрытии GTK отключает обработчики, и состояние освобождается).
    let keep = state.clone();
    dialog.connect_destroy(move |_| {
        let _ = &keep;
    });

    dialog.present();
    dialog
}

/// Виджеты, которые меняются вместе: карта, поля и доступность «Сохранить».
struct State {
    map: shumate::SimpleMap,
    lat: gtk::Entry,
    lon: gtk::Entry,
    save: gtk::Button,
    /// Поля заполняются из программы — не разбирать их как ввод.
    syncing: Cell<bool>,
    /// Начальное положение выставлено: движение карты — выбор точки.
    ready: Cell<bool>,
}

impl State {
    /// Записать координаты в поля.
    fn fill(&self, lat: f64, lon: f64) {
        self.syncing.set(true);
        self.lat.set_text(&geo::format_coord(lat));
        self.lon.set_text(&geo::format_coord(lon));
        self.syncing.set(false);
        self.save.set_sensitive(self.value().is_some());
    }

    /// Центр карты сдвинулся: точка под меткой — в поля. Если центр уже совпадает с полями
    /// (карту сдвинул ввод), поля не трогать — иначе вводимый текст переформатировался бы.
    fn center_moved(&self, lat: f64, lon: f64) {
        if !self.ready.get() {
            return;
        }
        // Карта повторяется по долготе — вернуть её в −180…180.
        let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
        let same = self.value().is_some_and(|(a, b)| (a - lat).abs() < 1e-7 && (b - lon).abs() < 1e-7);
        if !same {
            self.fill(lat, lon);
        }
    }

    fn center_on(&self, lat: f64, lon: f64) {
        if let Some(m) = self.map.map() {
            m.center_on(lat, lon);
        }
    }

    /// Координаты из полей, если они допустимы.
    fn value(&self) -> Option<(f64, f64)> {
        let lat = geo::parse_coord(&self.lat.text())?;
        let lon = geo::parse_coord(&self.lon.text())?;
        metadata::valid_coords(lat, lon).then_some((lat, lon))
    }
}

/// Размер капли-метки, px.
const PIN_W: i32 = 30;
const PIN_H: i32 = 42;

/// Красная капля с белой точкой. Остриё — точка на карте: снизу капли пустое поле той же
/// высоты, и при выравнивании по центру остриё приходится ровно на центр карты.
fn pin() -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(PIN_W);
    area.set_content_height(PIN_H);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.set_margin_bottom(PIN_H);
    area.set_can_target(false);
    area.set_draw_func(|_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let r = w / 2.0 - 2.5;
        let (cx, cy) = (w / 2.0, r + 2.5);
        let tip = h - 1.5;
        // Касательные из острия к окружности: половина угла при острие.
        let phi = (r / (tip - cy)).asin();
        let drop = |cr: &gtk::cairo::Context| {
            cr.move_to(cx, tip);
            cr.arc(cx, cy, r, std::f64::consts::PI - phi, 2.0 * std::f64::consts::PI + phi);
            cr.close_path();
        };
        // Тень.
        cr.save().ok();
        cr.translate(0.0, 1.0);
        drop(cr);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        cr.fill().ok();
        cr.restore().ok();
        drop(cr);
        cr.set_source_rgb(0.878, 0.106, 0.141);
        cr.fill_preserve().ok();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(2.0);
        cr.stroke().ok();
        cr.arc(cx, cy, r * 0.38, 0.0, 2.0 * std::f64::consts::PI);
        cr.fill().ok();
    });
    area
}

fn coord_entry() -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_width_chars(12);
    e.set_activates_default(true);
    e.set_input_purpose(gtk::InputPurpose::Number);
    e
}

fn field_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l
}
