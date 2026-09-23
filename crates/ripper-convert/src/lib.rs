//! Converters from Unity objects (read through `ripper-unity`) to the formats in `ripper-format`.

pub mod audio;
pub mod moc3;
pub mod motion;
pub mod movie;
pub mod texture;

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("{what}: {detail}")]
    Malformed { what: &'static str, detail: String },
    #[error("{0} is not supported: {1}")]
    Unsupported(&'static str, String),
    #[error(transparent)]
    Unity(#[from] ripper_unity::UnityError),
    #[error("PNG encoding failed: {0}")]
    Png(#[from] png::EncodingError),
    #[error("{0}")]
    Cri(String),
}

pub type Result<T> = std::result::Result<T, ConvertError>;

pub(crate) fn malformed(what: &'static str, detail: impl Into<String>) -> ConvertError {
    ConvertError::Malformed {
        what,
        detail: detail.into(),
    }
}
