//! Генерация миниатюр в кэш (JPEG, длинная сторона THUMB_SIZE).
use anyhow::{Context, Result};
use image::{codecs::jpeg::JpegEncoder, DynamicImage};
use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
};

pub const THUMB_SIZE: u32 = 320;

pub fn thumb_path(dir: &Path, photo: &Path) -> PathBuf {
    let hash = blake3::hash(photo.as_os_str().as_encoded_bytes());
    dir.join(format!("{}.jpg", hash.to_hex()))
}

fn is_heif(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
}

/// Создаёт миниатюру `out` и возвращает размеры оригинала (с учётом ориентации).
pub fn generate(src: &Path, orientation: u32, out: &Path) -> Result<(u32, u32)> {
    let (img, oriented) = decode(src)?;
    let thumb = img.thumbnail(THUMB_SIZE, THUMB_SIZE);
    let (mut w, mut h) = (img.width(), img.height());
    drop(img);
    let thumb = if oriented {
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

/// Возвращает изображение и флаг «ориентация уже применена декодером».
fn decode(src: &Path) -> Result<(DynamicImage, bool)> {
    if is_heif(src) {
        // libheif сам применяет трансформации irot/imir из контейнера.
        return decode_heif(src).map(|i| (i, true));
    }
    let img = image::ImageReader::open(src)?.with_guessed_format()?.decode()?;
    Ok((img, false))
}

#[cfg(feature = "heic")]
fn decode_heif(src: &Path) -> Result<DynamicImage> {
    use libheif_rs::{ColorSpace, HeifContext, LibHeif, RgbChroma};
    let lib = LibHeif::new();
    let ctx = HeifContext::read_from_file(src.to_str().context("путь не в UTF-8")?)?;
    let handle = ctx.primary_image_handle()?;
    let img = lib.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)?;
    let planes = img.planes();
    let plane = planes.interleaved.context("нет interleaved-плоскости")?;
    let (w, h, stride) = (plane.width, plane.height, plane.stride);
    let row = w as usize * 3;
    let mut buf = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        buf.extend_from_slice(&plane.data[y * stride..y * stride + row]);
    }
    let rgb = image::RgbImage::from_raw(w, h, buf).context("неверный буфер HEIF")?;
    Ok(DynamicImage::ImageRgb8(rgb))
}

#[cfg(not(feature = "heic"))]
fn decode_heif(_src: &Path) -> Result<DynamicImage> {
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
    use image::{GenericImageView, Rgb, RgbImage};

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
    fn generate_jpeg_thumbnail() {
        let dir = std::env::temp_dir().join(format!("geo-photo-library-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        RgbImage::new(800, 400).save(&src).unwrap();
        let out = thumb_path(&dir, &src);
        assert_eq!(generate(&src, 6, &out).unwrap(), (400, 800));
        let t = image::open(&out).unwrap();
        assert_eq!(t.dimensions(), (160, 320));
        std::fs::remove_dir_all(&dir).ok();
    }
}
