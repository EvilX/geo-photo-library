//! Сетка миниатюр, используется на обеих вкладках.
use super::menu::{icon_menu, Item};
use super::textures::{
    copy_to_clipboard, open_external, open_in_editor, set_thumbnail, show_in_folder, TextureCache, PHOTO_ICON, VIDEO_ICON,
};
use crate::{
    config::{self, SortOrder},
    db::Photo,
    i18n, video,
};
use gtk::{gdk, gio, glib, graphene, pango, prelude::*};
use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
};

/// Результаты перемещения в корзину: по одному на каждое фото.
pub type TrashResults = Vec<(Photo, Result<(), glib::Error>)>;
/// Обработчик результата перемещения фото в корзину.
type TrashedHandler = Box<dyn Fn(TrashResults)>;
/// Обработчик пункта меню над фото («Информация», «Найти на карте»).
type PhotoHandler = Box<dyn Fn(&Photo)>;
/// Обработчик пункта меню над всеми выделенными фото («Редактировать координаты»).
type PhotosHandler = Box<dyn Fn(Vec<Photo>)>;
/// Разрешение менять файлы фото: `false` — действие не выполнять (причину сообщает сам обработчик).
type GuardHandler = Box<dyn Fn(&[Photo]) -> bool>;

/// Обработчики действий контекстного меню, которые выполняет окно.
#[derive(Clone, Default)]
struct Handlers {
    trashed: Rc<RefCell<Option<TrashedHandler>>>,
    info: Rc<RefCell<Option<PhotoHandler>>>,
    show_on_map: Rc<RefCell<Option<PhotoHandler>>>,
    edit_exif: Rc<RefCell<Option<PhotosHandler>>>,
    picked: Rc<RefCell<Option<PhotoHandler>>>,
    guard: Rc<RefCell<Option<GuardHandler>>>,
}

impl Handlers {
    /// Можно ли сейчас менять файлы этих фото (в них не идёт запись).
    fn allowed(&self, photos: &[Photo]) -> bool {
        self.guard.borrow().as_ref().is_none_or(|f| f(photos))
    }
}

/// Шаг кнопок «−»/«+» у слайдера масштаба миниатюр, %.
const ZOOM_BUTTON_STEP: f64 = 10.0;

/// С какого размера списка ячеек вычищать из него уничтоженные.
const CELLS_PRUNE_AT: usize = 1024;

/// Верхний предел колонок при уменьшенных миниатюрах (см. `set_scale`).
const MAX_COLUMNS_LIMIT: u32 = 16;

#[derive(Clone)]
pub struct PhotoGrid {
    /// Сетка в прокрутке; поверх — подпись пустого списка.
    pub root: gtk::Overlay,
    pub view: gtk::GridView,
    /// Подпись посередине, пока список пуст.
    empty: gtk::Label,
    cache: Rc<TextureCache>,
    selection: gtk::MultiSelection,
    store: gio::ListStore,
    order: Rc<Cell<SortOrder>>,
    /// Сторона миниатюры при масштабе 100%.
    base_size: i32,
    /// Текущая сторона миниатюры в пикселях.
    size: Rc<Cell<i32>>,
    /// Предел колонок при масштабе 100%.
    base_max_columns: u32,
    /// Все созданные ячейки — чтобы менять размер картинок и привязывать фото заново
    /// без пересоздания списка.
    cells: Rc<RefCell<Vec<glib::WeakRef<gtk::ListItem>>>>,
    handlers: Handlers,
}

