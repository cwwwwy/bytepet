use std::path::PathBuf;

/// Unified error type for bytepet-core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("invalid pet manifest: {0}")]
    Manifest(String),

    #[error("invalid pet atlas: {0}")]
    Atlas(String),

    #[error("pet not found: {0}")]
    PetNotFound(String),

    #[error("provider error: {0}")]
    Provider(String),

    #[error("provider is not configured: {0}")]
    ProviderNotConfigured(String),

    #[error("authentication failed for provider {0}")]
    Unauthorized(String),

    #[error("request cancelled")]
    Cancelled,

    #[error("config error: {0}")]
    Config(String),

    #[error("memory error: {0}")]
    Memory(String),

    #[error("agent protocol error: {0}")]
    Agent(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("zip error: {0}")]
    Zip(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn manifest(msg: impl Into<String>) -> Self {
        Error::Manifest(msg.into())
    }
    pub fn atlas(msg: impl Into<String>) -> Self {
        Error::Atlas(msg.into())
    }
    pub fn config(msg: impl Into<String>) -> Self {
        Error::Config(msg.into())
    }
    pub fn memory(msg: impl Into<String>) -> Self {
        Error::Memory(msg.into())
    }
    pub fn provider(msg: impl Into<String>) -> Self {
        Error::Provider(msg.into())
    }
    pub fn not_found_path(path: impl Into<PathBuf>) -> Self {
        Error::NotFound(path.into().display().to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
