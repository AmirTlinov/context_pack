use std::sync::Arc;

use crate::{
    app::{
        ports::{CodeExcerptPort, FreshnessState, ListFilter, PackRepositoryPort},
        resolver::resolve_pack,
    },
    domain::{
        errors::{DomainError, Result},
        models::{Pack, RefSpec},
        types::{
            DiagramKey, LineRange, PackId, PackName, RefKey, RelativePath, SectionKey, Status,
        },
    },
};

pub struct InputUseCases {
    repo: Arc<dyn PackRepositoryPort>,
    excerpt: Arc<dyn CodeExcerptPort>,
}

pub struct UpsertRefRequest {
    pub section_key: String,
    pub ref_key: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub title: Option<String>,
    pub why: Option<String>,
    pub group: Option<String>,
}

pub struct UpsertDiagramRequest {
    pub section_key: String,
    pub diagram_key: String,
    pub title: String,
    pub mermaid: String,
    pub why: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum TouchTtlMode {
    SetMinutes(u64),
    ExtendMinutes(u64),
}

impl InputUseCases {
    pub fn new(repo: Arc<dyn PackRepositoryPort>, excerpt: Arc<dyn CodeExcerptPort>) -> Self {
        Self { repo, excerpt }
    }

    // ── identity resolution ───────────────────────────────────────────────────

    async fn resolve(&self, identifier: &str) -> Result<Pack> {
        resolve_pack(self.repo.as_ref(), identifier).await
    }

    async fn resolve_for_update(&self, identifier: &str, expected_revision: u64) -> Result<Pack> {
        let pack = self.resolve(identifier).await?;
        if pack.revision != expected_revision {
            return Err(DomainError::RevisionConflict {
                expected: expected_revision,
                actual: pack.revision,
            });
        }
        Ok(pack)
    }

