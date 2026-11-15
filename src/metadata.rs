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

/// Подробные данные EXIF для панели «Информация»: основные поля числами
/// (форматирует интерфейс) и список всех тегов «как есть».
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExifDetails {
    pub taken_at: Option<NaiveDateTime>,
    /// Часовой пояс съёмки, например `+03:00`.
    pub offset: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// Выдержка, с.
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    /// Фокусное расстояние, мм.
    pub focal_length: Option<f64>,
    /// Эквивалентное фокусное расстояние для 35-мм кадра, мм.
    pub focal_35mm: Option<u32>,
    /// Экспокоррекция, EV.
    pub exposure_bias: Option<f64>,
    pub flash_fired: Option<bool>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Высота над уровнем моря, м.
    pub altitude: Option<f64>,
    pub software: Option<String>,
    /// Все теги основного изображения: (имя тега, значение).
    pub all: Vec<(String, String)>,
}

/// Подробный EXIF файла; `None` — файла нет или в нём нет EXIF.
pub fn read_details(path: &Path) -> Option<ExifDetails> {
    let file = File::open(path).ok()?;
    let exif = Reader::new().read_from_container(&mut BufReader::new(file)).ok()?;
    Some(details(&exif))
}

/// Длинные двоичные значения (MakerNote, таблицы) в списке тегов не показываются.
const MAX_VALUE_LEN: usize = 120;

fn details(exif: &Exif) -> ExifDetails {
    let num = |tag| exif.get_field(tag, In::PRIMARY).and_then(|f| f.value.get_uint(0));
    let real = |tag| {
        exif.get_field(tag, In::PRIMARY).and_then(|f| match &f.value {
            Value::Rational(v) => v.first().map(|r| r.to_f64()),
            Value::SRational(v) => v.first().map(|r| r.to_f64()),
            _ => None,
        })
        .filter(|v| v.is_finite())
    };
    let altitude = real(Tag::GPSAltitude).map(|a| {
        // GPSAltitudeRef = 1 — ниже уровня моря.
        if num(Tag::GPSAltitudeRef) == Some(1) {
            -a
        } else {
            a
        }
    });
    let all = exif
        .fields()
        .filter(|f| f.ifd_num == In::PRIMARY && f.tag != Tag::MakerNote)
        .filter_map(|f| {
            let value = f.display_value().with_unit(exif).to_string();
            let value = value.trim().trim_matches('"').trim().to_string();
            (!value.is_empty() && value.chars().count() <= MAX_VALUE_LEN).then(|| (f.tag.to_string(), value))
        })
        .collect();
    ExifDetails {
        taken_at: [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
            .into_iter()
            .find_map(|t| ascii(exif, t).and_then(|s| parse_exif_datetime(&s))),
        offset: ascii(exif, Tag::OffsetTimeOriginal).or_else(|| ascii(exif, Tag::OffsetTime)),
        camera: camera_name(ascii(exif, Tag::Make).as_deref(), ascii(exif, Tag::Model).as_deref()),
        lens: ascii(exif, Tag::LensModel),
        exposure_time: real(Tag::ExposureTime).filter(|&t| t > 0.0),
        f_number: real(Tag::FNumber).filter(|&f| f > 0.0),
        iso: num(Tag::PhotographicSensitivity).filter(|&i| i > 0),
        focal_length: real(Tag::FocalLength).filter(|&f| f > 0.0),
        focal_35mm: num(Tag::FocalLengthIn35mmFilm).filter(|&f| f > 0),
        exposure_bias: real(Tag::ExposureBiasValue),
        // Младший бит Flash — «вспышка сработала».
        flash_fired: num(Tag::Flash).map(|f| f & 1 == 1),
        width: num(Tag::PixelXDimension).or_else(|| num(Tag::ImageWidth)),
        height: num(Tag::PixelYDimension).or_else(|| num(Tag::ImageLength)),
        altitude,
        software: ascii(exif, Tag::Software),
        all,
    }
}

/// Производитель и модель без повтора: «Apple iPhone 15», но «Canon EOS R6», а не «Canon Canon EOS R6».
pub fn camera_name(make: Option<&str>, model: Option<&str>) -> Option<String> {
    match (make, model) {
        (Some(make), Some(model)) if model.to_lowercase().starts_with(&make.to_lowercase()) => Some(model.into()),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (make, model) => make.or(model).map(str::to_string),
    }
}

/// Выдержка как принято на камерах: `1/250`, `0.5`, `2`.
pub fn format_exposure(t: f64) -> String {
    if t < 0.3 {
        format!("1/{}", (1.0 / t).round())
    } else {
        let s = format!("{t:.1}");
        s.trim_end_matches(".0").to_string()
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
    fn camera() {
        assert_eq!(camera_name(Some("Apple"), Some("iPhone 15 Pro")).as_deref(), Some("Apple iPhone 15 Pro"));
        assert_eq!(camera_name(Some("Canon"), Some("Canon EOS R6")).as_deref(), Some("Canon EOS R6"));
        assert_eq!(camera_name(Some("NIKON CORPORATION"), None).as_deref(), Some("NIKON CORPORATION"));
        assert_eq!(camera_name(None, None), None);
    }

    #[test]
    fn exposure() {
        assert_eq!(format_exposure(1.0 / 250.0), "1/250");
        assert_eq!(format_exposure(0.008), "1/125");
        assert_eq!(format_exposure(0.5), "0.5");
        assert_eq!(format_exposure(2.0), "2");
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
