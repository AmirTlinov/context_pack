use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::domain::{
    errors::{DomainError, Result},
    models::Pack,
    types::{LineRange, PackId, PackName, RelativePath, Status},
};

// ── Ports ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait PackRepositoryPort: Send + Sync {
    async fn create_new(&self, pack: &Pack) -> Result<()>;
    async fn save_with_expected_revision(&self, pack: &Pack, expected_revision: u64) -> Result<()>;
    async fn delete_pack_file(&self, id: &PackId) -> Result<bool>;
    async fn get_by_id(&self, id: &PackId) -> Result<Option<Pack>>;
    async fn get_by_name(&self, name: &PackName) -> Result<Option<Pack>>;
    async fn list_packs(&self, filter: ListFilter) -> Result<Vec<Pack>>;
    async fn purge_expired(&self) -> Result<()>;
}

#[async_trait]
pub trait CodeExcerptPort: Send + Sync {
    /// Safely read bounded lines from a repo-relative path.
    async fn read_lines(&self, path: &RelativePath, range: LineRange) -> Result<Snippet>;
}

// ── Transfer objects ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub status: Option<Status>,
    pub freshness: Option<FreshnessState>,
    pub query: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessState {
    #[default]
    Fresh,
    ExpiringSoon,
    Expired,
}

impl FreshnessState {
    pub const EXPIRING_SOON_THRESHOLD_SECONDS: i64 = 15 * 60;

    pub fn from_ttl_seconds(ttl_remaining_seconds: i64) -> Self {
        if ttl_remaining_seconds <= 0 {
            Self::Expired
        } else if ttl_remaining_seconds <= Self::EXPIRING_SOON_THRESHOLD_SECONDS {
            Self::ExpiringSoon
        } else {
            Self::Fresh
        }
    }

    pub fn from_pack(pack: &Pack, now: DateTime<Utc>) -> Self {
        Self::from_ttl_seconds(pack.ttl_remaining_seconds(now))
    }

    pub fn warning_text(self) -> Option<&'static str> {
        match self {
            Self::Fresh => None,
            Self::ExpiringSoon => Some("expiring soon — refresh or extend ttl"),
            Self::Expired => Some("expired — treat as stale evidence"),
        }
    }
}

impl fmt::Display for FreshnessState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fresh => write!(f, "fresh"),
            Self::ExpiringSoon => write!(f, "expiring_soon"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

impl FromStr for FreshnessState {
    type Err = DomainError;

    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        match raw.trim() {
            "fresh" => Ok(Self::Fresh),
            "expiring_soon" => Ok(Self::ExpiringSoon),
            "expired" => Ok(Self::Expired),
            other => Err(DomainError::InvalidData(format!(
                "'freshness' must be one of: fresh, expiring_soon, expired (got '{}')",
                other
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FreshnessState;

    #[test]
    fn freshness_state_boundaries_are_stable() {
        assert_eq!(
            FreshnessState::from_ttl_seconds(3600),
            FreshnessState::Fresh
        );
        assert_eq!(
            FreshnessState::from_ttl_seconds(FreshnessState::EXPIRING_SOON_THRESHOLD_SECONDS),
            FreshnessState::ExpiringSoon
        );
        assert_eq!(
            FreshnessState::from_ttl_seconds(FreshnessState::EXPIRING_SOON_THRESHOLD_SECONDS - 1),
            FreshnessState::ExpiringSoon
        );
        assert_eq!(FreshnessState::from_ttl_seconds(0), FreshnessState::Expired);
        assert_eq!(
            FreshnessState::from_ttl_seconds(-1),
            FreshnessState::Expired
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    /// Numbered lines: "   5: fn foo() {"
    pub body: String,
    pub total_lines: usize,
}
