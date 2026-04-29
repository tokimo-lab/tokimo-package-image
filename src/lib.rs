//! On-the-fly image resize, thumbnail generation, RAW preview & EXIF.

pub mod raw_preview;
pub mod vips;

mod error;
mod exif;
mod metadata;
mod resize;
mod thumbnail;

pub use error::ThumbnailError;
pub use exif::{ExifData, extract_exif, extract_exif_from_bytes};
pub use metadata::{
    extract_date_from_filename, extract_date_via_ffprobe, file_mtime_as_date, get_dimensions_via_ffprobe,
    get_image_dimensions, get_image_dimensions_from_bytes,
};
pub use thumbnail::ThumbnailGenerator;
pub use vips::OutputFormat;
