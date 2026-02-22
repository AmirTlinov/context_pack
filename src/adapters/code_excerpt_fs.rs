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

    /// Constructor for tests that need to control the byte limit without
    /// mutating environment variables (avoids thread-safety issues).
    #[cfg(test)]
    fn new_with_max(repo_root: PathBuf, max_source_bytes: usize) -> Result<Self> {
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
            max_source_bytes,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::LineRange;
    use tempfile::tempdir;

    fn rel(s: &str) -> RelativePath {
        RelativePath::new(s).unwrap()
    }

    fn range(start: usize, end: usize) -> LineRange {
        LineRange::new(start, end).unwrap()
    }

    #[tokio::test]
    async fn test_reads_exact_lines() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("src.rs"),
            "alpha\nbeta\ngamma\ndelta\nepsilon\n",
        )
        .unwrap();
        let adapter = CodeExcerptFsAdapter::new(dir.path().to_path_buf()).unwrap();
        let snippet = adapter
            .read_lines(&rel("src.rs"), range(2, 4))
            .await
            .unwrap();
        assert_eq!(snippet.line_start, 2);
        assert_eq!(snippet.line_end, 4);
        assert_eq!(snippet.total_lines, 5);
        assert!(snippet.body.contains("   2: beta"), "got: {}", snippet.body);
        assert!(
            snippet.body.contains("   3: gamma"),
            "got: {}",
            snippet.body
        );
        assert!(
            snippet.body.contains("   4: delta"),
            "got: {}",
            snippet.body
        );
        assert!(!snippet.body.contains("alpha"), "should not include line 1");
        assert!(
            !snippet.body.contains("epsilon"),
            "should not include line 5"
        );
    }

    #[tokio::test]
    async fn test_reads_single_line() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("one.rs"), "only\n").unwrap();
        let adapter = CodeExcerptFsAdapter::new(dir.path().to_path_buf()).unwrap();
        let snippet = adapter
            .read_lines(&rel("one.rs"), range(1, 1))
            .await
            .unwrap();
        assert_eq!(snippet.line_start, 1);
        assert_eq!(snippet.line_end, 1);
        assert!(snippet.body.contains("   1: only"));
    }

    #[tokio::test]
    async fn test_file_not_found_returns_stale_ref() {
        let dir = tempdir().unwrap();
        let adapter = CodeExcerptFsAdapter::new(dir.path().to_path_buf()).unwrap();
        let err = adapter
            .read_lines(&rel("missing.rs"), range(1, 1))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::StaleRef(_)),
            "expected StaleRef, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_path_outside_root_is_rejected() {
        // Only run on Unix where symlinks are available.
        #[cfg(not(unix))]
        return;

        #[cfg(unix)]
        {
            let dir_a = tempdir().unwrap();
            let dir_b = tempdir().unwrap();
            let secret = dir_b.path().join("secret.rs");
            std::fs::write(&secret, "secret content\n").unwrap();

            let link = dir_a.path().join("outside.rs");
            std::os::unix::fs::symlink(&secret, &link).unwrap();

            let adapter = CodeExcerptFsAdapter::new(dir_a.path().to_path_buf()).unwrap();
            let err = adapter
                .read_lines(&rel("outside.rs"), range(1, 1))
                .await
                .unwrap_err();
            assert!(
                matches!(err, DomainError::InvalidData(_)),
                "expected InvalidData for path outside root, got: {:?}",
                err
            );
        }
    }

    #[tokio::test]
    async fn test_file_too_large_is_rejected() {
        let dir = tempdir().unwrap();
        // "hello world\n" is 12 bytes; limit to 1 byte to force rejection.
        std::fs::write(dir.path().join("big.rs"), "hello world\n").unwrap();

        // Use the test-only constructor so we don't touch env vars at all.
        let adapter = CodeExcerptFsAdapter::new_with_max(dir.path().to_path_buf(), 1).unwrap();

        let err = adapter
            .read_lines(&rel("big.rs"), range(1, 1))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::InvalidData(_)),
            "expected InvalidData for oversized file, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_range_start_exceeds_total_lines() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("short.rs"), "line1\nline2\n").unwrap();
        let adapter = CodeExcerptFsAdapter::new(dir.path().to_path_buf()).unwrap();
        // File has 2 lines, but we ask for lines starting at 5.
        let err = adapter
            .read_lines(&rel("short.rs"), range(5, 6))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::StaleRef(_)),
            "expected StaleRef for out-of-bounds start, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_range_end_exceeds_total_lines() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("short.rs"), "line1\nline2\n").unwrap();
        let adapter = CodeExcerptFsAdapter::new(dir.path().to_path_buf()).unwrap();
        // File has 2 lines, but we ask for line 3 at the end.
        let err = adapter
            .read_lines(&rel("short.rs"), range(1, 3))
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::StaleRef(_)),
            "expected StaleRef for out-of-bounds end, got: {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_crlf_line_endings_are_trimmed() {
        let dir = tempdir().unwrap();
        // Write a file with CRLF line endings.
        std::fs::write(
            dir.path().join("win.rs"),
            "line_one\r\nline_two\r\nline_three\r\n",
        )
        .unwrap();
        let adapter = CodeExcerptFsAdapter::new(dir.path().to_path_buf()).unwrap();
        let snippet = adapter
            .read_lines(&rel("win.rs"), range(1, 3))
            .await
            .unwrap();
        // No \r should appear in the output.
        assert!(
            !snippet.body.contains('\r'),
            "CRLF not trimmed, got: {:?}",
            snippet.body
        );
        assert!(snippet.body.contains("line_one"));
        assert!(snippet.body.contains("line_two"));
    }
}
