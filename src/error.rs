pub type DabResult<T> = Result<T, DabError>;

#[derive(thiserror::Error, Debug)]
pub enum DabError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Decode error: {0}")]
    Decode(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Player error: {0}")]
    Player(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Temporary buffer underrun during streaming")]
    TemporaryBufferUnderrun,
}
