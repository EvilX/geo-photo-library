//! Генерация квадратных миниатюр (обрезка по центру, THUMB_SIZE×THUMB_SIZE) в кэш.
//!
//! Быстрые пути:
//! - JPEG декодируется сразу в уменьшенном виде (IDCT-масштабирование libjpeg-turbo, 1/8…1/1);
//! - HEIC: используется встроенная миниатюра (у iPhone/Samsung — 320×240 и больше),
//!   полное декодирование — только если подходящей нет;
//! - DNG: встроенное JPEG-превью (RAW не декодируется).
use crate::dng;
use anyhow::{anyhow, Context, Result};
use image::{codecs::jpeg::JpegEncoder, DynamicImage, RgbImage};
use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};

pub const THUMB_SIZE: u32 = 320;
/// Минимальная короткая сторона встроенной HEIC-миниатюры, которую ещё можно использовать.
const MIN_EMBEDDED_SIDE: u32 = 200;

pub fn thumb_path(dir: &Path, photo: &Path) -> PathBuf {
    let hash = blake3::hash(photo.as_os_str().as_encoded_bytes());
    dir.join(format!("{}.jpg", hash.to_hex()))
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Декодированное изображение для миниатюры.
struct Decoded {
    img: DynamicImage,
    /// Размеры оригинала до поворота по EXIF.
    dims: (u32, u32),
    /// Ориентация уже применена декодером (HEIC).
    oriented: bool,
}

/// Создаёт миниатюру `out` и возвращает размеры оригинала (с учётом ориентации).
pub fn generate(src: &Path, orientation: u32, out: &Path) -> Result<(u32, u32)> {
    let d = decode(src)?;
    let thumb = square(d.img);
    let (mut w, mut h) = d.dims;
    let thumb = if d.oriented {
        thumb
    } else {
        if (5..=8).contains(&orientation) {
            std::mem::swap(&mut w, &mut h);
        }
        apply_orientation(thumb, orientation)
    };
    let file = BufWriter::new(File::create(out).with_context(|| format!("создание {}", out.display()))?);
    JpegEncoder::new_with_quality(file, 85).encode_image(&thumb.to_rgb8())?;
    Ok((w, h))
}

/// Центральный квадрат, уменьшенный до THUMB_SIZE (без увеличения).
fn square(img: DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let sq = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    if side > THUMB_SIZE {
        sq.thumbnail_exact(THUMB_SIZE, THUMB_SIZE)
    } else {
        sq
    }
}

fn decode(src: &Path) -> Result<Decoded> {
    if has_ext(src, &["heic", "heif"]) {
        return decode_heif(src);
    }
    if has_ext(src, &["dng"]) {
        return decode_dng(src);
    }
    if has_ext(src, &["jpg", "jpeg"]) {
        match std::fs::read(src).map_err(anyhow::Error::from).and_then(|b| decode_jpeg_scaled(&b, THUMB_SIZE)) {
            Ok(d) => return Ok(d),
            // CMYK, повреждённые файлы и т.п. — пробуем универсальный декодер.
            Err(e) => eprintln!("{}: {e:#}, полное декодирование", src.display()),
        }
    }
    let img = image::ImageReader::open(src)?.with_guessed_format()?.decode()?;
    let dims = (img.width(), img.height());
    Ok(Decoded { img, dims, oriented: false })
}

/// Масштаб `n/8`, при котором короткая сторона ещё не меньше `target`.
fn jpeg_scale(w: u32, h: u32, target: u32) -> u8 {
    (target * 8).div_ceil(w.min(h).max(1)).clamp(1, 8) as u8
}

/// DNG: встроенное JPEG-превью; ориентация берётся из EXIF (IFD0), как у JPEG.
fn decode_dng(src: &Path) -> Result<Decoded> {
    let previews = dng::previews(src).context("разбор DNG")?;
    let p = dng::best_preview(&previews, THUMB_SIZE).context("в DNG нет JPEG-превью")?;
    let bytes = dng::read_preview(src, &p)?;
    decode_jpeg_scaled(&bytes, THUMB_SIZE)
}

/// JPEG с масштабированием при декодировании: для 12 Мп и target=320 — 1/8 (504×378).
fn decode_jpeg_scaled(jpeg: &[u8], target: u32) -> Result<Decoded> {
    // mozjpeg сообщает об ошибках libjpeg паникой.
    let res = std::panic::catch_unwind(|| -> std::io::Result<Decoded> {
        let mut d = mozjpeg::Decompress::new_mem(jpeg)?;
        let dims = (d.width() as u32, d.height() as u32);
        d.scale(jpeg_scale(dims.0, dims.1, target));
        let mut started = d.rgb()?;
        let (w, h) = (started.width() as u32, started.height() as u32);
        let px: Vec<u8> = started.read_scanlines()?;
        started.finish()?;
        let img = RgbImage::from_raw(w, h, px)
            .ok_or_else(|| std::io::Error::other("неверный размер буфера"))?;
        Ok(Decoded { img: DynamicImage::ImageRgb8(img), dims, oriented: false })
    });
    match res {
        Ok(r) => Ok(r?),
        Err(_) => Err(anyhow!("ошибка libjpeg")),
    }
}

#[cfg(feature = "heic")]
fn decode_heif(src: &Path) -> Result<Decoded> {
    use libheif_rs::{ColorSpace, HeifContext, ImageHandle, LibHeif, RgbChroma};

    /// Самая большая встроенная миниатюра той же ориентации и пропорций, что и основное фото.
    fn embedded_thumbnail(primary: &ImageHandle) -> Option<ImageHandle> {
        let (pw, ph) = (primary.width(), primary.height());
        let n = primary.number_of_thumbnails();
        if n == 0 || pw == 0 || ph == 0 {
            return None;
        }
        let mut ids = vec![0; n];
        let n = primary.thumbnail_ids(&mut ids);
        let primary_ratio = pw as f64 / ph as f64;
        ids[..n]
            .iter()
            .filter_map(|&id| primary.thumbnail(id).ok())
            .filter(|t| {
                let (w, h) = (t.width(), t.height());
                // Если у миниатюры нет того же поворота (irot), пропорции не совпадут.
                w.min(h) >= MIN_EMBEDDED_SIDE
                    && h > 0
                    && ((w as f64 / h as f64) / primary_ratio - 1.0).abs() < 0.05
            })
            .max_by_key(|t| t.width() * t.height())
    }

    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_file(src.to_str().context("путь не в UTF-8")?)?;
    let primary = ctx.primary_image_handle()?;
    // Размеры после трансформаций (irot) — как у отображаемого фото.
    let dims = (primary.width(), primary.height());
    let handle = embedded_thumbnail(&primary).unwrap_or(primary);
    // libheif сам применяет трансформации irot/imir из контейнера.
    let img = lib.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)?;
    let planes = img.planes();
    let plane = planes.interleaved.context("нет interleaved-плоскости")?;
    let (w, h, stride) = (plane.width, plane.height, plane.stride);
    let row = w as usize * 3;
    let mut buf = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        buf.extend_from_slice(&plane.data[y * stride..y * stride + row]);
    }
    let rgb = RgbImage::from_raw(w, h, buf).context("неверный буфер HEIF")?;
    Ok(Decoded { img: DynamicImage::ImageRgb8(rgb), dims, oriented: true })
}

