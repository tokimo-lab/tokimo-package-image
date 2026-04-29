use std::path::Path;
use std::sync::Arc;

use tokio::fs;
use tokio::sync::Semaphore;

use crate::error::ThumbnailError;
use crate::resize::{resize_to_format, resize_to_format_from_memory, resize_with_ffmpeg};
use crate::vips::OutputFormat;

/// Maximum concurrent thumbnail generation tasks.
///
/// Capped at half the available CPUs so the tokio async runtime retains
/// enough threads for I/O, DB queries, and other non-blocking work.
/// With vips `concurrency_set(1)`, each task occupies exactly one blocking
/// thread, so this directly controls the blocking-thread high-water mark.
///
/// Minimum of 2 so low-core machines still get some parallelism.
fn max_concurrent() -> usize {
    let cpus = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    (cpus / 2).max(2)
}

/// Image formats that can be decoded directly from memory (no temp-file needed).
const MEMORY_DECODABLE: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif"];

/// On-the-fly thumbnail generation with concurrency control.
///
/// Thumbnails are generated as WebP. Storage/caching is the caller's
/// responsibility (e.g. via `StorageProvider`).
pub struct ThumbnailGenerator {
    semaphore: Arc<Semaphore>,
}

impl Default for ThumbnailGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbnailGenerator {
    pub fn new() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent())),
        }
    }

    /// Generate a thumbnail from a local file path.
    /// Returns the encoded bytes and content type.
    pub async fn generate(
        &self,
        source_path: &str,
        width: u32,
        height: u32,
        format: OutputFormat,
        ffmpeg_bin: Option<&Path>,
    ) -> Result<(Vec<u8>, &'static str), ThumbnailError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ThumbnailError::SemaphoreClosed)?;

        let src = source_path.to_owned();
        let ffmpeg = ffmpeg_bin.map(std::borrow::ToOwned::to_owned);
        let w = width;
        let h = height;

        let bytes = tokio::task::spawn_blocking(move || match resize_to_format(&src, w, h, format) {
            Ok(b) => Ok(b),
            Err(img_err) => {
                if let Some(ffmpeg_path) = ffmpeg {
                    resize_with_ffmpeg(&ffmpeg_path, &src, w)
                } else {
                    Err(img_err)
                }
            }
        })
        .await
        .map_err(|e| ThumbnailError::Join(e.to_string()))??;

        Ok((bytes, format.mime_type()))
    }

    /// Generate a thumbnail from raw file bytes (for remote sources like SMB/SFTP or S3).
    ///
    /// For known image formats (JPEG, PNG, WebP, etc.) the bytes are decoded
    /// directly in memory — no temp file I/O.  Unknown/unsupported formats fall
    /// back to writing a temp file so that `FFmpeg` can handle them.
    #[allow(clippy::too_many_arguments)]
    pub async fn generate_from_bytes(
        &self,
        photo_id: &str,
        file_bytes: &[u8],
        ext: &str,
        width: u32,
        height: u32,
        format: OutputFormat,
        ffmpeg_bin: Option<&Path>,
    ) -> Result<(Vec<u8>, &'static str), ThumbnailError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| ThumbnailError::SemaphoreClosed)?;

        let ext_lower = ext.to_lowercase();

        if MEMORY_DECODABLE.contains(&ext_lower.as_str()) {
            // Fast path: decode from memory using libvips (shrink-on-load).
            // If libvips buffer API fails, fall through to the temp-file path which
            // uses the file-based libvips API (known to work reliably).
            let bytes_owned = file_bytes.to_vec();
            let ext_for_tmp = ext_lower.clone();
            let result = tokio::task::spawn_blocking(move || {
                resize_to_format_from_memory(&bytes_owned, &ext_for_tmp, width, height, format)
            })
            .await
            .map_err(|e| ThumbnailError::Join(e.to_string()))??;
            return Ok((result, format.mime_type()));
        }

        // Fallback: write temp file for ffmpeg / exotic formats
        let tmp_path = std::env::temp_dir().join(format!("tokimo_thumb_{photo_id}.{ext}"));
        fs::write(&tmp_path, file_bytes).await.map_err(ThumbnailError::Io)?;

        let tmp_str = tmp_path.to_string_lossy().to_string();
        let ffmpeg = ffmpeg_bin.map(std::borrow::ToOwned::to_owned);
        let w = width;
        let h = height;

        let result = tokio::task::spawn_blocking(move || match resize_to_format(&tmp_str, w, h, format) {
            Ok(b) => Ok(b),
            Err(img_err) => {
                if let Some(ffmpeg_path) = ffmpeg {
                    resize_with_ffmpeg(&ffmpeg_path, &tmp_str, w)
                } else {
                    Err(img_err)
                }
            }
        })
        .await
        .map_err(|e| ThumbnailError::Join(e.to_string()))?;

        let _ = fs::remove_file(&tmp_path).await;

        let bytes = result?;
        Ok((bytes, format.mime_type()))
    }
}
