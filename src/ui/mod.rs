pub mod about;
pub mod dates_tab;
pub mod info_panel;
pub mod map_tab;
pub mod photo_grid;
pub mod settings;
pub mod textures;
pub mod window;

pub const CSS: &str = "
.photo-thumb { border-radius: 6px; }
.photo-caption { font-size: smaller; opacity: 0.8; }
.map-thumb {
    border: 2px solid white;
    border-radius: 8px;
    box-shadow: 0 1px 5px rgba(0, 0, 0, 0.55);
    background: #888;
}
.map-badge {
    background: #e01b24;
    color: white;
    border-radius: 999px;
    padding: 0 6px;
    min-width: 12px;
    font-weight: bold;
    font-size: smaller;
}
.map-search { box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4); }
.photo-menu-item { padding: 4px 10px; font-weight: normal; }
.pane-title { font-weight: bold; padding: 6px 10px; }
";
