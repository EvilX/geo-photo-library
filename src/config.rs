use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// Последнее положение карты: центр и зум.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MapView {
    pub lat: f64,
    pub lon: f64,
    pub zoom: f64,
}

impl MapView {
    /// Положение, если оно осмысленное (конфиг правят руками).
    pub fn valid(self) -> Option<Self> {
        let ok = self.lat.is_finite()
            && self.lon.is_finite()
            && self.zoom.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lon);
        ok.then_some(self)
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
    /// Проверять при запуске, нет ли новой версии на GitHub.
    #[serde(default = "default_true")]
    pub check_updates: bool,
    /// Положение карты при закрытии окна; `None` — показать все фото.
    /// Таблица TOML — должна идти после простых полей.
    pub map_view: Option<MapView>,
}

pub const THUMB_SCALE_MIN: u32 = 50;
pub const THUMB_SCALE_MAX: u32 = 200;
pub const THUMB_SCALE_STEP: u32 = 5;

fn default_thumb_scale() -> u32 {
    100
}

fn default_true() -> bool {
    true
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
            check_updates: true,
            map_view: None,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        Self::load_from(&paths::config_file())
    }

    /// Прочитать настройки из файла. Файла нет — значения по умолчанию. Файл не разобран
    /// (ошибка синтаксиса, правили руками) — тоже они, но сам файл сохраняется рядом как
    /// `config.toml.bak`: иначе первое же сохранение настроек молча затёрло бы его.
    fn load_from(path: &Path) -> Self {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("Не удалось прочитать настройки {}: {e}", path.display());
                }
                return Self::default();
            }
        };
        let parsed = std::str::from_utf8(&bytes)
            .map_err(|e| e.to_string())
            .and_then(|text| toml::from_str::<Self>(text).map_err(|e| e.to_string()));
        match parsed {
            Ok(config) => config,
            Err(e) => {
                let backup = backup_path(path);
                eprintln!("Настройки {} не разобраны, используются значения по умолчанию: {e}", path.display());
                match std::fs::copy(path, &backup) {
                    Ok(_) => eprintln!("Прежний файл настроек сохранён как {}", backup.display()),
                    Err(e) => eprintln!("Не удалось сохранить копию настроек {}: {e}", backup.display()),
                }
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&paths::config_file())
    }

    /// Запись через временный файл и переименование: при сбое посреди записи прежние
    /// настройки остаются целыми.
    fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = with_suffix(path, ".tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }
}

/// Копия неразобранного файла настроек: `config.toml` → `config.toml.bak`.
fn backup_path(path: &Path) -> PathBuf {
    with_suffix(path, ".bak")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
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
    fn map_view() {
        let mut c = Config { library_path: Some("/p".into()), ..Config::default() };
        c.map_view = Some(MapView { lat: 34.68, lon: 32.96, zoom: 12.0 });
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.map_view, c.map_view);
        assert_eq!(back.library_path, c.library_path);
        // Старый конфиг без положения карты.
        assert_eq!(toml::from_str::<Config>("thumb_scale = 100").unwrap().map_view, None);
        assert!(MapView { lat: 91.0, lon: 0.0, zoom: 3.0 }.valid().is_none());
        assert!(MapView { lat: 0.0, lon: 0.0, zoom: f64::NAN }.valid().is_none());
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

    #[test]
    fn check_updates() {
        // Старый конфиг без поля — проверка включена.
        assert!(toml::from_str::<Config>("thumb_scale = 100").unwrap().check_updates);
        assert!(Config::default().check_updates);
        let c = Config { check_updates: false, map_view: Some(MapView { lat: 1.0, lon: 2.0, zoom: 3.0 }), ..Config::default() };
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert!(!back.check_updates);
    }

    #[test]
    fn broken_file_is_kept() {
        let dir = std::env::temp_dir().join(format!("gpl-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join("config.toml");
        // Файла нет — значения по умолчанию, копия не создаётся.
        assert!(Config::load_from(&path).library_path.is_none());
        assert!(!backup_path(&path).exists());
        // Сохранение создаёт каталог и не оставляет временный файл.
        let c = Config { library_path: Some("/p".into()), thumb_scale: 150, ..Config::default() };
        c.save_to(&path).unwrap();
        assert!(!with_suffix(&path, ".tmp").exists());
        let back = Config::load_from(&path);
        assert_eq!((back.library_path, back.thumb_scale), (Some("/p".into()), 150));
        assert!(!backup_path(&path).exists());
        // Испорченный файл: значения по умолчанию, а его содержимое — в копии, и
        // следующее сохранение её не трогает.
        let broken = "library_path = \"/photos\nthumb_scale = ";
        std::fs::write(&path, broken).unwrap();
        assert!(Config::load_from(&path).library_path.is_none());
        Config::default().save_to(&path).unwrap();
        assert_eq!(std::fs::read_to_string(backup_path(&path)).unwrap(), broken);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
