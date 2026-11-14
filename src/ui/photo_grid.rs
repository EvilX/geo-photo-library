//! Сетка миниатюр, используется на обеих вкладках.
use super::textures::{open_external, set_thumbnail, TextureCache};
use crate::db::Photo;
use gtk::{gio, glib, pango, prelude::*};
use std::rc::Rc;

#[derive(Clone)]
pub struct PhotoGrid {
    pub root: gtk::ScrolledWindow,
    pub view: gtk::GridView,
    pub selection: gtk::SingleSelection,
    store: gio::ListStore,
}

impl PhotoGrid {
    /// `size` — сторона миниатюры в логических пикселях, `max_columns` — предел колонок.
    ///
    /// GridView держит виджеты примерно для `max_columns × 30` элементов, поэтому
    /// завышенный предел отключает виртуализацию: создаются ячейки для всех фото.
    pub fn new(cache: Rc<TextureCache>, size: i32, max_columns: u32) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let image = gtk::Image::new();
            image.set_pixel_size(size);
            image.add_css_class("photo-thumb");
            image.set_overflow(gtk::Overflow::Hidden);
            let label = gtk::Label::new(None);
            label.add_css_class("photo-caption");
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_max_width_chars(1);
            let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
            b.set_margin_top(4);
            b.set_margin_bottom(4);
            b.append(&image);
            b.append(&label);
            item.set_child(Some(&b));
        });
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
            let photo = obj.borrow::<Photo>();
            let b = item.child().and_downcast::<gtk::Box>().unwrap();
            let image = b.first_child().and_downcast::<gtk::Image>().unwrap();
            let label = image.next_sibling().and_downcast::<gtk::Label>().unwrap();
            // Ячейка переиспользуется при прокрутке: ставим текстуру, только если
            // к моменту загрузки в ней всё ещё тот же элемент.
            let (weak_item, expected) = (item.downgrade(), obj.clone().upcast::<glib::Object>());
            set_thumbnail(&cache, &image, photo.thumb_path.as_deref(), move || {
                weak_item.upgrade().and_then(|i| i.item()).as_ref() == Some(&expected)
            });
            label.set_text(&photo.display_date());
            b.set_tooltip_text(Some(&format!("{}\n{}", photo.file_name(), photo.display_date())));
        });

        let view = gtk::GridView::new(Some(selection.clone()), Some(factory));
        view.set_max_columns(max_columns);
        view.set_min_columns(1);
        view.connect_activate(|view, pos| {
            let model = view.model().unwrap();
            if let Some(obj) = model.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                open_external(view, &obj.borrow::<Photo>().path);
            }
        });

        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&view)
            .build();
        Self { root, view, selection, store }
    }

    pub fn set_photos(&self, photos: &[Photo]) {
        let objs: Vec<glib::BoxedAnyObject> =
            photos.iter().cloned().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }

    pub fn photo(&self, pos: u32) -> Option<Photo> {
        self.store
            .item(pos)
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|o| o.borrow::<Photo>().clone())
    }

    pub fn selected_photo(&self) -> Option<Photo> {
        match self.selection.selected() {
            gtk::INVALID_LIST_POSITION => None,
            pos => self.photo(pos),
        }
    }

    /// Выделить фото с данным id и прокрутить к нему.
    pub fn select_id(&self, id: i64) -> bool {
        let n = self.store.n_items();
        let Some(pos) = (0..n).find(|&i| self.photo(i).is_some_and(|p| p.id == id)) else {
            return false;
        };
        self.selection.set_selected(pos);
        self.view.scroll_to(pos, gtk::ListScrollFlags::FOCUS, None);
        true
    }
}
