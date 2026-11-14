//! Замеры времени: `GEO_PHOTO_LIBRARY_TRACE=1 geo-photo-library` печатает длительность операций.
use std::{sync::OnceLock, time::Instant};

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("GEO_PHOTO_LIBRARY_TRACE").is_some())
}

pub struct Span {
    label: String,
    start: Instant,
}

impl Drop for Span {
    fn drop(&mut self) {
        if enabled() {
            let ms = self.start.elapsed().as_secs_f64() * 1000.0;
            eprintln!("[trace] {:<40} {ms:8.2} мс", self.label);
        }
    }
}

/// Замер до конца области видимости.
pub fn span(label: impl Into<String>) -> Span {
    Span { label: label.into(), start: Instant::now() }
}
