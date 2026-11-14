//! Бенчмарк этапов генерации миниатюр (однопоточно).
//! cargo run --release --example bench_thumbs -- <папка> [N]
#[path = "../src/metadata.rs"]
#[allow(dead_code)]
mod metadata;
#[path = "../src/dng.rs"]
#[allow(dead_code)]
mod dng;
#[path = "../src/thumbs.rs"]
#[allow(dead_code)]
mod thumbs;

use image::codecs::jpeg::JpegEncoder;
use std::{path::PathBuf, time::Instant};

fn files(dir: &str, ext: &str, n: usize) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
        .take(n)
        .collect()
}

fn bench<T>(label: &str, items: &[PathBuf], mut f: impl FnMut(&PathBuf) -> T) {
    let t = Instant::now();
    for p in items {
        std::hint::black_box(f(p));
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / items.len() as f64;
    println!("{label:<48} {ms:>8.1} мс/файл");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let jpg = files(&a[1], "jpg", n);
    let heic = files(&a[1], "heic", n);
    println!("JPEG: {} файлов, HEIC: {}", jpg.len(), heic.len());

    bench("EXIF (metadata::read), JPEG", &jpg, |p| metadata::read(p));
    bench("EXIF (metadata::read), HEIC", &heic, |p| metadata::read(p));
    bench("чтение файла в память, JPEG", &jpg, |p| std::fs::read(p).unwrap().len());

    let img = image::open(&jpg[0]).unwrap();
    bench("JPEG: image::open (полное декод.)", &jpg, |p| image::open(p).unwrap());
    let small = img.thumbnail(320, 320);
    bench("resize 12MP → 320 (DynamicImage::thumbnail)", &jpg[..1], |_| img.thumbnail(320, 320));
    bench("resize 12MP → 320 (resize Triangle)", &jpg[..1], |_| {
        img.resize(320, 320, image::imageops::FilterType::Triangle)
    });
    bench("encode JPEG 320px q85", &jpg, |_| {
        let mut v = Vec::new();
        JpegEncoder::new_with_quality(&mut v, 85).encode_image(&small.to_rgb8()).unwrap();
        v.len()
    });
    bench("JPEG: thumbs::generate (итого)", &jpg, |p| {
        thumbs::generate(p, 1, &std::env::temp_dir().join("bench_thumb.jpg")).unwrap()
    });

    bench("JPEG: gdk-pixbuf from_file_at_scale 320", &jpg, |p| {
        gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(p, 320, 320, true).unwrap()
    });

    if !heic.is_empty() {
        bench("HEIC: thumbs::generate (полное декод.)", &heic, |p| {
            thumbs::generate(p, 1, &std::env::temp_dir().join("bench_thumb.jpg")).unwrap()
        });
        bench("HEIC: число встроенных миниатюр", &heic[..1], |p| {
            let ctx = libheif_rs::HeifContext::read_from_file(p.to_str().unwrap()).unwrap();
            let n = ctx.primary_image_handle().unwrap().number_of_thumbnails();
            println!("  встроенных миниатюр: {n}");
        });
    }
    // Загрузка готовой миниатюры 320px в UI
    let thumbs_dir = std::env::var("THUMBS_DIR").unwrap_or_default();
    let tf: Vec<PathBuf> = files(&thumbs_dir, "jpg", 50);
    if !tf.is_empty() {
        bench("UI: gdk_pixbuf::Pixbuf::from_file (320px)", &tf, |p| {
            gtk::gdk_pixbuf::Pixbuf::from_file(p).unwrap()
        });
        bench("UI: gdk::Texture::from_filename (320px)", &tf, |p| {
            gtk::gdk::Texture::from_filename(p).unwrap()
        });
        bench("UI: image::open + crop + resize 112 (320px)", &tf, |p| {
            let im = image::open(p).unwrap();
            let side = im.width().min(im.height());
            im.crop_imm((im.width() - side) / 2, (im.height() - side) / 2, side, side)
                .resize_exact(112, 112, image::imageops::FilterType::Triangle)
        });
    }
    // HEIC со встроенной миниатюрой (как у iPhone/Samsung)
    let th: Vec<PathBuf> = files(&a[1].replace("library", "heic_thumb"), "heic", 1).into_iter().flat_map(|p| std::iter::repeat_n(p, 10)).collect();
    if !th.is_empty() {
        bench("HEIC c миниатюрой: полное декод. (generate)", &th, |p| {
            thumbs::generate(p, 1, &std::env::temp_dir().join("bench_thumb.jpg")).unwrap()
        });
        bench("HEIC c миниатюрой: декод. встроенной", &th, |p| {
            use libheif_rs::*;
            let lib = LibHeif::new();
            let ctx = HeifContext::read_from_file(p.to_str().unwrap()).unwrap();
            let primary = ctx.primary_image_handle().unwrap();
            let mut ids = [0u32; 1];
            assert_eq!(primary.thumbnail_ids(&mut ids), 1);
            let t = primary.thumbnail(ids[0]).unwrap();
            let img = lib.decode(&t, ColorSpace::Rgb(RgbChroma::Rgb), None).unwrap();
            (img.width(), img.height())
        });
    }
}
