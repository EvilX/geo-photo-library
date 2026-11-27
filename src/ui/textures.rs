//! Асинхронная загрузка миниатюр в текстуры с LRU-кэшем по объёму, и открытие файлов.
//!
//! Миниатюры уже квадратные (см. `thumbs`), поэтому здесь только декодирование JPEG
//! 320×320 в фоновом потоке; текстура создаётся в главном потоке.
use crate::{i18n, trace};
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

/// Сколько текстур держать в кэше. Сетки хранят ячейки примерно для 30 рядов: до 16 колонок
/// на вкладке дат и до 8 в списке карты — около 700 миниатюр, плюс маркеры карты и запас на
/// прокрутку назад. При меньшем пределе (было 128 МБ, ~430 текстур) кэш вытеснял то, что ещё
/// на экране: память это не освобождало (текстуру держит ячейка), а возврат к фото
/// декодировал его заново.
const MAX_TEXTURES: usize = 1024;
/// Предел памяти под текстуры: миниатюры — несжатые RGB 320×320 (см. `thumbs`), ~300 МБ.
const MAX_BYTES: usize = MAX_TEXTURES * 320 * 320 * 3;

/// Получатель результата загрузки: текстура или `None`, если миниатюра не прочиталась.
type Done = Box<dyn FnOnce(Option<&gdk::Texture>)>;
/// Получатель сообщений об ошибках открытия файлов.
type Reporter = Box<dyn Fn(&str)>;

/// Ожидающий загрузки: `wanted` проверяется в главном потоке перед декодированием,
/// `done` получает текстуру или `None`, если файл миниатюры не прочитался.
struct Waiter {
    wanted: Box<dyn Fn() -> bool>,
    done: Done,
}

struct Entry {
    texture: gdk::Texture,
    bytes: usize,
    last_used: u64,
}

pub struct TextureCache {
    entries: RefCell<HashMap<String, Entry>>,
    total_bytes: Cell<usize>,
    clock: Cell<u64>,
    /// Загрузки в очереди и в процессе: путь → ожидающие.
    pending: RefCell<HashMap<String, Vec<Waiter>>>,
    /// Пути, ждущие декодирования; берётся последний — он запрошен позже всех, то есть
    /// нужен тому, что сейчас на экране.
    queue: RefCell<Vec<String>>,
    /// Сколько декодирований идёт сейчас и сколько их может идти сразу.
    active: Cell<usize>,
    parallel: usize,
    /// Миниатюры, которые не удалось прочитать: повторно не декодируются, пока кэш
    /// не сброшен (`clear`, `forget`, `retry_failed`).
    failed: RefCell<HashSet<String>>,
    /// Растёт при сбросе кэша: результат декодирования, начатого до сброса, отбрасывается
    /// (файл миниатюры мог быть перезаписан), и файл читается заново.
    epoch: Cell<u64>,
}

impl Default for TextureCache {
    fn default() -> Self {
        Self {
            entries: RefCell::default(),
            total_bytes: Cell::new(0),
            clock: Cell::new(0),
            pending: RefCell::default(),
            queue: RefCell::default(),
            active: Cell::new(0),
            parallel: std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8),
            failed: RefCell::default(),
            epoch: Cell::new(0),
        }
    }
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

    /// Загрузить текстуру в фоне и вызвать `done` в главном потоке: с текстурой или с `None`,
    /// если миниатюра не читается. Если текстура уже в кэше — `done` вызывается сразу.
    ///
    /// `wanted` спрашивается перед самым декодированием: при быстрой прокрутке ячейка к этому
    /// времени уже показывает другое фото, и файл, который никому не нужен, не декодируется
    /// (`done` тогда не вызывается — ячейка запросит миниатюру снова, когда вернётся к фото).
    pub fn load(
        self: &Rc<Self>,
        path: &str,
        wanted: impl Fn() -> bool + 'static,
        done: impl FnOnce(Option<&gdk::Texture>) + 'static,
    ) {
        if let Some(t) = self.get(path) {
            done(Some(&t));
            return;
        }
        if self.failed.borrow().contains(path) {
            done(None);
            return;
        }
        let waiter = Waiter { wanted: Box::new(wanted), done: Box::new(done) };
        let mut pending = self.pending.borrow_mut();
        if let Some(waiting) = pending.get_mut(path) {
            waiting.push(waiter);
            return;
        }
        pending.insert(path.to_owned(), vec![waiter]);
        drop(pending);
        self.queue.borrow_mut().push(path.to_owned());
        self.pump();
    }

    /// Запустить декодирование из очереди, пока есть свободные места.
    fn pump(self: &Rc<Self>) {
        while self.active.get() < self.parallel {
            let Some(path) = self.queue.borrow_mut().pop() else { break };
            // Ожидающих вынимаем на время проверки: `wanted` — чужой код.
            let Some(mut waiters) = self.pending.borrow_mut().remove(&path) else { continue };
            waiters.retain(|w| (w.wanted)());
            if waiters.is_empty() {
                continue;
            }
            self.pending.borrow_mut().insert(path.clone(), waiters);
            self.active.set(self.active.get() + 1);
            let (weak, epoch) = (Rc::downgrade(self), self.epoch.get());
            glib::spawn_future_local(async move {
                let p = path.clone();
                // Паника декодера (`Err`) — та же неудача, что и нечитаемый файл.
                let pixels = gio::spawn_blocking(move || {
                    let _t = trace::span("    декод. текстуры (фон)");
                    decode(&p)
                })
                .await
                .ok()
                .flatten();
                let Some(this) = weak.upgrade() else { return };
                this.active.set(this.active.get() - 1);
                this.finish(path, pixels, epoch);
                this.pump();
            });
        }
    }

    /// Декодирование закончено: положить текстуру в кэш и сообщить ожидающим.
    fn finish(&self, path: String, pixels: Option<Pixels>, epoch: u64) {
        if epoch != self.epoch.get() {
            // Кэш сброшен, пока файл декодировался, — прочитать его заново.
            if self.pending.borrow().contains_key(&path) {
                self.queue.borrow_mut().push(path);
            }
            return;
        }
        let waiters = self.pending.borrow_mut().remove(&path).unwrap_or_default();
        let texture = pixels.map(|px| {
            gdk::MemoryTexture::new(px.width, px.height, gdk::MemoryFormat::R8g8b8, &px.bytes, px.stride)
                .upcast::<gdk::Texture>()
        });
        match &texture {
            Some(t) => self.insert(path, t.clone(), t.width() as usize * t.height() as usize * 3),
            None => {
                eprintln!("не удалось загрузить миниатюру {path}");
                self.failed.borrow_mut().insert(path);
            }
        }
        for w in waiters {
            (w.done)(texture.as_ref());
        }
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

    /// Убрать текстуру из кэша: файл миниатюры перезаписан (например, сменилась ориентация).
    pub fn forget(&self, path: &str) {
        if let Some(e) = self.entries.borrow_mut().remove(path) {
            self.total_bytes.set(self.total_bytes.get().saturating_sub(e.bytes));
        }
        self.failed.borrow_mut().remove(path);
        self.epoch.set(self.epoch.get() + 1);
    }

    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        self.total_bytes.set(0);
        self.failed.borrow_mut().clear();
        self.epoch.set(self.epoch.get() + 1);
    }

    /// Забыть нечитаемые миниатюры (сканер мог их пересоздать). `true` — такие были:
    /// ячейки с заглушками стоит привязать заново.
    pub fn retry_failed(&self) -> bool {
        let mut failed = self.failed.borrow_mut();
        let had = !failed.is_empty();
        failed.clear();
        had
    }
}

