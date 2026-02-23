use crate::{
    app::ports::PackRepositoryPort,
    domain::{
        errors::{DomainError, Result},
        models::Pack,
        types::{PackId, PackName},
    },
};

/// Resolve a pack by ID or name.
///
/// Algorithm:
/// 1. Trim whitespace; reject empty identifiers.
/// 2. If the identifier parses as a [`PackId`], look up by id.
///    - Found → return it.
///    - Not found → return [`DomainError::NotFound`] immediately (a valid UUID
///      that has no match should not silently fall back to a name lookup).
/// 3. Otherwise treat the identifier as a [`PackName`] and look up by name.
///    - Found → return it.
///    - Not found → return [`DomainError::NotFound`].
pub async fn resolve_pack(repo: &dyn PackRepositoryPort, identifier: &str) -> Result<Pack> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err(DomainError::InvalidData(
            "identifier (id or name) is required".into(),
        ));
    }

    // Try by id first
    if let Ok(id) = PackId::parse(identifier) {
        if let Some(pack) = repo.get_by_id(&id).await? {
            return Ok(pack);
        }
        return Err(DomainError::NotFound(format!(
            "pack '{}' not found",
            identifier
        )));
    }

    // Fall back to name lookup
    let name = PackName::new(identifier)?;
    if let Some(pack) = repo.get_by_name(&name).await? {
        return Ok(pack);
    }
    Err(DomainError::NotFound(format!(
        "pack '{}' not found",
        identifier
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use crate::app::ports::{ListFilter, PackRepositoryPort};

    // ── FakeRepo ─────────────────────────────────────────────────────────────

    struct FakeRepo(Mutex<HashMap<String, Pack>>);

    impl FakeRepo {
        fn empty() -> Self {
            Self(Mutex::new(HashMap::new()))
        }

        fn with(packs: Vec<Pack>) -> Self {
            let map = packs
                .into_iter()
                .map(|p| (p.id.as_str().to_string(), p))
                .collect();
            Self(Mutex::new(map))
        }
    }

    #[async_trait]
    impl PackRepositoryPort for FakeRepo {
        async fn create_new(&self, pack: &Pack) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(pack.id.as_str().to_string(), pack.clone());
            Ok(())
        }

        async fn save_with_expected_revision(&self, pack: &Pack, _expected: u64) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(pack.id.as_str().to_string(), pack.clone());
            Ok(())
        }

        async fn get_by_id(&self, id: &PackId) -> Result<Option<Pack>> {
            Ok(self.0.lock().unwrap().get(id.as_str()).cloned())
        }

        async fn get_by_name(&self, name: &PackName) -> Result<Option<Pack>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .values()
                .find(|p| p.name.as_ref().map(|n| n.as_str()) == Some(name.as_str()))
                .cloned())
        }

        async fn list_packs(&self, _filter: ListFilter) -> Result<Vec<Pack>> {
            Ok(self.0.lock().unwrap().values().cloned().collect())
        }

        async fn purge_expired(&self) -> Result<()> {
            Ok(())
        }

        async fn delete_pack_file(&self, id: &PackId) -> Result<bool> {
            Ok(self.0.lock().unwrap().remove(id.as_str()).is_some())
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_pack_with_name(name: &str) -> Pack {
        Pack::new(PackId::new(), Some(PackName::new(name).unwrap()))
    }

    fn make_pack_no_name() -> Pack {
        Pack::new(PackId::new(), None)
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// Happy path: valid PackId that exists in the repo → returns the pack.
    #[tokio::test]
    async fn resolve_by_id_success() {
        let pack = make_pack_no_name();
        let id_str = pack.id.as_str().to_string();
        let repo = FakeRepo::with(vec![pack.clone()]);

        let result = resolve_pack(&repo, &id_str).await.unwrap();
        assert_eq!(result.id.as_str(), id_str);
    }

    /// Valid PackId format but NOT present in the repo → returns NotFound WITHOUT
    /// trying name lookup.  This is the critical invariant: a UUID that misses must
    /// not silently fall through to the name branch.
    #[tokio::test]
    async fn resolve_by_id_not_found_no_name_fallthrough() {
        // Seed a pack whose *name* happens to be a valid PackId string.
        // If the resolver fell through to name lookup it would find this pack —
        // the test would then fail, proving the invariant was violated.
        let missing_id = PackId::new();
        let decoy_pack = Pack::new(
            PackId::new(),
            Some(PackName::new("valid-pack-name").unwrap()),
        );
        let repo = FakeRepo::with(vec![decoy_pack]);

        let err = resolve_pack(&repo, missing_id.as_str()).await.unwrap_err();
        assert!(
            matches!(err, DomainError::NotFound(_)),
            "expected NotFound, got: {:?}",
            err
        );
    }

    /// Identifier is not UUID format → falls through to name lookup and finds the pack.
    #[tokio::test]
    async fn resolve_by_name_success() {
        let pack = make_pack_with_name("my-feature-pack");
        let repo = FakeRepo::with(vec![pack.clone()]);

        let result = resolve_pack(&repo, "my-feature-pack").await.unwrap();
        assert_eq!(
            result.name.as_ref().map(|n| n.as_str()),
            Some("my-feature-pack")
        );
    }

    /// Empty string (or whitespace-only) → InvalidData.
    #[tokio::test]
    async fn resolve_empty_identifier_returns_invalid_data() {
        let repo = FakeRepo::empty();

        let err_empty = resolve_pack(&repo, "").await.unwrap_err();
        assert!(
            matches!(err_empty, DomainError::InvalidData(_)),
            "expected InvalidData for empty string, got: {:?}",
            err_empty
        );

        let err_ws = resolve_pack(&repo, "   ").await.unwrap_err();
        assert!(
            matches!(err_ws, DomainError::InvalidData(_)),
            "expected InvalidData for whitespace-only, got: {:?}",
            err_ws
        );
    }

    /// Non-UUID identifier that also doesn't match any pack name → NotFound.
    #[tokio::test]
    async fn resolve_name_not_found() {
        let repo = FakeRepo::empty();

        let err = resolve_pack(&repo, "does-not-exist").await.unwrap_err();
        assert!(
            matches!(err, DomainError::NotFound(_)),
            "expected NotFound, got: {:?}",
            err
        );
    }
}
