//! High-level thumbnail cache pipeline.
//!
//! - Check S3 cache → hit: return immediately
//! - Miss: fetch original from `ThumbOrigin` (S3 key / VFS file / HTTP URL)
//! - Resize + encode via `ThumbnailGenerator`
//! - Write back to S3 cache asynchronously (fire-and-forget)

pub mod error;
pub mod origin;
pub mod service;
pub mod storage;

pub use error::ThumbError;
pub use origin::{BytesReader, ThumbOrigin};
pub use service::ThumbnailService;
pub use storage::ThumbStorage;
