use async_trait::async_trait;
use std::io::ErrorKind;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    app::ports::{CodeExcerptPort, Snippet},
    domain::{
        errors::{DomainError, Result},
        types::{LineRange, RelativePath},
    },
};

const DEFAULT_MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;

fn parse_max_source_bytes_from_env() -> usize {
    std::env::var("CONTEXT_PACK_MAX_SOURCE_BYTES")
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_SOURCE_BYTES)
}

pub struct CodeExcerptFsAdapter {
    repo_root: PathBuf,
    canonical_repo_root: PathBuf,
    max_source_bytes: usize,
}

impl CodeExcerptFsAdapter {
    pub fn new(repo_root: PathBuf) -> Result<Self> {
        let canonical_repo_root = std::fs::canonicalize(&repo_root).map_err(|e| {
            DomainError::InvalidData(format!(
                "source root '{}' is invalid or does not exist: {}",
                repo_root.display(),
                e
            ))
        })?;
        Ok(Self {
            repo_root,
            canonical_repo_root,
            max_source_bytes: parse_max_source_bytes_from_env(),
        })
    }
}

#[async_trait]
impl CodeExcerptPort for CodeExcerptFsAdapter {
    async fn read_lines(&self, path: &RelativePath, range: LineRange) -> Result<Snippet> {
        let full_path = self.repo_root.join(path.as_str());
        let canonical_path = fs::canonicalize(&full_path).await.map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                DomainError::StaleRef(format!(
                    "file '{}' does not exist under source root",
                    path.as_str()
                ))
            } else {
                DomainError::Io(format!("failed to canonicalize '{}': {}", path.as_str(), e))
            }
        })?;

        if !canonical_path.starts_with(&self.canonical_repo_root) {
            return Err(DomainError::InvalidData(format!(
                "path '{}' resolves outside source root",
                path.as_str()
            )));
        }

        let meta = fs::metadata(&canonical_path).await.map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                DomainError::StaleRef(format!(
                    "file '{}' does not exist under source root",
                    path.as_str()
                ))
            } else {
                DomainError::Io(format!("failed to stat file '{}': {}", path.as_str(), e))
            }
        })?;
        if meta.len() as usize > self.max_source_bytes {
            return Err(DomainError::InvalidData(format!(
                "source file '{}' is too large: {} bytes (max {})",
                path.as_str(),
                meta.len(),
                self.max_source_bytes
            )));
        }

        let file = fs::File::open(&canonical_path).await.map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                DomainError::StaleRef(format!(
                    "file '{}' does not exist under source root",
                    path.as_str()
                ))
            } else {
                DomainError::Io(format!("failed to open file '{}': {}", path.as_str(), e))
            }
        })?;
        let mut reader = BufReader::new(file);
        let mut buf = String::new();
        let mut current_line = 0usize;
        let mut total_lines = 0usize;
        let mut excerpt = Vec::new();

        loop {
            buf.clear();
            let n = reader
                .read_line(&mut buf)
                .await
                .map_err(|e| DomainError::Io(format!("failed to read file '{}': {}", path, e)))?;
            if n == 0 {
                break;
            }
            current_line += 1;
            total_lines = current_line;

            if current_line >= range.start && current_line <= range.end {
                let line = buf.trim_end_matches(['\r', '\n']);
                excerpt.push(format!("{:>4}: {}", current_line, line));
            }
        }

        if range.start > total_lines {
            return Err(DomainError::StaleRef(format!(
                "file '{}' has {} lines but ref starts at {}",
                path.as_str(),
                total_lines,
                range.start
            )));
        }

        if range.end > total_lines {
            return Err(DomainError::StaleRef(format!(
                "file '{}' has {} lines but ref ends at {}",
                path.as_str(),
                total_lines,
                range.end
            )));
        }

        Ok(Snippet {
            path: path.as_str().to_string(),
            line_start: range.start,
            line_end: range.end,
            body: excerpt.join("\n"),
            total_lines,
        })
    }
}