impl PhotoGrid {
    /// `size` — сторона миниатюры в логических пикселях при масштабе 100%,
    /// `max_columns` — предел колонок.
    ///
    /// GridView держит виджеты примерно для `max_columns × 30` элементов, поэтому
    /// завышенный предел отключает виртуализацию: создаются ячейки для всех фото.
    pub fn new(cache: Rc<TextureCache>, size: i32, max_columns: u32) -> Self {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        // Несколько фото: Ctrl/Shift + щелчок, Ctrl+A, Shift + стрелки и рамка мышью.
        let selection = gtk::MultiSelection::new(Some(store.clone()));

        let base_size = size;
        let size = Rc::new(Cell::new(base_size));
        let cells: Rc<RefCell<Vec<glib::WeakRef<gtk::ListItem>>>> = Rc::default();

        // Контекстное меню: действия работают с фото, на котором нажали правую кнопку,
        // а если оно среди нескольких выделенных — со всеми выделенными (меню короче:
        // только копирование и удаление).
        let menu_photos: Rc<RefCell<Vec<Photo>>> = Rc::default();
        // Группа действий меню создаётся вместе с видом — ниже; до показа меню в ней
        // отключаются пункты, неприменимые к видео.
        let actions: Rc<OnceCell<gio::SimpleActionGroup>> = Rc::default();
        let handlers = Handlers::default();
        let (popover, popover_many) = (context_menu(), context_menu_many());
        for p in [&popover, &popover_many] {
            p.set_has_arrow(false);
            p.set_halign(gtk::Align::Start);
        }

        let factory = gtk::SignalListItemFactory::new();
        let (cur_size, all_cells) = (size.clone(), cells.clone());
        let (pop, pop_many, mp, sel, acts) =
            (popover.clone(), popover_many.clone(), menu_photos.clone(), selection.clone(), actions.clone());
        let picked = handlers.picked.clone();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let image = gtk::Image::new();
            image.set_pixel_size(cur_size.get());
            {
                // GridView при больших изменениях списка уничтожает ячейки и создаёт
                // новые — без чистки список слабых ссылок рос бы всё время работы.
                let mut cells = all_cells.borrow_mut();
                if cells.len() >= CELLS_PRUNE_AT && cells.len().is_power_of_two() {
                    cells.retain(|w| w.upgrade().is_some());
                }
                cells.push(item.downgrade());
            }
            image.add_css_class("photo-thumb");
            image.set_overflow(gtk::Overflow::Hidden);
            // Значок видео с длительностью — в правом нижнем углу миниатюры.
            // Значок, а не символ «▶»: его есть не в каждом шрифте.
            let badge = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            let play = gtk::Image::from_icon_name("media-playback-start-symbolic");
            play.set_pixel_size(10);
            badge.append(&play);
            badge.append(&gtk::Label::new(None));
            badge.add_css_class("video-badge");
            badge.set_halign(gtk::Align::End);
            badge.set_valign(gtk::Align::End);
            badge.set_can_target(false);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&image));
            overlay.add_overlay(&badge);
            overlay.set_halign(gtk::Align::Center);
            let label = gtk::Label::new(None);
            label.add_css_class("photo-caption");
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_max_width_chars(1);
            let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
            b.set_margin_top(4);
            b.set_margin_bottom(4);
            b.append(&overlay);
            b.append(&label);
            item.set_child(Some(&b));

            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_SECONDARY);
            let (weak_item, pop, pop_many, mp, sel, acts) =
                (item.downgrade(), pop.clone(), pop_many.clone(), mp.clone(), sel.clone(), acts.clone());
            click.connect_pressed(move |g, _, x, y| {
                let Some(item) = weak_item.upgrade() else { return };
                let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
                g.set_state(gtk::EventSequenceState::Claimed);
                let pos = item.position();
                let many = sel.is_selected(pos) && sel.selection().size() > 1;
                if many {
                    *mp.borrow_mut() = selected_in(&sel);
                } else {
                    sel.select_item(pos, true);
                    *mp.borrow_mut() = vec![obj.borrow::<Photo>().clone()];
                }
                if let Some(group) = acts.get() {
                    enable_for(group, &mp.borrow());
                }
                let pop = if many { &pop_many } else { &pop };
                let (Some(cell), Some(parent)) = (g.widget(), pop.parent()) else { return };
                let at = cell
                    .compute_point(&parent, &graphene::Point::new(x as f32, y as f32))
                    .unwrap_or(graphene::Point::new(x as f32, y as f32));
                pop.set_pointing_to(Some(&gdk::Rectangle::new(at.x() as i32, at.y() as i32, 1, 1)));
                pop.popup();
            });
            b.add_controller(click);

            // Обычный щелчок по фото (без Ctrl и Shift) — «выбрал это фото»: карта центрируется
            // на нём. Жест не перехватывает событие — выделение делает сам GridView.
            let pick = gtk::GestureClick::new();
            pick.set_button(gdk::BUTTON_PRIMARY);
            let (weak_item, picked) = (item.downgrade(), picked.clone());
            pick.connect_released(move |g, n, _, _| {
                let modifiers = gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK;
                if n != 1 || g.current_event_state().intersects(modifiers) {
                    return;
                }
                let Some(obj) = weak_item.upgrade().and_then(|i| i.item()).and_downcast::<glib::BoxedAnyObject>() else {
                    return;
                };
                let photo = obj.borrow::<Photo>().clone();
                if let Some(f) = picked.borrow().as_ref() {
                    f(&photo);
                }
            });
            b.add_controller(pick);
        });
        let bind_cache = cache.clone();
        factory.connect_bind(move |_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                bind_cell(&bind_cache, item);
            }
        });

        let view = gtk::GridView::new(Some(selection.clone()), Some(factory));
        view.set_enable_rubberband(true);
        popover.set_parent(&view);
        popover_many.set_parent(&view);
        // Привязанное вручную меню нужно и отвязать вручную (см. предупреждение GTK
        // «Finalizing … but it still has children left»).
        let (pop, pop_many) = (popover.clone(), popover_many.clone());
        view.connect_destroy(move |_| {
            pop.unparent();
            pop_many.unparent();
        });
        let group = photo_actions(&view, &menu_photos, &handlers);
        view.insert_action_group("photo", Some(&group));
        view.add_controller(key_shortcuts(&view, &selection, &menu_photos, &group, &cells, &popover, &popover_many));
        let _ = actions.set(group);
        view.set_max_columns(max_columns);
        view.set_min_columns(1);
        // Двойной щелчок или Enter — просмотр, с Ctrl — редактирование.
        view.connect_activate(|view, pos| {
            let model = view.model().unwrap();
            let Some(obj) = model.item(pos).and_downcast::<glib::BoxedAnyObject>() else { return };
            let path = obj.borrow::<Photo>().path.clone();
            if ctrl_pressed(view) {
                open_in_editor(view, &path);
            } else {
                open_external(view, &path);
            }
        });

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&view)
            .build();
        // Пустой список — не голое поле, а подпись посередине (текст задаёт вкладка).
        let empty = gtk::Label::new(None);
        empty.add_css_class("dim-label");
        empty.set_wrap(true);
        empty.set_justify(gtk::Justification::Center);
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        empty.set_margin_start(12);
        empty.set_margin_end(12);
        empty.set_can_target(false);
        let root = gtk::Overlay::new();
        root.set_child(Some(&scroll));
        root.add_overlay(&empty);
        let label = empty.downgrade();
        store.connect_items_changed(move |store, _, _, _| {
            if let Some(label) = label.upgrade() {
                label.set_visible(store.n_items() == 0);
            }
        });
        Self {
            root,
            view,
            empty,
            cache,
            selection,
            store,
            order: Rc::default(),
            base_size,
            base_max_columns: max_columns,
            size,
            cells,
            handlers,
        }
    }

    /// Вызывается после попытки переместить фото в корзину из контекстного меню.
    pub fn connect_trashed(&self, f: impl Fn(TrashResults) + 'static) {
        *self.handlers.trashed.borrow_mut() = Some(Box::new(f));
    }

    /// Вызывается пунктом «Информация» контекстного меню.
    pub fn connect_info(&self, f: impl Fn(&Photo) + 'static) {
        *self.handlers.info.borrow_mut() = Some(Box::new(f));
    }

    /// Вызывается пунктом «Найти на карте» контекстного меню.
    pub fn connect_show_on_map(&self, f: impl Fn(&Photo) + 'static) {
        *self.handlers.show_on_map.borrow_mut() = Some(Box::new(f));
    }

    pub fn connect_edit_exif(&self, f: impl Fn(Vec<Photo>) + 'static) {
        *self.handlers.edit_exif.borrow_mut() = Some(Box::new(f));
    }

    /// Спрашивается перед действием, которое меняет файлы фото (перемещение в корзину):
    /// `false` — не выполнять.
    pub fn connect_guard(&self, f: impl Fn(&[Photo]) -> bool + 'static) {
        *self.handlers.guard.borrow_mut() = Some(Box::new(f));
    }

    /// Подпись посередине пустого списка.
    pub fn set_empty_text(&self, text: &str) {
        if self.empty.text() != text {
            self.empty.set_text(text);
        }
    }

    /// Вызывается обычным щелчком по фото (без Ctrl и Shift).
    pub fn connect_picked(&self, f: impl Fn(&Photo) + 'static) {
        *self.handlers.picked.borrow_mut() = Some(Box::new(f));
    }

    /// Изменилось выделение (щелчком, клавишами, рамкой или из программы) или список.
    /// Удаление выделенных фото из списка `MultiSelection` сообщает только `items-changed`.
    pub fn connect_selection_changed(&self, f: impl Fn() + 'static) {
        let f = Rc::new(f);
        let g = f.clone();
        self.selection.connect_selection_changed(move |_, _, _| f());
        self.selection.connect_items_changed(move |_, _, _, _| g());
    }

    /// Масштаб миниатюр в процентах от исходного размера. Предел колонок меняется
    /// обратно пропорционально, но не выше [`MAX_COLUMNS_LIMIT`] — иначе GridView
    /// создаёт слишком много ячеек.
    pub fn set_scale(&self, percent: u32) {
        let size = (self.base_size as f64 * percent as f64 / 100.0).round() as i32;
        if self.size.replace(size) == size {
            return;
        }
        let columns = (self.base_max_columns as f64 * 100.0 / percent as f64).round() as u32;
        self.view.set_max_columns(columns.clamp(1, MAX_COLUMNS_LIMIT));
        self.cells.borrow_mut().retain(|w| match w.upgrade() {
            Some(item) => {
                if let Some(parts) = CellWidgets::of(&item) {
                    parts.image.set_pixel_size(size);
                }
                true
            }
            None => false,
        });
    }

    /// Привязать фото к показанным ячейкам заново: файлы миниатюр перезаписаны по тем же
    /// путям (после сканирования или записи EXIF), а сам список не менялся.
    pub fn rebind(&self) {
        let cells: Vec<gtk::ListItem> = self.cells.borrow().iter().filter_map(|w| w.upgrade()).collect();
        for item in cells {
            bind_cell(&self.cache, &item);
        }
    }

    /// `photos` — по возрастанию даты; показываются в текущем порядке сортировки.
    pub fn set_photos(&self, photos: &[Photo]) {
        self.set_objects(photos.iter().cloned().map(glib::BoxedAnyObject::new).collect());
    }

    /// Обновить список, не трогая то, что не изменилось: заменяется только участок между
    /// общим началом и общим концом прежнего и нового списков. Выделение и прокрутка
    /// остаются на месте — так убираются удалённые фото и добавляются найденные сканером.
    /// Выделенные фото, попавшие в заменённый участок, выделяются снова (без прокрутки).
    pub fn update_photos(&self, photos: &[Photo]) {
        let newest = self.order.get() == SortOrder::Newest;
        let (n_old, n_new) = (self.store.n_items(), photos.len() as u32);
        // i-е фото нового списка в порядке показа.
        let at = |i: u32| &photos[if newest { (n_new - 1 - i) as usize } else { i as usize }];
        let same = |old: u32, new: u32| {
            self.store.item(old).and_downcast::<glib::BoxedAnyObject>().is_some_and(|o| *o.borrow::<Photo>() == *at(new))
        };
        let max = n_old.min(n_new);
        let mut prefix = 0;
        while prefix < max && same(prefix, prefix) {
            prefix += 1;
        }
        if prefix == n_old && n_old == n_new {
            return;
        }
        let mut suffix = 0;
        while suffix < max - prefix && same(n_old - 1 - suffix, n_new - 1 - suffix) {
            suffix += 1;
        }
        let selected = self.selected_ids();
        let added: Vec<glib::BoxedAnyObject> =
            (prefix..n_new - suffix).map(|i| glib::BoxedAnyObject::new(at(i).clone())).collect();
        self.store.splice(prefix, n_old - prefix - suffix, &added);
        if !selected.is_empty() && self.selected_ids() != selected {
            self.select_ids(&selected);
        }
    }

    /// То же для готовых объектов с [`Photo`] внутри: карта создаёт их один раз при
    /// загрузке и при каждом движении передаёт те же объекты, не копируя данные фото.
    pub fn set_objects(&self, mut objs: Vec<glib::BoxedAnyObject>) {
        if self.order.get() == SortOrder::Newest {
            objs.reverse();
        }
        self.store.splice(0, self.store.n_items(), &objs);
    }

    /// Сменить сортировку: показанные фото переставляются, выбранное остаётся
    /// выбранным и прокручивается в видимую область, иначе — прокрутка в начало.
    pub fn set_order(&self, order: SortOrder) {
        if self.order.replace(order) == order {
            return;
        }
        let selected = self.selected_ids();
        let mut objs: Vec<glib::BoxedAnyObject> = (0..self.store.n_items())
            .filter_map(|i| self.store.item(i).and_downcast())
            .collect();
        objs.reverse();
        self.store.splice(0, self.store.n_items(), &objs);
        if !self.restore_selection(&selected) && !objs.is_empty() {
            self.view.scroll_to(0, gtk::ListScrollFlags::NONE, None);
        }
    }

    pub fn photo(&self, pos: u32) -> Option<Photo> {
        self.store
            .item(pos)
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|o| o.borrow::<Photo>().clone())
    }

    /// Выделенное фото, если оно одно.
    pub fn selected_photo(&self) -> Option<Photo> {
        let set = self.selection.selection();
        (set.size() == 1).then(|| self.photo(set.minimum())).flatten()
    }

    /// Выделенные фото по порядку показа.
    pub fn selected_photos(&self) -> Vec<Photo> {
        selected_in(&self.selection)
    }

    /// Сколько фото выделено.
    pub fn selected_count(&self) -> u64 {
        self.selection.selection().size()
    }

    /// id выделенных фото — без копирования данных фото.
    pub fn selected_ids(&self) -> Vec<i64> {
        let set = self.selection.selection();
        (0..set.size() as u32).filter_map(|i| self.id_at(set.nth(i))).collect()
    }

    fn id_at(&self, pos: u32) -> Option<i64> {
        self.store.item(pos).and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Photo>().id)
    }

    /// Выделить фото с данным id (только его) и прокрутить к нему.
    pub fn select_id(&self, id: i64) -> bool {
        // Сравниваем id на месте: `photo()` копировал бы каждое фото со строками.
        let Some(pos) = (0..self.store.n_items()).find(|&i| self.id_at(i) == Some(id)) else {
            return false;
        };
        self.selection.select_item(pos, true);
        self.view.scroll_to(pos, gtk::ListScrollFlags::FOCUS, None);
        true
    }

    /// Вернуть выделение после замены списка: одно фото — выделить и прокрутить к нему,
    /// несколько — выделить те, что остались в списке, без прокрутки. `false` — ни одного нет.
    pub fn restore_selection(&self, ids: &[i64]) -> bool {
        match ids {
            [] => false,
            [id] => self.select_id(*id),
            _ => self.select_ids(ids),
        }
    }

    /// Выделить фото с этими id, что есть в списке, без прокрутки. `false` — ни одного нет.
    fn select_ids(&self, ids: &[i64]) -> bool {
        let ids: std::collections::HashSet<i64> = ids.iter().copied().collect();
        let n = self.store.n_items();
        let set = gtk::Bitset::new_empty();
        for i in (0..n).filter(|&i| self.id_at(i).is_some_and(|id| ids.contains(&id))) {
            set.add(i);
        }
        let found = !set.is_empty();
        self.selection.set_selection(&set, &gtk::Bitset::new_range(0, n));
        found
    }

    /// Выделить первые `n` фото (отладочный хук).
    pub fn select_first(&self, n: u32) {
        let n = n.min(self.store.n_items());
        self.selection.set_selection(&gtk::Bitset::new_range(0, n), &gtk::Bitset::new_range(0, self.store.n_items()));
    }
}

