//! Integration tests that exercise the public API end-to-end.
//!
//! Each test builds a real image in memory using the `image` crate, writes
//! it to a temp file, and round-trips it through `ThumbnailGenerator` and
//! the metadata helpers.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use image::{ImageBuffer, Rgb, RgbImage};
use tokimo_package_image::{
    OutputFormat, ThumbnailGenerator, extract_date_from_filename, file_mtime_as_date, get_image_dimensions,
    get_image_dimensions_from_bytes,
};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_tmp_path(stem: &str, ext: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("tokimo_image_test_{stem}_{pid}_{nanos}_{n}.{ext}"))
}

/// Build a deterministic gradient RGB image of `w` x `h`.
fn make_rgb_image(w: u32, h: u32) -> RgbImage {
    ImageBuffer::from_fn(w, h, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    })
}

fn write_jpeg(w: u32, h: u32) -> PathBuf {
    let path = unique_tmp_path("jpg", "jpg");
    make_rgb_image(w, h)
        .save_with_format(&path, image::ImageFormat::Jpeg)
        .unwrap();
    path
}

fn write_png(w: u32, h: u32) -> PathBuf {
    let path = unique_tmp_path("png", "png");
    make_rgb_image(w, h)
        .save_with_format(&path, image::ImageFormat::Png)
        .unwrap();
    path
}

/// Read the encoded image header to check its dimensions and format.
fn decode_dims(bytes: &[u8]) -> (u32, u32) {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .unwrap();
    reader.into_dimensions().unwrap()
}

fn detect_format(bytes: &[u8]) -> image::ImageFormat {
    image::guess_format(bytes).unwrap()
}

// ── ThumbnailGenerator: file path ───────────────────────────────────────────

#[tokio::test]
async fn generate_jpeg_to_webp_thumbnail() {
    let src = write_jpeg(800, 600);
    let tg = ThumbnailGenerator::new();

    let (bytes, mime) = tg
        .generate(src.to_str().unwrap(), 200, 0, OutputFormat::Webp)
        .await
        .expect("thumbnail generation");

    assert_eq!(mime, "image/webp");
    assert!(!bytes.is_empty());
    assert_eq!(detect_format(&bytes), image::ImageFormat::WebP);

    let (w, h) = decode_dims(&bytes);
    assert_eq!(w, 200, "width should be requested 200, got {w}x{h}");
    // height should be proportional (≈150) — allow ±2 px for libvips rounding
    assert!(h.abs_diff(150) <= 2, "expected ~150, got {h}");

    let _ = std::fs::remove_file(&src);
}

#[tokio::test]
async fn generate_png_to_jpeg_thumbnail() {
    let src = write_png(640, 480);
    let tg = ThumbnailGenerator::new();

    let (bytes, mime) = tg
        .generate(src.to_str().unwrap(), 160, 120, OutputFormat::Jpeg)
        .await
        .expect("thumbnail generation");

    assert_eq!(mime, "image/jpeg");
    assert_eq!(detect_format(&bytes), image::ImageFormat::Jpeg);
    let (w, h) = decode_dims(&bytes);
    assert!(w <= 160 && h <= 120, "expected ≤ 160x120, got {w}x{h}");

    let _ = std::fs::remove_file(&src);
}

// ── ThumbnailGenerator: in-memory bytes ─────────────────────────────────────

#[tokio::test]
async fn generate_from_bytes_jpeg() {
    let src = write_jpeg(400, 300);
    let raw = std::fs::read(&src).unwrap();
    let tg = ThumbnailGenerator::new();

    let (bytes, mime) = tg
        .generate_from_bytes("test", &raw, "jpg", 100, 0, OutputFormat::Webp)
        .await
        .expect("thumbnail generation from bytes");

    assert_eq!(mime, "image/webp");
    assert_eq!(detect_format(&bytes), image::ImageFormat::WebP);
    let (w, _) = decode_dims(&bytes);
    assert_eq!(w, 100);

    let _ = std::fs::remove_file(&src);
}

#[tokio::test]
async fn generate_from_bytes_unknown_extension_falls_back_to_tempfile() {
    // Extension not in MEMORY_DECODABLE → temp-file libvips path.
    // The bytes are still a real JPEG so libvips is happy.
    let src = write_jpeg(300, 200);
    let raw = std::fs::read(&src).unwrap();
    let tg = ThumbnailGenerator::new();

    let (bytes, mime) = tg
        .generate_from_bytes("test", &raw, "xyz", 80, 0, OutputFormat::Webp)
        .await
        .expect("thumbnail generation via temp file");

    assert_eq!(mime, "image/webp");
    let (w, _) = decode_dims(&bytes);
    assert_eq!(w, 80);

    let _ = std::fs::remove_file(&src);
}

// ── Metadata helpers ────────────────────────────────────────────────────────

#[test]
fn dimensions_from_path_and_bytes_agree() {
    let src = write_png(123, 77);
    let raw = std::fs::read(&src).unwrap();

    let from_path = get_image_dimensions(src.to_str().unwrap()).expect("path dims");
    let from_bytes = get_image_dimensions_from_bytes(&raw).expect("bytes dims");

    assert_eq!(from_path, (123, 77));
    assert_eq!(from_bytes, (123, 77));

    let _ = std::fs::remove_file(&src);
}

#[test]
fn dimensions_returns_none_for_garbage() {
    let garbage = b"not an image at all -- nothing to decode here";
    assert!(get_image_dimensions_from_bytes(garbage).is_none());
}

#[test]
fn file_mtime_returns_formatted_date() {
    let src = write_jpeg(10, 10);
    let dt = file_mtime_as_date(src.to_str().unwrap()).expect("mtime");

    // Format must be "YYYY-MM-DD HH:MM:SS"
    assert_eq!(dt.len(), 19, "expected 19 chars in {dt:?}");
    assert_eq!(dt.as_bytes()[4], b'-');
    assert_eq!(dt.as_bytes()[7], b'-');
    assert_eq!(dt.as_bytes()[10], b' ');
    assert_eq!(dt.as_bytes()[13], b':');
    assert_eq!(dt.as_bytes()[16], b':');

    let _ = std::fs::remove_file(&src);
}

// ── Date-from-filename parser ───────────────────────────────────────────────

#[test]
fn extract_date_from_filename_pixpin() {
    assert_eq!(
        extract_date_from_filename("PixPin_2024-05-17_09-12-30.png").as_deref(),
        Some("2024-05-17 09:12:30"),
    );
}

#[test]
fn extract_date_from_filename_compact_14_digits() {
    assert_eq!(
        extract_date_from_filename("578080_20260120004259_1.png").as_deref(),
        Some("2026-01-20 00:42:59"),
    );
}

#[test]
fn extract_date_from_filename_img_underscore() {
    assert_eq!(
        extract_date_from_filename("IMG_20230815_142359.jpg").as_deref(),
        Some("2023-08-15 14:23:59"),
    );
}

#[test]
fn extract_date_from_filename_screenshot_underscore() {
    assert_eq!(
        extract_date_from_filename("Screenshot_20220101_235959.jpg").as_deref(),
        Some("2022-01-01 23:59:59"),
    );
}

#[test]
fn extract_date_from_filename_iso_no_time() {
    assert_eq!(
        extract_date_from_filename("photo-2024-12-31.heic").as_deref(),
        Some("2024-12-31 00:00:00"),
    );
}

#[test]
fn extract_date_from_filename_invalid_returns_none() {
    assert!(extract_date_from_filename("random_filename.jpg").is_none());
    assert!(extract_date_from_filename("IMG_99999999_999999.jpg").is_none());
}
