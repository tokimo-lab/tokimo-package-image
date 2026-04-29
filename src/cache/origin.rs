use std::path::Path;
use std::sync::Arc;

/// Minimal byte-reader for VFS / local / remote backends.
#[async_trait::async_trait]
pub trait BytesReader: Send + Sync {
    async fn read_bytes(&self, path: &Path, offset: u64, limit: Option<u64>) -> Result<Vec<u8>, String>;
}

/// Where the original image bytes come from.
///
/// The caller (e.g. `ThumbnailResolver` in `rust-server`) resolves the entity to
/// a `ThumbOrigin` before calling `ThumbnailService::get_or_generate`.
pub enum ThumbOrigin {
    /// Original is already in the `ThumbStorage` (S3).
    /// `key` is the storage key of the source image (e.g. `library-images/movies/{id}/poster.jpg`).
    Storage { key: String },

    /// Original lives in a VFS mount (local disk, SFTP, SMB, …).
    /// The `Arc<dyn BytesReader>` is pre-resolved by the caller.
    Vfs { reader: Arc<dyn BytesReader>, path: String },

    /// Original is a file on the local filesystem (absolute path).
    /// Preferred over `Vfs` for local files — avoids VFS overhead.
    LocalFile { abs_path: String },

    /// Original is at an external HTTP(S) URL.
    /// The raw bytes are cached in storage at `ext-cache/{sha256(url)}.{ext}` on first fetch.
    Http { url: String, ext: String },
}
