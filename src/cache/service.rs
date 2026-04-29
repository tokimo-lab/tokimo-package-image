use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use hex::ToHex;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::cache::error::ThumbError;
use crate::cache::origin::ThumbOrigin;
use crate::cache::storage::ThumbStorage;

/// Stateless thumbnail service.
///
/// Holds shared dependencies (storage, generator, ffmpeg path) behind `Arc`s
/// so it can be cheaply cloned and passed to `tokio::spawn`.
#[derive(Clone)]
pub struct ThumbnailService {
    storage: Arc<dyn ThumbStorage>,
    generator: Arc<crate::ThumbnailGenerator>,
    ffmpeg_bin: Option<PathBuf>,
}

impl ThumbnailService {
    pub fn new(
        storage: Arc<dyn ThumbStorage>,
        generator: Arc<crate::ThumbnailGenerator>,
        ffmpeg_bin: Option<PathBuf>,
    ) -> Self {
        Self {
            storage,
            generator,
            ffmpeg_bin,
        }
    }

    // ── Key convention ────────────────────────────────────────────────────────

    /// Derive the S3 cache key for a thumbnail.
    ///
    /// Format: `thumbs/{entity_type}/{entity_id}.{w}x{h}.{ext}`
    ///
    /// `h = 0` → proportional resize to width `w`.
    pub fn cache_key(entity_type: &str, entity_id: &str, w: u32, h: u32, format: crate::vips::OutputFormat) -> String {
        format!("thumbs/{entity_type}/{entity_id}.{w}x{h}.{}", format.ext())
    }

    // ── Main entry points ─────────────────────────────────────────────────────

    /// Entity-based thumbnail.  Cache key: `thumbs/{type}/{id}.{w}x{h}.{ext}`
    ///
    /// 1. Check S3 cache → hit: return immediately
    /// 2. Fetch original bytes from `origin`
    /// 3. Resize + encode to target format
    /// 4. Write back to S3 asynchronously
    /// 5. Return bytes to caller
    pub async fn get_or_generate(
        &self,
        entity_type: &str,
        entity_id: &str,
        origin: ThumbOrigin,
        w: u32,
        h: u32,
        format: crate::vips::OutputFormat,
    ) -> Result<Bytes, ThumbError> {
        let cache_key = Self::cache_key(entity_type, entity_id, w, h, format);
        self.generate_with_cache_key(origin, cache_key, entity_id, w, h, format)
            .await
    }

    /// S3-key-based thumbnail.  Cache key: `{original_key}.{w}x{h}.{ext}`
    ///
    /// Designed for callers that only know the storage key of the original
    /// (e.g. `/storage/library-images/movies/…/poster.jpg`) but not the
    /// entity ID.  The thumbnail is stored co-located with the original,
    /// matching the CDN-style naming scheme.
    pub async fn get_or_generate_s3(
        &self,
        original_key: &str,
        w: u32,
        h: u32,
        format: crate::vips::OutputFormat,
    ) -> Result<Bytes, ThumbError> {
        let cache_key = format!("thumbs/{original_key}.{w}x{h}.{}", format.ext());
        let origin = ThumbOrigin::Storage {
            key: original_key.to_string(),
        };
        self.generate_with_cache_key(origin, cache_key, original_key, w, h, format)
            .await
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Core generation logic: check cache, fetch origin, resize, write back.
    async fn generate_with_cache_key(
        &self,
        origin: ThumbOrigin,
        cache_key: String,
        label: &str,
        w: u32,
        h: u32,
        format: crate::vips::OutputFormat,
    ) -> Result<Bytes, ThumbError> {
        // 1. Cache hit
        if let Some(cached) = self.storage.get(&cache_key).await {
            return Ok(cached);
        }

        // 2. Fetch original
        let (raw, ext) = self.fetch_origin(origin).await?;

        // 3. Generate thumbnail
        let thumb = self
            .generator
            .generate_from_bytes(label, &raw, &ext, w, h, format, self.ffmpeg_bin.as_deref())
            .await
            .map_err(|e| ThumbError::Generate(e.to_string()))?;

        let thumb_bytes = Bytes::from(thumb.0);
        let mime = format.mime_type();

        // 4. Async write-back (fire-and-forget — do not block the response)
        let svc = self.clone();
        let key = cache_key;
        let to_store = thumb_bytes.clone();
        tokio::spawn(async move {
            if let Err(e) = svc.storage.put(&key, to_store, Some(mime)).await {
                warn!("[tokimo-image] cache write failed for {key}: {e}");
            }
        });

        Ok(thumb_bytes)
    }

    async fn fetch_origin(&self, origin: ThumbOrigin) -> Result<(Bytes, String), ThumbError> {
        match origin {
            ThumbOrigin::Storage { key } => {
                let ext = key.rsplit('.').next().unwrap_or("jpg").to_string();
                let bytes = self.storage.get(&key).await.ok_or(ThumbError::SourceNotFound(key))?;
                Ok((bytes, ext))
            }

            ThumbOrigin::Vfs { reader, path } => {
                let ext = path.rsplit('.').next().unwrap_or("jpg").to_string();
                let bytes = reader
                    .read_bytes(Path::new(&path), 0, None)
                    .await
                    .map_err(ThumbError::Vfs)?;
                Ok((Bytes::from(bytes), ext))
            }

            ThumbOrigin::LocalFile { abs_path } => {
                let ext = abs_path.rsplit('.').next().unwrap_or("jpg").to_string();
                let bytes = tokio::fs::read(&abs_path)
                    .await
                    .map_err(|e| ThumbError::Vfs(format!("local read {abs_path}: {e}")))?;
                Ok((Bytes::from(bytes), ext))
            }

            ThumbOrigin::Http { url, ext } => self.fetch_http(&url, &ext).await,
        }
    }

    /// Fetch from HTTP URL, using `ext-cache/{sha256(url)}.{ext}` as a raw-bytes cache.
    async fn fetch_http(&self, url: &str, ext: &str) -> Result<(Bytes, String), ThumbError> {
        let hash: String = Sha256::digest(url.as_bytes()).encode_hex();
        let http_cache_key = format!("ext-cache/{hash}.{ext}");

        // Check raw-bytes cache first
        if let Some(cached) = self.storage.get(&http_cache_key).await {
            return Ok((cached, ext.to_string()));
        }

        // Fetch from origin URL
        let response = reqwest::get(url).await.map_err(|e| ThumbError::Http(e.to_string()))?;

        if !response.status().is_success() {
            return Err(ThumbError::Http(format!("HTTP {} for {url}", response.status())));
        }

        let raw = response.bytes().await.map_err(|e| ThumbError::Http(e.to_string()))?;

        // Async cache the raw source bytes (O(1) clone — just increments refcount)
        let svc = self.clone();
        let key = http_cache_key;
        let to_store = raw.clone();
        let mime = format!("image/{ext}");
        tokio::spawn(async move {
            if let Err(e) = svc.storage.put(&key, to_store, Some(&mime)).await {
                warn!("[tokimo-image] ext-cache write failed for {key}: {e}");
            }
        });

        Ok((raw, ext.to_string()))
    }
}
