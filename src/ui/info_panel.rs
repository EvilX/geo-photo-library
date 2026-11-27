//! Панель «Информация» справа от вкладок: данные файла и EXIF выбранной фотографии.
//! EXIF читается в фоне; пока панель скрыта, ничего не читается.
use crate::{
    db::Photo,
    i18n::{self, Info},
    metadata::{self, ExifDetails},
    video,
};
use gtk::{gio, glib, pango, prelude::*};
use std::{cell::Cell, path::Path, rc::Rc};

const WIDTH: i32 = 320;

pub struct InfoPanel {
    pub root: gtk::Box,
    pub close: gtk::Button,
    content: gtk::Box,
    /// id показанного фото — чтобы не перечитывать EXIF при том же выборе.
    shown: Cell<Option<i64>>,
    /// Номер последнего чтения — результаты устаревших чтений отбрасываются.
    generation: Cell<u64>,
}

/// Что прочитано с диска в фоне.
struct Loaded {
    size: Option<u64>,
    exif: Option<ExifDetails>,
}

impl InfoPanel {
    pub fn new() -> Rc<Self> {
        let title = gtk::Label::new(Some(i18n::info()));
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.add_css_class("pane-title");
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.set_valign(gtk::Align::Center);
        close.set_margin_end(6);
        close.set_tooltip_text(Some(i18n::close()));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.append(&title);
        header.append(&close);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();

        let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
        inner.append(&header);
        inner.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        inner.append(&scroll);
        inner.set_size_request(WIDTH, -1);
        inner.set_hexpand(false);

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        root.append(&inner);
        // По умолчанию панель скрыта; открывается пунктом «Информация» меню фото.
        root.set_visible(false);

        Rc::new(Self { root, close, content, shown: Cell::new(None), generation: Cell::new(0) })
    }

    pub fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    pub fn open(&self) {
        self.root.set_visible(true);
    }

    pub fn hide(&self) {
        self.root.set_visible(false);
        // При следующем открытии данные перечитываются — файл мог измениться.
        self.shown.set(None);
        self.generation.set(self.generation.get() + 1);
    }

    /// Показать данные фото (`None` — ничего не выбрано). EXIF читается в фоне.
    pub fn show_photo(self: &Rc<Self>, photo: Option<Photo>) {
        let id = photo.as_ref().map(|p| p.id);
        if id.is_some() && self.shown.get() == id {
            return;
        }
        self.shown.set(id);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let Some(photo) = photo else {
            self.set_placeholder(i18n::select_photo());
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let path = photo.path.clone();
            let loaded = gio::spawn_blocking(move || {
                let path = Path::new(&path);
                Loaded { size: std::fs::metadata(path).ok().map(|m| m.len()), exif: metadata::read_details(path) }
            })
            .await;
            let Some(t) = weak.upgrade() else { return };
            if t.generation.get() != generation {
                return;
            }
            match loaded {
                Ok(loaded) => t.render(&photo, &loaded),
                Err(_) => t.set_placeholder(i18n::no_exif()),
            }
        });
    }

    /// Ничего не показывать (выделено несколько фото).
    pub fn show_nothing(&self) {
        self.shown.set(None);
        self.generation.set(self.generation.get() + 1);
        self.clear();
    }

    fn clear(&self) {
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
    }

    fn set_placeholder(&self, text: &str) {
        self.clear();
        let label = gtk::Label::new(Some(text));
        label.add_css_class("dim-label");
        label.set_wrap(true);
        label.set_margin_top(24);
        self.content.append(&label);
    }

    fn render(&self, photo: &Photo, loaded: &Loaded) {
        self.clear();
        // Все разделы — в одной сетке, чтобы значения стояли в одну колонку.
        let main = grid();
        self.content.append(&main);
        let exif = loaded.exif.as_ref();
        let path = Path::new(&photo.path);
        let folder = path.parent().map(|p| p.display().to_string());

        let mut file = vec![(Info::Name, Some(photo.file_name().to_string()))];
        file.push((Info::Folder, folder));
        file.push((Info::Size, loaded.size.map(i18n::file_size)));
        file.push((Info::Dimensions, exif.and_then(|e| Some(i18n::dimensions(e.width?, e.height?)))));
        section(&main, Info::File, file);

        let Some(e) = exif else {
            section(&main, Info::Location, vec![(Info::Coordinates, coords(photo))]);
            self.set_note(i18n::no_exif());
            return;
        };
        let taken = match (e.taken_at, &e.offset) {
            (Some(d), Some(off)) => Some(format!("{} ({off})", i18n::datetime(&d))),
            (Some(d), None) => Some(i18n::datetime(&d)),
            (None, _) => None,
        };
        section(
            &main,
            Info::Shooting,
            vec![
                (Info::Taken, taken),
                (Info::Duration, e.duration.map(video::format_duration)),
                (Info::Camera, e.camera.clone()),
                (Info::Lens, e.lens.clone()),
                (Info::Exposure, e.exposure_time.map(|t| i18n::exposure(&metadata::format_exposure(t)))),
                (Info::Aperture, e.f_number.map(i18n::aperture)),
                (Info::Iso, e.iso.map(|i| i.to_string())),
                (Info::FocalLength, e.focal_length.map(|f| i18n::focal_length(f, e.focal_35mm))),
                (Info::ExposureBias, e.exposure_bias.map(i18n::exposure_bias)),
                (Info::Flash, e.flash_fired.map(|f| i18n::flash(f).to_string())),
                (Info::Software, e.software.clone()),
            ],
        );
        section(
            &main,
            Info::Location,
            vec![(Info::Coordinates, coords(photo)), (Info::Altitude, e.altitude.map(i18n::altitude))],
        );
        if !e.all.is_empty() {
            let grid = grid();
            for (row, (tag, value)) in e.all.iter().enumerate() {
                attach_row(&grid, row as i32, tag, value);
            }
            let expander = gtk::Expander::new(Some(i18n::info_label(Info::AllTags)));
            expander.set_child(Some(&grid));
            grid.set_margin_top(8);
            self.content.append(&expander);
        }
    }

    fn set_note(&self, text: &str) {
        let label = gtk::Label::new(Some(text));
        label.add_css_class("dim-label");
        label.set_xalign(0.0);
        self.content.append(&label);
    }
}

