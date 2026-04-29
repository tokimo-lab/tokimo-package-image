use async_trait::async_trait;
use bytes::Bytes;

/// Minimal storage interface required by `ThumbnailService`.
///
/// Implement this for your `StorageProvider` adapter in the server crate.
#[async_trait]
pub trait ThumbStorage: Send + Sync {
    /// Return cached bytes if the key exists, `None` otherwise.
    async fn get(&self, key: &str) -> Option<Bytes>;

    /// Persist bytes at `key`. `content_type` is a MIME hint (e.g. `"image/webp"`).
    async fn put(&self, key: &str, data: Bytes, content_type: Option<&str>) -> Result<(), String>;
}
