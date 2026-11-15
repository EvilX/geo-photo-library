//! Окно «О программе».
use crate::{i18n, APP_ID, APP_NAME};
use gtk::prelude::*;

pub fn show(parent: &gtk::ApplicationWindow) {
    let dialog = gtk::AboutDialog::builder()
        .transient_for(parent)
        .modal(true)
        .program_name(APP_NAME)
        .logo_icon_name(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(i18n::app_summary())
        .website("https://github.com/EvilX/geo-photo-library")
        .authors(["Leonid Nasedkin", "Claude"])
        .copyright("© 2026 Leonid Nasedkin & Claude")
        .build();
    dialog.present();
}