/// Раздел с заголовком в конце сетки; пустые поля пропускаются, пустой раздел не показывается.
fn section(grid: &gtk::Grid, title: Info, rows: Vec<(Info, Option<String>)>) {
    let rows: Vec<(Info, String)> = rows.into_iter().filter_map(|(k, v)| Some((k, v?))).collect();
    if rows.is_empty() {
        return;
    }
    let mut row = next_row(grid);
    let heading = gtk::Label::new(Some(i18n::info_label(title)));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    if row > 0 {
        heading.set_margin_top(14);
    }
    grid.attach(&heading, 0, row, 2, 1);
    for (key, value) in &rows {
        row += 1;
        attach_row(grid, row, i18n::info_label(*key), value);
    }
}

/// Номер первой свободной строки сетки.
fn next_row(grid: &gtk::Grid) -> i32 {
    let mut rows = 0;
    let mut child = grid.first_child();
    while let Some(c) = child {
        let (_, row, _, height) = grid.query_child(&c);
        rows = rows.max(row + height);
        child = c.next_sibling();
    }
    rows
}

fn coords(photo: &Photo) -> Option<String> {
    photo.coords().map(|(lat, lon)| format!("{lat:.6}, {lon:.6}"))
}

fn grid() -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(4);
    grid
}

/// Строка «подпись — значение»; значение можно выделить и скопировать.
fn attach_row(grid: &gtk::Grid, row: i32, key: &str, value: &str) {
    let k = gtk::Label::new(Some(key));
    k.add_css_class("dim-label");
    k.set_xalign(0.0);
    k.set_yalign(0.0);
    k.set_wrap(true);
    k.set_max_width_chars(14);
    let v = gtk::Label::new(Some(value));
    v.set_xalign(0.0);
    v.set_yalign(0.0);
    v.set_hexpand(true);
    v.set_wrap(true);
    v.set_wrap_mode(pango::WrapMode::WordChar);
    v.set_max_width_chars(20);
    v.set_selectable(true);
    grid.attach(&k, 0, row, 1, 1);
    grid.attach(&v, 1, row, 1, 1);
}
