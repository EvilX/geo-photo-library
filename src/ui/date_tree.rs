//! Дерево дат год → месяц → день со счётчиками фото: вкладка «По датам» и фильтр карты.
use crate::{db::DateCount, i18n};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

/// Идентификатор узла дерева: (год, месяц, день).
pub type NodeKey = (i32, Option<u32>, Option<u32>);

/// Год строки «Всё время» — такого года в БД нет.
const ALL_TIME: i32 = i32::MIN;

#[derive(Debug, Clone, Copy)]
pub struct DateNode {
    pub year: i32,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub count: u32,
}

impl DateNode {
    pub fn key(&self) -> NodeKey {
        (self.year, self.month, self.day)
    }

    /// Строка «Всё время» над годами (только в фильтре карты).
    pub fn is_all_time(&self) -> bool {
        self.year == ALL_TIME
    }

    fn of_row(row: &gtk::TreeListRow) -> Option<Self> {
        Some(*row.item().and_downcast::<glib::BoxedAnyObject>()?.borrow::<DateNode>())
    }

    fn label(&self) -> String {
        match (self.month, self.day) {
            _ if self.is_all_time() => i18n::all_time().to_string(),
            (Some(m), Some(d)) => i18n::day(m, d),
            (Some(m), None) => i18n::month(m).to_string(),
            _ => self.year.to_string(),
        }
    }

    pub fn title(&self) -> String {
        match (self.month, self.day) {
            _ if self.is_all_time() => i18n::all_time().to_string(),
            (Some(m), Some(d)) => i18n::full_day(self.year, m, d),
            (Some(m), None) => format!("{} {}", i18n::month(m), self.year),
            _ => i18n::year(self.year),
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
    if node.is_all_time() {
        return None;
    }
    let in_year = counts.iter().copied().filter(|c| c.year == node.year);
    let nodes = match (node.month, node.day) {
        (_, Some(_)) => return None,
        (Some(m), None) => group(in_year.filter(|c| c.month == m), |c| DateNode {
            year: c.year,
            month: Some(c.month),
            day: Some(c.day),
            count: 0,
        }),
        // Месяцы — от новых к старым (декабрь сверху), как годы; дни — по порядку.
        (None, None) => {
            let mut months = group(in_year, |c| DateNode {
                year: c.year,
                month: Some(c.month),
                day: None,
                count: 0,
            });
            months.reverse();
            months
        }
    };
    (!nodes.is_empty()).then(|| store_of(nodes).upcast())
}

pub struct DateTree {
    /// Список в прокрутке — кладётся в окно как есть.
    pub root: gtk::ScrolledWindow,
    pub selection: gtk::SingleSelection,
    /// Первой строкой — «Всё время».
    all_time: bool,
    counts: Rc<RefCell<Vec<DateCount>>>,
    years: gio::ListStore,
    tree: gtk::TreeListModel,
    /// Дерево перестраивается — смену выбора в это время не обрабатывать.
    rebuilding: Cell<bool>,
    /// Узел, выбранный пользователем: выделяется снова, когда опять появится в дереве.
    wanted: Cell<Option<NodeKey>>,
    /// Обработчики смены выбора пользователем (`connect_changed`).
    changed: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl DateTree {
    pub fn new(all_time: bool) -> Rc<Self> {
        let counts: Rc<RefCell<Vec<DateCount>>> = Rc::default();
        let years = gio::ListStore::new::<glib::BoxedAnyObject>();
        let c = counts.clone();
        let tree = gtk::TreeListModel::new(years.clone(), false, false, move |obj| {
            let node = *obj.downcast_ref::<glib::BoxedAnyObject>()?.borrow::<DateNode>();
            children(&c.borrow(), node)
        });
        let selection = gtk::SingleSelection::new(Some(tree.clone()));
        if all_time {
            // В фильтре карты выбранного периода может не быть среди строк — тогда
            // ничего не выделено, а не первая строка.
            selection.set_autoselect(false);
            selection.set_can_unselect(true);
        }

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

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("navigation-sidebar");
        list.connect_activate(|list, pos| {
            if let Some(row) = list.model().and_then(|m| m.item(pos)).and_downcast::<gtk::TreeListRow>() {
                row.set_expanded(!row.is_expanded());
            }
        });
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&list)
            .vexpand(true)
            .build();

        let this = Rc::new(Self {
            root,
            selection,
            all_time,
            counts,
            years,
            tree,
            rebuilding: Cell::new(false),
            wanted: Cell::new(all_time.then_some((ALL_TIME, None, None))),
            changed: RefCell::default(),
        });
        let weak = Rc::downgrade(&this);
        this.selection.connect_selected_item_notify(move |_| {
            if let Some(t) = weak.upgrade() {
                t.selection_changed();
            }
        });
        if all_time {
            // Свёрнутый год развернули обратно: строка выбранного периода вернулась в дерево —
            // выделить её снова. Из простоя: в самом сигнале модель ещё перестраивается.
            let weak = Rc::downgrade(&this);
            this.tree.connect_items_changed(move |_, _, _, added| {
                let Some(t) = weak.upgrade() else { return };
                if added == 0 || t.rebuilding.get() || t.selection.selected_item().is_some() {
                    return;
                }
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(t) = weak.upgrade() {
                        t.reselect_wanted();
                    }
                });
            });
        }
        this
    }