/// Виджеты ячейки сетки, собранные в `connect_setup`.
struct CellWidgets {
    /// Вся ячейка: картинка со значком видео и подпись под ней.
    root: gtk::Box,
    image: gtk::Image,
    /// Значок видео с длительностью поверх картинки.
    badge: gtk::Box,
    caption: gtk::Label,
}

impl CellWidgets {
    fn of(item: &gtk::ListItem) -> Option<Self> {
        let root = item.child().and_downcast::<gtk::Box>()?;
        let overlay = root.first_child().and_downcast::<gtk::Overlay>()?;
        let image = overlay.child().and_downcast::<gtk::Image>()?;
        let badge = image.next_sibling().and_downcast::<gtk::Box>()?;
        let caption = overlay.next_sibling().and_downcast::<gtk::Label>()?;
        Some(Self { root, image, badge, caption })
    }
}

/// Показать в ячейке её фото: миниатюра, значок видео, подпись.
fn bind_cell(cache: &Rc<TextureCache>, item: &gtk::ListItem) {
    let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
    let Some(cell) = CellWidgets::of(item) else { return };
    let photo = obj.borrow::<Photo>();
    cell.badge.set_visible(photo.video);
    if let Some(duration) = cell.badge.last_child().and_downcast::<gtk::Label>() {
        let text = photo.duration.filter(|_| photo.video).map(video::format_duration);
        duration.set_visible(text.is_some());
        duration.set_text(text.as_deref().unwrap_or_default());
    }
    // Ячейка переиспользуется при прокрутке: декодируем и ставим текстуру, только если
    // в ней всё ещё тот же элемент.
    let (weak_item, expected) = (item.downgrade(), obj.clone().upcast::<glib::Object>());
    let icon = if photo.video { VIDEO_ICON } else { PHOTO_ICON };
    set_thumbnail(cache, &cell.image, photo.thumb_path.as_deref(), icon, move || {
        weak_item.upgrade().and_then(|i| i.item()).as_ref() == Some(&expected)
    });
    let date = photo.display_date();
    cell.caption.set_text(&date);
    let name = format!("{}\n{date}", photo.file_name());
    cell.root.set_tooltip_text(Some(&name));
    // Для программ экранного доступа ячейка — имя файла и дата, а не безымянная картинка.
    super::a11y_label(&cell.root, &format!("{}, {date}", photo.file_name()));
}

