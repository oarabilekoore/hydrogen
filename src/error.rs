use thiserror::Error;

#[derive(Debug, Error)]
pub enum GoodSearchError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("External tool failed: {0}")]
    ExternalTool(String),
}

pub type Result<T> = std::result::Result<T, GoodSearchError>;
