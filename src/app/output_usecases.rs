use std::fmt::Write as FmtWrite;
use std::sync::Arc;

use crate::{
    app::{
        ports::{CodeExcerptPort, ListFilter, PackRepositoryPort},
        resolver::resolve_pack,
    },
    domain::{
        errors::{DomainError, Result},
        models::Pack,
        types::Status,
    },
};

pub struct OutputUseCases {
    repo: Arc<dyn PackRepositoryPort>,
    excerpt: Arc<dyn CodeExcerptPort>,
}

impl OutputUseCases {
    pub fn new(repo: Arc<dyn PackRepositoryPort>, excerpt: Arc<dyn CodeExcerptPort>) -> Self {
        Self { repo, excerpt }
    }

    // ── identity resolution ───────────────────────────────────────────────────

    async fn resolve(&self, identifier: &str) -> Result<Pack> {
        resolve_pack(self.repo.as_ref(), identifier).await
    }

    // ── list ──────────────────────────────────────────────────────────────────

    pub async fn list_filtered(
        &self,
        status: Option<Status>,
        query: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<Pack>> {
        self.repo
            .list_packs(ListFilter {
                status,
                query,
                limit,
                offset,
            })
            .await
    }

    // ── render ────────────────────────────────────────────────────────────────

    pub async fn get_rendered(
        &self,
        identifier: &str,
        status_filter: Option<Status>,
    ) -> Result<String> {
        let pack = self.resolve(identifier).await?;

        if let Some(required) = status_filter {
            if pack.status != required {
                return Err(DomainError::InvalidState(format!(
                    "pack status is '{}', expected '{}'",
                    pack.status, required
                )));
            }
        }

        self.render_pack(&pack).await
    }

    async fn render_pack(&self, pack: &Pack) -> Result<String> {
        let mut out = String::with_capacity(2048);

        // ── [LEGEND] ──────────────────────────────────────────────────────────
        out.push_str("[LEGEND]\n");
        let title = pack
            .title
            .as_deref()
            .or(pack.name.as_ref().map(|n| n.as_str()))
            .unwrap_or("Untitled");
        let _ = write!(out, "# Context pack: {}\n\n", title);
        let _ = writeln!(out, "- id: {}", pack.id);
        if let Some(name) = &pack.name {
            let _ = writeln!(out, "- name: {}", name);
        }
        let _ = writeln!(out, "- status: {}", pack.status);
        let _ = writeln!(out, "- revision: {}", pack.revision);
        let _ = writeln!(out, "- expires_at: {}", pack.expires_at.to_rfc3339());
        let _ = writeln!(
            out,
            "- ttl_remaining: {}",
            pack.ttl_remaining_human(chrono::Utc::now())
        );
        if !pack.tags.is_empty() {
            let _ = writeln!(out, "- tags: {}", pack.tags.join(", "));
        }
        if let Some(brief) = &pack.brief {
            let _ = writeln!(out, "- brief: {}", brief);
        }

        // ── [CONTENT] ─────────────────────────────────────────────────────────
        out.push_str("\n[CONTENT]\n");

        for section in &pack.sections {
            let _ = write!(out, "\n## {} [{}]\n", section.title, section.key);
            if let Some(desc) = &section.description {
                let _ = write!(out, "\n{}\n", desc);
            }

            // refs grouped by `group`
            let groups = Pack::refs_grouped_in_section(section);
            for (group_name, refs) in &groups {
                let _ = write!(out, "\n### group: {}\n", group_name);
                for r in refs {
                    let _ = write!(out, "\n#### {} [{}]\n", r.key, section.key);
                    if let Some(t) = &r.title {
                        let _ = write!(out, "**{}**\n\n", t);
                    }
                    let _ = writeln!(out, "- path: {}", r.path);
                    let _ = writeln!(out, "- lines: {}-{}", r.lines.start, r.lines.end);
                    if let Some(why) = &r.why {
                        let _ = writeln!(out, "- why: {}", why);
                    }

                    // fetch actual code excerpt
                    match self.excerpt.read_lines(&r.path, r.lines).await {
                        Ok(snippet) => {
                            let lang = lang_from_path(r.path.as_str());
                            let _ = write!(out, "\n```{}\n{}\n```\n", lang, snippet.body);
                        }
                        Err(DomainError::StaleRef(msg)) => {
                            let _ = write!(out, "\n> stale ref: {}\n", msg);
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            // diagrams for this section
            if !section.diagrams.is_empty() {
                out.push_str("\n### Diagrams\n");
                for d in &section.diagrams {
                    let _ = write!(out, "\n#### {}\n", d.title);
                    if let Some(why) = &d.why {
                        let _ = write!(out, "_{}_\n\n", why);
                    }
                    let _ = write!(out, "```mermaid\n{}\n```\n", d.mermaid);
                }
            }
        }

        Ok(out)
    }
}

fn lang_from_path(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "sh" | "bash" => "bash",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "sql" => "sql",
        "md" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "proto" => "protobuf",
        _ => "",
    }
}
