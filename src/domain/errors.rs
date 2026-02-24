use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const REVISION_CONFLICT_CHANGED_KEYS_LIMIT: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinalizeRefIssue {
    pub section_key: String,
    pub ref_key: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionConflictDiagnostics {
    pub expected_revision: u64,
    pub current_revision: u64,
    pub last_updated_at: String,
    pub changed_section_keys: Vec<String>,
    pub guidance: String,
}

pub fn revision_conflict_guidance(current_revision: u64) -> String {
    format!(
        "re-read latest pack via get, merge intent, retry with expected_revision={current_revision}"
    )
}

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

    #[error(
        "revision conflict: expected {expected_revision}, current {current_revision}; {guidance}"
    )]
    RevisionConflictDetailed {
        expected_revision: u64,
        current_revision: u64,
        last_updated_at: String,
        changed_section_keys: Vec<String>,
        guidance: String,
    },

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("finalize validation failed: {message}")]
    FinalizeValidation {
        message: String,
        missing_sections: Vec<String>,
        missing_fields: Vec<String>,
        invalid_refs: Vec<FinalizeRefIssue>,
    },

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
