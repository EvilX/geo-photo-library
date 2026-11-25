//! Асинхронная загрузка миниатюр в текстуры с LRU-кэшем по объёму, и открытие файлов.
//!
//! Миниатюры уже квадратные (см. `thumbs`), поэтому здесь только декодирование JPEG
//! 320×320 в фоновом потоке; текстура создаётся в главном потоке.
use crate::trace;
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

/// Предел памяти под текстуры (несжатые RGB).
const MAX_BYTES: usize = 128 << 20;

type Callback = Box<dyn FnOnce(&gdk::Texture)>;

struct Entry {
    texture: gdk::Texture,
    bytes: usize,
    last_used: u64,
}

#[derive(Default)]
pub struct TextureCache {
    entries: RefCell<HashMap<String, Entry>>,
    total_bytes: Cell<usize>,
    clock: Cell<u64>,
    /// Загрузки в процессе: путь → ожидающие колбэки.
    pending: RefCell<HashMap<String, Vec<Callback>>>,
}

/// Декодированный RGB-буфер (создаётся в рабочем потоке).
struct Pixels {
    width: i32,
    height: i32,
    stride: usize,
    bytes: glib::Bytes,
}

fn decode(path: &str) -> Option<Pixels> {
    let img = image::open(path).ok()?.into_rgb8();
    let (w, h) = img.dimensions();
    Some(Pixels {
        width: w as i32,
        height: h as i32,
        stride: w as usize * 3,
        bytes: glib::Bytes::from_owned(img.into_raw()),
    })
}

impl TextureCache {
    fn tick(&self) -> u64 {
        let t = self.clock.get() + 1;
        self.clock.set(t);
        t
    }

    /// Текстура из кэша, без загрузки.
    pub fn get(&self, path: &str) -> Option<gdk::Texture> {
        let tick = self.tick();
        let mut entries = self.entries.borrow_mut();
        let e = entries.get_mut(path)?;
        e.last_used = tick;
        Some(e.texture.clone())
    }

    /// Загрузить текстуру в фоне и вызвать `done` в главном потоке.
    /// Если текстура уже в кэше — `done` вызывается сразу.
    pub fn load(self: &Rc<Self>, path: &str, done: impl FnOnce(&gdk::Texture) + 'static) {
        if let Some(t) = self.get(path) {
            done(&t);
            return;
        }
        let mut pending = self.pending.borrow_mut();
        if let Some(waiting) = pending.get_mut(path) {
            waiting.push(Box::new(done));
            return;
        }
        pending.insert(path.to_owned(), vec![Box::new(done)]);
        drop(pending);

        let weak = Rc::downgrade(self);
        let path = path.to_owned();
        glib::spawn_future_local(async move {
            let p = path.clone();
            let pixels = gio::spawn_blocking(move || {
                let _t = trace::span("    декод. текстуры (фон)");
                decode(&p)
            })
            .await
            .ok()
            .flatten();
            let Some(this) = weak.upgrade() else { return };
            let callbacks = this.pending.borrow_mut().remove(&path).unwrap_or_default();
            let Some(px) = pixels else {
                eprintln!("не удалось загрузить миниатюру {path}");
                return;
            };
            let size = px.bytes.len();
            let texture = gdk::MemoryTexture::new(px.width, px.height, gdk::MemoryFormat::R8g8b8, &px.bytes, px.stride)
                .upcast::<gdk::Texture>();
            this.insert(path, texture.clone(), size);
            for cb in callbacks {
                cb(&texture);
            }
        });
    }

