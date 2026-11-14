//! Вкладка «По датам»: дерево год → месяц → день и сетка фото.
use super::{photo_grid::PhotoGrid, textures::TextureCache};
use crate::{
    db::{DateCount, Db},
    trace,
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

const MONTHS: [&str; 12] = [
    "Январь", "Февраль", "Март", "Апрель", "Май", "Июнь",
    "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];
const MONTHS_GEN: [&str; 12] = [
    "января", "февраля", "марта", "апреля", "мая", "июня",
    "июля", "августа", "сентября", "октября", "ноября", "декабря",
];

#[derive(Debug, Clone, Copy)]
struct DateNode {
    year: i32,
    month: Option<u32>,
    day: Option<u32>,
    count: u32,
}

/// Идентификатор узла дерева: (год, месяц, день).
type NodeKey = (i32, Option<u32>, Option<u32>);

impl DateNode {
    fn key(&self) -> NodeKey {
        (self.year, self.month, self.day)
    }

    fn of_row(row: &gtk::TreeListRow) -> Option<Self> {
        Some(*row.item().and_downcast::<glib::BoxedAnyObject>()?.borrow::<DateNode>())
    }

    fn label(&self) -> String {
        match (self.month, self.day) {
            (Some(m), Some(d)) => format!("{d} {}", MONTHS_GEN[m as usize - 1]),
            (Some(m), None) => MONTHS[m as usize - 1].to_string(),
            _ => self.year.to_string(),
        }
    }

    fn title(&self) -> String {
        match (self.month, self.day) {
            (Some(m), Some(d)) => format!("{d} {} {}", MONTHS_GEN[m as usize - 1], self.year),
            (Some(m), None) => format!("{} {}", MONTHS[m as usize - 1], self.year),
            _ => format!("{} год", self.year),
        }
    }
}

fn store_of(nodes: impl IntoIterator<Item = DateNode>) -> gio::ListStore {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for n in nodes {
        store.append(&glib::BoxedAnyObject::new(n));
    }
    store
}

/// Группировка счётчиков по ключу с суммированием; порядок входа сохраняется.
fn group(counts: impl Iterator<Item = DateCount>, key: impl Fn(&DateCount) -> DateNode) -> Vec<DateNode> {
    let mut out: Vec<DateNode> = Vec::new();
    for c in counts {
        let k = key(&c);
        match out.last_mut() {
            Some(last) if (last.year, last.month, last.day) == (k.year, k.month, k.day) => {
                last.count += c.count
            }
            _ => out.push(DateNode { count: c.count, ..k }),
        }
    }
    out
}

fn children(counts: &[DateCount], node: DateNode) -> Option<gio::ListModel> {
    let in_year = counts.iter().copied().filter(|c| c.year == node.year);
    let nodes = match (node.month, node.day) {
        (_, Some(_)) => return None,
        (Some(m), None) => group(in_year.filter(|c| c.month == m), |c| DateNode {
            year: c.year,
            month: Some(c.month),
            day: Some(c.day),
            count: 0,
        }),
        (None, None) => group(in_year, |c| DateNode {
            year: c.year,
            month: Some(c.month),
            day: None,
            count: 0,
        }),
    };
    (!nodes.is_empty()).then(|| store_of(nodes).upcast())
}

pub struct DatesTab {
    pub root: gtk::Paned,
    db: Rc<Db>,
    counts: Rc<RefCell<Vec<DateCount>>>,
    years: gio::ListStore,
    tree: gtk::TreeListModel,
    tree_selection: gtk::SingleSelection,
    title: gtk::Label,
    grid: PhotoGrid,
    /// Не реагировать на смену выбора, пока дерево перестраивается.
    rebuilding: Cell<bool>,
    /// Что сейчас показано в сетке: узел и число фото — чтобы не перезагружать то же самое.
    shown: Cell<Option<(NodeKey, u32)>>,
}

impl DatesTab {
    pub fn new(db: Rc<Db>, cache: Rc<TextureCache>) -> Rc<Self> {
        let counts: Rc<RefCell<Vec<DateCount>>> = Rc::default();
        let years = gio::ListStore::new::<glib::BoxedAnyObject>();
        let c = counts.clone();
        let tree = gtk::TreeListModel::new(years.clone(), false, false, move |obj| {
            let node = *obj.downcast_ref::<glib::BoxedAnyObject>()?.borrow::<DateNode>();
            children(&c.borrow(), node)
        });
        let tree_selection = gtk::SingleSelection::new(Some(tree.clone()));

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            let count = gtk::Label::new(None);
            count.add_css_class("dim-label");
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            b.append(&label);
            b.append(&count);
            let expander = gtk::TreeExpander::new();
            expander.set_child(Some(&b));
            item.set_child(Some(&expander));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
            let expander = item.child().and_downcast::<gtk::TreeExpander>().unwrap();
            expander.set_list_row(Some(&row));
            let Some(obj) = row.item().and_downcast::<glib::BoxedAnyObject>() else { return };
            let node = *obj.borrow::<DateNode>();
            let b = expander.child().and_downcast::<gtk::Box>().unwrap();
            let label = b.first_child().and_downcast::<gtk::Label>().unwrap();
            let count = label.next_sibling().and_downcast::<gtk::Label>().unwrap();
            label.set_text(&node.label());
            count.set_text(&node.count.to_string());
        });

        let list = gtk::ListView::new(Some(tree_selection.clone()), Some(factory));
        list.add_css_class("navigation-sidebar");
        list.connect_activate(|list, pos| {
            if let Some(row) = list.model().and_then(|m| m.item(pos)).and_downcast::<gtk::TreeListRow>() {
                row.set_expanded(!row.is_expanded());
            }
        });
        let tree_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .vexpand(true)
            .build();

        let grid = PhotoGrid::new(cache, 160, 12);
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.add_css_class("pane-title");
        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.append(&title);
        right.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        right.append(&grid.root);

        let root = gtk::Paned::new(gtk::Orientation::Horizontal);
        root.set_start_child(Some(&tree_scroll));
        root.set_end_child(Some(&right));
        root.set_resize_start_child(false);
        root.set_shrink_start_child(false);
        root.set_position(240);

        let this = Rc::new(Self {
            root,
            db,
            counts,
            years,
            tree,
            tree_selection,
            title,
            grid,
            rebuilding: Cell::new(false),
            shown: Cell::new(None),
        });
        let weak = Rc::downgrade(&this);
        this.tree_selection.connect_selected_item_notify(move |_| {
            if let Some(t) = weak.upgrade() {
                if !t.rebuilding.get() {
                    t.load_selected(false);
                }
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
        // Годы — от новых к старым.
        let mut years = group(counts.iter().copied(), |c| DateNode {
            year: c.year,
            month: None,
            day: None,
            count: 0,
        });
        years.reverse();

        let expanded: HashSet<NodeKey> = (0..self.tree.n_items())
            .filter_map(|i| self.tree.row(i))
            .filter(|r| r.is_expanded())
            .filter_map(|r| DateNode::of_row(&r).map(|n| n.key()))
            .collect();
        let selected = self.selected_node().map(|n| n.key());

        self.rebuilding.set(true);
        *self.counts.borrow_mut() = counts;
        let objs: Vec<glib::BoxedAnyObject> = years.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.years.splice(0, self.years.n_items(), &objs);
        // Раскрытие узла вставляет детей сразу после него — они тоже будут обойдены.
        let mut i = 0;
        while i < self.tree.n_items() {
            if let Some(node) = self.tree.row(i).and_then(|r| {
                let n = DateNode::of_row(&r)?;
                if expanded.contains(&n.key()) {
                    r.set_expanded(true);
                }
                Some(n)
            }) {
                if Some(node.key()) == selected {
                    self.tree_selection.set_selected(i);
                }
            }
            i += 1;
        }
        self.rebuilding.set(false);
        self.load_selected(force);
    }

    fn selected_node(&self) -> Option<DateNode> {
        DateNode::of_row(&self.tree_selection.selected_item().and_downcast::<gtk::TreeListRow>()?)
    }

    fn load_selected(&self, force: bool) {
        let Some(node) = self.selected_node() else {
            self.shown.set(None);
            self.title.set_text("Нет фотографий");
            self.grid.set_photos(&[]);
            return;
        };
        if !force && self.shown.get() == Some((node.key(), node.count)) {
            return;
        }
        self.shown.set(Some((node.key(), node.count)));
        let _t = trace::span(format!("dates.load {}", node.title()));
        let photos = self.db.photos_by_date(node.year, node.month, node.day).unwrap_or_default();
        self.title.set_text(&format!("{} — {} фото", node.title(), photos.len()));
        self.grid.set_photos(&photos);
    }
}
