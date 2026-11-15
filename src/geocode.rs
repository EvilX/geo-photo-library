//! Поиск местности по названию через Nominatim (геокодер OpenStreetMap).
//!
//! Правила сервиса: не больше одного запроса в секунду, осмысленный User-Agent,
//! без автодополнения на каждое нажатие — поэтому запрос уходит только по Enter.
use anyhow::{bail, Context, Result};
use gtk::glib::{self, translate::IntoGlib};
use serde::Deserialize;
use soup::prelude::*;

const ENDPOINT: &str = "https://nominatim.openstreetmap.org/search";
const USER_AGENT: &str = concat!(
    "GeoPhotoLibrary/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/EvilX/geo-photo-library)"
);
const LIMIT: usize = 8;

/// Найденное место.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Границы (юг, север, запад, восток), если сервис их вернул.
    pub bbox: Option<(f64, f64, f64, f64)>,
}

impl Place {
    /// Углы прямоугольника места или сама точка.
    pub fn corners(&self) -> Vec<(f64, f64)> {
        match self.bbox {
            Some((s, n, w, e)) => vec![(s, w), (n, e)],
            None => vec![(self.lat, self.lon)],
        }
    }
}

/// Сессия HTTP для запросов к геокодеру (libsoup, та же, что у тайлов карты).
pub fn session() -> soup::Session {
    let s = soup::Session::new();
    s.set_user_agent(USER_AGENT);
    s.set_timeout(15);
    s
}

/// URL запроса; `lang` — код языка для названий в ответе.
pub fn search_url(query: &str, lang: &str) -> String {
    let q = glib::Uri::escape_string(query.trim(), None, false);
    format!("{ENDPOINT}?format=jsonv2&limit={LIMIT}&accept-language={lang}&q={q}")
}

/// Найти места по строке запроса.
pub async fn search(session: &soup::Session, query: &str, lang: &str) -> Result<Vec<Place>> {
    let msg = soup::Message::new("GET", &search_url(query, lang)).context("URL запроса")?;
    let body = session
        .send_and_read_future(&msg, glib::Priority::DEFAULT)
        .await
        .context("запрос к Nominatim")?;
    let status = msg.status();
    if status != soup::Status::Ok {
        bail!("Nominatim: HTTP {}", status.into_glib());
    }
    parse(&body)
}

#[derive(Deserialize)]
struct Raw {
    lat: String,
    lon: String,
    display_name: String,
    #[serde(default)]
    boundingbox: Vec<String>,
}

/// Разбор ответа Nominatim (`format=jsonv2`): координаты там строками.
pub fn parse(json: &[u8]) -> Result<Vec<Place>> {
    let raw: Vec<Raw> = serde_json::from_slice(json).context("ответ Nominatim")?;
    Ok(raw
        .into_iter()
        .filter_map(|r| {
            let lat = r.lat.parse().ok()?;
            let lon = r.lon.parse().ok()?;
            let b: Vec<f64> = r.boundingbox.iter().filter_map(|v| v.parse().ok()).collect();
            let bbox = (b.len() == 4).then(|| (b[0], b[1], b[2], b[3]));
            Some(Place { name: r.display_name, lat, lon, bbox })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_escapes_query() {
        let url = search_url("  Αθήνα & co ", "el");
        assert!(url.starts_with(ENDPOINT));
        assert!(url.contains("accept-language=el"));
        assert!(url.ends_with("&q=%CE%91%CE%B8%CE%AE%CE%BD%CE%B1%20%26%20co"), "{url}");
    }

    #[test]
    fn parses_places() {
        let json = br#"[
            {"lat":"37.9755648","lon":"23.7348324","display_name":"Athens, Greece",
             "boundingbox":["37.8","38.1","23.6","23.9"],"place_id":1},
            {"lat":"bad","lon":"0","display_name":"skip"},
            {"lat":"55.75","lon":"37.62","display_name":"Moscow"}
        ]"#;
        let places = parse(json).unwrap();
        assert_eq!(places.len(), 2);
        assert_eq!(places[0].name, "Athens, Greece");
        assert_eq!(places[0].bbox, Some((37.8, 38.1, 23.6, 23.9)));
        assert_eq!(places[0].corners(), vec![(37.8, 23.6), (38.1, 23.9)]);
        assert_eq!(places[1].bbox, None);
        assert_eq!(places[1].corners(), vec![(55.75, 37.62)]);
    }

    #[test]
    fn empty_and_broken_responses() {
        assert!(parse(b"[]").unwrap().is_empty());
        assert!(parse(b"<html>").is_err());
    }
}