/// Клавиши сетки над выделенными фото: Delete — в корзину, Ctrl+C — копировать,
/// Menu и Shift+F10 — контекстное меню у первого выделенного фото.
fn key_shortcuts(
    view: &gtk::GridView,
    selection: &gtk::MultiSelection,
    menu_photos: &Rc<RefCell<Vec<Photo>>>,
    group: &gio::SimpleActionGroup,
    cells: &Rc<RefCell<Vec<glib::WeakRef<gtk::ListItem>>>>,
    popover: &gtk::PopoverMenu,
    popover_many: &gtk::PopoverMenu,
) -> gtk::ShortcutController {
    let keys = gtk::ShortcutController::new();
    // Действие меню над выделенными фото; без выделения клавиша не перехватывается.
    let prepare = {
        let (sel, mp, group) = (selection.clone(), menu_photos.clone(), group.clone());
        move || {
            let photos = selected_in(&sel);
            if photos.is_empty() {
                return false;
            }
            enable_for(&group, &photos);
            *mp.borrow_mut() = photos;
            true
        }
    };
    for (trigger, action) in [("Delete|KP_Delete", "trash"), ("<Control>c", "copy")] {
        let (prepare, group) = (prepare.clone(), group.clone());
        let run = gtk::CallbackAction::new(move |_, _| {
            if !prepare() {
                return glib::Propagation::Proceed;
            }
            group.activate_action(action, None);
            glib::Propagation::Stop
        });
        keys.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string(trigger), Some(run)));
    }
    let (view, sel, cells, pop, pop_many) =
        (view.downgrade(), selection.clone(), cells.clone(), popover.downgrade(), popover_many.downgrade());
    let menu = gtk::CallbackAction::new(move |_, _| {
        let (Some(view), Some(pop), Some(pop_many)) = (view.upgrade(), pop.upgrade(), pop_many.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        if !prepare() {
            return glib::Propagation::Proceed;
        }
        let set = sel.selection();
        // Меню — у первой выделенной ячейки, что сейчас на экране; иначе в углу сетки.
        let cell = cells
            .borrow()
            .iter()
            .filter_map(|w| w.upgrade())
            .filter(|i| i.item().is_some() && set.contains(i.position()))
            .min_by_key(|i| i.position())
            .and_then(|i| i.child());
        let at = cell
            .filter(|c| c.is_mapped())
            .and_then(|c| c.compute_bounds(&view))
            .map(|r| (r.x() + r.width() / 2.0, r.y() + r.height() / 2.0))
            .unwrap_or((12.0, 12.0));
        let pop = if set.size() > 1 { pop_many } else { pop };
        pop.set_pointing_to(Some(&gdk::Rectangle::new(at.0 as i32, at.1 as i32, 1, 1)));
        pop.popup();
        glib::Propagation::Stop
    });
    keys.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("Menu|<Shift>F10"), Some(menu)));
    keys
}

