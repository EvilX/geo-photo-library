use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Порядок фото в сетках миниатюр.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// Новые сверху.
    #[default]
    Newest,
    /// Старые сверху.
    Oldest,
}

impl SortOrder {
    /// Порядок пунктов в выпадающем списке.
    pub const ALL: [SortOrder; 2] = [SortOrder::Newest, SortOrder::Oldest];

    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|&o| o == self).unwrap_or(0) as u32
    }

    pub fn from_index(i: u32) -> Self {
        Self::ALL.get(i as usize).copied().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Корневая папка библиотеки фотографий.
    pub library_path: Option<PathBuf>,
    /// Сортировка фото в сетках.
    #[serde(default)]
    pub sort: SortOrder,
    /// Масштаб миниатюр в процентах (100 — исходный размер сетки).
    #[serde(default = "default_thumb_scale")]
    pub thumb_scale: u32,
    /// Просмотрщик фото: id `.desktop`-файла; `None` — программа по умолчанию для типа файла.
    pub viewer: Option<String>,
    /// Редактор фото: id `.desktop`-файла; `None` — найти графический редактор автоматически.
    pub editor: Option<String>,
}

pub const THUMB_SCALE_MIN: u32 = 50;
pub const THUMB_SCALE_MAX: u32 = 200;
pub const THUMB_SCALE_STEP: u32 = 5;

fn default_thumb_scale() -> u32 {
    100
}

/// Масштаб, приведённый к допустимому диапазону и шагу.
pub fn snap_thumb_scale(v: f64) -> u32 {
    let step = THUMB_SCALE_STEP as f64;
    ((v / step).round() * step).clamp(THUMB_SCALE_MIN as f64, THUMB_SCALE_MAX as f64) as u32
}

impl Default for Config {
    fn default() -> Self {
        Self {
            library_path: None,
            sort: SortOrder::default(),
            thumb_scale: default_thumb_scale(),
            viewer: None,
            editor: None,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(paths::config_file())
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::config_file();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_order() {
        // Старый конфиг без поля sort — новые сверху.
        let c: Config = toml::from_str("library_path = \"/p\"").unwrap();
        assert_eq!(c.sort, SortOrder::Newest);
        let c: Config = toml::from_str("sort = \"oldest\"").unwrap();
        assert_eq!(c.sort, SortOrder::Oldest);
        assert!(toml::to_string(&c).unwrap().contains("sort = \"oldest\""));
        assert_eq!(c.thumb_scale, 100);
        assert_eq!(Config::default().thumb_scale, 100);
        for o in SortOrder::ALL {
            assert_eq!(SortOrder::from_index(o.index()), o);
        }
    }

    #[test]
    fn thumb_scale() {
        assert_eq!(snap_thumb_scale(100.0), 100);
        assert_eq!(snap_thumb_scale(102.4), 100);
        assert_eq!(snap_thumb_scale(103.0), 105);
        assert_eq!(snap_thumb_scale(10.0), 50);
        assert_eq!(snap_thumb_scale(500.0), 200);
        let c: Config = toml::from_str("thumb_scale = 150").unwrap();
        assert_eq!(c.thumb_scale, 150);
        assert_eq!((c.viewer, c.editor), (None, None));
        let c: Config = toml::from_str("editor = \"org.gimp.GIMP.desktop\"").unwrap();
        assert_eq!(c.editor.as_deref(), Some("org.gimp.GIMP.desktop"));
    }
}
