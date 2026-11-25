//! Уведомление о новой версии: полоса над вкладками с номером версии, ссылкой
//! на сайт программы и кнопкой закрытия. Показывается, только если на GitHub есть
//! выпуск новее текущего; ошибки сети пишутся в stderr и пользователю не видны.
use crate::{geocode, i18n, update};
use anyhow::{bail, Context, Result};
use gtk::{glib, glib::translate::IntoGlib, prelude::*};
use soup::prelude::*;

pub struct UpdateBanner {
    pub root: gtk::Revealer,
    label: gtk::Label,
}

impl UpdateBanner {
    pub fn new() -> Self {
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let link = gtk::LinkButton::with_label(update::SITE_URL, i18n::open_site());
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.set_tooltip_text(Some(i18n::close()));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("update-banner");
        bar.append(&label);
        bar.append(&link);
        bar.append(&close);
        let root = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .child(&bar)
            .build();
        // Полосой владеет Revealer — обработчик держит его слабой ссылкой.
        let r = root.downgrade();
        close.connect_clicked(move |_| {
            if let Some(r) = r.upgrade() {
                r.set_reveal_child(false);
            }
        });
        Self { root, label }
    }

    /// Спросить GitHub о последнем выпуске и показать полосу, если он новее.
    pub fn check(&self) {
        let (root, label) = (self.root.downgrade(), self.label.downgrade());
        glib::spawn_future_local(async move {
            let version = match update::debug_override() {
                Some(v) => v,
                None => match latest().await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("Проверка обновлений: {e:#}");
                        return;
                    }
                },
            };
            let (Some(version), Some(root), Some(label)) = (version, root.upgrade(), label.upgrade()) else { return };
            label.set_text(&i18n::update_available(&version));
            root.set_reveal_child(true);
        });
    }
}

/// Версия последнего выпуска, если она новее текущей.
async fn latest() -> Result<Option<String>> {
    let session = geocode::session();
    let msg = soup::Message::new("GET", update::RELEASES_API).context("URL запроса")?;
    if let Some(h) = msg.request_headers() {
        h.append("Accept", "application/vnd.github+json");
    }
    let body = session
        .send_and_read_future(&msg, glib::Priority::LOW)
        .await
        .context("запрос к GitHub")?;
    let status = msg.status();
    if status != soup::Status::Ok {
        bail!("GitHub: HTTP {}", status.into_glib());
    }
    Ok(update::newer_than_current(&body))
}
