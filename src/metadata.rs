use std::io::Cursor;

use crate::exif::{extract_exif, extract_exif_from_bytes};

/// Try to extract a date/time from a filename using common patterns.
///
/// Supported patterns:
///   - `PixPin_YYYY-MM-DD_HH-MM-SS.ext`
///   - `NNNNNN_YYYYMMDDHHMMSS_N.ext` (e.g., `578080_20260120004259_1.png`)
///   - `IMG_YYYYMMDD_HHMMSS.ext`
///   - `Screenshot_YYYYMMDD-HHMMSS.ext`
///   - Any filename containing `YYYY-MM-DD` with optional `_HH-MM-SS`
///
/// Returns a normalized `"YYYY-MM-DD HH:MM:SS"` string.
pub fn extract_date_from_filename(filename: &str) -> Option<String> {
    let stem = filename.rsplit_once('.').map_or(filename, |(s, _)| s);
    let parts: Vec<&str> = stem.split('_').collect();

    // Pattern 1: 14-digit compact timestamp (YYYYMMDDHHMMSS) in any underscore-separated part
    for part in &parts {
        if part.len() == 14
            && part.bytes().all(|b| b.is_ascii_digit())
            && let Some(dt) = parse_compact_datetime(part)
        {
            return Some(dt);
        }
    }

    // Pattern 2: YYYY-MM-DD somewhere in the filename with optional _HH-MM-SS or THH:MM:SS
    let bytes = stem.as_bytes();
    let len = bytes.len();
    for i in 0..len.saturating_sub(9) {
        if i + 10 <= len
            && bytes[i + 4] == b'-'
            && bytes[i + 7] == b'-'
            && bytes[i..i + 4].iter().all(u8::is_ascii_digit)
            && bytes[i + 5..i + 7].iter().all(u8::is_ascii_digit)
            && bytes[i + 8..i + 10].iter().all(u8::is_ascii_digit)
        {
            let date = &stem[i..i + 10];
            let (y, m, d) = (&date[0..4], &date[5..7], &date[8..10]);
            let year: u32 = y.parse().ok()?;
            let month: u32 = m.parse().ok()?;
            let day: u32 = d.parse().ok()?;
            if !(1970..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
                continue;
            }

            // Try to find time after the date
            if i + 19 <= len {
                let sep = bytes[i + 10];
                if sep == b'_' || sep == b' ' || sep == b'T' {
                    let time_slice = &stem[i + 11..i + 19];
                    let time_str = time_slice.replace('-', ":");
                    if time_str.len() == 8
                        && time_str.as_bytes()[2] == b':'
                        && time_str.as_bytes()[5] == b':'
                        && time_str.bytes().filter(u8::is_ascii_digit).count() == 6
                    {
                        return Some(format!("{date} {time_str}"));
                    }
                }
            }
            return Some(format!("{date} 00:00:00"));
        }
    }

    // Pattern 3: 8-digit date (YYYYMMDD) in any underscore-separated part + optional time
    for (idx, part) in parts.iter().enumerate() {
        if part.len() == 8 && part.bytes().all(|b| b.is_ascii_digit()) {
            let (y, m, d) = (&part[0..4], &part[4..6], &part[6..8]);
            let year: u32 = y.parse().ok()?;
            let month: u32 = m.parse().ok()?;
            let day: u32 = d.parse().ok()?;
            if !(1970..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
                continue;
            }
            // Try time from the next part
            if let Some(next) = parts.get(idx + 1) {
                let t = next.replace('-', "");
                if t.len() >= 6 && t[..6].bytes().all(|b| b.is_ascii_digit()) {
                    let (h, mi, s) = (&t[0..2], &t[2..4], &t[4..6]);
                    return Some(format!("{y}-{m}-{d} {h}:{mi}:{s}"));
                }
            }
            return Some(format!("{y}-{m}-{d} 00:00:00"));
        }
    }

    None
}

