use super::errors::{DomainError, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path};
use std::str::FromStr;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;
pub const MAX_REF_LINE_SPAN: usize = 2_000;

static TOKEN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9_\-]{1,63}$").expect("token regex must compile"));
static PACK_ID_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^pk_[a-z2-7]{8}$").expect("pack id regex must compile"));

// ── PackId ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackId(String);

impl PackId {
    pub fn new() -> Self {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
        let mut rng = rand::thread_rng();
        let mut suffix = String::with_capacity(8);
        for _ in 0..8 {
            let idx = rand::Rng::gen_range(&mut rng, 0..ALPHABET.len());
            suffix.push(ALPHABET[idx] as char);
        }
        Self(format!("pk_{}", suffix))
    }

    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            return Err(DomainError::InvalidData("PackId cannot be empty".into()));
        }
        if !PACK_ID_RE.is_match(s) {
            return Err(DomainError::InvalidData(
                "PackId must match ^pk_[a-z2-7]{8}$".into(),
            ));
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PackId {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for PackId {
    type Err = DomainError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl fmt::Display for PackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── PackName ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PackName(String);

impl PackName {
    pub fn new(s: &str) -> Result<Self> {
        let s = s.trim().to_string();
        if s.len() < 3 {
            return Err(DomainError::InvalidData(
                "PackName must be at least 3 characters".into(),
            ));
        }
        if s.len() > 120 {
            return Err(DomainError::InvalidData(
                "PackName too long (max 120)".into(),
            ));
        }
        if PACK_ID_RE.is_match(&s) {
            return Err(DomainError::InvalidData(
                "PackName must not look like PackId (^pk_[a-z2-7]{8}$)".into(),
            ));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PackName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── SectionKey ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SectionKey(String);

impl SectionKey {
    pub fn new(s: &str) -> Result<Self> {
        validate_token("section_key", s.trim())?;
        Ok(Self(s.trim().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SectionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── RefKey ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RefKey(String);

impl RefKey {
    pub fn new(s: &str) -> Result<Self> {
        validate_token("ref_key", s.trim())?;
        Ok(Self(s.trim().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RefKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── DiagramKey ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagramKey(String);

impl DiagramKey {
    pub fn new(s: &str) -> Result<Self> {
        validate_token("diagram_key", s.trim())?;
        Ok(Self(s.trim().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DiagramKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── RelativePath ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelativePath(String);

impl RelativePath {
    pub fn new(s: &str) -> Result<Self> {
        let s = s.trim().replace('\\', "/");
        if s.is_empty() {
            return Err(DomainError::InvalidData(
                "RelativePath cannot be empty".into(),
            ));
        }
        if s.len() > 1024 {
            return Err(DomainError::InvalidData("RelativePath too long".into()));
        }
        let path = Path::new(&s);
        if path.is_absolute() {
            return Err(DomainError::InvalidData(
                "Only repository-relative paths are allowed".into(),
            ));
        }
        if path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(DomainError::InvalidData(
                "Path must not contain parent directory segments".into(),
            ));
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── LineRange ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

impl LineRange {
    pub fn new(start: usize, end: usize) -> Result<Self> {
        if start == 0 {
            return Err(DomainError::InvalidData(
                "line_start must be >= 1 (1-indexed)".into(),
            ));
        }
        if end < start {
            return Err(DomainError::InvalidData(format!(
                "line_end ({}) must be >= line_start ({})",
                end, start
            )));
        }
        let span = end - start + 1;
        if span > MAX_REF_LINE_SPAN {
            return Err(DomainError::InvalidData(format!(
                "line range is too large: {} lines (max {})",
                span, MAX_REF_LINE_SPAN
            )));
        }
        Ok(Self { start, end })
    }
}

// ── Status ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Draft,
    Finalized,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Draft => write!(f, "draft"),
            Status::Finalized => write!(f, "finalized"),
        }
    }
}

impl FromStr for Status {
    type Err = DomainError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "draft" => Ok(Status::Draft),
            "finalized" => Ok(Status::Finalized),
            _ => Err(DomainError::InvalidData(format!("Invalid status: {}", s))),
        }
    }
}

// ── private helpers ───────────────────────────────────────────────────────────

fn validate_token(name: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(DomainError::InvalidData(format!(
            "{} cannot be empty",
            name
        )));
    }
    if !TOKEN_RE.is_match(value) {
        return Err(DomainError::InvalidData(format!(
            "{} must match ^[a-z0-9][a-z0-9_-]{{1,63}}$ (lowercase alphanumeric + underscores/hyphens)",
            name
        )));
    }
    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_name_validation() {
        assert!(PackName::new("").is_err());
        assert!(PackName::new("  ").is_err());
        assert!(PackName::new("ab").is_err(), "too short");
        assert!(PackName::new("abc").is_ok());
        assert!(PackName::new(&"x".repeat(121)).is_err(), "too long");
    }

    #[test]
    fn test_pack_id_validation() {
        let id = PackId::new();
        assert!(PackId::parse(id.as_str()).is_ok());
        assert!(id.as_str().starts_with("pk_"));
        assert_eq!(id.as_str().len(), 11);
        assert!(PackId::parse(id.as_str()).is_ok());
        assert!(PackId::parse("not-an-id").is_err());
        assert!(PackId::parse("pk_abcdefg1").is_err(), "only base32 chars");
    }

    #[test]
    fn test_pack_name_must_not_look_like_pack_id() {
        assert!(PackName::new("pk_abcdef23").is_err());
    }

    #[test]
    fn test_section_key_validation() {
        assert!(SectionKey::new("").is_err());
        assert!(SectionKey::new("valid-key").is_ok());
        assert!(SectionKey::new("UPPER").is_err(), "uppercase not allowed");
        assert!(SectionKey::new("has space").is_err());
        assert!(
            SectionKey::new("a").is_err(),
            "too short (min 2 chars after first)"
        );
    }

    #[test]
    fn test_relative_path_validation() {
        assert!(RelativePath::new("").is_err());
        assert!(RelativePath::new("/abs/path").is_err());
        assert!(RelativePath::new("..").is_err());
        assert!(RelativePath::new("tests/../foo").is_err());
        assert!(RelativePath::new("a\\..\\b.rs").is_err());
        assert!(RelativePath::new("src/main.rs").is_ok());
        assert!(RelativePath::new("nested/deep/file.ts").is_ok());
    }

    #[test]
    fn test_line_range_validation() {
        assert!(LineRange::new(0, 10).is_err(), "0-indexed is invalid");
        assert!(LineRange::new(10, 5).is_err(), "start > end is invalid");
        assert!(LineRange::new(1, 10).is_ok());
        assert!(LineRange::new(5, 5).is_ok(), "single line is ok");
    }

    #[test]
    fn test_line_range_span_limit() {
        assert!(LineRange::new(1, MAX_REF_LINE_SPAN + 1).is_err());
        assert!(LineRange::new(1, MAX_REF_LINE_SPAN).is_ok());
    }

    #[test]
    fn test_status_from_str() {
        assert_eq!("draft".parse::<Status>().unwrap(), Status::Draft);
        assert_eq!("finalized".parse::<Status>().unwrap(), Status::Finalized);
        assert!("unknown".parse::<Status>().is_err());
    }
}
