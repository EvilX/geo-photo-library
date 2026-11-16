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

/// Обработчик результата перемещения фото в корзину.
type TrashedHandler = Box<dyn Fn(&Photo, Result<(), glib::Error>)>;
/// Обработчик пункта меню над фото («Информация», «Найти на карте»).
type PhotoHandler = Box<dyn Fn(&Photo)>;

/// Обработчики действий контекстного меню, которые выполняет окно.
#[derive(Clone, Default)]
struct Handlers {
    trashed: Rc<RefCell<Option<TrashedHandler>>>,
    info: Rc<RefCell<Option<PhotoHandler>>>,
    show_on_map: Rc<RefCell<Option<PhotoHandler>>>,
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
    pub selection: gtk::SingleSelection,
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
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let base_size = size;
        let size = Rc::new(Cell::new(base_size));
        let images: Rc<RefCell<Vec<glib::WeakRef<gtk::Image>>>> = Rc::default();

        // Контекстное меню: действия работают с фото, на котором нажали правую кнопку.
        let menu_photo: Rc<RefCell<Option<Photo>>> = Rc::default();
        let handlers = Handlers::default();
        let popover = context_menu();
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);

        let factory = gtk::SignalListItemFactory::new();
        let (cur_size, all_images) = (size.clone(), images.clone());
        let (pop, mp, sel) = (popover.clone(), menu_photo.clone(), selection.clone());
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
            let (weak_item, pop, mp, sel) = (item.downgrade(), pop.clone(), mp.clone(), sel.clone());
            click.connect_pressed(move |g, _, x, y| {
                let Some(item) = weak_item.upgrade() else { return };
                let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
                g.set_state(gtk::EventSequenceState::Claimed);
                sel.set_selected(item.position());
                *mp.borrow_mut() = Some(obj.borrow::<Photo>().clone());
                let (Some(cell), Some(parent)) = (g.widget(), pop.parent()) else { return };
                let at = cell
                    .compute_point(&parent, &graphene::Point::new(x as f32, y as f32))
                    .unwrap_or(graphene::Point::new(x as f32, y as f32));
                pop.set_pointing_to(Some(&gdk::Rectangle::new(at.x() as i32, at.y() as i32, 1, 1)));
                pop.popup();
            });
            b.add_controller(click);
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
        popover.set_parent(&view);
        // Привязанное вручную меню нужно и отвязать вручную (см. предупреждение GTK
        // «Finalizing … but it still has children left»).
        let pop = popover.clone();
        view.connect_destroy(move |_| pop.unparent());
        view.insert_action_group("photo", Some(&photo_actions(&view, &menu_photo, &handlers)));
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
    pub fn connect_trashed(&self, f: impl Fn(&Photo, Result<(), glib::Error>) + 'static) {
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
        let selected = self.selected_photo().map(|p| p.id);
        let mut objs: Vec<glib::BoxedAnyObject> = (0..self.store.n_items())
            .filter_map(|i| self.store.item(i).and_downcast())
            .collect();
        objs.reverse();
        self.store.splice(0, self.store.n_items(), &objs);
        if !selected.is_some_and(|id| self.select_id(id)) && !objs.is_empty() {
            self.view.scroll_to(0, gtk::ListScrollFlags::NONE, None);
        }
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
        // Сравниваем id на месте: `photo()` копировал бы каждое фото со строками.
        let has_id = |i: &u32| {
            self.store.item(*i).and_downcast::<glib::BoxedAnyObject>().is_some_and(|o| o.borrow::<Photo>().id == id)
        };
        let Some(pos) = (0..n).find(has_id) else {
            return false;
        };
        self.selection.set_selected(pos);
        self.view.scroll_to(pos, gtk::ListScrollFlags::FOCUS, None);
        true
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

/// Действия контекстного меню (группа `photo`) над фото из `menu_photo`.
fn photo_actions(
    view: &gtk::GridView,
    menu_photo: &Rc<RefCell<Option<Photo>>>,
    handlers: &Handlers,
) -> gio::SimpleActionGroup {
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: fn(&gtk::GridView, &Photo, &Handlers)| {
        let action = gio::SimpleAction::new(name, None);
        let (view, mp, handlers) = (view.downgrade(), menu_photo.clone(), handlers.clone());
        action.connect_activate(move |_, _| {
            let (Some(view), Some(photo)) = (view.upgrade(), mp.borrow().clone()) else { return };
            f(&view, &photo, &handlers);
        });
        group.add_action(&action);
    };
    add("copy", |v, p, _| copy_to_clipboard(v, &p.path));
    add("view", |v, p, _| open_external(v, &p.path));
    add("edit", |v, p, _| open_in_editor(v, &p.path));
    add("show-in-folder", |v, p, _| show_in_folder(v, &p.path));
    add("info", |_, p, handlers| {
        if let Some(f) = handlers.info.borrow().as_ref() {
            f(p);
        }
    });
    add("show-on-map", |_, p, handlers| {
        if let Some(f) = handlers.show_on_map.borrow().as_ref() {
            f(p);
        }
    });
    add("trash", |_, p, handlers| {
        let (p, handler) = (p.clone(), handlers.trashed.clone());
        let file = gio::File::for_path(&p.path);
        file.trash_async(glib::Priority::DEFAULT, gio::Cancellable::NONE, move |res| {
            if let Some(f) = handler.borrow().as_ref() {
                f(&p, res);
            }
        });
    });
    group
}

/// Зажат ли сейчас Ctrl (сигнал `activate` не передаёт событие).
fn ctrl_pressed(widget: &impl IsA<gtk::Widget>) -> bool {
    widget
        .display()
        .default_seat()
        .and_then(|seat| seat.keyboard())
        .is_some_and(|kb| kb.modifier_state().contains(gdk::ModifierType::CONTROL_MASK))
}
