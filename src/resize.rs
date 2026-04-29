use std::io::Cursor;
use std::path::Path;

use image::ImageFormat;
use image::imageops::FilterType;
use image::metadata::Orientation;

use crate::error::ThumbnailError;
use crate::raw_preview;
use crate::vips;
use crate::vips::OutputFormat;

/// CPU-bound: decode image, apply EXIF orientation, resize, encode to target format.
///
/// Uses libvips for shrink-on-load when possible, falling back to the `image`
/// crate + `FFmpeg` for formats vips cannot handle.
pub(crate) fn resize_to_format(
    source_path: &str,
    width: u32,
    height: u32,
    format: OutputFormat,
) -> Result<Vec<u8>, ThumbnailError> {
    // For RAW formats, extract the embedded JPEG preview first
    let ext = Path::new(source_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if raw_preview::is_raw_with_preview(ext)
        && let Ok(file_data) = std::fs::read(source_path)
        && let Some(jpeg_data) = raw_preview::extract_raw_preview(&file_data)
    {
        // Use libvips to resize the extracted JPEG preview
        if let Ok(out) = vips::thumbnail_to_format(&jpeg_data, width, height, format) {
            return Ok(out);
        }
        // Fallback: image crate
        if let Ok(mut img) = image::load_from_memory_with_format(&jpeg_data, ImageFormat::Jpeg) {
            if let Some(orientation) = read_exif_orientation(source_path) {
                img.apply_orientation(orientation);
            }
            let (orig_w, orig_h) = (img.width(), img.height());
            if orig_w > width {
                let h = (f64::from(orig_h) * f64::from(width) / f64::from(orig_w)).round() as u32;
                let resized = img.resize_exact(width, h, FilterType::Triangle);
                let mut buf = Cursor::new(Vec::with_capacity(32 * 1024));
                resized
                    .write_to(&mut buf, image_format_for(format))
                    .map_err(ThumbnailError::Image)?;
                return Ok(buf.into_inner());
            }
            let mut buf = Cursor::new(Vec::new());
            img.write_to(&mut buf, image_format_for(format))
                .map_err(ThumbnailError::Image)?;
            return Ok(buf.into_inner());
        }
    }

    // Primary path: libvips shrink-on-load from file
    if let Ok(out) = vips::thumbnail_file_to_format(source_path, width, height, format) {
        return Ok(out);
    }

    // Fallback: image crate (for formats vips can't handle on this system)
    resize_to_format_image_crate(source_path, width, height, format)
}

/// Fallback resize using the `image` crate (full-resolution decode).
fn resize_to_format_image_crate(
    source_path: &str,
    width: u32,
    height: u32,
    format: OutputFormat,
) -> Result<Vec<u8>, ThumbnailError> {
    let mut img = image::open(Path::new(source_path)).map_err(ThumbnailError::Image)?;

    if let Some(orientation) = read_exif_orientation(source_path) {
        img.apply_orientation(orientation);
    }

    let (orig_w, orig_h) = (img.width(), img.height());
    let target_h = if height == 0 {
        (f64::from(orig_h) * f64::from(width) / f64::from(orig_w)).round() as u32
    } else {
        height
    };

    if orig_w <= width && (height == 0 || orig_h <= height) {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image_format_for(format))
            .map_err(ThumbnailError::Image)?;
        return Ok(buf.into_inner());
    }

    let resized = img.resize_exact(width, target_h, FilterType::Triangle);

    let mut buf = Cursor::new(Vec::with_capacity(32 * 1024));
    resized
        .write_to(&mut buf, image_format_for(format))
        .map_err(ThumbnailError::Image)?;

    Ok(buf.into_inner())
}

