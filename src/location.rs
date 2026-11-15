//! Текущее местоположение через портал `org.freedesktop.portal.Location` (за ним GeoClue).
//!
//! Портал работает и для обычной установки, и внутри Flatpak, без дополнительных прав.
//! Протокол: CreateSession → подписка на LocationUpdated и Response → Start → первая
//! координата → Session.Close. Подписки снимаются сами при уничтожении (`SignalSubscription`).
use gtk::{gio, glib, prelude::*};
use std::{
    cell::Cell,
    collections::HashMap,
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const LOCATION: &str = "org.freedesktop.portal.Location";
/// Точность «как можно точнее» (EXACT): GPS, если есть, иначе Wi-Fi.
const ACCURACY_EXACT: u32 = 5;
/// GeoClue по Wi-Fi обычно отвечает за секунды; дольше ждать нет смысла.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Найденное местоположение.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    /// Радиус точности, м.
    pub accuracy: f64,
}

/// Почему местоположение не получено.
#[derive(Debug)]
pub enum Error {
    /// Пользователь или настройки системы запретили доступ.
    Denied,
    /// Портала нет, геолокация выключена, тайм-аут и т. п.
    Unavailable(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Denied => write!(f, "доступ запрещён"),
            Error::Unavailable(e) => write!(f, "{e}"),
        }
    }
}

impl From<glib::Error> for Error {
    fn from(e: glib::Error) -> Self {
        Error::Unavailable(e.message().to_string())
    }
}

/// Что приходит из сигналов портала.
enum Event {
    Fix(Fix),
    Response(u32),
}

/// Уникальные токены запросов в пределах процесса.
fn token() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!("geophotolibrary{}", N.fetch_add(1, Ordering::Relaxed))
}

