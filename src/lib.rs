//! On-the-fly image resize, thumbnail generation, RAW preview & EXIF.

pub mod cache;
pub mod raw_preview;
pub mod vips;

mod error;
mod exif;
mod metadata;
mod resize;
mod thumbnail;

pub use cache::{BytesReader, ThumbError, ThumbOrigin, ThumbStorage, ThumbnailService};
pub use error::ThumbnailError;
pub use exif::{ExifData, extract_exif, extract_exif_from_bytes};
pub use metadata::{
    extract_date_from_filename, file_mtime_as_date, get_image_dimensions, get_image_dimensions_from_bytes,
};
pub use thumbnail::ThumbnailGenerator;
pub use vips::OutputFormat;
