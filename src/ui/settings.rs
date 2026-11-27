//! Диалог настроек: путь к библиотеке фотографий, просмотрщик и редактор.
use super::textures::image_apps;
use crate::{config::Config, i18n};
use gtk::{gio, glib, prelude::*};
use std::path::{Path, PathBuf};

/// Выпадающий список программ: первый пункт — «Автоматически» (`None`).
struct AppChoice {
    dropdown: gtk::DropDown,
    ids: Vec<Option<String>>,
}

impl AppChoice {
    fn new(apps: &[gio::AppInfo], current: Option<&str>) -> Self {
        let mut ids = vec![None];
        let mut labels = vec![i18n::automatic().to_string()];
        for app in apps {
            ids.push(app.id().map(String::from));
            labels.push(app.display_name().to_string());
        }
        // Выбранной раньше программы нет в списке (удалена) — показываем её id,
        // чтобы выбор не потерялся молча; запуск всё равно откатится на автоматический.
        if let Some(id) = current.filter(|id| !ids.iter().any(|x| x.as_deref() == Some(*id))) {
            ids.push(Some(id.to_string()));
            labels.push(id.to_string());
        }
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let dropdown = gtk::DropDown::from_strings(&refs);
        let pos = ids.iter().position(|x| x.as_deref() == current).unwrap_or(0);
        dropdown.set_selected(pos as u32);
        dropdown.set_hexpand(true);
        Self { dropdown, ids }
    }

    fn selected(&self) -> Option<String> {
        self.ids.get(self.dropdown.selected() as usize).cloned().flatten()
    }
}

/// «~» и «~/…» в начале пути — домашний каталог (и «~\…» на Windows).
fn expand_home(text: &str, home: &Path) -> PathBuf {
    match text.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with(std::path::is_separator) => home.join(rest.trim_start_matches(std::path::is_separator)),
        _ => PathBuf::from(text),
    }
}

/// Папка библиотеки из введённого текста: существующий каталог, путь абсолютный и
/// канонический. `None` — такой папки нет.
fn library_dir(text: &str, home: &Path) -> Option<PathBuf> {
    if text.is_empty() {
        return None;
    }
    let path = expand_home(text, home);
    if !path.is_dir() {
        return None;
    }
    // На Windows `canonicalize` даёт `\\?\C:\…` — такой путь виден в настройках и не всеми
    // программами понимается; там хватает абсолютного пути.
    if cfg!(windows) {
        std::path::absolute(&path).ok()
    } else {
        path.canonicalize().ok()
    }
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.add_css_class("heading");
    l
}

pub fn show(parent: &gtk::ApplicationWindow, config: &Config, on_save: impl Fn(Config) + 'static) {
    let current = config.library_path.as_deref();
    let dialog = gtk::Window::builder()
        .title(i18n::settings())
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
    let browse = gtk::Button::with_label(i18n::browse());
    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_xalign(0.0);
    error.set_visible(false);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&browse);

    let cancel = gtk::Button::with_label(i18n::cancel());
    let save = gtk::Button::with_label(i18n::save());
    save.add_css_class("suggested-action");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&cancel);
    buttons.append(&save);

    let title = heading(i18n::photo_folder());
    let hint = gtk::Label::new(Some(i18n::folder_hint()));
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

    let apps = image_apps();
    let viewer = AppChoice::new(&apps, config.viewer.as_deref());
    let editor = AppChoice::new(&apps, config.editor.as_deref());
    let grid = gtk::Grid::new();
    grid.set_row_spacing(6);
    grid.set_column_spacing(12);
    grid.set_margin_top(8);
    let viewer_label = gtk::Label::new(Some(i18n::photo_viewer()));
    viewer_label.set_xalign(0.0);
    let editor_label = gtk::Label::new(Some(i18n::photo_editor()));
    editor_label.set_xalign(0.0);
    grid.attach(&viewer_label, 0, 0, 1, 1);
    grid.attach(&viewer.dropdown, 1, 0, 1, 1);
    grid.attach(&editor_label, 0, 1, 1, 1);
    grid.attach(&editor.dropdown, 1, 1, 1, 1);
    let apps_hint = gtk::Label::new(Some(i18n::apps_hint()));
    apps_hint.set_xalign(0.0);
    apps_hint.set_wrap(true);
    apps_hint.add_css_class("dim-label");
    // В песочнице Flatpak программы хоста не видны — список пуст, файлы
    // открываются через портал с выбором программы.
    if !apps.is_empty() {
        content.append(&grid);
        content.append(&apps_hint);
    }
    let check_updates = gtk::CheckButton::with_label(i18n::check_updates());
    check_updates.set_active(config.check_updates);
    check_updates.set_margin_top(8);
    content.append(&check_updates);
    content.append(&buttons);
    dialog.set_child(Some(&content));

    // Обработчики на дочерних виджетах держат диалог слабой ссылкой: GTK4 при закрытии
    // окна не разрывает циклы «окно → кнопка → обработчик → окно», и с сильной ссылкой
    // каждый открытый диалог (и `on_save` с состоянием приложения) оставался бы в памяти.
    let (d, e) = (dialog.downgrade(), entry.clone());
    browse.connect_clicked(move |_| {
        let Some(d) = d.upgrade() else { return };
        let chooser = gtk::FileDialog::builder().title(i18n::photo_folder()).modal(true).build();
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

    let d = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });

    let d = dialog.downgrade();
    let config = config.clone();
    save.connect_clicked(move |_| {
        // В настройки — абсолютный канонический путь: относительный зависел бы от каталога,
        // из которого запущена программа, а «~» раскрывает только оболочка.
        let Some(path) = library_dir(entry.text().trim(), &glib::home_dir()) else {
            error.set_text(i18n::folder_missing());
            error.set_visible(true);
            return;
        };
        on_save(Config {
            library_path: Some(path),
            viewer: viewer.selected(),
            editor: editor.selected(),
            check_updates: check_updates.is_active(),
            ..config.clone()
        });
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });

    let key = gtk::EventControllerKey::new();
    let d = dialog.downgrade();
    key.connect_key_pressed(move |_, k, _, _| {
        if k == gtk::gdk::Key::Escape {
            if let Some(d) = d.upgrade() {
                d.close();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(key);
    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_and_relative_paths() {
        let home = Path::new("/home/user");
        assert_eq!(expand_home("~", home), home);
        assert_eq!(expand_home("~/Pictures", home), home.join("Pictures"));
        assert_eq!(expand_home("~other/Pictures", home), Path::new("~other/Pictures"));
        assert_eq!(expand_home("/data/~/x", home), Path::new("/data/~/x"));

        let tmp = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(library_dir("~", &tmp), Some(tmp.clone()));
        assert_eq!(library_dir("", &tmp), None);
        assert_eq!(library_dir("/no/such/folder/gpl", &tmp), None);
        // Относительный путь становится абсолютным.
        let dir = library_dir(".", &tmp).unwrap();
        assert!(dir.is_absolute());
        #[cfg(not(windows))]
        assert_eq!(dir, std::env::current_dir().unwrap().canonicalize().unwrap());
    }
}
