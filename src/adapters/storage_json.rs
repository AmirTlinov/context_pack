use async_trait::async_trait;
use chrono::Utc;
use fs2::FileExt;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tokio::task;

use crate::{
    app::ports::{ListFilter, PackRepositoryPort},
    domain::{
        errors::{DomainError, Result},
        models::Pack,
        types::{PackId, PackName},
    },
};

const DEFAULT_MAX_PACK_BYTES: usize = 512 * 1024;

/// Minimal pack metadata needed for TTL purge scanning.
/// Avoids deserializing full Pack (sections, refs, diagrams).
#[derive(serde::Deserialize)]
struct PackMeta {
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn parse_max_pack_bytes_from_env() -> usize {
    std::env::var("CONTEXT_PACK_MAX_PACK_BYTES")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_PACK_BYTES)
}

pub struct JsonStorageAdapter {
    pub(crate) storage_dir: PathBuf,
    max_pack_bytes: usize,
}

impl JsonStorageAdapter {
    pub fn new(storage_dir: PathBuf) -> Self {
        Self {
            storage_dir,
            max_pack_bytes: parse_max_pack_bytes_from_env(),
        }
    }

    fn repo_lock_path(storage_dir: &Path) -> PathBuf {
        storage_dir.join(".repo.lock")
    }

    fn pack_path(storage_dir: &Path, id: &PackId) -> PathBuf {
        storage_dir.join(format!("{}.json", id.as_str()))
    }

    fn ensure_dir_sync(storage_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(storage_dir)
            .map_err(|e| DomainError::Io(format!("failed to create storage dir: {}", e)))
    }

    fn encode(pack: &Pack) -> Result<String> {
        Ok(serde_json::to_string(pack)?)
    }

    #[cfg(test)]
    fn new_with_max(storage_dir: PathBuf, max_pack_bytes: usize) -> Self {
        Self {
            storage_dir,
            max_pack_bytes,
        }
    }

    fn payload_too_large_error(path: &str, actual: usize, max: usize) -> DomainError {
        DomainError::InvalidData(format!(
            "pack '{}' payload is too large: {} bytes (max {})",
            path, actual, max
        ))
    }

    fn encoded_pack_payload(pack: &Pack, max_pack_bytes: usize) -> Result<String> {
        let payload = Self::encode(pack)?;
        if payload.len() > max_pack_bytes {
            return Err(Self::payload_too_large_error(
                pack.id.as_str(),
                payload.len(),
                max_pack_bytes,
            ));
        }
        Ok(payload)
    }

    fn is_recoverable_pack_read_error(err: &DomainError) -> bool {
        matches!(
            err,
            DomainError::Io(_) | DomainError::Deserialize(_) | DomainError::InvalidData(_),
        )
    }