/// Определить текущее местоположение (одна координата).
pub async fn current() -> Result<Fix, Error> {
    let conn = gio::bus_get_future(gio::BusType::Session).await?;
    let call = |iface: &str, method: &str, path: &str, args: glib::Variant| {
        conn.call_future(Some(PORTAL), path, iface, method, Some(&args), None, gio::DBusCallFlags::NONE, -1)
    };

    let options = HashMap::from([
        ("session_handle_token".to_string(), token().to_variant()),
        ("accuracy".to_string(), ACCURACY_EXACT.to_variant()),
        ("distance-threshold".to_string(), 0u32.to_variant()),
        ("time-threshold".to_string(), 0u32.to_variant()),
    ]);
    let reply = call(LOCATION, "CreateSession", PORTAL_PATH, (options,).to_variant()).await?;
    let session = object_path(&reply).ok_or_else(|| Error::Unavailable("CreateSession: нет пути сессии".into()))?;

    let (tx, rx) = async_channel::unbounded::<Event>();
    let fix_tx = tx.clone();
    // Сессия в сигнале — первым аргументом типа «путь объекта». Фильтр D-Bus `arg0`
    // сравнивает только строки и такой сигнал молча отбрасывает, поэтому сессию
    // сверяем сами.
    let own_session = session.clone();
    let _updates = conn.subscribe_to_signal(
        Some(PORTAL),
        Some(LOCATION),
        Some("LocationUpdated"),
        Some(PORTAL_PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |s| {
            if signal_session(s.parameters).as_deref() != Some(own_session.as_str()) {
                return;
            }
            if let Some(fix) = parse_location(s.parameters) {
                let _ = fix_tx.try_send(Event::Fix(fix));
            }
        },
    );
    // Путь запроса известен заранее — подписываемся до Start, чтобы не пропустить ответ.
    let handle_token = token();
    let sender = conn.unique_name().map(|n| n.trim_start_matches(':').replace('.', "_")).unwrap_or_default();
    let request_path = format!("{PORTAL_PATH}/request/{sender}/{handle_token}");
    let _response = conn.subscribe_to_signal(
        Some(PORTAL),
        Some("org.freedesktop.portal.Request"),
        Some("Response"),
        Some(&request_path),
        None,
        gio::DBusSignalFlags::NONE,
        move |s| {
            if let Some(code) = s.parameters.child_value(0).get::<u32>() {
                let _ = tx.try_send(Event::Response(code));
            }
        },
    );

    let closed = Cell::new(false);
    let close = || async {
        if !closed.replace(true) {
            let _ = call("org.freedesktop.portal.Session", "Close", &session, ().to_variant()).await;
        }
    };

    let start_options = HashMap::from([("handle_token".to_string(), handle_token.to_variant())]);
    // Start (o session_handle, s parent_window, a{sv} options): путь — именно тип `o`.
    let session_path = glib::variant::ObjectPath::try_from(session.clone())
        .map_err(|_| Error::Unavailable(format!("неверный путь сессии {session}")))?;
    let started = call(LOCATION, "Start", PORTAL_PATH, (session_path, "", start_options).to_variant()).await;
    if let Err(e) = started {
        close().await;
        return Err(e.into());
    }

    let wait = async {
        while let Ok(event) = rx.recv().await {
            match event {
                Event::Fix(fix) => return Ok(fix),
                // 0 — запуск разрешён, ждём координату; 1 — отказ пользователя; 2 — иная ошибка.
                Event::Response(0) => {}
                Event::Response(1) => return Err(Error::Denied),
                Event::Response(code) => return Err(Error::Unavailable(format!("Start: код ответа {code}"))),
            }
        }
        Err(Error::Unavailable("канал сигналов закрыт".into()))
    };
    let result = glib::future_with_timeout(TIMEOUT, wait)
        .await
        .unwrap_or_else(|_| Err(Error::Unavailable("тайм-аут".into())));
    close().await;
    result
}

/// Путь объекта из ответа `(o)`.
fn object_path(reply: &glib::Variant) -> Option<String> {
    let v = reply.try_child_value(0)?;
    v.str().map(str::to_string)
}

/// Сессия, к которой относится сигнал `LocationUpdated`.
fn signal_session(params: &glib::Variant) -> Option<String> {
    params.try_child_value(0)?.str().map(str::to_string)
}

/// Разбор сигнала `LocationUpdated (o session, a{sv} location)`.
fn parse_location(params: &glib::Variant) -> Option<Fix> {
    let dict = glib::VariantDict::new(Some(&params.try_child_value(1)?));
    let lat: f64 = dict.lookup("Latitude").ok()??;
    let lon: f64 = dict.lookup("Longitude").ok()??;
    let accuracy: f64 = dict.lookup("Accuracy").ok().flatten().unwrap_or(0.0);
    let valid = (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon);
    valid.then_some(Fix { lat, lon, accuracy: accuracy.max(0.0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(fields: &[(&str, glib::Variant)]) -> glib::Variant {
        let dict = glib::VariantDict::new(None);
        for (k, v) in fields {
            dict.insert_value(k, v);
        }
        let path = glib::variant::ObjectPath::try_from("/org/freedesktop/portal/desktop/session/1_2/t").unwrap();
        glib::Variant::tuple_from_iter([path.to_variant(), dict.end()])
    }

    #[test]
    fn parses_location_signal() {
        let fix = parse_location(&signal(&[
            ("Latitude", 34.68.to_variant()),
            ("Longitude", 32.96.to_variant()),
            ("Accuracy", 25.0.to_variant()),
            ("Altitude", (-1.7976931348623157e308f64).to_variant()),
        ]));
        assert_eq!(fix, Some(Fix { lat: 34.68, lon: 32.96, accuracy: 25.0 }));
        let sig = signal(&[]);
        assert_eq!(signal_session(&sig).as_deref(), Some("/org/freedesktop/portal/desktop/session/1_2/t"));
    }

    #[test]
    fn rejects_incomplete_or_invalid() {
        assert_eq!(parse_location(&signal(&[("Latitude", 1.0.to_variant())])), None);
        assert_eq!(parse_location(&signal(&[("Latitude", 95.0.to_variant()), ("Longitude", 0.0.to_variant())])), None);
        let no_accuracy = parse_location(&signal(&[("Latitude", 1.0.to_variant()), ("Longitude", 2.0.to_variant())]));
        assert_eq!(no_accuracy.map(|f| f.accuracy), Some(0.0));
    }

    #[test]
    fn reads_object_path() {
        let path = glib::variant::ObjectPath::try_from("/a/b").unwrap();
        let reply = glib::Variant::tuple_from_iter([path.to_variant()]);
        assert_eq!(object_path(&reply).as_deref(), Some("/a/b"));
    }
}
