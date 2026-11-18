//! Вкладка «По датам»: дерево год → месяц → день и сетка фото.
use super::{
    date_tree::{DateTree, NodeKey},
    photo_grid::{self, PhotoGrid},
    textures::TextureCache,
};
use crate::{config::SortOrder, db::Db, i18n, trace};
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

pub struct DatesTab {
    pub root: gtk::Paned,
    /// Выбор сортировки в заголовке сетки.
    pub sort: gtk::DropDown,
    /// Масштаб миниатюр под сеткой.
    pub zoom: gtk::Scale,
    db: Rc<Db>,
    tree: Rc<DateTree>,
    title: gtk::Label,
    grid: PhotoGrid,
    /// Что сейчас показано в сетке: узел и число фото — чтобы не перезагружать то же самое.
    shown: Cell<Option<(NodeKey, u32)>>,
}

impl DatesTab {
    pub fn new(db: Rc<Db>, cache: Rc<TextureCache>) -> Rc<Self> {
        let tree = DateTree::new(false);

        let grid = PhotoGrid::new(cache, 160, 12);
        let title = gtk::Label::new(None);
        let sort = photo_grid::sort_dropdown();
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.append(&photo_grid::pane_header(&title, &sort));
        right.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        right.append(&grid.root);
        let zoom = photo_grid::zoom_scale();
        right.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        right.append(&photo_grid::pane_footer(&zoom));

        let root = gtk::Paned::new(gtk::Orientation::Horizontal);
        root.set_start_child(Some(&tree.root));
        root.set_end_child(Some(&right));
        root.set_resize_start_child(false);
        root.set_shrink_start_child(false);
        root.set_position(240);

        let this = Rc::new(Self { root, sort, zoom, db, tree, title, grid, shown: Cell::new(None) });
        let weak = Rc::downgrade(&this);
        this.tree.connect_changed(move || {
            if let Some(t) = weak.upgrade() {
                t.load_selected(false);
            }
        });
        this
    }

    /// Перечитать дерево дат из БД. Раскрытые узлы и выбранная дата сохраняются;
    /// сетка перезагружается, только если у выбранной даты изменилось число фото
    /// или `force` (после сканирования — миниатюры могли пересоздаться).
    pub fn reload(&self, force: bool) {
        let _t = trace::span("dates.reload");
        let counts = self.db.date_counts().unwrap_or_else(|e| {
            eprintln!("БД: {e:#}");
            Vec::new()
        });
        self.tree.set_counts(counts);
        self.load_selected(force);
    }

    pub fn set_order(&self, order: SortOrder) {
        self.grid.set_order(order);
    }

    pub fn grid(&self) -> &PhotoGrid {
        &self.grid
    }

    pub fn set_thumb_scale(&self, percent: u32) {
        self.grid.set_scale(percent);
    }

    fn load_selected(&self, force: bool) {
        let Some(node) = self.tree.selected() else {
            self.shown.set(None);
            self.title.set_text(i18n::no_photos());
            self.grid.set_photos(&[]);
            return;
        };
        if !force && self.shown.get() == Some((node.key(), node.count)) {
            return;
        }
        self.shown.set(Some((node.key(), node.count)));
        let _t = trace::span(format!("dates.load {}", node.title()));
        let photos = self.db.photos_by_date(node.year, node.month, node.day).unwrap_or_default();
        self.title.set_text(&format!("{} — {}", node.title(), i18n::photos(photos.len())));
        self.grid.set_photos(&photos);
    }
}