    fn insert(&self, path: String, texture: gdk::Texture, bytes: usize) {
        let last_used = self.tick();
        let mut entries = self.entries.borrow_mut();
        if let Some(old) = entries.insert(path, Entry { texture, bytes, last_used }) {
            self.total_bytes.set(self.total_bytes.get() - old.bytes);
        }
        self.total_bytes.set(self.total_bytes.get() + bytes);
        if self.total_bytes.get() > MAX_BYTES {
            // Вытесняем самые давно использованные до 3/4 предела.
            let mut by_age: Vec<(u64, String)> = entries.iter().map(|(k, e)| (e.last_used, k.clone())).collect();
            by_age.sort_unstable();
            for (_, key) in by_age {
                if self.total_bytes.get() <= MAX_BYTES / 4 * 3 {
                    break;
                }
                if let Some(e) = entries.remove(&key) {
                    self.total_bytes.set(self.total_bytes.get() - e.bytes);
                }
            }
        }
    }

    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        self.total_bytes.set(0);
    }
}

/// Показать миниатюру в `image`: сразу из кэша, иначе после фоновой загрузки.
/// `still_valid` проверяется перед установкой — виджет мог быть переиспользован.
pub fn set_thumbnail(
    cache: &Rc<TextureCache>,
    image: &gtk::Image,
    thumb: Option<&str>,
    still_valid: impl Fn() -> bool + 'static,
) {
    let Some(path) = thumb else {
        image.set_icon_name(Some("image-x-generic-symbolic"));
        return;
    };
    if let Some(t) = cache.get(path) {
        image.set_paintable(Some(&t));
        return;
    }
    image.clear();
    let weak = image.downgrade();
    cache.load(path, move |t| {
        if let Some(image) = weak.upgrade() {
            if still_valid() {
                image.set_paintable(Some(t));
            }
        }
    });
}

/// Программы, выбранные в настройках (id `.desktop`-файлов). Интерфейс однопоточный,
/// поэтому выбор хранится в thread-local главного потока.
#[derive(Debug, Clone, Default)]
pub struct PreferredApps {
    pub viewer: Option<String>,
    pub editor: Option<String>,
}

thread_local! {
    static PREFERRED: RefCell<PreferredApps> = RefCell::default();
}

pub fn set_preferred_apps(apps: PreferredApps) {
    PREFERRED.with(|p| *p.borrow_mut() = apps);
}

#[cfg(unix)]
fn app_by_id(id: Option<String>) -> Option<gio::AppInfo> {
    gio::DesktopAppInfo::new(&id?).map(|a| a.upcast())
}

/// На Windows `.desktop` нет: программа ищется по id среди зарегистрированных в системе.
#[cfg(windows)]
fn app_by_id(id: Option<String>) -> Option<gio::AppInfo> {
    let id = id?;
    gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|x| x == id))
}

/// Запустить программу с файлом; `false`, если не получилось.
fn launch_app(widget: &impl IsA<gtk::Widget>, app: &gio::AppInfo, path: &str) -> bool {
    let ctx = widget.display().app_launch_context();
    match app.launch(&[gio::File::for_path(path)], Some(&ctx)) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("не удалось запустить {}: {e}", app.name());
            false
        }
    }
}

/// Открыть фото в просмотрщике из настроек, иначе — в программе по умолчанию для
/// его типа. Если её нет, система сама предложит выбрать программу.
pub fn open_external(widget: &impl IsA<gtk::Widget>, path: &str) {
    let viewer = PREFERRED.with(|p| p.borrow().viewer.clone());
    if app_by_id(viewer).is_some_and(|app| launch_app(widget, &app, path)) {
        return;
    }
    launch_with_portal(widget, path, false);
}

/// Открыть фото в редакторе из настроек, иначе — в найденном графическом редакторе.
/// Если ни один не запустился, показать окно выбора программы.
pub fn open_in_editor(widget: &impl IsA<gtk::Widget>, path: &str) {
    let editor = PREFERRED.with(|p| p.borrow().editor.clone());
    for app in [app_by_id(editor), find_editor(path)].into_iter().flatten() {
        if launch_app(widget, &app, path) {
            return;
        }
    }
    launch_with_portal(widget, path, true);
}