    fn remove_corrupt_pack_file(path: &Path, err: &DomainError, stage: &str) {
        tracing::warn!(
            "removing unreadable pack '{}' during {}: {}",
            path.display(),
            stage,
            err
        );
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(remove_err) => {
                tracing::warn!(
                    "failed to remove unreadable pack '{}': {}",
                    path.display(),
                    remove_err
                );
            }
        }
    }

    fn read_pack_for_lookup(path: &Path, max_pack_bytes: usize) -> Result<Option<Pack>> {
        match Self::read_pack_from_path(path, max_pack_bytes) {
            Ok(pack) => Ok(Some(pack)),
            Err(err) if Self::is_recoverable_pack_read_error(&err) => {
                Self::remove_corrupt_pack_file(path, &err, "read");
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    fn decode(content: &str) -> Result<Pack> {
        let pack: Pack = serde_json::from_str(content)?;
        pack.migrate_schema()
    }

    fn decode_with_path(path: &Path, content: &str) -> Result<Pack> {
        match Self::decode(content) {
            Ok(pack) => Ok(pack),
            Err(DomainError::MigrationRequired(msg)) => Err(DomainError::MigrationRequired(
                format!("{} [path={}]", msg, path.display()),
            )),
            Err(err @ DomainError::InvalidData(_)) => Err(err),
            Err(err) => Err(DomainError::Deserialize(format!(
                "failed to decode pack '{}': {}",
                path.display(),
                err
            ))),
        }
    }

    fn list_pack_paths_sync(storage_dir: &Path) -> Result<Vec<PathBuf>> {
        if !storage_dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(storage_dir)
            .map_err(|e| DomainError::Io(format!("failed to read storage dir: {}", e)))?
        {
            let entry = entry.map_err(|e| DomainError::Io(format!("dir entry error: {}", e)))?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|v| v.to_str()) == Some("json") {
                out.push(path);
            }
        }
        Ok(out)
    }

    fn read_pack_from_path(path: &Path, max_pack_bytes: usize) -> Result<Pack> {
        let meta = std::fs::metadata(path).map_err(|e| {
            DomainError::Io(format!(
                "failed to stat pack file '{}': {}",
                path.display(),
                e
            ))
        })?;
        if !meta.is_file() {
            return Err(DomainError::Io(format!(
                "pack path '{}' is not a file",
                path.display()
            )));
        }
        if usize::try_from(meta.len()).unwrap_or(usize::MAX) > max_pack_bytes {
            return Err(DomainError::Io(format!(
                "pack file '{}' is too large: {} bytes (max {})",
                path.display(),
                meta.len(),
                max_pack_bytes
            )));
        }
        let raw = std::fs::read_to_string(path).map_err(|e| {
            DomainError::Io(format!(
                "failed to read pack file '{}': {}",
                path.display(),
                e
            ))
        })?;
        Self::decode_with_path(path, &raw)
    }

    fn write_pack_atomic(storage_dir: &Path, pack: &Pack, max_pack_bytes: usize) -> Result<()> {
        let path = Self::pack_path(storage_dir, &pack.id);
        let tmp = storage_dir.join(format!("{}.tmp", pack.id.as_str()));
        let content = Self::encoded_pack_payload(pack, max_pack_bytes)?;
        std::fs::write(&tmp, content)
            .map_err(|e| DomainError::Io(format!("failed to write tmp pack: {}", e)))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| DomainError::Io(format!("failed to rename pack file: {}", e)))?;
        Ok(())
    }

    fn read_pack_meta_from_path(path: &Path, max_pack_bytes: usize) -> Option<PackMeta> {
        let file_len = usize::try_from(std::fs::metadata(path).ok()?.len()).unwrap_or(usize::MAX);
        if file_len > max_pack_bytes {
            Self::remove_corrupt_pack_file(
                path,
                &DomainError::Io(format!(
                    "pack file '{}' is too large: {} bytes (max {})",
                    path.display(),
                    file_len,
                    max_pack_bytes
                )),
                "purge",
            );
            return None;
        }
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) => {
                Self::remove_corrupt_pack_file(
                    path,
                    &DomainError::Io(format!(
                        "failed to read pack file '{}': {}",
                        path.display(),
                        err
                    )),
                    "purge",
                );
                return None;
            }
        };
        serde_json::from_str::<PackMeta>(&raw).ok().or_else(|| {
            Self::remove_corrupt_pack_file(
                path,
                &DomainError::InvalidData(format!(
                    "failed to parse pack metadata from '{}'",
                    path.display()
                )),
                "purge",
            );
            None
        })
    }

    fn purge_expired_sync(storage_dir: &Path, max_pack_bytes: usize) -> Result<()> {
        let now = Utc::now();
        let paths = Self::list_pack_paths_sync(storage_dir)?;
        for path in paths {
            let meta = Self::read_pack_meta_from_path(&path, max_pack_bytes);
            let is_expired = meta
                .and_then(|m| m.expires_at)
                .map(|t| t <= now)
                .unwrap_or(false);
            if is_expired {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(DomainError::Io(format!(
                            "failed to remove expired pack '{}': {}",
                            path.display(),
                            e
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn delete_pack_file_sync(storage_dir: &Path, id: &PackId) -> Result<bool> {
        let path = Self::pack_path(storage_dir, id);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
            Err(e) => Err(DomainError::Io(format!(
                "failed to delete pack file: {}",
                e
            ))),
        }
    }

    fn load_all_active_sync(storage_dir: &Path, max_pack_bytes: usize) -> Result<Vec<Pack>> {
        let now = Utc::now();
        let mut packs = Vec::new();
        for path in Self::list_pack_paths_sync(storage_dir)? {
            let pack = match Self::read_pack_for_lookup(&path, max_pack_bytes)? {
                Some(pack) => pack,
                None => continue,
            };
            if !pack.is_expired(now) {
                packs.push(pack);
            }
        }
        packs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(packs)
    }

    async fn purge_expired_locked(&self) -> Result<()> {
        let storage_dir = self.storage_dir.clone();
        let max_pack_bytes = self.max_pack_bytes;
        task::spawn_blocking(move || -> Result<()> {
            Self::ensure_dir_sync(&storage_dir)?;
            let lock_path = Self::repo_lock_path(&storage_dir);
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(|e| {
                    DomainError::Io(format!(
                        "failed to open repo lock '{}': {}",
                        lock_path.display(),
                        e
                    ))
                })?;
            lock.lock_exclusive()
                .map_err(|e| DomainError::Io(format!("failed to lock repo: {}", e)))?;
            Self::purge_expired_sync(&storage_dir, max_pack_bytes)?;
            lock.unlock()
                .map_err(|e| DomainError::Io(format!("failed to unlock repo: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))??;
        Ok(())
    }
}

#[async_trait]
impl PackRepositoryPort for JsonStorageAdapter {
    async fn create_new(&self, pack: &Pack) -> Result<()> {
        let storage_dir = self.storage_dir.clone();
        let max_pack_bytes = self.max_pack_bytes;
        let pack = pack.clone();
        task::spawn_blocking(move || -> Result<()> {
            Self::ensure_dir_sync(&storage_dir)?;
            let lock_path = Self::repo_lock_path(&storage_dir);
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(|e| {
                    DomainError::Io(format!(
                        "failed to open repo lock '{}': {}",
                        lock_path.display(),
                        e
                    ))
                })?;
            lock.lock_exclusive()
                .map_err(|e| DomainError::Io(format!("failed to lock repo: {}", e)))?;
            Self::purge_expired_sync(&storage_dir, max_pack_bytes)?;

            let path = Self::pack_path(&storage_dir, &pack.id);
            if path.exists() {
                if let Err(e) = lock.unlock() {
                    tracing::warn!("failed to unlock repo lock: {e}");
                }
                return Err(DomainError::PackIdConflict(pack.id.to_string()));
            }

            if let Some(new_name) = &pack.name {
                for existing_path in Self::list_pack_paths_sync(&storage_dir)? {
                    let existing = match Self::read_pack_for_lookup(&existing_path, max_pack_bytes)?
                    {
                        Some(existing) => existing,
                        None => continue,
                    };
                    if existing.name.as_ref() == Some(new_name) {
                        if let Err(e) = lock.unlock() {
                            tracing::warn!("failed to unlock repo lock: {e}");
                        }
                        return Err(DomainError::Conflict(format!(
                            "pack with name '{}' already exists",
                            new_name
                        )));
                    }
                }
            }

            Self::write_pack_atomic(&storage_dir, &pack, max_pack_bytes)?;
            lock.unlock()
                .map_err(|e| DomainError::Io(format!("failed to unlock repo: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))??;
        Ok(())
    }

    async fn save_with_expected_revision(&self, pack: &Pack, expected_revision: u64) -> Result<()> {
        let storage_dir = self.storage_dir.clone();
        let max_pack_bytes = self.max_pack_bytes;
        let pack = pack.clone();
        task::spawn_blocking(move || -> Result<()> {
            Self::ensure_dir_sync(&storage_dir)?;
            let lock_path = Self::repo_lock_path(&storage_dir);
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(|e| {
                    DomainError::Io(format!(
                        "failed to open repo lock '{}': {}",
                        lock_path.display(),
                        e
                    ))
                })?;
            lock.lock_exclusive()
                .map_err(|e| DomainError::Io(format!("failed to lock repo: {}", e)))?;
            Self::purge_expired_sync(&storage_dir, max_pack_bytes)?;

            let path = Self::pack_path(&storage_dir, &pack.id);
            if !path.exists() {
                if let Err(e) = lock.unlock() {
                    tracing::warn!("failed to unlock repo lock: {e}");
                }
                return Err(DomainError::NotFound(format!(
                    "pack '{}' not found",
                    pack.id
                )));
            }
            let current = Self::read_pack_for_lookup(&path, max_pack_bytes)?
                .ok_or_else(|| DomainError::NotFound(format!("pack '{}' not found", pack.id)))?;
            if current.revision != expected_revision {
                if let Err(e) = lock.unlock() {
                    tracing::warn!("failed to unlock repo lock: {e}");
                }
                return Err(DomainError::RevisionConflict {
                    expected: expected_revision,
                    actual: current.revision,
                });
            }

            Self::write_pack_atomic(&storage_dir, &pack, max_pack_bytes)?;
            lock.unlock()
                .map_err(|e| DomainError::Io(format!("failed to unlock repo: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))??;
        Ok(())
    }

    async fn delete_pack_file(&self, id: &PackId) -> Result<bool> {
        let storage_dir = self.storage_dir.clone();
        let id = id.clone();
        let removed = task::spawn_blocking(move || -> Result<bool> {
            Self::ensure_dir_sync(&storage_dir)?;
            let lock_path = Self::repo_lock_path(&storage_dir);
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
                .map_err(|e| {
                    DomainError::Io(format!(
                        "failed to open repo lock '{}': {}",
                        lock_path.display(),
                        e
                    ))
                })?;
            lock.lock_exclusive()
                .map_err(|e| DomainError::Io(format!("failed to lock repo: {}", e)))?;
            let removed = Self::delete_pack_file_sync(&storage_dir, &id);
            if let Err(err) = lock.unlock() {
                tracing::warn!("failed to unlock repo lock: {err}");
            }
            removed
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))??;
        Ok(removed)
    }

    async fn get_by_id(&self, id: &PackId) -> Result<Option<Pack>> {
        let storage_dir = self.storage_dir.clone();
        let id = id.clone();
        let max_pack_bytes = self.max_pack_bytes;
        task::spawn_blocking(move || -> Result<Option<Pack>> {
            let path = Self::pack_path(&storage_dir, &id);
            if !path.exists() {
                return Ok(None);
            }
            let pack = match Self::read_pack_for_lookup(&path, max_pack_bytes)? {
                Some(pack) => pack,
                None => return Ok(None),
            };
            if pack.is_expired(Utc::now()) {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(DomainError::Io(format!(
                            "failed to remove expired pack '{}': {}",
                            path.display(),
                            e
                        )));
                    }
                }
                return Ok(None);
            }
            Ok(Some(pack))
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))?
    }

    async fn get_by_name(&self, name: &PackName) -> Result<Option<Pack>> {
        let storage_dir = self.storage_dir.clone();
        let max_pack_bytes = self.max_pack_bytes;
        let name = name.clone();
        task::spawn_blocking(move || -> Result<Option<Pack>> {
            let mut matches = Self::load_all_active_sync(&storage_dir, max_pack_bytes)?
                .into_iter()
                .filter(|pack| pack.name.as_ref() == Some(&name))
                .collect::<Vec<_>>();
            match matches.len() {
                0 => Ok(None),
                1 => Ok(matches.pop()),
                _ => Err(DomainError::Ambiguous(format!(
                    "multiple packs found for name '{}'",
                    name
                ))),
            }
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))?
    }

    async fn list_packs(&self, filter: ListFilter) -> Result<Vec<Pack>> {
        let storage_dir = self.storage_dir.clone();
        let max_pack_bytes = self.max_pack_bytes;
        task::spawn_blocking(move || -> Result<Vec<Pack>> {
            let packs = Self::load_all_active_sync(&storage_dir, max_pack_bytes)?;
            let filtered: Vec<Pack> = packs
                .into_iter()
                .filter(|pack| {
                    if let Some(s) = filter.status {
                        if pack.status != s {
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
                            pack.title.as_deref().unwrap_or(""),
                            pack.name.as_ref().map(|n| n.as_str()).unwrap_or(""),
                            pack.brief.as_deref().unwrap_or("")
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
            Ok(filtered
                .into_iter()
                .skip(offset)
                .take(filter.limit.unwrap_or(usize::MAX))
                .collect())
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))?
    }

    async fn purge_expired(&self) -> Result<()> {
        self.purge_expired_locked().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{PackId, PackName};
    use chrono::Duration;
    use std::path::Path;
    use tempfile::tempdir;

    fn make_pack() -> Pack {
        Pack::new(PackId::new(), Some(PackName::new("test-pack").unwrap()))
    }

    fn make_expired_pack() -> Pack {
        let mut pack = Pack::new(PackId::new(), None);
        pack.expires_at = Utc::now() - Duration::seconds(1);
        pack
    }

    fn oversized_pack_by_growth(target_max: usize) -> Pack {
        let mut pack = make_pack();
        let mut payload_len = 0usize;
        let mut repeat = 1usize;
        while payload_len <= target_max {
            let size = target_max.saturating_mul(repeat.max(1));
            pack.brief = Some("x".repeat(size));
            payload_len = JsonStorageAdapter::encode(&pack).unwrap().len();
            repeat += 1;
        }
        pack
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let pack = make_pack();
        let encoded = JsonStorageAdapter::encode(&pack).unwrap();
        let decoded = JsonStorageAdapter::decode(&encoded).unwrap();
        assert_eq!(decoded.id, pack.id);
        assert_eq!(decoded.name, pack.name);
        assert_eq!(decoded.schema_version, pack.schema_version);
    }

    #[tokio::test]
    async fn test_create_new_rejects_oversized_payload_with_validation_error() {
        let dir = tempdir().unwrap();
        let adapter = JsonStorageAdapter::new_with_max(dir.path().to_path_buf(), 128);
        let pack = oversized_pack_by_growth(128);
        let err = adapter.create_new(&pack).await.unwrap_err();
        assert!(
            matches!(err, DomainError::InvalidData(_)),
            "expected validation error, got: {:?}",
            err
        );
        assert!(
            err.to_string().contains("payload is too large"),
            "expected too-large payload message, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_save_with_expected_revision_rejects_oversized_payload_with_validation_error() {
        let dir = tempdir().unwrap();
        let base_pack = make_pack();
        let create_adapter = JsonStorageAdapter::new_with_max(dir.path().to_path_buf(), 2048);
        create_adapter.create_new(&base_pack).await.unwrap();

        let mut update_pack = base_pack.clone();
        update_pack.brief = Some("x".repeat(4096));
        assert!(
            !update_pack.brief.as_ref().unwrap().is_empty(),
            "sanity: updated pack should have large payload"
        );

        let save_adapter = JsonStorageAdapter::new_with_max(dir.path().to_path_buf(), 1024);
        let err = save_adapter
            .save_with_expected_revision(&update_pack, update_pack.revision)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::InvalidData(_)),
            "expected validation error, got: {:?}",
            err
        );
        assert!(
            err.to_string().contains("payload is too large"),
            "expected too-large payload message, got: {err}"
        );
    }

    #[test]
    fn test_decode_rejects_malformed() {
        assert!(JsonStorageAdapter::decode("not-json").is_err());
        assert!(JsonStorageAdapter::decode("{}").is_err());
    }

    #[test]
    fn test_purge_expired_removes_only_expired() {
        let dir = tempdir().unwrap();
        let active = make_pack();
        let expired = make_expired_pack();

        // Write both packs
        JsonStorageAdapter::write_pack_atomic(dir.path(), &active, DEFAULT_MAX_PACK_BYTES).unwrap();
        JsonStorageAdapter::write_pack_atomic(dir.path(), &expired, DEFAULT_MAX_PACK_BYTES)
            .unwrap();

        // Both files should exist
        assert!(dir
            .path()
            .join(format!("{}.json", active.id.as_str()))
            .exists());
        assert!(dir
            .path()
            .join(format!("{}.json", expired.id.as_str()))
            .exists());

        // Run purge
        JsonStorageAdapter::purge_expired_sync(dir.path(), DEFAULT_MAX_PACK_BYTES).unwrap();

        // Active pack should remain, expired should be gone
        assert!(
            dir.path()
                .join(format!("{}.json", active.id.as_str()))
                .exists(),
            "active pack should still exist"
        );
        assert!(
            !dir.path()
                .join(format!("{}.json", expired.id.as_str()))
                .exists(),
            "expired pack should have been removed"
        );
    }

    #[test]
    fn test_purge_expired_removes_unreadable_and_oversized_pack_files() {
        let dir = tempdir().unwrap();
        let max = 1024usize;

        let active = make_pack();
        JsonStorageAdapter::write_pack_atomic(dir.path(), &active, max).unwrap();
        let active_path = dir.path().join(format!("{}.json", active.id.as_str()));

        let corrupted_path = dir.path().join("pk_corrupt.json");
        std::fs::write(&corrupted_path, "not-json").unwrap();

        let oversized = oversized_pack_by_growth(max);
        let oversized_path = dir.path().join(format!("{}.json", oversized.id.as_str()));
        let oversized_payload = JsonStorageAdapter::encode(&oversized).unwrap();
        assert!(
            oversized_payload.len() > max,
            "oversized payload must exceed max"
        );
        std::fs::write(&oversized_path, oversized_payload).unwrap();

        assert!(
            active_path.exists(),
            "active pack should exist before purge"
        );
        assert!(
            corrupted_path.exists(),
            "corrupt pack should exist before purge"
        );
        assert!(
            oversized_path.exists(),
            "oversized pack should exist before purge"
        );

        JsonStorageAdapter::purge_expired_sync(dir.path(), max).unwrap();

        assert!(active_path.exists(), "active pack should remain");
        assert!(
            !corrupted_path.exists(),
            "corrupt pack should be removed by purge"
        );
        assert!(
            !oversized_path.exists(),
            "oversized pack should be removed by purge"
        );
    }

    #[test]
    fn test_load_all_active_excludes_expired() {
        let dir = tempdir().unwrap();
        let active = make_pack();
        let expired = make_expired_pack();

        JsonStorageAdapter::write_pack_atomic(dir.path(), &active, DEFAULT_MAX_PACK_BYTES).unwrap();
        JsonStorageAdapter::write_pack_atomic(dir.path(), &expired, DEFAULT_MAX_PACK_BYTES)
            .unwrap();

        let loaded =
            JsonStorageAdapter::load_all_active_sync(dir.path(), DEFAULT_MAX_PACK_BYTES).unwrap();

        assert_eq!(loaded.len(), 1, "only 1 active pack expected");
        assert_eq!(
            loaded[0].id, active.id,
            "loaded pack should be the active one"
        );
    }

    #[test]
    fn test_list_pack_paths_nonexistent_dir() {
        let result =
            JsonStorageAdapter::list_pack_paths_sync(Path::new("/nonexistent/path/xyz/abc_12345"));
        assert!(result.is_ok(), "should not error on missing dir");
        assert!(result.unwrap().is_empty(), "should return empty vec");
    }

    #[tokio::test]
    async fn test_delete_pack_file_removes_target_without_reading_payload() {
        let dir = tempdir().unwrap();
        let storage =
            JsonStorageAdapter::new_with_max(dir.path().to_path_buf(), DEFAULT_MAX_PACK_BYTES);
        let valid = make_pack();
        let bad_id = PackId::new();
        let bad_path = dir.path().join(format!("{}.json", bad_id.as_str()));

        JsonStorageAdapter::write_pack_atomic(dir.path(), &valid, DEFAULT_MAX_PACK_BYTES).unwrap();
        std::fs::write(&bad_path, "not-json").unwrap();

        assert!(
            storage
                .delete_pack_file(&bad_id)
                .await
                .expect("delete operation should succeed"),
            "delete should report removed for existing file"
        );
        assert!(
            !bad_path.exists(),
            "bad file should be removed by deterministic deletion path"
        );
        assert!(
            std::fs::metadata(dir.path().join(format!("{}.json", valid.id.as_str()))).is_ok(),
            "valid file should remain"
        );
        assert!(
            !storage
                .delete_pack_file(&bad_id)
                .await
                .expect("delete operation should not fail on missing file"),
            "delete should return false when file is already gone"
        );
    }

    #[test]
    fn test_write_pack_atomic_persists_and_is_decodable() {
        let dir = tempdir().unwrap();
        let pack = make_pack();

        JsonStorageAdapter::write_pack_atomic(dir.path(), &pack, DEFAULT_MAX_PACK_BYTES).unwrap();

        let expected_path = dir.path().join(format!("{}.json", pack.id.as_str()));
        assert!(expected_path.exists(), "pack file should exist after write");

        // Should be decodable
        let decoded =
            JsonStorageAdapter::read_pack_from_path(&expected_path, DEFAULT_MAX_PACK_BYTES)
                .unwrap();
        assert_eq!(decoded.id, pack.id);
        assert_eq!(decoded.schema_version, pack.schema_version);
    }

    #[test]
    fn test_load_all_active_skips_and_removes_corrupt_or_oversized_pack() {
        let dir = tempdir().unwrap();
        let max = 1024usize;

        let valid = make_pack();
        JsonStorageAdapter::write_pack_atomic(dir.path(), &valid, max).unwrap();
        let valid_path = dir.path().join(format!("{}.json", valid.id.as_str()));
        assert!(valid_path.exists(), "valid pack file should exist");

        let corrupt_path = dir.path().join("pk_corrupt.json");
        std::fs::write(&corrupt_path, "not-json").unwrap();
        assert!(corrupt_path.exists(), "corrupt pack file should exist");

        let oversized = oversized_pack_by_growth(1024);
        let oversized_path = dir.path().join(format!("{}.json", oversized.id.as_str()));
        let oversized_payload = JsonStorageAdapter::encode(&oversized).unwrap();
        assert!(
            oversized_payload.len() > max,
            "oversized payload must exceed limit"
        );
        // write oversized payload directly to bypass pre-persist guards and simulate a corrupted pack on disk
        std::fs::write(&oversized_path, oversized_payload).unwrap();
        assert!(oversized_path.exists(), "oversized pack file should exist");

        let loaded = JsonStorageAdapter::load_all_active_sync(dir.path(), max).unwrap();
        assert_eq!(loaded.len(), 1, "expected only one valid pack");
        assert_eq!(loaded[0].id, valid.id);

        assert!(
            !corrupt_path.exists(),
            "corrupt pack should be removed by recovery"
        );
        assert!(
            !oversized_path.exists(),
            "oversized pack should be removed by recovery"
        );
    }

    #[tokio::test]
    async fn test_get_by_id_returns_none_for_corrupt_pack_and_recovers_file() {
        let dir = tempdir().unwrap();
        let max = 256usize;
        let adapter = JsonStorageAdapter::new_with_max(dir.path().to_path_buf(), max);
        let corrupt_id = PackId::new();
        let path = dir.path().join(format!("{}.json", corrupt_id.as_str()));
        std::fs::write(&path, "not-json").unwrap();

        let result = adapter.get_by_id(&corrupt_id).await.unwrap();
        assert!(result.is_none());
        assert!(
            !path.exists(),
            "corrupt pack file should be removed by recovery"
        );
    }

    #[test]
    fn test_decode_with_path_wraps_migration_error() {
        // A JSON that deserializes but fails schema migration (schema_version != CURRENT_SCHEMA_VERSION)
        // Encode a valid pack, then manually patch schema_version to an unsupported value.
        let pack = make_pack();
        let mut json_val: serde_json::Value =
            serde_json::from_str(&JsonStorageAdapter::encode(&pack).unwrap()).unwrap();
        json_val["schema_version"] = serde_json::Value::Number(1u64.into());
        let patched = serde_json::to_string(&json_val).unwrap();

        let fake_path = Path::new("/storage/pk_testtest.json");
        let err = JsonStorageAdapter::decode_with_path(fake_path, &patched).unwrap_err();

        match err {
            DomainError::MigrationRequired(msg) => {
                assert!(
                    msg.contains("pk_testtest.json"),
                    "error message should contain file path, got: {msg}"
                );
            }
            other => panic!("expected MigrationRequired, got: {:?}", other),
        }
    }
}