/// In-memory decode path for known image formats — no temp file I/O.
///
/// 1. Try libvips `thumbnail_buffer` (shrink-on-load, fastest)
/// 2. If buffer API fails, write to temp file and use libvips file API (still fast)
/// 3. Last resort: `image` crate (full decode, slow for large images)
pub(crate) fn resize_to_format_from_memory(
    file_bytes: &[u8],
    ext: &str,
    width: u32,
    height: u32,
    format: OutputFormat,
) -> Result<Vec<u8>, ThumbnailError> {
    // 1. libvips from buffer (best: shrink-on-load, no disk I/O)
    match vips::thumbnail_to_format(file_bytes, width, height, format) {
        Ok(out) => return Ok(out),
        Err(e) => tracing::warn!("[tokimo-package-image] libvips buffer failed ({e}), trying file path"),
    }

    // 2. libvips from temp file (still fast, avoids full image decode)
    let tmp_path = std::env::temp_dir().join(format!(
        "tokimo_vips_{}_{}.{ext}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    if std::fs::write(&tmp_path, file_bytes).is_ok() {
        let tmp_str = tmp_path.to_string_lossy().to_string();
        let result = vips::thumbnail_file_to_format(&tmp_str, width, height, format);
        let _ = std::fs::remove_file(&tmp_path);
        match result {
            Ok(out) => return Ok(out),
            Err(e) => tracing::warn!("[tokimo-package-image] libvips file failed ({e}), using image crate"),
        }
    }

    // 3. image crate fallback (full-resolution decode — slow for large images)
    let mut img = image::load_from_memory(file_bytes).map_err(ThumbnailError::Image)?;

    let mut cursor = std::io::Cursor::new(file_bytes);
    if let Ok(exif) = exif::Reader::new().read_from_container(&mut cursor)
        && let Some(field) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        && let exif::Value::Short(ref v) = field.value
        && let Some(&val) = v.first()
        && let Some(orientation) = image::metadata::Orientation::from_exif(val as u8)
    {
        img.apply_orientation(orientation);
    }

    let (orig_w, orig_h) = (img.width(), img.height());
    let target_h = if height == 0 {
        (f64::from(orig_h) * f64::from(width) / f64::from(orig_w)).round() as u32
    } else {
        height
    };

    if orig_w <= width && (height == 0 || orig_h <= height) {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image_format_for(format))
            .map_err(ThumbnailError::Image)?;
        return Ok(buf.into_inner());
    }

    let resized = img.resize_exact(width, target_h, FilterType::Triangle);

    let mut buf = Cursor::new(Vec::with_capacity(32 * 1024));
    resized
        .write_to(&mut buf, image_format_for(format))
        .map_err(ThumbnailError::Image)?;

    Ok(buf.into_inner())
}

fn image_format_for(format: OutputFormat) -> ImageFormat {
    match format {
        OutputFormat::Webp => ImageFormat::WebP,
        OutputFormat::Jpeg => ImageFormat::Jpeg,
        OutputFormat::Png => ImageFormat::Png,
    }
}

/// Read EXIF orientation tag from a JPEG/TIFF file.
fn read_exif_orientation(path: &str) -> Option<Orientation> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    if let exif::Value::Short(ref v) = field.value {
        let val = *v.first()? as u8;
        return Orientation::from_exif(val);
    }
    None
}

/// Fallback: use ffmpeg FFI to decode the image to JPEG, then re-encode as WebP.
/// Handles formats the `image` crate cannot decode (HEIC, AVIF, RAW, etc.).
pub(crate) fn resize_with_ffmpeg(_ffmpeg_bin: &Path, source_path: &str, width: u32) -> Result<Vec<u8>, ThumbnailError> {
    use tokimo_package_ffmpeg::image::{ImageDecodeOptions, ImageFormat as FfImageFormat, decode_image};

    let opts = ImageDecodeOptions {
        width: Some(width),
        format: FfImageFormat::Jpeg,
        quality: 2,
    };
    let jpeg_data = decode_image(Path::new(source_path), &opts)
        .map_err(|e| ThumbnailError::Ffmpeg(format!("FFI decode failed: {e}")))?;

    let mut img = image::load_from_memory_with_format(&jpeg_data, ImageFormat::Jpeg).map_err(ThumbnailError::Image)?;

    // FFmpeg doesn't auto-rotate; apply EXIF orientation from the original file
    if let Some(orientation) = read_exif_orientation(source_path) {
        img.apply_orientation(orientation);
    }

    let mut buf = Cursor::new(Vec::with_capacity(32 * 1024));
    img.write_to(&mut buf, ImageFormat::WebP)
        .map_err(ThumbnailError::Image)?;

    Ok(buf.into_inner())
}
