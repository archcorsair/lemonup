use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LemonupError {
    #[error("unable to resolve LemonUp config/data directories for this platform")]
    PathsUnavailable,
    #[error("missing config file at {0}")]
    ConfigMissing(PathBuf),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("toml deserialize error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, LemonupError>;
