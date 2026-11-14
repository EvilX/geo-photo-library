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

/// Открыть фото в системном просмотрщике.
pub fn open_external(widget: &impl IsA<gtk::Widget>, path: &str) {
    let parent = widget.root().and_downcast::<gtk::Window>();
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.launch(parent.as_ref(), gio::Cancellable::NONE, |res| {
        if let Err(e) = res {
            eprintln!("не удалось открыть файл: {e}");
        }
    });
}
