use async_trait::async_trait;
use chrono::Utc;
use fs2::FileExt;
use std::fs::OpenOptions;
use std::path::PathBuf;
use tokio::fs;
use tokio::task;

use crate::{
    app::ports::{ListFilter, PackRepositoryPort},
    domain::{
        errors::{DomainError, Result},
        models::Pack,
        types::{PackId, PackName},
    },
};

pub struct MarkdownStorageAdapter {
    pub(crate) storage_dir: PathBuf,
}

impl MarkdownStorageAdapter {
    pub fn new(storage_dir: PathBuf) -> Self {
        Self { storage_dir }
    }

    fn pack_path(&self, id: &PackId) -> PathBuf {
        self.storage_dir.join(format!("{}.md", id.as_str()))
    }

    fn pack_lock_path(&self, id: &PackId) -> PathBuf {
        self.storage_dir.join(format!("{}.lock", id.as_str()))
    }

    fn create_lock_path(&self) -> PathBuf {
        self.storage_dir.join(".create.lock")
    }

    /// Encode pack to YAML frontmatter format.
    fn encode(pack: &Pack) -> Result<String> {
        let yaml = serde_yaml::to_string(pack)?;
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str(&yaml);
        if !yaml.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("---\n");
        Ok(out)
    }

    /// Decode a YAML frontmatter file into a Pack, running schema migration.
    fn decode(content: &str) -> Result<Pack> {
        let after_first = content
            .strip_prefix("---\n")
            .ok_or_else(|| DomainError::Io("pack file must start with '---'".into()))?;
        let mut offset = 0usize;
        let mut end = None;
        for line in after_first.split_inclusive('\n') {
            if line.trim_end_matches(['\r', '\n']) == "---" {
                end = Some(offset);
                break;
            }
            offset += line.len();
        }
        let end =
            end.ok_or_else(|| DomainError::Io("pack file is missing closing '---'".into()))?;
        let yaml = &after_first[..end];
        if yaml.trim().is_empty() {
            return Err(DomainError::Io("pack file has empty frontmatter".into()));
        }
        let pack: Pack = serde_yaml::from_str(yaml)?;
        pack.migrate_schema()
    }

    fn decode_with_path(path: &std::path::Path, content: &str) -> Result<Pack> {
        match Self::decode(content) {
            Ok(pack) => Ok(pack),
            Err(DomainError::MigrationRequired(msg)) => Err(DomainError::MigrationRequired(
                format!("{} [path={}]", msg, path.display()),
            )),
            Err(err) => Err(DomainError::Io(format!(
                "failed to decode pack '{}': {}",
                path.display(),
                err
            ))),
        }
    }

    async fn ensure_dir(&self) -> Result<()> {
        if fs::metadata(&self.storage_dir).await.is_err() {
            fs::create_dir_all(&self.storage_dir)
                .await
                .map_err(|e| DomainError::Io(format!("failed to create storage dir: {}", e)))?;
        }
        Ok(())
    }

