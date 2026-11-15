//! Меню с иконками у пунктов.
//!
//! `PopoverMenu` в GTK4 не рисует иконки обычных пунктов модели, поэтому каждый пункт —
//! своя кнопка (атрибут `custom`), а разделы и разделители остаются от модели.
use gtk::{gio, prelude::*};

/// Пункт меню.
pub struct Item {
    pub label: &'static str,
    /// Действие вида `группа.имя`.
    pub action: &'static str,
    pub icon: &'static str,
    /// Сочетание клавиш для подсказки справа (`<Control>comma`); само сочетание
    /// регистрируется отдельно через `set_accels_for_action`.
    pub accel: Option<&'static str>,
}

impl Item {
    pub const fn new(label: &'static str, action: &'static str, icon: &'static str) -> Self {
        Self { label, action, icon, accel: None }
    }

    pub const fn accel(mut self, accel: &'static str) -> Self {
        self.accel = Some(accel);
        self
    }
}

/// Меню из разделов (между ними — разделители).
pub fn icon_menu(sections: Vec<Vec<Item>>) -> gtk::PopoverMenu {
    let model = gio::Menu::new();
    for items in &sections {
        let section = gio::Menu::new();
        for item in items {
            let entry = gio::MenuItem::new(Some(item.label), None);
            entry.set_attribute_value("custom", Some(&custom_id(item.action).to_variant()));
            section.append_item(&entry);
        }
        model.append_section(None, &section);
    }
    let popover = gtk::PopoverMenu::from_model(Some(&model));
    for item in sections.iter().flatten() {
        popover.add_child(&item_button(item, &popover), &custom_id(item.action));
    }
    popover
}

fn custom_id(action: &str) -> String {
    action.replace('.', "-")
}

fn item_button(item: &Item, popover: &gtk::PopoverMenu) -> gtk::Button {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&gtk::Image::from_icon_name(item.icon));
    let text = gtk::Label::new(Some(item.label));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    row.append(&text);
    if let Some((key, mods)) = item.accel.and_then(gtk::accelerator_parse) {
        let accel = gtk::Label::new(Some(&gtk::accelerator_get_label(key, mods)));
        accel.add_css_class("dim-label");
        accel.set_margin_start(24);
        row.append(&accel);
    }
    let button = gtk::Button::builder().child(&row).action_name(item.action).build();
    button.add_css_class("flat");
    button.add_css_class("menu-item");
    let pop = popover.downgrade();
    button.connect_clicked(move |_| {
        if let Some(p) = pop.upgrade() {
            p.popdown();
        }
    });
    button
}