#[cfg(not(feature = "heic"))]
fn decode_heif(_src: &Path) -> Result<Decoded> {
    anyhow::bail!("поддержка HEIC отключена при сборке")
}

/// Применяет EXIF Orientation (см. TIFF 6.0 / Pillow `exif_transpose`).
pub fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, Rgb};

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("geo-photo-library-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 2x1: левый пиксель красный, правый синий.
    fn sample() -> DynamicImage {
        let mut img = RgbImage::new(2, 1);
        img.put_pixel(0, 0, Rgb([255, 0, 0]));
        img.put_pixel(1, 0, Rgb([0, 0, 255]));
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn orientation_dims_and_pixels() {
        let r = apply_orientation(sample(), 6); // повернуть на 90° по часовой
        assert_eq!(r.dimensions(), (1, 2));
        assert_eq!(r.get_pixel(0, 0).0[0], 255); // красный сверху
        let r = apply_orientation(sample(), 8);
        assert_eq!(r.get_pixel(0, 0).0[2], 255); // синий сверху
        let r = apply_orientation(sample(), 2);
        assert_eq!(r.get_pixel(0, 0).0[2], 255);
        let r = apply_orientation(sample(), 5); // transpose
        assert_eq!(r.dimensions(), (1, 2));
        assert_eq!(r.get_pixel(0, 0).0[0], 255);
    }

    #[test]
    fn scale_choice() {
        assert_eq!(jpeg_scale(4032, 3024, 320), 1); // → 504×378
        assert_eq!(jpeg_scale(2000, 1500, 320), 2); // → 500×375
        assert_eq!(jpeg_scale(640, 480, 320), 6);
        assert_eq!(jpeg_scale(200, 100, 320), 8); // маленькие — без масштаба
    }

    #[test]
    fn square_thumbnail_png() {
        let dir = tmpdir("thumb-png");
        let src = dir.join("src.png");
        RgbImage::new(800, 400).save(&src).unwrap();
        let out = thumb_path(&dir, &src);
        assert_eq!(generate(&src, 6, &out).unwrap(), (400, 800));
        assert_eq!(image::open(&out).unwrap().dimensions(), (320, 320));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn scaled_jpeg_and_orientation() {
        let dir = tmpdir("thumb-jpg");
        let src = dir.join("src.jpg");
        // Верхняя половина красная, нижняя синяя.
        let img = RgbImage::from_fn(2000, 1500, |_, y| if y < 750 { Rgb([255, 0, 0]) } else { Rgb([0, 0, 255]) });
        img.save(&src).unwrap();

        let d = decode_jpeg_scaled(&std::fs::read(&src).unwrap(), THUMB_SIZE).unwrap();
        assert_eq!(d.dims, (2000, 1500));
        assert_eq!(d.img.dimensions(), (500, 375));

        let out = thumb_path(&dir, &src);
        assert_eq!(generate(&src, 6, &out).unwrap(), (1500, 2000));
        let t = image::open(&out).unwrap().to_rgb8();
        assert_eq!(t.dimensions(), (320, 320));
        // После поворота на 90° по часовой красная (верхняя) половина оказывается справа.
        assert!(t.get_pixel(300, 160).0[0] > 200 && t.get_pixel(20, 160).0[2] > 200);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dng_preview_thumbnail() {
        let dir = tmpdir("thumb-dng");
        let jpg = dir.join("preview.jpg");
        RgbImage::from_fn(1200, 900, |x, _| Rgb([(x % 256) as u8, 80, 160])).save(&jpg).unwrap();
        let src = dir.join("IMG_0001.DNG");
        std::fs::write(&src, crate::dng::tests::fake_dng(&std::fs::read(&jpg).unwrap(), 1200, 900)).unwrap();
        let out = thumb_path(&dir, &src);
        assert_eq!(generate(&src, 1, &out).unwrap(), (1200, 900));
        assert_eq!(image::open(&out).unwrap().dimensions(), (320, 320));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broken_jpeg_is_error_not_crash() {
        let dir = tmpdir("thumb-bad");
        let src = dir.join("bad.jpg");
        std::fs::write(&src, b"not a jpeg at all").unwrap();
        assert!(generate(&src, 1, &dir.join("out.jpg")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
