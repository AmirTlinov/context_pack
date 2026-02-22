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
            Err(err) => Err(DomainError::Io(format!(
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
        if meta.len() as usize > max_pack_bytes {
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

    fn write_pack_atomic(storage_dir: &Path, pack: &Pack) -> Result<()> {
        let path = Self::pack_path(storage_dir, &pack.id);
        let tmp = storage_dir.join(format!("{}.tmp", pack.id.as_str()));
        let content = Self::encode(pack)?;
        std::fs::write(&tmp, content)
            .map_err(|e| DomainError::Io(format!("failed to write tmp pack: {}", e)))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| DomainError::Io(format!("failed to rename pack file: {}", e)))?;
        Ok(())
    }

    fn purge_expired_sync(storage_dir: &Path, max_pack_bytes: usize) -> Result<()> {
        let now = Utc::now();
        let paths = Self::list_pack_paths_sync(storage_dir)?;
        for path in paths {
            let pack = Self::read_pack_from_path(&path, max_pack_bytes)?;
            if pack.is_expired(now) {
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

    fn load_all_active_sync(storage_dir: &Path, max_pack_bytes: usize) -> Result<Vec<Pack>> {
        let now = Utc::now();
        let mut packs = Vec::new();
        for path in Self::list_pack_paths_sync(storage_dir)? {
            let pack = Self::read_pack_from_path(&path, max_pack_bytes)?;
            if !pack.is_expired(now) {
                packs.push(pack);
            }
        }
        packs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(packs)
    }

    async fn purge_expired(&self) -> Result<()> {
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
                lock.unlock().ok();
                return Err(DomainError::Conflict(format!(
                    "pack id '{}' already exists",
                    pack.id
                )));
            }

            if let Some(new_name) = &pack.name {
                for existing_path in Self::list_pack_paths_sync(&storage_dir)? {
                    let existing = Self::read_pack_from_path(&existing_path, max_pack_bytes)?;
                    if existing.name.as_ref() == Some(new_name) {
                        lock.unlock().ok();
                        return Err(DomainError::Conflict(format!(
                            "pack with name '{}' already exists",
                            new_name
                        )));
                    }
                }
            }

            Self::write_pack_atomic(&storage_dir, &pack)?;
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
                lock.unlock().ok();
                return Err(DomainError::NotFound(format!(
                    "pack '{}' not found",
                    pack.id
                )));
            }
            let current = Self::read_pack_from_path(&path, max_pack_bytes)?;
            if current.revision != expected_revision {
                lock.unlock().ok();
                return Err(DomainError::RevisionConflict {
                    expected: expected_revision,
                    actual: current.revision,
                });
            }

            Self::write_pack_atomic(&storage_dir, &pack)?;
            lock.unlock()
                .map_err(|e| DomainError::Io(format!("failed to unlock repo: {}", e)))?;
            Ok(())
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))??;
        Ok(())
    }

    async fn get_by_id(&self, id: &PackId) -> Result<Option<Pack>> {
        self.purge_expired().await?;
        let storage_dir = self.storage_dir.clone();
        let id = id.clone();
        let max_pack_bytes = self.max_pack_bytes;
        task::spawn_blocking(move || -> Result<Option<Pack>> {
            let path = Self::pack_path(&storage_dir, &id);
            if !path.exists() {
                return Ok(None);
            }
            let pack = Self::read_pack_from_path(&path, max_pack_bytes)?;
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
        self.purge_expired().await?;
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
                _ => Err(DomainError::Conflict(format!(
                    "multiple packs found for name '{}'",
                    name
                ))),
            }
        })
        .await
        .map_err(|e| DomainError::Io(format!("task execution failed: {}", e)))?
    }

    async fn list_packs(&self, filter: ListFilter) -> Result<Vec<Pack>> {
        self.purge_expired().await?;
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
}

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
        let encoded = JsonStorageAdapter::encode(&pack).unwrap();
        let decoded = JsonStorageAdapter::decode(&encoded).unwrap();
        assert_eq!(decoded.id, pack.id);
        assert_eq!(decoded.name, pack.name);
        assert_eq!(decoded.schema_version, pack.schema_version);
    }

    #[test]
    fn test_decode_rejects_malformed() {
        assert!(JsonStorageAdapter::decode("not-json").is_err());
        assert!(JsonStorageAdapter::decode("{}").is_err());
    }
}
