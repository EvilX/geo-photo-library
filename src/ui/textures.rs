//! Квадратные текстуры-превью (обрезка по центру) с кэшем, и открытие файлов.
use gtk::{gdk, gdk_pixbuf, gio, prelude::*};
use std::{cell::RefCell, collections::HashMap};

const MAX_ENTRIES: usize = 4000;

#[derive(Default)]
pub struct TextureCache {
    map: RefCell<HashMap<(String, i32), gdk::Texture>>,
}

impl TextureCache {
    /// Квадратная текстура стороной `size` px из файла миниатюры.
    pub fn square(&self, path: &str, size: i32) -> Option<gdk::Texture> {
        let key = (path.to_owned(), size);
        if let Some(t) = self.map.borrow().get(&key) {
            return Some(t.clone());
        }
        let _t = crate::trace::span(format!("    текстура {size}px"));
        let tex = load_square(path, size)?;
        let mut map = self.map.borrow_mut();
        if map.len() >= MAX_ENTRIES {
            map.clear();
        }
        map.insert(key, tex.clone());
        Some(tex)
    }

    pub fn clear(&self) {
        self.map.borrow_mut().clear();
    }
}

fn load_square(path: &str, size: i32) -> Option<gdk::Texture> {
    let pb = gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let (w, h) = (pb.width(), pb.height());
    let side = w.min(h);
    let sub = pb.new_subpixbuf((w - side) / 2, (h - side) / 2, side, side);
    let scaled = sub.scale_simple(size, size, gdk_pixbuf::InterpType::Bilinear)?;
    Some(gdk::Texture::for_pixbuf(&scaled))
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
