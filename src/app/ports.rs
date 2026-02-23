use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::{
    errors::Result,
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
    pub query: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
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
