//! Вкладка «По датам»: дерево год → месяц → день и сетка фото.
use super::{
    date_tree::{DateTree, NodeKey},
    photo_grid::{self, PhotoGrid},
    textures::TextureCache,
};
use crate::{
    config::SortOrder,
    db::{Db, GeoFilter},
    i18n, trace,
};
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
    /// Фильтр по координатам (выпадающий список над деревом).
    filter: Cell<GeoFilter>,
    title: gtk::Label,
    grid: PhotoGrid,
    /// Что сейчас показано в сетке: узел и число фото — чтобы не перезагружать то же самое.
    shown: Cell<Option<(NodeKey, u32)>>,
}

impl DatesTab {
    pub fn new(db: Rc<Db>, cache: Rc<TextureCache>) -> Rc<Self> {
        let tree = DateTree::new(false);
        let labels: Vec<&str> = GeoFilter::ALL.iter().map(|&f| i18n::geo_filter_label(f)).collect();
        let filter = gtk::DropDown::from_strings(&labels);
        filter.set_tooltip_text(Some(i18n::geo_filter_tooltip()));
        filter.set_margin_top(6);
        filter.set_margin_bottom(6);
        filter.set_margin_start(6);
        filter.set_margin_end(6);
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.append(&filter);
        left.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        left.append(&tree.root);

        let grid = PhotoGrid::new(cache, 160, 12);
        grid.set_empty_text(i18n::no_photos());
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
        root.set_start_child(Some(&left));
        root.set_end_child(Some(&right));
        root.set_resize_start_child(false);
        root.set_shrink_start_child(false);
        root.set_position(240);

        let this = Rc::new(Self {
            root,
            sort,
            zoom,
            db,
            tree,
            filter: Cell::new(GeoFilter::All),
            title,
            grid,
            shown: Cell::new(None),
        });
        let weak = Rc::downgrade(&this);
        filter.connect_selected_notify(move |d| {
            if let Some(t) = weak.upgrade() {
                t.filter.set(GeoFilter::from_index(d.selected()));
                // Число фото у выбранной даты может совпасть, а сами фото — другие.
                t.reload(true);
            }
        });
        if let Some(f) = GeoFilter::from_env() {
            filter.set_selected(f.index());
        }
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
        let counts = self.db.date_counts(self.filter.get()).unwrap_or_else(|e| {
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

    /// Показать фото выбранного узла. Другой узел — список заменяется целиком и
    /// прокручивается в начало. Тот же узел (подгрузка во время сканирования, удаление,
    /// запись EXIF) — список только обновляется: выделение и прокрутка остаются на месте,
    /// а при `force` ячейки привязываются заново — миниатюры могли быть перезаписаны.
    fn load_selected(&self, force: bool) {
        let Some(node) = self.tree.selected() else {
            self.shown.set(None);
            self.title.set_text(i18n::no_photos());
            self.grid.set_photos(&[]);
            return;
        };
        let shown = self.shown.get();
        if !force && shown == Some((node.key(), node.count)) {
            return;
        }
        let same_node = shown.is_some_and(|(key, _)| key == node.key());
        self.shown.set(Some((node.key(), node.count)));
        let _t = trace::span(format!("dates.load {}", node.title()));
        let photos = self.db.photos_by_date(node.year, node.month, node.day, self.filter.get()).unwrap_or_else(|e| {
            eprintln!("БД: {e:#}");
            Vec::new()
        });
        self.title.set_text(&format!("{} — {}", node.title(), i18n::photos(photos.len())));
        if same_node {
            self.grid.update_photos(&photos);
            if force {
                self.grid.rebind();
            }
        } else {
            self.grid.set_photos(&photos);
        }
    }
}