/// Открыть папку с фото в файловом менеджере и выделить в ней файл.
///
/// GTK вызывает `org.freedesktop.FileManager1.ShowItems` (Nautilus, Dolphin, Nemo,
/// Caja, Thunar), во Flatpak — портал OpenURI; если ни того ни другого нет, просто
/// открывает папку.
pub fn show_in_folder(widget: &impl IsA<gtk::Widget>, path: &str) {
    let parent = widget.root().and_downcast::<gtk::Window>();
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.open_containing_folder(parent.as_ref(), gio::Cancellable::NONE, |res| {
        if let Err(e) = res {
            eprintln!("не удалось показать файл в папке: {e}");
        }
    });
}

/// Программы, которые открывают фото (JPEG, HEIC, PNG), по имени; без этой программы.
pub fn image_apps() -> Vec<gio::AppInfo> {
    let own_id = format!("{}.desktop", crate::APP_ID);
    let mut apps: Vec<gio::AppInfo> = Vec::new();
    for t in ["image/jpeg", "image/heic", "image/png"] {
        for app in gio::AppInfo::all_for_type(t) {
            let Some(id) = app.id() else { continue };
            if id != own_id && app.should_show() && !apps.iter().any(|a| a.id().as_ref() == Some(&id)) {
                apps.push(app);
            }
        }
    }
    apps.sort_by_key(|a| a.name().to_lowercase());
    apps
}

/// `always_ask` — показать окно выбора программы даже при наличии программы по умолчанию.
fn launch_with_portal(widget: &impl IsA<gtk::Widget>, path: &str, always_ask: bool) {
    let parent = widget.root().and_downcast::<gtk::Window>();
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.set_always_ask(always_ask);
    launcher.launch(parent.as_ref(), gio::Cancellable::NONE, |res| {
        if let Err(e) = res {
            eprintln!("не удалось открыть файл: {e}");
        }
    });
}

/// Редактор изображений для файла: программа, зарегистрированная для его MIME-типа,
/// из категорий RasterGraphics или Photography, но не просмотрщик (Viewer)
/// и не эта программа. Раньше идут рекомендованные системой.
#[cfg(unix)]
fn find_editor(path: &str) -> Option<gio::AppInfo> {
    let (content_type, _) = gio::content_type_guess(Some(path), None);
    let own_id = format!("{}.desktop", crate::APP_ID);
    gio::AppInfo::all_for_type(&content_type).into_iter().find(|app| {
        if app.id().is_some_and(|id| id == own_id) {
            return false;
        }
        let Some(categories) = app.downcast_ref::<gio::DesktopAppInfo>().and_then(|d| d.categories()) else {
            return false;
        };
        is_editor_categories(&categories)
    })
}

/// На Windows категорий программ (RasterGraphics…) нет — сразу окно выбора программы.
#[cfg(windows)]
fn find_editor(_path: &str) -> Option<gio::AppInfo> {
    None
}

#[cfg_attr(windows, allow(dead_code))]
fn is_editor_categories(categories: &str) -> bool {
    let has = |c: &str| categories.split(';').any(|x| x == c);
    (has("RasterGraphics") || has("Photography")) && !has("Viewer")
}

/// Положить файлы в буфер обмена: как файлы (вставка в файловом менеджере) и как пути по строке.
pub fn copy_to_clipboard(widget: &impl IsA<gtk::Widget>, paths: &[&str]) {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    let uris: Vec<String> = files.iter().map(|f| f.uri().to_string()).collect();
    let providers = [
        gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()),
        gdk::ContentProvider::for_bytes(
            "x-special/gnome-copied-files",
            &glib::Bytes::from_owned(format!("copy\n{}", uris.join("\n"))),
        ),
        gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &glib::Bytes::from_owned(paths.join("\n"))),
    ];
    widget.clipboard().set_content(Some(&gdk::ContentProvider::new_union(&providers))).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_categories() {
        assert!(is_editor_categories("Graphics;2DGraphics;RasterGraphics;GTK;")); // GIMP
        assert!(is_editor_categories("Graphics;Photography;GTK;")); // darktable
        assert!(!is_editor_categories("GNOME;GTK;Graphics;2DGraphics;RasterGraphics;Viewer;")); // Loupe
        assert!(!is_editor_categories("Network;WebBrowser;"));
    }
}