/// Выпадающий список сортировки для заголовка панели с фото.
pub fn sort_dropdown() -> gtk::DropDown {
    let labels: Vec<&str> = SortOrder::ALL.iter().map(|&o| i18n::sort_label(o)).collect();
    let dropdown = gtk::DropDown::from_strings(&labels);
    dropdown.set_tooltip_text(Some(i18n::sort_tooltip()));
    dropdown.set_valign(gtk::Align::Center);
    dropdown.set_margin_end(6);
    dropdown
}

/// Заголовок панели: подпись слева, выбор сортировки справа.
pub fn pane_header(title: &gtk::Label, sort: &gtk::DropDown) -> gtk::Box {
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_ellipsize(pango::EllipsizeMode::End);
    title.add_css_class("pane-title");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.append(title);
    header.append(sort);
    header
}

/// Слайдер масштаба миниатюр (50…200%, шаг 5%) для нижней строки панели.
pub fn zoom_scale() -> gtk::Scale {
    let (min, max, step) = (
        config::THUMB_SCALE_MIN as f64,
        config::THUMB_SCALE_MAX as f64,
        config::THUMB_SCALE_STEP as f64,
    );
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    scale.adjustment().set_page_increment(25.0);
    scale.set_value(100.0);
    scale.set_digits(0);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_format_value_func(|_, v| format!("{v:.0}%"));
    // Отметка исходного размера.
    scale.add_mark(100.0, gtk::PositionType::Bottom, None);
    // Под отметку GTK резервирует место снизу, и полоса оказывалась выше кнопок «−»/«+»:
    // такой же отступ сверху (класс `thumb-zoom` в CSS) возвращает её на середину строки.
    scale.add_css_class("thumb-zoom");
    scale.set_size_request(160, -1);
    scale.set_tooltip_text(Some(i18n::thumb_size_tooltip()));
    scale
}