    async fn load_all_active(&self) -> Result<Vec<Pack>> {
        if fs::metadata(&self.storage_dir).await.is_err() {
            return Ok(Vec::new());
        }
        let now = Utc::now();
        let mut packs = Vec::new();
        let mut entries = fs::read_dir(&self.storage_dir)
            .await
            .map_err(|e| DomainError::Io(format!("failed to read storage dir: {}", e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| DomainError::Io(format!("dir entry error: {}", e)))?
        {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let content = fs::read_to_string(&path)
                .await
                .map_err(|e| DomainError::Io(format!("failed to read pack file: {}", e)))?;
            let pack = Self::decode_with_path(&path, &content)?;
            if !pack.is_expired(now) {
                packs.push(pack);
            }
        }

        packs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(packs)
    }

    async fn purge_expired(&self) -> Result<()> {
        if fs::metadata(&self.storage_dir).await.is_err() {
            return Ok(());
        }

        let now = Utc::now();
        let mut entries = fs::read_dir(&self.storage_dir)
            .await
            .map_err(|e| DomainError::Io(format!("failed to read storage dir: {}", e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| DomainError::Io(format!("dir entry error: {}", e)))?
        {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let content = fs::read_to_string(&path)
                .await
                .map_err(|e| DomainError::Io(format!("failed to read pack file: {}", e)))?;
            let pack = Self::decode_with_path(&path, &content)?;
            if pack.is_expired(now) {
                fs::remove_file(&path).await.map_err(|e| {
                    DomainError::Io(format!(
                        "failed to remove expired pack '{}': {}",
                        path.display(),
                        e
                    ))
                })?;
            }
        }
        Ok(())
    }

    async fn write_pack_with_revision_guard(
        &self,
        pack: &Pack,
        expected_revision: u64,
    ) -> Result<()> {
        self.ensure_dir().await?;
        let storage_dir = self.storage_dir.clone();
        let pack = pack.clone();
        let lock_path = self.pack_lock_path(&pack.id);

        task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(&storage_dir)
                .map_err(|e| DomainError::Io(format!("failed to create storage dir: {}", e)))?;

            let lock_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(|e| {
                    DomainError::Io(format!(
                        "failed to open pack lock '{}': {}",
                        lock_path.display(),
                        e
                    ))
                })?;
            lock_file.lock_exclusive().map_err(|e| {
                DomainError::Io(format!("failed to lock pack '{}': {}", pack.id, e))
            })?;

            let path = storage_dir.join(format!("{}.md", pack.id.as_str()));
            let current_raw = std::fs::read_to_string(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    DomainError::NotFound(format!("pack '{}' not found", pack.id))
                } else {
                    DomainError::Io(format!(
                        "failed to read current pack '{}': {}",
                        path.display(),
                        e
                    ))
                }
            })?;

            let current_pack = MarkdownStorageAdapter::decode(&current_raw)?;
            if current_pack.revision != expected_revision {
                return Err(DomainError::RevisionConflict {
                    expected: expected_revision,
                    actual: current_pack.revision,
                });
            }

            let tmp_path = storage_dir.join(format!("{}.tmp", pack.id.as_str()));
            let content = MarkdownStorageAdapter::encode(&pack)?;
            std::fs::write(&tmp_path, &content)
                .map_err(|e| DomainError::Io(format!("failed to write tmp pack: {}", e)))?;
            std::fs::rename(&tmp_path, &path)
                .map_err(|e| DomainError::Io(format!("failed to rename pack file: {}", e)))?;

            lock_file.unlock().map_err(|e| {
                DomainError::Io(format!("failed to unlock pack '{}': {}", pack.id, e))
            })?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))??;

        Ok(())
    }
}