/// Значок фото без миниатюры.
pub const PHOTO_ICON: &str = "image-x-generic-symbolic";
/// Значок видео без миниатюры (нет `ffmpeg`).
pub const VIDEO_ICON: &str = "video-x-generic-symbolic";

/// Показать миниатюру в `image`: сразу из кэша, иначе после фоновой загрузки.
/// `still_valid` проверяется перед декодированием и перед установкой — виджет мог быть
/// переиспользован. Без миниатюры или если её файл не читается — значок `icon` (фото или видео).
pub fn set_thumbnail(
    cache: &Rc<TextureCache>,
    image: &gtk::Image,
    thumb: Option<&str>,
    icon: &'static str,
    still_valid: impl Fn() -> bool + 'static,
) {
    let Some(path) = thumb else {
        image.set_icon_name(Some(icon));
        return;
    };
    if let Some(t) = cache.get(path) {
        image.set_paintable(Some(&t));
        return;
    }
    image.clear();
    let weak = image.downgrade();
    let valid = Rc::new(move || weak.upgrade().filter(|_| still_valid()));
    let wanted = valid.clone();
    cache.load(
        path,
        move || wanted().is_some(),
        move |t| {
            let Some(image) = valid() else { return };
            match t {
                Some(t) => image.set_paintable(Some(t)),
                None => image.set_icon_name(Some(icon)),
            }
        },
    );
}

thread_local! {
    /// Куда сообщать об ошибках запуска программ — строка состояния окна.
    static REPORT: RefCell<Option<Reporter>> = RefCell::default();
}

/// Задать получателя сообщений о том, что файл не удалось открыть.
pub fn set_error_reporter(f: impl Fn(&str) + 'static) {
    REPORT.with(|r| *r.borrow_mut() = Some(Box::new(f)));
}

/// Файл не открылся (нет программы, отказ портала): в stderr и в строку состояния.
/// Закрытое пользователем окно выбора программы — не ошибка.
fn report_open_error(path: &str, e: &glib::Error) {
    if e.matches(gtk::DialogError::Dismissed) || e.matches(gio::IOErrorEnum::Cancelled) {
        return;
    }
    eprintln!("не удалось открыть {path}: {e}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned());
    let text = i18n::open_failed(name.as_deref().unwrap_or(path), e.message());
    REPORT.with(|r| {
        if let Some(f) = r.borrow().as_ref() {
            f(&text);
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
/// Видео — всегда в программе по умолчанию: просмотрщик из настроек — для фото.
pub fn open_external(widget: &impl IsA<gtk::Widget>, path: &str) {
    if crate::video::is_video(std::path::Path::new(path)) {
        return launch_with_portal(widget, path, false);
    }
    let viewer = PREFERRED.with(|p| p.borrow().viewer.clone());
    if app_by_id(viewer).is_some_and(|app| launch_app(widget, &app, path)) {
        return;
    }
    launch_with_portal(widget, path, false);
}

/// Открыть фото в редакторе из настроек, иначе — в найденном графическом редакторе.
/// Если ни один не запустился, показать окно выбора программы.
pub fn open_in_editor(widget: &impl IsA<gtk::Widget>, path: &str) {
    if crate::video::is_video(std::path::Path::new(path)) {
        return launch_with_portal(widget, path, false);
    }
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
    let path = path.to_owned();
    launcher.open_containing_folder(parent.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Err(e) = res {
            report_open_error(&path, &e);
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
    let path = path.to_owned();
    launcher.launch(parent.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Err(e) = res {
            report_open_error(&path, &e);
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
