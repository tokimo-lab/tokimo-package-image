use std::collections::HashMap;

/// Extracted EXIF metadata from an image file.
#[derive(Debug, Default)]
pub struct ExifData {
    pub taken_at: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub focal_length: Option<f64>,
    pub aperture: Option<f64>,
    pub shutter_speed: Option<String>,
    pub iso: Option<i32>,
    pub orientation: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
    pub gps_altitude: Option<f64>,
    pub focal_length_35mm: Option<i32>,
    pub exposure_mode: Option<String>,
    pub white_balance: Option<String>,
    pub flash: Option<String>,
    pub metering_mode: Option<String>,
    pub scene_type: Option<String>,
    pub software: Option<String>,
    pub exposure_compensation: Option<f64>,
    pub color_space: Option<String>,
    /// Full raw EXIF as key-value pairs for JSON storage.
    pub raw_tags: HashMap<String, String>,
}

/// Shared helper: extract all EXIF tag values from a parsed `exif::Exif` container.
fn parse_exif_tags(exif: &exif::Exif) -> ExifData {
    let mut data = ExifData::default();

    if let Some(field) = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY) {
        data.taken_at = Some(field.display_value().to_string());
    } else if let Some(field) = exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY) {
        data.taken_at = Some(field.display_value().to_string());
    }

    if let Some(field) = exif.get_field(exif::Tag::Make, exif::In::PRIMARY) {
        data.camera_make = Some(field.display_value().to_string().trim_matches('"').to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::Model, exif::In::PRIMARY) {
        data.camera_model = Some(field.display_value().to_string().trim_matches('"').to_string());
    }

    if let Some(field) = exif.get_field(exif::Tag::LensModel, exif::In::PRIMARY) {
        data.lens_model = Some(field.display_value().to_string().trim_matches('"').to_string());
    }

    if let Some(field) = exif.get_field(exif::Tag::FocalLength, exif::In::PRIMARY)
        && let exif::Value::Rational(ref v) = field.value
        && let Some(r) = v.first()
        && r.denom != 0
    {
        data.focal_length = Some(f64::from(r.num) / f64::from(r.denom));
    }

    if let Some(field) = exif.get_field(exif::Tag::FNumber, exif::In::PRIMARY)
        && let exif::Value::Rational(ref v) = field.value
        && let Some(r) = v.first()
        && r.denom != 0
    {
        data.aperture = Some(f64::from(r.num) / f64::from(r.denom));
    }

    if let Some(field) = exif.get_field(exif::Tag::ExposureTime, exif::In::PRIMARY) {
        data.shutter_speed = Some(field.display_value().to_string());
    }

    if let Some(field) = exif.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY)
        && let exif::Value::Short(ref v) = field.value
        && let Some(&iso) = v.first()
    {
        data.iso = Some(i32::from(iso));
    }

    if let Some(field) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        && let exif::Value::Short(ref v) = field.value
        && let Some(&o) = v.first()
    {
        data.orientation = Some(i32::from(o));
    }

    if let Some(field) = exif.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY) {
        if let exif::Value::Long(ref v) = field.value {
            if let Some(&w) = v.first() {
                data.width = Some(w as i32);
            }
        } else if let exif::Value::Short(ref v) = field.value
            && let Some(&w) = v.first()
        {
            data.width = Some(i32::from(w));
        }
    }
    if let Some(field) = exif.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY) {
        if let exif::Value::Long(ref v) = field.value {
            if let Some(&h) = v.first() {
                data.height = Some(h as i32);
            }
        } else if let exif::Value::Short(ref v) = field.value
            && let Some(&h) = v.first()
        {
            data.height = Some(i32::from(h));
        }
    }

    if let (Some(lat_field), Some(lat_ref)) = (
        exif.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLatitudeRef, exif::In::PRIMARY),
    ) && let exif::Value::Rational(ref v) = lat_field.value
        && v.len() >= 3
    {
        let deg = f64::from(v[0].num) / f64::from(v[0].denom);
        let min = f64::from(v[1].num) / f64::from(v[1].denom);
        let sec = f64::from(v[2].num) / f64::from(v[2].denom);
        let mut lat = deg + min / 60.0 + sec / 3600.0;
        if lat_ref.display_value().to_string().contains('S') {
            lat = -lat;
        }
        data.gps_latitude = Some(lat);
    }
    if let (Some(lon_field), Some(lon_ref)) = (
        exif.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY),
        exif.get_field(exif::Tag::GPSLongitudeRef, exif::In::PRIMARY),
    ) && let exif::Value::Rational(ref v) = lon_field.value
        && v.len() >= 3
    {
        let deg = f64::from(v[0].num) / f64::from(v[0].denom);
        let min = f64::from(v[1].num) / f64::from(v[1].denom);
        let sec = f64::from(v[2].num) / f64::from(v[2].denom);
        let mut lon = deg + min / 60.0 + sec / 3600.0;
        if lon_ref.display_value().to_string().contains('W') {
            lon = -lon;
        }
        data.gps_longitude = Some(lon);
    }
    if let Some(field) = exif.get_field(exif::Tag::GPSAltitude, exif::In::PRIMARY)
        && let exif::Value::Rational(ref v) = field.value
        && let Some(r) = v.first()
        && r.denom != 0
    {
        data.gps_altitude = Some(f64::from(r.num) / f64::from(r.denom));
    }

    // New structured fields
    if let Some(field) = exif.get_field(exif::Tag::FocalLengthIn35mmFilm, exif::In::PRIMARY)
        && let exif::Value::Short(ref v) = field.value
        && let Some(&val) = v.first()
    {
        data.focal_length_35mm = Some(i32::from(val));
    }

    if let Some(field) = exif.get_field(exif::Tag::ExposureMode, exif::In::PRIMARY) {
        data.exposure_mode = Some(field.display_value().to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::WhiteBalance, exif::In::PRIMARY) {
        data.white_balance = Some(field.display_value().to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::Flash, exif::In::PRIMARY) {
        data.flash = Some(field.display_value().to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::MeteringMode, exif::In::PRIMARY) {
        data.metering_mode = Some(field.display_value().to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::SceneType, exif::In::PRIMARY) {
        data.scene_type = Some(field.display_value().to_string());
    }
    if let Some(field) = exif.get_field(exif::Tag::Software, exif::In::PRIMARY) {
        data.software = Some(field.display_value().to_string().trim_matches('"').to_string());
    }

    if let Some(field) = exif.get_field(exif::Tag::ExposureBiasValue, exif::In::PRIMARY)
        && let exif::Value::SRational(ref v) = field.value
        && let Some(r) = v.first()
        && r.denom != 0
    {
        data.exposure_compensation = Some(f64::from(r.num) / f64::from(r.denom));
    }

    if let Some(field) = exif.get_field(exif::Tag::ColorSpace, exif::In::PRIMARY) {
        data.color_space = Some(field.display_value().to_string());
    }

    // Collect all EXIF tags as raw key-value pairs
    for field in exif.fields() {
        let tag_name = field.tag.to_string();
        let value_str = field.display_value().to_string();
        if !value_str.starts_with("(Binary") && value_str.len() < 500 {
            data.raw_tags.insert(tag_name, value_str);
        }
    }

    data
}

/// Extract EXIF metadata from a local image file. Returns `None` if no EXIF data found.
/// This is CPU-bound and should be called from `spawn_blocking`.
pub fn extract_exif(path: &str) -> Option<ExifData> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf_reader = std::io::BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut buf_reader).ok()?;
    Some(parse_exif_tags(&exif))
}

/// Extract EXIF metadata from raw file bytes (works for JPEG, TIFF).
/// For HEIC/AVIF, kamadak-exif may not work — caller should try ffprobe fallback.
/// This is CPU-bound and should be called from `spawn_blocking`.
pub fn extract_exif_from_bytes(bytes: &[u8]) -> Option<ExifData> {
    let mut cursor = std::io::Cursor::new(bytes);
    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut cursor).ok()?;
    Some(parse_exif_tags(&exif))
}