#[async_trait]
impl PackRepositoryPort for MarkdownStorageAdapter {
    async fn create_new(&self, pack: &Pack) -> Result<()> {
        self.purge_expired().await?;
        self.ensure_dir().await?;

        let storage_dir = self.storage_dir.clone();
        let create_lock_path = self.create_lock_path();
        let pack = pack.clone();

        task::spawn_blocking(move || -> Result<()> {
            std::fs::create_dir_all(&storage_dir)
                .map_err(|e| DomainError::Io(format!("failed to create storage dir: {}", e)))?;

            let lock_file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&create_lock_path)
                .map_err(|e| {
                    DomainError::Io(format!(
                        "failed to open create lock '{}': {}",
                        create_lock_path.display(),
                        e
                    ))
                })?;
            lock_file.lock_exclusive().map_err(|e| {
                DomainError::Io(format!(
                    "failed to lock create mutex '{}': {}",
                    create_lock_path.display(),
                    e
                ))
            })?;

            let path = storage_dir.join(format!("{}.md", pack.id.as_str()));
            if path.exists() {
                lock_file.unlock().ok();
                return Err(DomainError::Conflict(format!(
                    "pack id '{}' already exists",
                    pack.id
                )));
            }

            if let Some(new_name) = &pack.name {
                let now = Utc::now();
                for entry in std::fs::read_dir(&storage_dir).map_err(|e| {
                    DomainError::Io(format!(
                        "failed to read storage dir '{}': {}",
                        storage_dir.display(),
                        e
                    ))
                })? {
                    let entry =
                        entry.map_err(|e| DomainError::Io(format!("dir entry error: {}", e)))?;
                    let path = entry.path();
                    if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                        continue;
                    }
                    let raw = std::fs::read_to_string(&path).map_err(|e| {
                        DomainError::Io(format!(
                            "failed to read pack file '{}': {}",
                            path.display(),
                            e
                        ))
                    })?;
                    let existing = MarkdownStorageAdapter::decode_with_path(&path, &raw)?;
                    if existing.is_expired(now) {
                        std::fs::remove_file(&path).map_err(|e| {
                            DomainError::Io(format!(
                                "failed to remove expired pack '{}': {}",
                                path.display(),
                                e
                            ))
                        })?;
                        continue;
                    }
                    if existing.name.as_ref() == Some(new_name) {
                        lock_file.unlock().ok();
                        return Err(DomainError::Conflict(format!(
                            "pack with name '{}' already exists",
                            new_name
                        )));
                    }
                }
            }

            let tmp_path = storage_dir.join(format!("{}.tmp", pack.id.as_str()));
            let content = MarkdownStorageAdapter::encode(&pack)?;
            std::fs::write(&tmp_path, &content)
                .map_err(|e| DomainError::Io(format!("failed to write tmp pack: {}", e)))?;
            std::fs::rename(&tmp_path, &path)
                .map_err(|e| DomainError::Io(format!("failed to rename pack file: {}", e)))?;

            lock_file.unlock().map_err(|e| {
                DomainError::Io(format!(
                    "failed to unlock create mutex '{}': {}",
                    create_lock_path.display(),
                    e
                ))
            })?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))??;

        Ok(())
    }

    async fn save_with_expected_revision(&self, pack: &Pack, expected_revision: u64) -> Result<()> {
        self.purge_expired().await?;
        self.write_pack_with_revision_guard(pack, expected_revision)
            .await
    }

    async fn get_by_id(&self, id: &PackId) -> Result<Option<Pack>> {
        self.purge_expired().await?;
        let path = self.pack_path(id);
        match fs::metadata(&path).await {
            Ok(meta) => {
                if !meta.is_file() {
                    return Err(DomainError::Io(format!(
                        "pack path '{}' is not a file",
                        path.display()
                    )));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(DomainError::Io(format!(
                    "failed to stat pack file '{}': {}",
                    path.display(),
                    e
                )));
            }
        }
        let content = fs::read_to_string(&path)
            .await
            .map_err(|e| DomainError::Io(format!("failed to read pack file: {}", e)))?;
        let pack = Self::decode_with_path(&path, &content)?;
        if pack.is_expired(Utc::now()) {
            fs::remove_file(&path).await.map_err(|e| {
                DomainError::Io(format!(
                    "failed to remove expired pack '{}': {}",
                    path.display(),
                    e
                ))
            })?;
            return Ok(None);
        }
        Ok(Some(pack))
    }

    async fn get_by_name(&self, name: &PackName) -> Result<Option<Pack>> {
        self.purge_expired().await?;
        let packs = self.load_all_active().await?;
        let mut matches = packs
            .into_iter()
            .filter(|p| p.name.as_ref() == Some(name))
            .collect::<Vec<_>>();
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.pop()),
            _ => Err(DomainError::Conflict(format!(
                "multiple packs found for name '{}'",
                name
            ))),
        }
    }

    async fn list_packs(&self, filter: ListFilter) -> Result<Vec<Pack>> {
        self.purge_expired().await?;
        let packs = self.load_all_active().await?;

        let filtered: Vec<Pack> = packs
            .into_iter()
            .filter(|p| {
                if let Some(s) = filter.status {
                    if p.status != s {
                        return false;
                    }
                }
                if let Some(ref q) = filter.query {
                    let q_lower = q.trim().to_lowercase();
                    if q_lower.is_empty() {
                        return true;
                    }
                    let haystack = format!(
                        "{} {} {}",
                        p.title.as_deref().unwrap_or(""),
                        p.name.as_ref().map(|n| n.as_str()).unwrap_or(""),
                        p.brief.as_deref().unwrap_or(""),
                    )
                    .to_lowercase();
                    if !haystack.contains(&q_lower) {
                        return false;
                    }
                }
                true
            })
            .collect();

        let offset = filter.offset.unwrap_or(0);
        let paged: Vec<Pack> = filtered
            .into_iter()
            .skip(offset)
            .take(filter.limit.unwrap_or(usize::MAX))
            .collect();

        Ok(paged)
    }
}

// ── unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{PackId, PackName};

    fn make_pack() -> Pack {
        Pack::new(PackId::new(), Some(PackName::new("test-pack").unwrap()))
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let pack = make_pack();
        let encoded = MarkdownStorageAdapter::encode(&pack).unwrap();
        assert!(encoded.starts_with("---\n"), "must start with YAML fence");
        let decoded = MarkdownStorageAdapter::decode(&encoded).unwrap();
        assert_eq!(decoded.id, pack.id);
        assert_eq!(decoded.name, pack.name);
        assert_eq!(decoded.schema_version, pack.schema_version);
    }

    #[test]
    fn test_decode_rejects_malformed() {
        assert!(MarkdownStorageAdapter::decode("not yaml at all").is_err());
        assert!(
            MarkdownStorageAdapter::decode("---\n---\n").is_err(),
            "empty frontmatter"
        );
    }
}
