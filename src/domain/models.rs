use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::{
    errors::{DomainError, Result},
    types::{
        DiagramKey, LineRange, PackId, PackName, RefKey, RelativePath, SectionKey, Status,
        CURRENT_SCHEMA_VERSION,
    },
};

// ── CodeRef ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeRef {
    pub key: RefKey,
    pub path: RelativePath,
    pub lines: LineRange,
    pub title: Option<String>,
    pub why: Option<String>,
    pub group: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefSpec {
    pub key: RefKey,
    pub path: RelativePath,
    pub lines: LineRange,
    pub title: Option<String>,
    pub why: Option<String>,
    pub group: Option<String>,
}

// ── Diagram ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagram {
    pub key: DiagramKey,
    pub title: String,
    pub mermaid: String,
    pub why: Option<String>,
}

// ── Section ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub key: SectionKey,
    pub title: String,
    pub description: Option<String>,
    pub refs: Vec<CodeRef>,
    pub diagrams: Vec<Diagram>,
}

// ── Pack (aggregate root) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub schema_version: u32,
    pub id: PackId,
    pub name: Option<PackName>,
    pub title: Option<String>,
    pub brief: Option<String>,
    pub status: Status,
    pub tags: Vec<String>,
    pub sections: Vec<Section>,
    pub revision: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl Pack {
    pub fn new(id: PackId, name: Option<PackName>) -> Self {
        let now = Utc::now();
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            id,
            name,
            title: None,
            brief: None,
            status: Status::Draft,
            tags: Vec::new(),
            sections: Vec::new(),
            revision: 1,
            created_at: now,
            updated_at: now,
            expires_at: now + Duration::hours(24),
        }
    }

    // ── invariant guards ──────────────────────────────────────────────────────

    pub fn assert_mutable(&self) -> Result<()> {
        if self.status == Status::Finalized {
            return Err(DomainError::InvalidState(format!(
                "pack {} is finalized and immutable; transition to draft first",
                self.id
            )));
        }
        Ok(())
    }

    pub(crate) fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
    }

    // ── lifecycle FSM ─────────────────────────────────────────────────────────

    pub fn set_status(&mut self, status: Status) -> Result<()> {
        if self.status == status {
            return Ok(());
        }
        match (self.status, status) {
            (Status::Draft, Status::Finalized) => {
                let has_content = self
                    .sections
                    .iter()
                    .any(|s| !s.refs.is_empty() || !s.diagrams.is_empty());
                if !has_content {
                    return Err(DomainError::InvalidState(
                        "cannot finalize a pack without refs or diagrams".into(),
                    ));
                }
                self.status = status;
                self.touch();
                Ok(())
            }
            (Status::Finalized, Status::Draft) => {
                self.status = status;
                self.touch();
                Ok(())
            }
            _ => Err(DomainError::InvalidState(
                "unsupported status transition".into(),
            )),
        }
    }

    pub fn set_ttl_from_now(&mut self, minutes: u64, now: DateTime<Utc>) -> Result<()> {
        let duration = ttl_duration(minutes)?;
        self.expires_at = now + duration;
        self.touch();
        Ok(())
    }

    pub fn set_ttl_on_create(&mut self, minutes: u64, now: DateTime<Utc>) -> Result<()> {
        let duration = ttl_duration(minutes)?;
        self.expires_at = now + duration;
        Ok(())
    }

    pub fn extend_ttl(&mut self, minutes: u64, now: DateTime<Utc>) -> Result<()> {
        let duration = ttl_duration(minutes)?;
        let base = if self.expires_at > now {
            self.expires_at
        } else {
            now
        };
        self.expires_at = base + duration;
        self.touch();
        Ok(())
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }

    pub fn ttl_remaining_seconds(&self, now: DateTime<Utc>) -> i64 {
        if self.is_expired(now) {
            0
        } else {
            (self.expires_at - now).num_seconds()
        }
    }

    pub fn ttl_remaining_human(&self, now: DateTime<Utc>) -> String {
        human_ttl(self.ttl_remaining_seconds(now))
    }

    // ── metadata ──────────────────────────────────────────────────────────────

    pub fn set_meta(
        &mut self,
        title: Option<String>,
        brief: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<()> {
        self.assert_mutable()?;
        if title.is_none() && brief.is_none() && tags.is_none() {
            return Err(DomainError::InvalidData(
                "set_meta requires at least one of: title, brief, tags".into(),
            ));
        }

        let mut changed = false;
        if let Some(t) = title {
            if self.title.as_ref() != Some(&t) {
                self.title = Some(t);
                changed = true;
            }
        }
        if let Some(b) = brief {
            if self.brief.as_ref() != Some(&b) {
                self.brief = Some(b);
                changed = true;
            }
        }
        if let Some(tg) = tags {
            if self.tags != tg {
                self.tags = tg;
                changed = true;
            }
        }
        if changed {
            self.touch();
        }
        Ok(())
    }

    // ── section management ────────────────────────────────────────────────────

    pub fn upsert_section(
        &mut self,
        key: SectionKey,
        title: String,
        description: Option<String>,
        order: Option<usize>,
    ) -> Result<()> {
        self.assert_mutable()?;
        let existing_pos = self.sections.iter().position(|s| s.key == key);
        let mut section = if let Some(idx) = existing_pos {
            self.sections.remove(idx)
        } else {
            Section {
                key,
                title: String::new(),
                description: None,
                refs: Vec::new(),
                diagrams: Vec::new(),
            }
        };
        section.title = title;
        if let Some(d) = description {
            section.description = Some(d);
        }
        let insert_at = match order {
            Some(o) => o.min(self.sections.len()),
            None => existing_pos
                .map(|idx| idx.min(self.sections.len()))
                .unwrap_or(self.sections.len()),
        };
        self.sections.insert(insert_at, section);
        self.touch();
        Ok(())
    }

    pub fn delete_section(&mut self, key: &SectionKey) -> Result<()> {
        self.assert_mutable()?;
        let before = self.sections.len();
        self.sections.retain(|s| s.key != *key);
        if self.sections.len() == before {
            return Err(DomainError::NotFound(format!(
                "section '{}' not found",
                key
            )));
        }
        self.touch();
        Ok(())
    }

    // ── ref management ────────────────────────────────────────────────────────

    fn get_section_mut(&mut self, section_key: &SectionKey) -> Result<&mut Section> {
        self.sections
            .iter_mut()
            .find(|s| s.key == *section_key)
            .ok_or_else(|| DomainError::NotFound(format!("section '{}' not found", section_key)))
    }

    pub fn upsert_ref(&mut self, section_key: &SectionKey, spec: RefSpec) -> Result<()> {
        self.assert_mutable()?;
        let section = self.get_section_mut(section_key)?;
        let new_ref = CodeRef {
            key: spec.key.clone(),
            path: spec.path,
            lines: spec.lines,
            title: spec.title,
            why: spec.why,
            group: spec.group,
        };
        if let Some(existing) = section.refs.iter_mut().find(|r| r.key == spec.key) {
            *existing = new_ref;
        } else {
            section.refs.push(new_ref);
        }
        self.touch();
        Ok(())
    }

    pub fn delete_ref(&mut self, section_key: &SectionKey, ref_key: &RefKey) -> Result<()> {
        self.assert_mutable()?;
        let section = self.get_section_mut(section_key)?;
        let before = section.refs.len();
        section.refs.retain(|r| r.key != *ref_key);
        if section.refs.len() == before {
            return Err(DomainError::NotFound(format!(
                "ref '{}' not found",
                ref_key
            )));
        }
        self.touch();
        Ok(())
    }

    // ── diagram management ────────────────────────────────────────────────────

    pub fn upsert_diagram(
        &mut self,
        section_key: &SectionKey,
        diagram_key: DiagramKey,
        title: String,
        mermaid: String,
        why: Option<String>,
    ) -> Result<()> {
        self.assert_mutable()?;
        let section = self.get_section_mut(section_key)?;
        let new_diagram = Diagram {
            key: diagram_key.clone(),
            title,
            mermaid,
            why,
        };
        if let Some(existing) = section.diagrams.iter_mut().find(|d| d.key == diagram_key) {
            *existing = new_diagram;
        } else {
            section.diagrams.push(new_diagram);
        }
        self.touch();
        Ok(())
    }

    // ── query helpers ─────────────────────────────────────────────────────────

    /// Refs within a section grouped by the `group` field.
    pub fn refs_grouped_in_section<'a>(section: &'a Section) -> BTreeMap<String, Vec<&'a CodeRef>> {
        let mut map: BTreeMap<String, Vec<&'a CodeRef>> = BTreeMap::new();
        for r in &section.refs {
            let g = r.group.clone().unwrap_or_else(|| "ungrouped".to_string());
            map.entry(g).or_default().push(r);
        }
        map
    }

    // ── schema migration ──────────────────────────────────────────────────────

    pub fn migrate_schema(self) -> Result<Self> {
        PackId::parse(self.id.as_str())?;
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(DomainError::MigrationRequired(format!(
                "unsupported schema version {} (expected {})",
                self.schema_version, CURRENT_SCHEMA_VERSION
            )));
        }
        Ok(self)
    }
}

