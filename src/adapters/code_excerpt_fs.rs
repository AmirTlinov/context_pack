use async_trait::async_trait;
use std::path::PathBuf;
use tokio::fs;

use crate::{
    app::ports::{CodeExcerptPort, Snippet},
    domain::{
        errors::{DomainError, Result},
        types::{LineRange, RelativePath},
    },
};

pub struct CodeExcerptFsAdapter {
    repo_root: PathBuf,
    canonical_repo_root: PathBuf,
}

impl CodeExcerptFsAdapter {
    pub fn new(repo_root: PathBuf) -> Self {
        let canonical_repo_root =
            std::fs::canonicalize(&repo_root).unwrap_or_else(|_| repo_root.clone());
        Self {
            repo_root,
            canonical_repo_root,
        }
    }
}

#[async_trait]
impl CodeExcerptPort for CodeExcerptFsAdapter {
    async fn read_lines(&self, path: &RelativePath, range: LineRange) -> Result<Snippet> {
        let full_path = self.repo_root.join(path.as_str());
        let canonical_path = fs::canonicalize(&full_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
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

        let content = fs::read_to_string(&canonical_path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                DomainError::StaleRef(format!(
                    "file '{}' does not exist under source root",
                    path.as_str()
                ))
            } else {
                DomainError::Io(format!("failed to read file '{}': {}", path.as_str(), e))
            }
        })?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();
        let start_idx = range.start.saturating_sub(1);

        if start_idx >= total_lines {
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

        let excerpt_lines = &lines[start_idx..range.end];

        // Number lines with right-aligned 4-digit line number: "   5: content"
        let body = excerpt_lines
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{:>4}: {}", range.start + i, line))
            .collect::<Vec<_>>()
            .join("\n");

        Ok(Snippet {
            path: path.as_str().to_string(),
            line_start: range.start,
            line_end: range.end,
            body,
            total_lines,
        })
    }
}