fn parse_compact_datetime(s: &str) -> Option<String> {
    let (y, m, d) = (&s[0..4], &s[4..6], &s[6..8]);
    let (h, mi, sc) = (&s[8..10], &s[10..12], &s[12..14]);
    let year: u32 = y.parse().ok()?;
    let month: u32 = m.parse().ok()?;
    let day: u32 = d.parse().ok()?;
    if !(1970..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!("{y}-{m}-{d} {h}:{mi}:{sc}"))
}

/// Get the file modification time as a formatted date string.
/// Returns `"YYYY-MM-DD HH:MM:SS"` or `None`.
pub fn file_mtime_as_date(path: &str) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let dt: chrono::DateTime<chrono::Utc> = modified.into();
    Some(dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// Get image dimensions from a local file path (header-only read, very fast).
/// Works for JPEG, PNG, WebP, GIF, BMP, TIFF via the `image` crate.
/// Falls back to EXIF PixelXDimension/PixelYDimension for unsupported formats.
/// CPU-bound — call from `spawn_blocking`.
pub fn get_image_dimensions(path: &str) -> Option<(i32, i32)> {
    if let Ok(reader) = image::ImageReader::open(path)
        && let Ok(reader) = reader.with_guessed_format()
        && let Ok((w, h)) = reader.into_dimensions()
    {
        return Some((w as i32, h as i32));
    }
    // Fallback to EXIF for formats the image crate can't decode (e.g. HEIC)
    let exif = extract_exif(path)?;
    match (exif.width, exif.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => None,
    }
}

/// Get image dimensions from raw bytes (header-only read).
/// Falls back to EXIF when the `image` crate can't determine dimensions.
/// CPU-bound — call from `spawn_blocking`.
pub fn get_image_dimensions_from_bytes(bytes: &[u8]) -> Option<(i32, i32)> {
    if let Ok(reader) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()
        && let Ok((w, h)) = reader.into_dimensions()
    {
        return Some((w as i32, h as i32));
    }
    let exif = extract_exif_from_bytes(bytes)?;
    match (exif.width, exif.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => None,
    }
}

/// Get image dimensions via FFI probe (for HEIC/HEIF and other container formats).
/// CPU-bound — call from `spawn_blocking`.
pub fn get_dimensions_via_ffprobe(_ffprobe_bin: &std::path::Path, file_path: &str) -> Option<(i32, i32)> {
    let info = tokimo_package_ffmpeg::probe_file(file_path).ok()?;
    let video = info.streams.iter().find(|s| s.codec_type == "video")?;
    let vi = video.video.as_ref()?;
    if vi.width > 0 && vi.height > 0 {
        Some((vi.width, vi.height))
    } else {
        None
    }
}

/// Extract date/time from a file using FFI probe (handles HEIC, AVIF, etc.).
/// `_ffprobe_bin` is kept for API compatibility; `file_path` must be a local file.
/// Returns date string in `"YYYY-MM-DD HH:MM:SS"` format.
pub fn extract_date_via_ffprobe(_ffprobe_bin: &std::path::Path, file_path: &str) -> Option<String> {
    let info = tokimo_package_ffmpeg::probe_file(file_path).ok()?;

    // Search format tags first, then stream tags
    let creation_time = info
        .format
        .tags
        .get("creation_time")
        .or_else(|| info.format.tags.get("com.apple.quicktime.creationdate"))
        .or_else(|| info.streams.first().and_then(|s| s.tags.get("creation_time")))?;

    // Try RFC 3339 / ISO 8601 with timezone
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(creation_time) {
        return Some(dt.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    // Try "YYYY-MM-DDTHH:MM:SS.fZ" (common ffprobe format)
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(creation_time, "%Y-%m-%dT%H:%M:%S%.fZ") {
        return Some(dt.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    // Try "YYYY-MM-DDTHH:MM:SS" without trailing Z
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(creation_time, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt.format("%Y-%m-%d %H:%M:%S").to_string());
    }

    None
}
