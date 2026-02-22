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
