use std::sync::Arc;

use crate::{
    app::ports::{CodeExcerptPort, ListFilter, PackRepositoryPort},
    domain::{
        errors::{DomainError, Result},
        models::Pack,
        types::{PackId, PackName, Status},
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

    // ── identity resolution (same as input, no sharing to keep layers clean) ──

    async fn resolve(&self, identifier: &str) -> Result<Pack> {
        let identifier = identifier.trim();
        if identifier.is_empty() {
            return Err(DomainError::InvalidData(
                "identifier (id or name) is required".into(),
            ));
        }
        if let Ok(id) = PackId::parse(identifier) {
            if let Some(pack) = self.repo.get_by_id(&id).await? {
                return Ok(pack);
            }
            return Err(DomainError::NotFound(format!(
                "pack '{}' not found",
                identifier
            )));
        }
        let name = PackName::new(identifier)?;
        if let Some(pack) = self.repo.get_by_name(&name).await? {
            return Ok(pack);
        }
        Err(DomainError::NotFound(format!(
            "pack '{}' not found",
            identifier
        )))
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
        let mut out = String::new();

        // ── [LEGEND] ──────────────────────────────────────────────────────────
        out.push_str("[LEGEND]\n");
        let title = pack
            .title
            .as_deref()
            .or(pack.name.as_ref().map(|n| n.as_str()))
            .unwrap_or("Untitled");
        out.push_str(&format!("# Context pack: {}\n\n", title));
        out.push_str(&format!("- id: {}\n", pack.id));
        if let Some(name) = &pack.name {
            out.push_str(&format!("- name: {}\n", name));
        }
        out.push_str(&format!("- status: {}\n", pack.status));
        out.push_str(&format!("- revision: {}\n", pack.revision));
        out.push_str(&format!("- expires_at: {}\n", pack.expires_at.to_rfc3339()));
        out.push_str(&format!(
            "- ttl_remaining: {}\n",
            pack.ttl_remaining_human(chrono::Utc::now())
        ));
        if !pack.tags.is_empty() {
            out.push_str(&format!("- tags: {}\n", pack.tags.join(", ")));
        }
        if let Some(brief) = &pack.brief {
            out.push_str(&format!("- brief: {}\n", brief));
        }

        // ── [CONTENT] ─────────────────────────────────────────────────────────
        out.push_str("\n[CONTENT]\n");

        for section in &pack.sections {
            out.push_str(&format!("\n## {} [{}]\n", section.title, section.key));
            if let Some(desc) = &section.description {
                out.push_str(&format!("\n{}\n", desc));
            }

            // refs grouped by `group`
            let groups = Pack::refs_grouped_in_section(section);
            for (group_name, refs) in &groups {
                out.push_str(&format!("\n### group: {}\n", group_name));
                for r in refs {
                    out.push_str(&format!("\n#### {} [{}]\n", r.key, section.key));
                    if let Some(t) = &r.title {
                        out.push_str(&format!("**{}**\n\n", t));
                    }
                    out.push_str(&format!("- path: {}\n", r.path));
                    out.push_str(&format!("- lines: {}-{}\n", r.lines.start, r.lines.end));
                    if let Some(why) = &r.why {
                        out.push_str(&format!("- why: {}\n", why));
                    }

                    // fetch actual code excerpt
                    match self.excerpt.read_lines(&r.path, r.lines).await {
                        Ok(snippet) => {
                            let lang = lang_from_path(r.path.as_str());
                            out.push_str(&format!("\n```{}\n{}\n```\n", lang, snippet.body));
                        }
                        Err(DomainError::StaleRef(msg)) => {
                            out.push_str(&format!("\n> stale ref: {}\n", msg));
                        }
                        Err(e) => return Err(e),
                    }
                }
            }

            // diagrams for this section
            if !section.diagrams.is_empty() {
                out.push_str("\n### Diagrams\n");
                for d in &section.diagrams {
                    out.push_str(&format!("\n#### {}\n", d.title));
                    if let Some(why) = &d.why {
                        out.push_str(&format!("_{}_\n\n", why));
                    }
                    out.push_str(&format!("```mermaid\n{}\n```\n", d.mermaid));
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
