use std::fmt;

#[derive(Debug)]
pub enum ThumbError {
    /// Source image not found (Storage key missing, VFS path gone, HTTP 404, …)
    SourceNotFound(String),
    /// Storage read/write failure
    Storage(String),
    /// VFS read failure
    Vfs(String),
    /// HTTP fetch failure
    Http(String),
    /// Image resize / encode failure
    Generate(String),
}

impl fmt::Display for ThumbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotFound(s) => write!(f, "thumbnail source not found: {s}"),
            Self::Storage(s) => write!(f, "storage error: {s}"),
            Self::Vfs(s) => write!(f, "vfs error: {s}"),
            Self::Http(s) => write!(f, "http fetch error: {s}"),
            Self::Generate(s) => write!(f, "thumbnail generate error: {s}"),
        }
    }
}

impl std::error::Error for ThumbError {}
