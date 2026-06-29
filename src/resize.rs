use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use image::ImageFormat;
use image::imageops::FilterType;
use image::metadata::Orientation;

use crate::error::ThumbnailError;
use crate::raw_preview;
use crate::vips;
use crate::vips::OutputFormat;

const FFMPEG_DECODABLE: &[&str] = &["heic", "heif", "avif"];

/// CPU-bound: decode image, apply EXIF orientation, resize, encode to target format.
///
/// Uses libvips for shrink-on-load when possible, falling back to the `image`
/// crate for formats vips cannot handle.
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
    if is_heic_ext(ext) {
        return ffmpeg_ffi_file_to_format(source_path, width, format).or_else(|e| {
            tracing::warn!("[tokimo-package-image] HEIC FFI decode failed ({e}), trying ffmpeg CLI");
            ffmpeg_file_to_format(source_path, width, height, format).map_err(ThumbnailError::External)
        });
    }

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

    let vips_result = vips::thumbnail_file_to_format(source_path, width, height, format);
    if let Ok(out) = vips_result {
        return Ok(out);
    }

    if needs_ffmpeg_fallback(ext) {
        return ffmpeg_file_to_format(source_path, width, height, format).map_err(ThumbnailError::External);
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
    if is_heic_ext(ext) {
        return ffmpeg_ffi_bytes_to_format(file_bytes, ext, width, format).or_else(|e| {
            tracing::warn!("[tokimo-package-image] HEIC FFI decode failed ({e}), trying ffmpeg CLI");
            ffmpeg_cli_bytes_to_format(file_bytes, ext, width, height, format)
        });
    }

    // 1. libvips from buffer (best: shrink-on-load, no disk I/O)
    match vips::thumbnail_to_format(file_bytes, width, height, format) {
        Ok(out) => return Ok(out),
        Err(e) => tracing::warn!("[tokimo-package-image] libvips buffer failed ({e}), trying file path"),
    }

    // 2. libvips from temp file (still fast, avoids full image decode)
    let temp_ext = ext_for_temp_path(ext);
    let tmp_path = std::env::temp_dir().join(format!(
        "tokimo_vips_{}_{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
        temp_ext
    ));
    if std::fs::write(&tmp_path, file_bytes).is_ok() {
        let tmp_str = tmp_path.to_string_lossy().to_string();
        let result = vips::thumbnail_file_to_format(&tmp_str, width, height, format);
        match result {
            Ok(out) => {
                let _ = std::fs::remove_file(&tmp_path);
                return Ok(out);
            }
            Err(e) => tracing::warn!("[tokimo-package-image] libvips file failed ({e}), trying fallback decoder"),
        }

        if needs_ffmpeg_fallback(ext) {
            let result = ffmpeg_file_to_format(&tmp_str, width, height, format).map_err(ThumbnailError::External);
            let _ = std::fs::remove_file(&tmp_path);
            return result;
        }

        let _ = std::fs::remove_file(&tmp_path);
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

fn needs_ffmpeg_fallback(ext: &str) -> bool {
    let match_ext = ext_for_match(ext);
    FFMPEG_DECODABLE.iter().any(|candidate| match_ext == *candidate)
}

fn is_heic_ext(ext: &str) -> bool {
    matches!(ext_for_match(ext).as_str(), "heic" | "heif")
}

fn ext_for_match(ext: &str) -> String {
    ext.trim()
        .trim_matches(['"', '\'', '`', '\\'])
        .trim_start_matches('.')
        .trim_matches(['"', '\'', '`', '\\'])
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

fn ext_for_temp_path(ext: &str) -> String {
    let cleaned: String = ext
        .trim()
        .trim_matches(['"', '\'', '`', '\\'])
        .trim_start_matches('.')
        .trim_matches(['"', '\'', '`', '\\'])
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if cleaned.is_empty() { "bin".to_string() } else { cleaned }
}

fn ffmpeg_file_to_format(source_path: &str, width: u32, height: u32, format: OutputFormat) -> Result<Vec<u8>, String> {
    let ffmpeg = ffmpeg_binary().ok_or_else(|| "ffmpeg binary not found".to_string())?;
    let scale = match (width, height) {
        (0, 0) => None,
        (0, h) => Some(format!("scale=-1:{h}")),
        (w, 0) => Some(format!("scale={w}:-1")),
        (w, h) => Some(format!("scale={w}:{h}")),
    };
    let codec = match format {
        OutputFormat::Webp => "libwebp",
        OutputFormat::Jpeg => "mjpeg",
        OutputFormat::Png => "png",
    };

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i", source_path]);

    if is_heic_path(source_path) {
        let filter = format!("[0:g:0]{}[out]", scale.as_deref().unwrap_or("scale=-1:-1"));
        cmd.args(["-filter_complex", &filter, "-map", "[out]"]);
    } else {
        cmd.args(["-frames:v", "1"]);
        if let Some(scale) = &scale {
            cmd.args(["-vf", scale]);
        }
    }

    match format {
        OutputFormat::Webp => {
            cmd.args(["-vcodec", codec, "-quality", "80"]);
        }
        OutputFormat::Jpeg => {
            cmd.args(["-vcodec", codec, "-q:v", "3"]);
        }
        OutputFormat::Png => {
            cmd.args(["-vcodec", codec]);
        }
    }
    cmd.args(["-f", "image2pipe", "pipe:1"]);

    let output = cmd.output().map_err(|e| format!("spawn ffmpeg: {e}"))?;
    if output.status.success() && !output.stdout.is_empty() {
        return Ok(output.stdout);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("ffmpeg exited with {}: {}", output.status, stderr.trim()))
}

fn ffmpeg_cli_bytes_to_format(
    file_bytes: &[u8],
    ext: &str,
    width: u32,
    height: u32,
    format: OutputFormat,
) -> Result<Vec<u8>, ThumbnailError> {
    let temp_ext = ext_for_temp_path(ext);
    let tmp_path = std::env::temp_dir().join(format!(
        "tokimo_heic_thumb_{}_{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
        temp_ext
    ));
    std::fs::write(&tmp_path, file_bytes).map_err(ThumbnailError::Io)?;
    let tmp_str = tmp_path.to_string_lossy().to_string();
    let result = ffmpeg_file_to_format(&tmp_str, width, height, format).map_err(ThumbnailError::External);
    let _ = std::fs::remove_file(&tmp_path);
    result
}

fn ffmpeg_ffi_file_to_format(source_path: &str, width: u32, format: OutputFormat) -> Result<Vec<u8>, ThumbnailError> {
    let opts = tokimo_package_ffmpeg::image::ImageDecodeOptions {
        width: (width > 0).then_some(width),
        format: ffmpeg_ffi_format(format),
        quality: ffmpeg_ffi_quality(format),
    };
    tokimo_package_ffmpeg::image::decode_image(Path::new(source_path), &opts)
        .map_err(|e| ThumbnailError::External(format!("FFI decode failed: {e}")))
}

fn ffmpeg_ffi_bytes_to_format(
    file_bytes: &[u8],
    ext: &str,
    width: u32,
    format: OutputFormat,
) -> Result<Vec<u8>, ThumbnailError> {
    let filename_hint = format!("input.{}", ext_for_temp_path(ext));
    let opts = tokimo_package_ffmpeg::image::ImageDecodeOptions {
        width: (width > 0).then_some(width),
        format: ffmpeg_ffi_format(format),
        quality: ffmpeg_ffi_quality(format),
    };
    tokimo_package_ffmpeg::image::decode_image_from_bytes(file_bytes, &filename_hint, &opts)
        .map_err(|e| ThumbnailError::External(format!("FFI decode failed: {e}")))
}

fn ffmpeg_ffi_format(format: OutputFormat) -> tokimo_package_ffmpeg::image::ImageFormat {
    match format {
        OutputFormat::Webp => tokimo_package_ffmpeg::image::ImageFormat::WebP,
        OutputFormat::Jpeg => tokimo_package_ffmpeg::image::ImageFormat::Jpeg,
        OutputFormat::Png => tokimo_package_ffmpeg::image::ImageFormat::Png,
    }
}

fn ffmpeg_ffi_quality(format: OutputFormat) -> u8 {
    match format {
        OutputFormat::Webp => 80,
        OutputFormat::Jpeg => 3,
        OutputFormat::Png => 0,
    }
}

fn is_heic_path(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|ext| ext.to_str()).unwrap_or("");
    is_heic_ext(ext)
}

fn ffmpeg_binary() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("TOKIMO_FFMPEG_BIN") {
        let path = std::path::PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    for root_var in ["TOKIMO_WORKSPACE_ROOT", "TOKIMO_PROJECT_ROOT"] {
        if let Ok(root) = std::env::var(root_var) {
            let candidate = std::path::PathBuf::from(root)
                .join("bin")
                .join("tokimo-lib")
                .join("current")
                .join("bin")
                .join("ffmpeg");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let opt_candidate = std::path::PathBuf::from("/opt/tokimo/bin/tokimo-lib/current/bin/ffmpeg");
    if opt_candidate.is_file() {
        return Some(opt_candidate);
    }

    Some(std::path::PathBuf::from("ffmpeg"))
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
