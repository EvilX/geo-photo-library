//! Проверка обновлений: последний выпуск на GitHub и сравнение с текущей версией.
//!
//! Сам запрос делает интерфейс (GTK — libsoup, macOS — URLSession), ядро только
//! разбирает ответ. Ошибки сети пользователю не показываются: уведомление
//! появляется, только если найдена версия новее текущей.
use anyhow::{Context, Result};
use serde::Deserialize;

/// Последний выпуск (без черновиков и pre-release).
pub const RELEASES_API: &str = "https://api.github.com/repos/EvilX/geo-photo-library/releases/latest";
/// Сайт программы: ссылка из уведомления о новой версии.
pub const SITE_URL: &str = "https://evilx.github.io/geo-photo-library/";
/// Текущая версия.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Отладка: версия «последнего выпуска» без запроса к GitHub.
pub const DEBUG_ENV: &str = "GEO_PHOTO_LIBRARY_UPDATE";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

/// Версия последнего выпуска из ответа GitHub, без префикса `v`.
pub fn parse_latest(json: &[u8]) -> Result<String> {
    let r: Release = serde_json::from_slice(json).context("ответ GitHub")?;
    Ok(r.tag_name.trim().trim_start_matches(['v', 'V']).to_owned())
}

/// Части версии `1.3.5` → `[1, 3, 5]`; `None` — не версия.
fn parts(v: &str) -> Option<Vec<u64>> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    if v.is_empty() {
        return None;
    }
    v.split('.').map(|p| p.parse().ok()).collect()
}

/// `latest` новее `current`; недостающие части — нули (`1.4` == `1.4.0`).
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parts(latest), parts(current)) else { return false };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// Версия из ответа GitHub, если она новее текущей.
pub fn newer_than_current(json: &[u8]) -> Option<String> {
    match parse_latest(json) {
        Ok(v) => is_newer(&v, CURRENT).then_some(v),
        Err(e) => {
            eprintln!("Проверка обновлений: {e:#}");
            None
        }
    }
}

/// Подменённая для отладки версия выпуска (`GEO_PHOTO_LIBRARY_UPDATE`): `Some(None)` —
/// подмена задана, но версия не новее; `None` — подмены нет, нужен запрос.
pub fn debug_override() -> Option<Option<String>> {
    let v = std::env::var(DEBUG_ENV).ok().filter(|v| !v.is_empty())?;
    Some(is_newer(&v, CURRENT).then(|| v.trim().trim_start_matches(['v', 'V']).to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let json = br#"{"tag_name": "v1.4", "name": "1.4", "draft": false}"#;
        assert_eq!(parse_latest(json).unwrap(), "1.4");
        assert_eq!(parse_latest(br#"{"tag_name": "2.0.1"}"#).unwrap(), "2.0.1");
        assert!(parse_latest(b"not json").is_err());
        assert!(parse_latest(br#"{"message": "Not Found"}"#).is_err());
    }

    #[test]
    fn compare() {
        assert!(is_newer("1.4", "1.3.5"));
        assert!(is_newer("1.3.6", "1.3.5"));
        assert!(is_newer("v2", "1.9.9"));
        assert!(is_newer("1.10", "1.9.0"));
        assert!(!is_newer("1.4", "1.4.0"));
        assert!(!is_newer("1.4.0", "1.4"));
        assert!(!is_newer("1.3", "1.3.5"));
        assert!(!is_newer("garbage", "1.0.0"));
        assert!(!is_newer("1.4-beta", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
    }

    #[test]
    fn current() {
        let newer = format!(r#"{{"tag_name": "v{}"}}"#, "999.0");
        assert_eq!(newer_than_current(newer.as_bytes()).as_deref(), Some("999.0"));
        let same = format!(r#"{{"tag_name": "v{CURRENT}"}}"#);
        assert_eq!(newer_than_current(same.as_bytes()), None);
    }
}
