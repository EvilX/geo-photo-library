//! Чтение EXIF: дата съёмки, GPS, ориентация, камера.
//! kamadak-exif умеет JPEG, HEIF/HEIC, TIFF/DNG, PNG, WebP.
use chrono::NaiveDateTime;
use exif::{Exif, In, Reader, Tag, Value};
use std::{fs::File, io::BufReader, path::Path};

#[derive(Debug, Clone, PartialEq)]
pub struct PhotoMeta {
    pub taken_at: Option<NaiveDateTime>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// EXIF Orientation (1..=8), 1 — без поворота.
    pub orientation: u32,
    pub make: Option<String>,
    pub model: Option<String>,
}

impl Default for PhotoMeta {
    fn default() -> Self {
        Self { taken_at: None, lat: None, lon: None, orientation: 1, make: None, model: None }
    }
}

pub fn read(path: &Path) -> PhotoMeta {
    let Ok(file) = File::open(path) else {
        return PhotoMeta::default();
    };
    let mut reader = BufReader::new(file);
    match Reader::new().read_from_container(&mut reader) {
        Ok(exif) => from_exif(&exif),
        Err(_) => PhotoMeta::default(),
    }
}

fn from_exif(exif: &Exif) -> PhotoMeta {
    let taken_at = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .into_iter()
        .find_map(|t| ascii(exif, t).and_then(|s| parse_exif_datetime(&s)));
    let (lat, lon) = match gps(exif) {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };
    let orientation = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1);
    PhotoMeta {
        taken_at,
        lat,
        lon,
        orientation,
        make: ascii(exif, Tag::Make),
        model: ascii(exif, Tag::Model),
    }
}

fn ascii(exif: &Exif, tag: Tag) -> Option<String> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Ascii(v) => v
            .first()
            .map(|b| String::from_utf8_lossy(b).trim_matches(char::from(0)).trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

fn rationals(exif: &Exif, tag: Tag) -> Option<Vec<f64>> {
    let field = exif.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Rational(v) => Some(v.iter().map(|r| r.to_f64()).collect()),
        _ => None,
    }
}

fn gps(exif: &Exif) -> Option<(f64, f64)> {
    let lat_neg = ascii(exif, Tag::GPSLatitudeRef).is_some_and(|r| r.eq_ignore_ascii_case("S"));
    let lon_neg = ascii(exif, Tag::GPSLongitudeRef).is_some_and(|r| r.eq_ignore_ascii_case("W"));
    let lat = dms_to_decimal(&rationals(exif, Tag::GPSLatitude)?, lat_neg)?;
    let lon = dms_to_decimal(&rationals(exif, Tag::GPSLongitude)?, lon_neg)?;
    valid_coords(lat, lon).then_some((lat, lon))
}

/// Градусы/минуты/секунды → десятичные градусы.
pub fn dms_to_decimal(dms: &[f64], negative: bool) -> Option<f64> {
    if dms.is_empty() || dms.len() > 3 || dms.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let v = dms[0] + dms.get(1).unwrap_or(&0.0) / 60.0 + dms.get(2).unwrap_or(&0.0) / 3600.0;
    Some(if negative { -v } else { v })
}

/// Отбрасываем мусор: выход за диапазон и «нулевой остров» (0,0), который пишут некоторые телефоны без фикса.
pub fn valid_coords(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat.abs() < 1e-6 && lon.abs() < 1e-6)
}

pub fn parse_exif_datetime(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    let s = s.get(..19).unwrap_or(s);
    ["%Y:%m:%d %H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y:%m:%dT%H:%M:%S", "%Y-%m-%dT%H:%M:%S"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .filter(|d| chrono::Datelike::year(d) >= 1900)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dms() {
        let v = dms_to_decimal(&[55.0, 45.0, 21.0], false).unwrap();
        assert!((v - 55.755833).abs() < 1e-5);
        assert_eq!(dms_to_decimal(&[37.5], true), Some(-37.5));
        assert_eq!(dms_to_decimal(&[], false), None);
        assert_eq!(dms_to_decimal(&[f64::NAN, 0.0, 0.0], false), None);
    }

    #[test]
    fn coords() {
        assert!(valid_coords(55.75, 37.61));
        assert!(!valid_coords(0.0, 0.0));
        assert!(!valid_coords(91.0, 0.0));
    }

    #[test]
    fn datetime() {
        let d = parse_exif_datetime("2023:07:14 12:30:45").unwrap();
        assert_eq!(d.to_string(), "2023-07-14 12:30:45");
        assert!(parse_exif_datetime("2023:07:14 12:30:45+03:00").is_some());
        assert!(parse_exif_datetime("2023-07-14 12:30:45").is_some());
        assert!(parse_exif_datetime("0000:00:00 00:00:00").is_none());
        assert!(parse_exif_datetime("    ").is_none());
    }
}