    async fn validate_refs_resolvable_before_finalize(&self, pack: &Pack) -> Result<()> {
        let mut stale = Vec::new();
        for section in &pack.sections {
            for code_ref in &section.refs {
                match self
                    .excerpt
                    .read_lines(&code_ref.path, code_ref.lines)
                    .await
                {
                    Ok(_) => {}
                    Err(DomainError::StaleRef(msg)) => {
                        stale.push(format!(
                            "{}::{} ({}:{}-{}): {}",
                            section.key,
                            code_ref.key,
                            code_ref.path,
                            code_ref.lines.start,
                            code_ref.lines.end,
                            msg
                        ));
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        if stale.is_empty() {
            return Ok(());
        }

        let sample = stale
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        Err(DomainError::InvalidState(format!(
            "cannot finalize: stale refs detected ({} total): {}",
            stale.len(),
            sample
        )))
    }

    // ── queries ───────────────────────────────────────────────────────────────

    pub async fn list(
        &self,
        status: Option<Status>,
        query: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<Pack>> {
        self.list_with_freshness(status, query, limit, offset, None)
            .await
    }

    pub async fn list_with_freshness(
        &self,
        status: Option<Status>,
        query: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
        freshness: Option<FreshnessState>,
    ) -> Result<Vec<Pack>> {
        self.repo
            .list_packs(ListFilter {
                status,
                freshness,
                query,
                limit,
                offset,
            })
            .await
    }

    pub async fn get(&self, identifier: &str) -> Result<Pack> {
        self.resolve(identifier).await
    }

    pub async fn delete_pack_file(&self, identifier: &str) -> Result<bool> {
        let pack_id = PackId::parse(identifier)?;
        self.repo.delete_pack_file(&pack_id).await
    }

    // ── pack lifecycle ────────────────────────────────────────────────────────

    pub async fn create_with_tags_ttl(
        &self,
        name: Option<String>,
        title: Option<String>,
        brief: Option<String>,
        tags: Option<Vec<String>>,
        ttl_minutes: u64,
    ) -> Result<Pack> {
        let pack_name = name.as_deref().map(PackName::new).transpose()?;
        for _ in 0..8 {
            let mut pack = Pack::new(PackId::new(), pack_name.clone());
            pack.set_ttl_on_create(ttl_minutes, pack.created_at)?;
            if let Some(t) = &title {
                pack.title = Some(t.clone());
            }
            if let Some(b) = &brief {
                pack.brief = Some(b.clone());
            }
            if let Some(tg) = &tags {
                pack.tags = tg.clone();
            }

            match self.repo.create_new(&pack).await {
                Ok(()) => return Ok(pack),
                Err(DomainError::PackIdConflict(_)) => continue,
                Err(e) => return Err(e),
            }
        }

        Err(DomainError::Conflict(
            "failed to allocate unique pack id".into(),
        ))
    }

    pub async fn set_status_checked(
        &self,
        identifier: &str,
        status: Status,
        expected_revision: u64,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;

        if status == Status::Finalized {
            self.validate_refs_resolvable_before_finalize(&pack).await?;
        }

        pack.set_status(status)?;
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }

    pub async fn set_meta_checked(
        &self,
        identifier: &str,
        title: Option<String>,
        brief: Option<String>,
        tags: Option<Vec<String>>,
        expected_revision: u64,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;
        pack.set_meta(title, brief, tags)?;
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }

    // ── section management ────────────────────────────────────────────────────

    pub async fn upsert_section_checked(
        &self,
        identifier: &str,
        section_key: &str,
        title: String,
        description: Option<String>,
        order: Option<usize>,
        expected_revision: u64,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;
        let key = SectionKey::new(section_key)?;
        pack.upsert_section(key, title, description, order)?;
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }

    pub async fn delete_section_checked(
        &self,
        identifier: &str,
        section_key: &str,
        expected_revision: u64,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;
        let key = SectionKey::new(section_key)?;
        pack.delete_section(&key)?;
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }

    // ── ref management ────────────────────────────────────────────────────────

    pub async fn upsert_ref_checked(
        &self,
        identifier: &str,
        request: UpsertRefRequest,
        expected_revision: u64,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;
        let section_key = SectionKey::new(&request.section_key)?;
        pack.upsert_ref(
            &section_key,
            RefSpec {
                key: RefKey::new(&request.ref_key)?,
                path: RelativePath::new(&request.path)?,
                lines: LineRange::new(request.line_start, request.line_end)?,
                title: request.title,
                why: request.why,
                group: request.group,
            },
        )?;
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }

    pub async fn delete_ref_checked(
        &self,
        identifier: &str,
        section_key: &str,
        ref_key: &str,
        expected_revision: u64,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;
        pack.delete_ref(&SectionKey::new(section_key)?, &RefKey::new(ref_key)?)?;
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }

    // ── diagram management ────────────────────────────────────────────────────

    pub async fn upsert_diagram_checked(
        &self,
        identifier: &str,
        request: UpsertDiagramRequest,
        expected_revision: u64,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;
        pack.upsert_diagram(
            &SectionKey::new(&request.section_key)?,
            DiagramKey::new(&request.diagram_key)?,
            request.title,
            request.mermaid,
            request.why,
        )?;
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }

    pub async fn touch_ttl_checked(
        &self,
        identifier: &str,
        expected_revision: u64,
        mode: TouchTtlMode,
    ) -> Result<Pack> {
        let mut pack = self
            .resolve_for_update(identifier, expected_revision)
            .await?;
        match mode {
            TouchTtlMode::SetMinutes(minutes) => {
                pack.set_ttl_from_now(minutes, chrono::Utc::now())?;
            }
            TouchTtlMode::ExtendMinutes(minutes) => {
                pack.extend_ttl(minutes, chrono::Utc::now())?;
            }
        }
        self.repo
            .save_with_expected_revision(&pack, expected_revision)
            .await?;
        Ok(pack)
    }
}
