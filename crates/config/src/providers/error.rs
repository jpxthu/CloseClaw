//! ConfigError definition

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Schema validation failed: {0}")]
    SchemaError(String),

    #[error("Invalid value for field '{field}': {message}")]
    ValueError { field: String, message: String },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Missing required agent id in {path}")]
    MissingId { path: String },

    #[error("failed to parse credential file {path}: {error}")]
    ParseError {
        path: std::path::PathBuf,
        error: String,
    },

    #[error("credential validation failed for {path}: {message}")]
    ValidationError {
        path: std::path::PathBuf,
        message: String,
    },
}
