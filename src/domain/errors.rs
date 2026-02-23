use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid data: {0}")]
    InvalidData(String),

    #[error("ttl required: {0}")]
    TtlRequired(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("ambiguous: {message}")]
    Ambiguous {
        message: String,
        candidates: Vec<String>,
    },

    #[error("revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("stale ref: {0}")]
    StaleRef(String),

    #[error("{0}")]
    Io(String),

    #[error("failed to deserialize: {0}")]
    Deserialize(String),

    #[error("schema migration required: {0}")]
    MigrationRequired(String),

    #[error("pack id already exists: {0}")]
    PackIdConflict(String),
}

pub type Result<T> = std::result::Result<T, DomainError>;

impl From<std::io::Error> for DomainError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<serde_json::Error> for DomainError {
    fn from(err: serde_json::Error) -> Self {
        Self::Deserialize(err.to_string())
    }
}