fn ttl_duration(minutes: u64) -> Result<Duration> {
    if minutes == 0 {
        return Err(DomainError::InvalidData(
            "ttl_minutes must be >= 1".to_string(),
        ));
    }
    if minutes > 5 * 365 * 24 * 60 {
        return Err(DomainError::InvalidData(
            "ttl_minutes is too large (max 5 years)".to_string(),
        ));
    }
    let mins_i64 = i64::try_from(minutes)
        .map_err(|_| DomainError::InvalidData("ttl_minutes is out of range".to_string()))?;
    Ok(Duration::minutes(mins_i64))
}

fn human_ttl(seconds: i64) -> String {
    if seconds <= 0 {
        return "expired".to_string();
    }
    if seconds < 60 {
        return "<1m".to_string();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{}m", minutes);
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{}h", hours);
    }
    let days = hours / 24;
    format!("{}d", days)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::{LineRange, PackId, PackName, RefKey, RelativePath, SectionKey};

    fn make_pack() -> Pack {
        Pack::new(PackId::new(), Some(PackName::new("test-pack").unwrap()))
    }

    #[test]
    fn test_finalized_pack_is_immutable() {
        let mut pack = make_pack();
        let sk = SectionKey::new("sec-one").unwrap();
        pack.upsert_section(sk.clone(), "Section One".into(), None, None)
            .unwrap();
        pack.upsert_ref(
            &sk,
            RefSpec {
                key: RefKey::new("ref-one").unwrap(),
                path: RelativePath::new("src/main.rs").unwrap(),
                lines: LineRange::new(1, 10).unwrap(),
                title: None,
                why: None,
                group: None,
            },
        )
        .unwrap();
        pack.set_status(Status::Finalized).unwrap();

        let res = pack.upsert_section(SectionKey::new("sec-two").unwrap(), "X".into(), None, None);
        assert!(res.is_err(), "must not mutate finalized pack");
        match res.unwrap_err() {
            DomainError::InvalidState(_) => {}
            e => panic!("expected InvalidState, got {:?}", e),
        }
    }

    #[test]
    fn test_cannot_finalize_empty_pack() {
        let mut pack = make_pack();
        let res = pack.set_status(Status::Finalized);
        assert!(res.is_err());
    }

    #[test]
    fn test_cannot_finalize_with_only_empty_section() {
        let mut pack = make_pack();
        pack.upsert_section(SectionKey::new("sec-one").unwrap(), "S".into(), None, None)
            .unwrap();
        assert!(pack.set_status(Status::Finalized).is_err());
    }

    #[test]
    fn test_revision_increments_on_mutation() {
        let mut pack = make_pack();
        assert_eq!(pack.revision, 1);
        let sk = SectionKey::new("sec-one").unwrap();
        pack.upsert_section(sk.clone(), "S".into(), None, None)
            .unwrap();
        assert_eq!(pack.revision, 2);
        pack.upsert_ref(
            &sk,
            RefSpec {
                key: RefKey::new("ref-one").unwrap(),
                path: RelativePath::new("src/main.rs").unwrap(),
                lines: LineRange::new(1, 5).unwrap(),
                title: None,
                why: None,
                group: None,
            },
        )
        .unwrap();
        assert_eq!(pack.revision, 3);
    }

    #[test]
    fn test_upsert_ref_replaces_existing() {
        let mut pack = make_pack();
        let sk = SectionKey::new("sec-one").unwrap();
        pack.upsert_section(sk.clone(), "S".into(), None, None)
            .unwrap();
        let rk = RefKey::new("ref-one").unwrap();
        pack.upsert_ref(
            &sk,
            RefSpec {
                key: rk.clone(),
                path: RelativePath::new("a.rs").unwrap(),
                lines: LineRange::new(1, 5).unwrap(),
                title: None,
                why: None,
                group: None,
            },
        )
        .unwrap();
        pack.upsert_ref(
            &sk,
            RefSpec {
                key: rk.clone(),
                path: RelativePath::new("b.rs").unwrap(),
                lines: LineRange::new(2, 7).unwrap(),
                title: None,
                why: None,
                group: None,
            },
        )
        .unwrap();
        assert_eq!(pack.sections[0].refs.len(), 1);
        assert_eq!(pack.sections[0].refs[0].path.as_str(), "b.rs");
    }

    #[test]
    fn test_delete_ref_not_found_returns_error() {
        let mut pack = make_pack();
        let sk = SectionKey::new("sec-one").unwrap();
        pack.upsert_section(sk.clone(), "S".into(), None, None)
            .unwrap();
        let res = pack.delete_ref(&sk, &RefKey::new("nonexistent").unwrap());
        assert!(res.is_err());
    }

    #[test]
    fn test_migrate_schema_ahead_returns_error() {
        let mut pack = make_pack();
        pack.schema_version = 999;
        assert!(pack.migrate_schema().is_err());
    }

    #[test]
    fn test_migrate_schema_rejects_non_compact_pack_id() {
        let mut pack = make_pack();
        pack.id = PackId::parse("pk_abcdef23").unwrap();
        // force invalid id without parse API
        let raw = serde_json::to_string(&pack)
            .unwrap()
            .replace("pk_abcdef23", "invalid-id");
        let parsed: Pack = serde_json::from_str(&raw).unwrap();
        assert!(parsed.migrate_schema().is_err());
    }

    #[test]
    fn test_migrate_schema_rejects_older_schema() {
        let mut pack = make_pack();
        pack.schema_version = CURRENT_SCHEMA_VERSION - 1;
        assert!(pack.migrate_schema().is_err());
    }

    #[test]
    fn test_set_ttl_and_extend_ttl() {
        let mut pack = make_pack();
        let now = Utc::now();
        pack.set_ttl_from_now(30, now).unwrap();
        assert!(pack.expires_at > now);
        let expires_after_set = pack.expires_at;

        pack.extend_ttl(10, now).unwrap();
        assert!(pack.expires_at > expires_after_set);
    }

    #[test]
    fn test_ttl_must_be_positive() {
        let mut pack = make_pack();
        assert!(pack.set_ttl_from_now(0, Utc::now()).is_err());
        assert!(pack.extend_ttl(0, Utc::now()).is_err());
    }

    #[test]
    fn test_set_meta_requires_fields() {
        let mut pack = make_pack();
        assert!(pack.set_meta(None, None, None).is_err());
    }

    #[test]
    fn test_is_expired_when_past() {
        let mut pack = Pack::new(PackId::new(), None);
        pack.expires_at = Utc::now() - Duration::seconds(1);
        assert!(
            pack.is_expired(Utc::now()),
            "pack with past expires_at should be expired"
        );
    }

    #[test]
    fn test_is_expired_when_future() {
        let pack = Pack::new(PackId::new(), None);
        // default expires_at is now + 24h
        assert!(
            !pack.is_expired(Utc::now()),
            "freshly created pack should not be expired"
        );
    }

    #[test]
    fn test_ttl_remaining_human_expired() {
        let mut pack = Pack::new(PackId::new(), None);
        pack.expires_at = Utc::now() - Duration::seconds(10);
        assert_eq!(pack.ttl_remaining_human(Utc::now()), "expired");
    }

    #[test]
    fn test_ttl_remaining_human_minutes() {
        let mut pack = Pack::new(PackId::new(), None);
        // Set expires_at to exactly 5 minutes from now; pass the same `now` to both
        let now = Utc::now();
        pack.expires_at = now + Duration::seconds(300);
        assert_eq!(pack.ttl_remaining_human(now), "5m");
    }

    #[test]
    fn test_ttl_remaining_human_hours() {
        let mut pack = Pack::new(PackId::new(), None);
        // Set expires_at to exactly 2 hours from now; pass the same `now` to both
        let now = Utc::now();
        pack.expires_at = now + Duration::seconds(7200);
        assert_eq!(pack.ttl_remaining_human(now), "2h");
    }

    #[test]
    fn test_extend_ttl_from_now_when_already_expired() {
        let mut pack = Pack::new(PackId::new(), None);
        // Make the pack already expired
        pack.expires_at = Utc::now() - Duration::seconds(100);
        let now = Utc::now();
        // extend_ttl should extend from now (not from the expired time)
        pack.extend_ttl(60, now).unwrap();
        let remaining = pack.ttl_remaining_seconds(now);
        // Should be approximately 60 minutes = 3600 seconds
        assert!(
            remaining > 3500 && remaining <= 3600,
            "remaining should be ~3600s, got: {remaining}"
        );
    }
}