/// Нижняя строка панели: справа слайдер масштаба между кнопками «−» и «+».
pub fn pane_footer(zoom: &gtk::Scale) -> gtk::Box {
    let smaller = zoom_button(zoom, "zoom-out-symbolic", i18n::thumbs_smaller(), -ZOOM_BUTTON_STEP);
    let larger = zoom_button(zoom, "zoom-in-symbolic", i18n::thumbs_larger(), ZOOM_BUTTON_STEP);
    // На краях диапазона соответствующая кнопка неактивна. Слайдер получает общую
    // для вкладок настройку позже (`set_adjustment`), поэтому следим и за её сменой.
    let update = {
        let (smaller, larger) = (smaller.clone(), larger.clone());
        move |scale: &gtk::Scale| {
            let adj = scale.adjustment();
            smaller.set_sensitive(adj.value() > adj.lower());
            larger.set_sensitive(adj.value() < adj.upper());
        }
    };
    update(zoom);
    zoom.connect_value_changed(update.clone());
    zoom.connect_adjustment_notify(update);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    footer.set_halign(gtk::Align::End);
    footer.set_margin_start(6);
    footer.set_margin_end(6);
    footer.append(&smaller);
    footer.append(zoom);
    footer.append(&larger);
    footer
}

/// Кнопка шага масштаба; значение округляется к сетке слайдера и не выходит за пределы.
fn zoom_button(zoom: &gtk::Scale, icon: &str, tooltip: &str, step: f64) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    super::a11y_label(&button, tooltip);
    let zoom = zoom.downgrade();
    button.connect_clicked(move |_| {
        if let Some(zoom) = zoom.upgrade() {
            let adj = zoom.adjustment();
            adj.set_value(config::snap_thumb_scale(adj.value() + step) as f64);
        }
    });
    button
}

