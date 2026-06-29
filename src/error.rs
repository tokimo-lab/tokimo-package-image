#[derive(Debug)]
pub enum ThumbnailError {
    Io(std::io::Error),
    Image(image::ImageError),
    External(String),
    Join(String),
    SemaphoreClosed,
}

impl std::fmt::Display for ThumbnailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Image(e) => write!(f, "image error: {e}"),
            Self::External(e) => write!(f, "external decoder error: {e}"),
            Self::Join(e) => write!(f, "task join error: {e}"),
            Self::SemaphoreClosed => write!(f, "thumbnail semaphore closed"),
        }
    }
}