    /// Вызывать `f` при смене выбора пользователем (не при перестройке дерева и не когда
    /// выбранная строка скрылась под свёрнутым узлом).
    pub fn connect_changed(self: &Rc<Self>, f: impl Fn() + 'static) {
        self.changed.borrow_mut().push(Rc::new(f));
    }

    /// Выбор в списке изменился.
    ///
    /// В фильтре карты `GtkSingleSelection` снимает выделение в двух случаях, и различить их
    /// можно только по дереву. Строки выбранного периода больше нет среди показанных — её
    /// скрыло сворачивание года или месяца: период остаётся прежним, обработчики не зовутся.
    /// Строка на месте — выделение сняли сами (Ctrl + щелчок): выбирается «Всё время», чтобы
    /// не остаться без подсвеченной строки.
    fn selection_changed(&self) {
        if self.rebuilding.get() {
            return;
        }
        match self.selected() {
            Some(node) => self.wanted.set(Some(node.key())),
            None if self.all_time => {
                if self.wanted.get().is_some_and(|key| self.position(key).is_none()) {
                    return;
                }
                if self.tree.n_items() > 0 {
                    // Выбор «Всё время» придёт сюда же ещё раз — уже с выделенной строкой.
                    self.selection.set_selected(0);
                    return;
                }
                self.wanted.set(None);
            }
            None => self.wanted.set(None),
        }
        let handlers = self.changed.borrow().clone();
        for f in handlers {
            f();
        }
    }

    /// Снова выделить строку выбранного периода, если она есть в дереве, а выделения нет.
    /// Период при этом не меняется — обработчики не зовутся.
    fn reselect_wanted(&self) {
        if self.rebuilding.get() || self.selection.selected_item().is_some() {
            return;
        }
        if let Some(pos) = self.wanted.get().and_then(|key| self.position(key)) {
            self.rebuilding.set(true);
            self.selection.set_selected(pos);
            self.rebuilding.set(false);
        }
    }

    /// Номер строки узла среди показанных (под свёрнутым узлом строки нет).
    fn position(&self, key: NodeKey) -> Option<u32> {
        (0..self.tree.n_items()).find(|&i| self.tree.row(i).and_then(|r| DateNode::of_row(&r)).is_some_and(|n| n.key() == key))
    }

    /// Заменить счётчики: годы — от новых к старым. Раскрытые узлы и выбор сохраняются.
    /// Если выбранного узла больше нет: на вкладке дат выбирается первая строка, в фильтре
    /// карты выделение снимается (период остаётся и выделится, когда узел вернётся).
    pub fn set_counts(&self, counts: Vec<DateCount>) {
        let mut years = group(counts.iter().copied(), |c| DateNode { year: c.year, month: None, day: None, count: 0 });
        years.reverse();
        if self.all_time {
            let total = counts.iter().map(|c| c.count).sum();
            years.insert(0, DateNode { year: ALL_TIME, month: None, day: None, count: total });
        }

        let mut expanded: HashSet<NodeKey> = (0..self.tree.n_items())
            .filter_map(|i| self.tree.row(i))
            .filter(|r| r.is_expanded())
            .filter_map(|r| DateNode::of_row(&r).map(|n| n.key()))
            .collect();
        let selected = self.wanted.get().or_else(|| self.selected().map(|n| n.key()));
        // Год и месяц выбранного периода раскрыты всегда: даже если его узел на время
        // пропадал из дерева, при возвращении выбранная строка снова видна.
        if let Some((year, Some(month), day)) = selected {
            expanded.insert((year, None, None));
            if day.is_some() {
                expanded.insert((year, Some(month), None));
            }
        }

        self.rebuilding.set(true);
        *self.counts.borrow_mut() = counts;
        let objs: Vec<glib::BoxedAnyObject> = years.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.years.splice(0, self.years.n_items(), &objs);
        // Раскрытие узла вставляет детей сразу после него — они тоже будут обойдены.
        let mut found = false;
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
                    self.selection.set_selected(i);
                    found = true;
                }
            }
            i += 1;
        }
        if !found {
            if self.all_time {
                self.selection.set_selected(gtk::INVALID_LIST_POSITION);
            } else if self.tree.n_items() > 0 {
                self.selection.set_selected(0);
                self.wanted.set(self.selected().map(|n| n.key()));
            }
        }
        self.rebuilding.set(false);
    }

    pub fn selected(&self) -> Option<DateNode> {
        DateNode::of_row(&self.selection.selected_item().and_downcast::<gtk::TreeListRow>()?)
    }

    /// Выбрать узел (отладочный хук); `false` — такого узла нет среди показанных строк.
    pub fn select(&self, key: NodeKey) -> bool {
        let pos = self.position(key);
        if let Some(i) = pos {
            self.selection.set_selected(i);
        }
        pos.is_some()
    }

    /// Выбрать строку «Всё время» (фильтр карты), как это сделал бы пользователь;
    /// `false` — строки ещё нет (дерево не заполнено).
    pub fn select_all_time(&self) -> bool {
        self.all_time && self.select((ALL_TIME, None, None))
    }
}