/// Контекстное меню фото с иконками у пунктов.
fn context_menu() -> gtk::PopoverMenu {
    icon_menu(vec![
        vec![Item::new(i18n::copy(), "photo.copy", "edit-copy-symbolic").accel("<Control>c")],
        vec![
            Item::new(i18n::view(), "photo.view", "view-reveal-symbolic"),
            Item::new(i18n::edit(), "photo.edit", "document-edit-symbolic"),
            Item::new(i18n::show_in_folder(), "photo.show-in-folder", "folder-open-symbolic"),
        ],
        vec![
            Item::new(i18n::info(), "photo.info", "help-about-symbolic"),
            Item::new(i18n::show_on_map(), "photo.show-on-map", "mark-location-symbolic"),
            Item::new(i18n::edit_exif(), "photo.edit-exif", "document-edit-symbolic"),
        ],
        vec![Item::new(i18n::delete(), "photo.trash", "user-trash-symbolic").accel("Delete")],
    ])
}

/// Меню над несколькими выделенными фото: только действия, применимые ко всем сразу.
fn context_menu_many() -> gtk::PopoverMenu {
    icon_menu(vec![
        vec![Item::new(i18n::copy(), "photo.copy", "edit-copy-symbolic").accel("<Control>c")],
        vec![Item::new(i18n::edit_exif(), "photo.edit-exif", "document-edit-symbolic")],
        vec![Item::new(i18n::delete(), "photo.trash", "user-trash-symbolic").accel("Delete")],
    ])
}

/// Пункты меню, которые не применимы к видео: редактор фото, карта (координат у видео нет)
/// и EXIF. Над несколькими фото «Редактировать EXIF» доступен, если среди них есть фото.
fn enable_for(group: &gio::SimpleActionGroup, photos: &[Photo]) {
    let any_photo = photos.iter().any(|p| !p.video);
    for name in ["edit", "show-on-map", "edit-exif"] {
        if let Some(a) = group.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            a.set_enabled(any_photo);
        }
    }
}

