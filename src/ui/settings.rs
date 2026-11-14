//! Диалог настроек: путь к библиотеке фотографий.
use gtk::{gio, glib, prelude::*};
use std::path::{Path, PathBuf};

pub fn show(parent: &gtk::ApplicationWindow, current: Option<&Path>, on_save: impl Fn(PathBuf) + 'static) {
    let dialog = gtk::Window::builder()
        .title("Настройки")
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .resizable(false)
        .build();

    let entry = gtk::Entry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("/home/user/Pictures"));
    if let Some(p) = current {
        entry.set_text(&p.to_string_lossy());
    }
    let browse = gtk::Button::with_label("Выбрать…");
    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_xalign(0.0);
    error.set_visible(false);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&browse);

    let cancel = gtk::Button::with_label("Отмена");
    let save = gtk::Button::with_label("Сохранить");
    save.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&save);

    let title = gtk::Label::new(Some("Папка с фотографиями"));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    let hint = gtk::Label::new(Some(
        "Папка сканируется рекурсивно при каждом запуске: JPEG, HEIC, PNG, WebP, TIFF, DNG.",
    ));
    hint.set_xalign(0.0);
    hint.set_wrap(true);
    hint.add_css_class("dim-label");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.append(&title);
    content.append(&row);
    content.append(&hint);
    content.append(&error);
    content.append(&buttons);
    dialog.set_child(Some(&content));

    let (d, e) = (dialog.clone(), entry.clone());
    browse.connect_clicked(move |_| {
        let chooser = gtk::FileDialog::builder().title("Папка с фотографиями").modal(true).build();
        let text = e.text();
        if !text.is_empty() {
            chooser.set_initial_folder(Some(&gio::File::for_path(text.as_str())));
        }
        let e = e.clone();
        chooser.select_folder(Some(&d), gio::Cancellable::NONE, move |res| {
            if let Some(path) = res.ok().and_then(|f| f.path()) {
                e.set_text(&path.to_string_lossy());
            }
        });
    });

    let d = dialog.clone();
    cancel.connect_clicked(move |_| d.close());

    let d = dialog.clone();
    save.connect_clicked(move |_| {
        let path = PathBuf::from(entry.text().trim());
        if !path.is_dir() {
            error.set_text("Папка не существует");
            error.set_visible(true);
            return;
        }
        on_save(path);
        d.close();
    });

    let key = gtk::EventControllerKey::new();
    let d = dialog.clone();
    key.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            d.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(key);
    dialog.present();
}
