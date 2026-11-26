pub mod about;
pub mod coords_dialog;
pub mod date_tree;
pub mod dates_tab;
pub mod info_panel;
pub mod map_tab;
pub mod menu;
pub mod photo_grid;
pub mod place_search;
pub mod settings;
pub mod textures;
pub mod update_banner;
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
.map-filter {
    background: @window_bg_color;
    border-radius: 10px;
    box-shadow: 0 1px 6px rgba(0, 0, 0, 0.4);
}
.menu-item { padding: 4px 10px; min-height: 26px; margin: 0; font-weight: normal; }
.my-location {
    background: #3584e4;
    border: 3px solid white;
    border-radius: 999px;
    min-width: 14px;
    min-height: 14px;
    box-shadow: 0 0 0 6px rgba(53, 132, 228, 0.25), 0 1px 4px rgba(0, 0, 0, 0.5);
}
.pane-title { font-weight: bold; padding: 6px 10px; }
/* Высота области отметок слайдера в Adwaita: отступ 6px + риска 6px. */
.thumb-zoom { margin-top: 12px; }
.update-banner {
    background: alpha(@accent_bg_color, 0.15);
    border-bottom: 1px solid alpha(currentColor, 0.15);
    padding: 2px 6px 2px 12px;
}
";