/// Выделенные фото по порядку показа.
fn selected_in(selection: &gtk::MultiSelection) -> Vec<Photo> {
    let set = selection.selection();
    (0..set.size() as u32)
        .filter_map(|i| selection.item(set.nth(i)).and_downcast::<glib::BoxedAnyObject>())
        .map(|o| o.borrow::<Photo>().clone())
        .collect()
}

/// Действия контекстного меню (группа `photo`) над фото из `menu_photos`: копирование,
/// координаты и удаление — над всеми, остальные — над первым (меню с ними открывается для одного фото).
fn photo_actions(
    view: &gtk::GridView,
    menu_photos: &Rc<RefCell<Vec<Photo>>>,
    handlers: &Handlers,
) -> gio::SimpleActionGroup {
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: fn(&gtk::GridView, &[Photo], &Handlers)| {
        let action = gio::SimpleAction::new(name, None);
        let (view, mp, handlers) = (view.downgrade(), menu_photos.clone(), handlers.clone());
        action.connect_activate(move |_, _| {
            let Some(view) = view.upgrade() else { return };
            let photos = mp.borrow().clone();
            if !photos.is_empty() {
                f(&view, &photos, &handlers);
            }
        });
        group.add_action(&action);
    };
    add("copy", |v, ps, _| copy_to_clipboard(v, &ps.iter().map(|p| p.path.as_str()).collect::<Vec<_>>()));
    add("view", |v, ps, _| open_external(v, &ps[0].path));
    add("edit", |v, ps, _| open_in_editor(v, &ps[0].path));
    add("show-in-folder", |v, ps, _| show_in_folder(v, &ps[0].path));
    add("info", |_, ps, handlers| {
        if let Some(f) = handlers.info.borrow().as_ref() {
            f(&ps[0]);
        }
    });
    add("show-on-map", |_, ps, handlers| {
        if let Some(f) = handlers.show_on_map.borrow().as_ref() {
            f(&ps[0]);
        }
    });
    add("edit-exif", |_, ps, handlers| {
        if let Some(f) = handlers.edit_exif.borrow().as_ref() {
            f(ps.iter().filter(|p| !p.video).cloned().collect());
        }
    });
    // После подтверждения файлы перемещаются параллельно; обработчик получает все результаты разом.
    add("trash", |v, ps, handlers| {
        if !handlers.allowed(ps) {
            return;
        }
        let subject = match ps {
            [p] => p.file_name().to_string(),
            _ => i18n::photos(ps.len()),
        };
        let dialog = gtk::AlertDialog::builder()
            .modal(true)
            .message(i18n::confirm_trash(&subject))
            .detail(i18n::confirm_trash_detail())
            .buttons([i18n::cancel(), i18n::move_to_trash()])
            .cancel_button(0)
            .default_button(0)
            .build();
        let (ps, handlers) = (ps.to_vec(), handlers.clone());
        dialog.choose(v.root().and_downcast_ref::<gtk::Window>(), gio::Cancellable::NONE, move |res| {
            // Пока окно было открыто, в эти файлы могла начаться запись.
            if res == Ok(1) && handlers.allowed(&ps) {
                trash(&ps, &handlers);
            }
        });
    });
    group
}

/// Переместить фото в корзину (параллельно) и отдать обработчику все результаты разом.
fn trash(ps: &[Photo], handlers: &Handlers) {
    let total = ps.len();
    let results: Rc<RefCell<TrashResults>> = Rc::default();
    for p in ps {
        let (p, handler, results) = (p.clone(), handlers.trashed.clone(), results.clone());
        let file = gio::File::for_path(&p.path);
        file.trash_async(glib::Priority::DEFAULT, gio::Cancellable::NONE, move |res| {
            results.borrow_mut().push((p, res));
            if results.borrow().len() < total {
                return;
            }
            let done = results.take();
            if let Some(f) = handler.borrow().as_ref() {
                f(done);
            }
        });
    }
}

/// Зажат ли сейчас Ctrl (сигнал `activate` не передаёт событие).
fn ctrl_pressed(widget: &impl IsA<gtk::Widget>) -> bool {
    widget
        .display()
        .default_seat()
        .and_then(|seat| seat.keyboard())
        .is_some_and(|kb| kb.modifier_state().contains(gdk::ModifierType::CONTROL_MASK))
}
