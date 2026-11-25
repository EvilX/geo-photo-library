//! Сетка миниатюр, используется на обеих вкладках.
use super::menu::{icon_menu, Item};
use super::textures::{copy_to_clipboard, open_external, open_in_editor, set_thumbnail, show_in_folder, TextureCache};
use crate::{
    config::{self, SortOrder},
    db::Photo,
    i18n,
};
use gtk::{gdk, gio, glib, graphene, pango, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Результаты перемещения в корзину: по одному на каждое фото.
pub type TrashResults = Vec<(Photo, Result<(), glib::Error>)>;
/// Обработчик результата перемещения фото в корзину.
type TrashedHandler = Box<dyn Fn(TrashResults)>;
/// Обработчик пункта меню над фото («Информация», «Найти на карте»).
type PhotoHandler = Box<dyn Fn(&Photo)>;

/// Обработчики действий контекстного меню, которые выполняет окно.
#[derive(Clone, Default)]
struct Handlers {
    trashed: Rc<RefCell<Option<TrashedHandler>>>,
    info: Rc<RefCell<Option<PhotoHandler>>>,
    show_on_map: Rc<RefCell<Option<PhotoHandler>>>,
    picked: Rc<RefCell<Option<PhotoHandler>>>,
}

/// Шаг кнопок «−»/«+» у слайдера масштаба миниатюр, %.
const ZOOM_BUTTON_STEP: f64 = 10.0;

/// С какого размера списка картинок ячеек вычищать из него уничтоженные.
const IMAGES_PRUNE_AT: usize = 1024;

/// Верхний предел колонок при уменьшенных миниатюрах (см. `set_scale`).
const MAX_COLUMNS_LIMIT: u32 = 16;

#[derive(Clone)]
pub struct PhotoGrid {
    pub root: gtk::ScrolledWindow,
    pub view: gtk::GridView,
    selection: gtk::MultiSelection,
    store: gio::ListStore,
    order: Rc<Cell<SortOrder>>,
    /// Сторона миниатюры при масштабе 100%.
    base_size: i32,
    /// Текущая сторона миниатюры в пикселях.
    size: Rc<Cell<i32>>,
    /// Предел колонок при масштабе 100%.
    base_max_columns: u32,
    /// Картинки всех созданных ячеек — чтобы менять их размер без пересоздания.
    images: Rc<RefCell<Vec<glib::WeakRef<gtk::Image>>>>,
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
        let images: Rc<RefCell<Vec<glib::WeakRef<gtk::Image>>>> = Rc::default();

        // Контекстное меню: действия работают с фото, на котором нажали правую кнопку,
        // а если оно среди нескольких выделенных — со всеми выделенными (меню короче:
        // только копирование и удаление).
        let menu_photos: Rc<RefCell<Vec<Photo>>> = Rc::default();
        let handlers = Handlers::default();
        let (popover, popover_many) = (context_menu(), context_menu_many());
        for p in [&popover, &popover_many] {
            p.set_has_arrow(false);
            p.set_halign(gtk::Align::Start);
        }

        let factory = gtk::SignalListItemFactory::new();
        let (cur_size, all_images) = (size.clone(), images.clone());
        let (pop, pop_many, mp, sel) = (popover.clone(), popover_many.clone(), menu_photos.clone(), selection.clone());
        let picked = handlers.picked.clone();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let image = gtk::Image::new();
            image.set_pixel_size(cur_size.get());
            {
                // GridView при больших изменениях списка уничтожает ячейки и создаёт
                // новые — без чистки список слабых ссылок рос бы всё время работы.
                let mut images = all_images.borrow_mut();
                if images.len() >= IMAGES_PRUNE_AT && images.len().is_power_of_two() {
                    images.retain(|w| w.upgrade().is_some());
                }
                images.push(image.downgrade());
            }
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

            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_SECONDARY);
            let (weak_item, pop, pop_many, mp, sel) =
                (item.downgrade(), pop.clone(), pop_many.clone(), mp.clone(), sel.clone());
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
        view.insert_action_group("photo", Some(&photo_actions(&view, &menu_photos, &handlers)));
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

        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&view)
            .build();
        Self {
            root,
            view,
            selection,
            store,
            order: Rc::default(),
            base_size,
            base_max_columns: max_columns,
            size,
            images,
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
        self.images.borrow_mut().retain(|w| match w.upgrade() {
            Some(image) => {
                image.set_pixel_size(size);
                true
            }
            None => false,
        });
    }

    /// `photos` — по возрастанию даты; показываются в текущем порядке сортировки.
    pub fn set_photos(&self, photos: &[Photo]) {
        self.set_objects(photos.iter().cloned().map(glib::BoxedAnyObject::new).collect());
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
            _ => {
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
        }
    }

    /// Выделить первые `n` фото (отладочный хук).
    pub fn select_first(&self, n: u32) {
        let n = n.min(self.store.n_items());
        self.selection.set_selection(&gtk::Bitset::new_range(0, n), &gtk::Bitset::new_range(0, self.store.n_items()));
    }
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
        vec![Item::new(i18n::copy(), "photo.copy", "edit-copy-symbolic")],
        vec![
            Item::new(i18n::view(), "photo.view", "view-reveal-symbolic"),
            Item::new(i18n::edit(), "photo.edit", "document-edit-symbolic"),
            Item::new(i18n::show_in_folder(), "photo.show-in-folder", "folder-open-symbolic"),
        ],
        vec![
            Item::new(i18n::info(), "photo.info", "help-about-symbolic"),
            Item::new(i18n::show_on_map(), "photo.show-on-map", "mark-location-symbolic"),
        ],
        vec![Item::new(i18n::delete(), "photo.trash", "user-trash-symbolic")],
    ])
}

/// Меню над несколькими выделенными фото: только действия, применимые ко всем сразу.
fn context_menu_many() -> gtk::PopoverMenu {
    icon_menu(vec![
        vec![Item::new(i18n::copy(), "photo.copy", "edit-copy-symbolic")],
        vec![Item::new(i18n::delete(), "photo.trash", "user-trash-symbolic")],
    ])
}

/// Выделенные фото по порядку показа.
fn selected_in(selection: &gtk::MultiSelection) -> Vec<Photo> {
    let set = selection.selection();
    (0..set.size() as u32)
        .filter_map(|i| selection.item(set.nth(i)).and_downcast::<glib::BoxedAnyObject>())
        .map(|o| o.borrow::<Photo>().clone())
        .collect()
}

/// Действия контекстного меню (группа `photo`) над фото из `menu_photos`: копирование
/// и удаление — над всеми, остальные — над первым (меню с ними открывается для одного фото).
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
    // После подтверждения файлы перемещаются параллельно; обработчик получает все результаты разом.
    add("trash", |v, ps, handlers| {
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
            if res == Ok(1) {
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
