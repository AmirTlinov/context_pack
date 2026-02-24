use std::collections::BTreeSet;
use std::fmt::{self, Write as FmtWrite};
use std::str::FromStr;
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
    app::{
        ports::{CodeExcerptPort, FreshnessState, ListFilter, PackRepositoryPort},
        resolver::resolve_pack,
    },
    domain::{
        errors::{DomainError, Result},
        models::Pack,
        types::Status,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    #[default]
    Full,
    Compact,
}

impl fmt::Display for OutputMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutputMode::Full => write!(f, "full"),
            OutputMode::Compact => write!(f, "compact"),
        }
    }
}

impl FromStr for OutputMode {
    type Err = DomainError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim() {
            "full" => Ok(Self::Full),
            "compact" => Ok(Self::Compact),
            other => Err(DomainError::InvalidData(format!(
                "'mode' must be one of: full, compact (got '{}')",
                other
            ))),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OutputGetRequest {
    pub status_filter: Option<Status>,
    pub mode: Option<OutputMode>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub cursor: Option<String>,
    pub match_regex: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputCursorV1 {
    v: u8,
    pack_id: String,
    revision: u64,
    next_offset: usize,
    fingerprint: String,
    mode: OutputMode,
    status_filter: Option<Status>,
    limit: Option<usize>,
    match_regex: Option<String>,
}

#[derive(Debug, Clone)]
struct EffectiveGetArgs {
    status_filter: Option<Status>,
    mode: OutputMode,
    limit: Option<usize>,
    start_offset: usize,
    match_regex: Option<String>,
    paging_active: bool,
    fingerprint: String,
}

#[derive(Debug, Clone)]
enum ChunkKind {
    Ref { group: String },
    Diagram,
}

#[derive(Debug, Clone)]
struct RenderChunk {
    section_title: String,
    section_key: String,
    section_description: Option<String>,
    kind: ChunkKind,
    ref_key: Option<String>,
    stale_ref: bool,
    body_markdown: String,
    searchable_text: String,
}

const COMPACT_SIGNAL_LIMIT: usize = 3;
const COMPACT_NAV_HINT_LIMIT: usize = 5;

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
        self.list_filtered_with_freshness(status, query, limit, offset, None)
            .await
    }

    pub async fn list_filtered_with_freshness(
        &self,
        status: Option<Status>,
        query: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
        freshness: Option<FreshnessState>,
    ) -> Result<Vec<Pack>> {
        self.repo
            .list_packs(ListFilter {
                status,
                freshness,
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
        self.get_rendered_with_request(
            identifier,
            OutputGetRequest {
                status_filter,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn get_rendered_with_request(
        &self,
        identifier: &str,
        request: OutputGetRequest,
    ) -> Result<String> {
        let pack = self.resolve(identifier).await?;
        let args = self.resolve_effective_get_args(&pack, request)?;

        if let Some(required) = args.status_filter {
            if pack.status != required {
                return Err(DomainError::InvalidState(format!(
                    "pack status is '{}', expected '{}'",
                    pack.status, required
                )));
            }
        }

        let regex = compile_match_regex(args.match_regex.as_deref())?;

        if !args.paging_active && args.mode == OutputMode::Full && regex.is_none() {
            return self.render_pack(&pack).await;
        }

        self.render_pack_advanced(&pack, &args, regex.as_ref())
            .await
    }

    fn resolve_effective_get_args(
        &self,
        pack: &Pack,
        request: OutputGetRequest,
    ) -> Result<EffectiveGetArgs> {
        if request.cursor.is_some() && request.offset.is_some() {
            return Err(invalid_cursor(
                "provide either 'offset' or 'cursor', not both",
            ));
        }

        let default_mode = request.mode.unwrap_or_default();
        let paging_requested =
            request.limit.is_some() || request.offset.is_some() || request.cursor.is_some();

        match request.cursor {
            Some(raw_cursor) => {
                let cursor = decode_cursor_v1(&raw_cursor)?;
                if cursor.pack_id != pack.id.as_str() {
                    return Err(invalid_cursor("pack id mismatch"));
                }
                if cursor.revision != pack.revision {
                    return Err(invalid_cursor("pack revision changed"));
                }

                let effective_mode = request.mode.unwrap_or(cursor.mode);
                let effective_status = request.status_filter.or(cursor.status_filter);
                let effective_limit = request.limit.or(cursor.limit);
                let effective_match = request.match_regex.or(cursor.match_regex);

                if let Some(limit) = effective_limit {
                    if limit == 0 {
                        return Err(DomainError::InvalidData(
                            "'limit' must be >= 1 when paging is active".into(),
                        ));
                    }
                }

                let fingerprint = request_fingerprint(
                    effective_mode,
                    effective_status,
                    effective_limit,
                    effective_match.as_deref(),
                );
                if cursor.fingerprint != fingerprint {
                    return Err(invalid_cursor("request fingerprint mismatch"));
                }

                Ok(EffectiveGetArgs {
                    status_filter: effective_status,
                    mode: effective_mode,
                    limit: effective_limit,
                    start_offset: cursor.next_offset,
                    match_regex: effective_match,
                    paging_active: true,
                    fingerprint,
                })
            }
            None => {
                if paging_requested {
                    if let Some(limit) = request.limit {
                        if limit == 0 {
                            return Err(DomainError::InvalidData(
                                "'limit' must be >= 1 when paging is active".into(),
                            ));
                        }
                    }
                }
                let fingerprint = request_fingerprint(
                    default_mode,
                    request.status_filter,
                    request.limit,
                    request.match_regex.as_deref(),
                );
                Ok(EffectiveGetArgs {
                    status_filter: request.status_filter,
                    mode: default_mode,
                    limit: request.limit,
                    start_offset: request.offset.unwrap_or(0),
                    match_regex: request.match_regex,
                    paging_active: paging_requested,
                    fingerprint,
                })
            }
        }
    }

    async fn render_pack(&self, pack: &Pack) -> Result<String> {
        let mut out = String::with_capacity(2048);

        // ── [LEGEND] ──────────────────────────────────────────────────────────
        out.push_str("[LEGEND]\n");
        write_legend_header(&mut out, pack);

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

    async fn render_pack_advanced(
        &self,
        pack: &Pack,
        args: &EffectiveGetArgs,
        regex: Option<&Regex>,
    ) -> Result<String> {
        let mut chunks = self.collect_chunks(pack, args.mode).await?;

        if let Some(matcher) = regex {
            chunks.retain(|chunk| matcher.is_match(&chunk.searchable_text));
        }

        let total_chunks = chunks.len();
        let start = args.start_offset.min(total_chunks);
        let end = match args.limit {
            Some(limit) => start.saturating_add(limit).min(total_chunks),
            None => total_chunks,
        };
        let page_chunks = &chunks[start..end];

        let has_more = end < total_chunks;
        let next_cursor = if args.paging_active && has_more {
            Some(encode_cursor_v1(&OutputCursorV1 {
                v: 1,
                pack_id: pack.id.as_str().to_string(),
                revision: pack.revision,
                next_offset: end,
                fingerprint: args.fingerprint.clone(),
                mode: args.mode,
                status_filter: args.status_filter,
                limit: args.limit,
                match_regex: args.match_regex.clone(),
            })?)
        } else {
            None
        };

        let mut out = String::with_capacity(2048);
        out.push_str("[LEGEND]\n");
        write_legend_header(&mut out, pack);
        if args.mode == OutputMode::Compact {
            let _ = writeln!(out, "- mode: compact");
        }
        if let Some(pattern) = &args.match_regex {
            let _ = writeln!(out, "- match: {}", pattern);
        }
        if args.paging_active {
            let _ = writeln!(out, "- paging: active");
            let _ = writeln!(out, "- offset: {}", start);
            match args.limit {
                Some(limit) => {
                    let _ = writeln!(out, "- limit: {}", limit);
                }
                None => {
                    let _ = writeln!(out, "- limit: all");
                }
            }
            let _ = writeln!(
                out,
                "- has_more: {}",
                if has_more { "true" } else { "false" }
            );
            let _ = writeln!(out, "- next: {}", next_cursor.as_deref().unwrap_or("null"));
            let _ = writeln!(out, "- chunks_total: {}", total_chunks);
            let _ = writeln!(out, "- chunks_returned: {}", page_chunks.len());
        }

        out.push_str("\n[CONTENT]\n");
        if args.mode == OutputMode::Compact {
            write_compact_handoff_summary(&mut out, pack, &chunks, page_chunks, has_more);
        }

        let mut current_section_key: Option<&str> = None;
        let mut current_group: Option<&str> = None;
        let mut diagrams_open = false;

        for chunk in page_chunks {
            if current_section_key != Some(chunk.section_key.as_str()) {
                current_section_key = Some(chunk.section_key.as_str());
                current_group = None;
                diagrams_open = false;

                let _ = write!(
                    out,
                    "\n## {} [{}]\n",
                    chunk.section_title, chunk.section_key
                );
                if let Some(desc) = &chunk.section_description {
                    let _ = write!(out, "\n{}\n", desc);
                }
            }

            match &chunk.kind {
                ChunkKind::Ref { group } => {
                    if current_group != Some(group.as_str()) {
                        let _ = write!(out, "\n### group: {}\n", group);
                        current_group = Some(group.as_str());
                    }
                    out.push_str(&chunk.body_markdown);
                }
                ChunkKind::Diagram => {
                    if !diagrams_open {
                        out.push_str("\n### Diagrams\n");
                        diagrams_open = true;
                        current_group = None;
                    }
                    out.push_str(&chunk.body_markdown);
                }
            }
        }

        if page_chunks.is_empty() {
            out.push_str("\n_No chunks matched current filters._\n");
        }

        Ok(out)
    }

    async fn collect_chunks(&self, pack: &Pack, mode: OutputMode) -> Result<Vec<RenderChunk>> {
        let mut chunks = Vec::new();

        for section in &pack.sections {
            let section_key = section.key.as_str().to_string();
            let section_title = section.title.clone();
            let section_description = section.description.clone();

            let groups = Pack::refs_grouped_in_section(section);
            for (group_name, refs) in &groups {
                for r in refs {
                    let mut body_markdown = String::new();
                    let mut searchable_text = String::new();

                    let _ = write!(body_markdown, "\n#### {} [{}]\n", r.key, section.key);
                    if let Some(t) = &r.title {
                        let _ = write!(body_markdown, "**{}**\n\n", t);
                        let _ = writeln!(searchable_text, "{}", t);
                    }
                    let _ = writeln!(body_markdown, "- path: {}", r.path);
                    let _ = writeln!(body_markdown, "- lines: {}-{}", r.lines.start, r.lines.end);
                    let _ = writeln!(searchable_text, "{}", r.path);
                    let _ = writeln!(searchable_text, "{}-{}", r.lines.start, r.lines.end);
                    if let Some(why) = &r.why {
                        let _ = writeln!(body_markdown, "- why: {}", why);
                        let _ = writeln!(searchable_text, "{}", why);
                    }

                    match self.excerpt.read_lines(&r.path, r.lines).await {
                        Ok(snippet) => {
                            let _ = writeln!(searchable_text, "{}", snippet.body);
                            if mode == OutputMode::Full {
                                let lang = lang_from_path(r.path.as_str());
                                let _ =
                                    write!(body_markdown, "\n```{}\n{}\n```\n", lang, snippet.body);
                            }
                        }
                        Err(DomainError::StaleRef(msg)) => {
                            let _ = write!(body_markdown, "\n> stale ref: {}\n", msg);
                            let _ = writeln!(searchable_text, "{}", msg);
                        }
                        Err(e) => return Err(e),
                    }

                    chunks.push(RenderChunk {
                        section_title: section_title.clone(),
                        section_key: section_key.clone(),
                        section_description: section_description.clone(),
                        kind: ChunkKind::Ref {
                            group: group_name.clone(),
                        },
                        ref_key: Some(r.key.as_str().to_string()),
                        stale_ref: body_markdown.contains("> stale ref:"),
                        body_markdown,
                        searchable_text,
                    });
                }
            }

            for diagram in &section.diagrams {
                let mut body_markdown = String::new();
                let mut searchable_text = String::new();

                let _ = write!(body_markdown, "\n#### {}\n", diagram.title);
                let _ = writeln!(searchable_text, "{}", diagram.title);
                if let Some(why) = &diagram.why {
                    let _ = write!(body_markdown, "_{}_\n\n", why);
                    let _ = writeln!(searchable_text, "{}", why);
                }
                let _ = write!(body_markdown, "```mermaid\n{}\n```\n", diagram.mermaid);
                let _ = writeln!(searchable_text, "{}", diagram.mermaid);

                chunks.push(RenderChunk {
                    section_title: section_title.clone(),
                    section_key: section_key.clone(),
                    section_description: section_description.clone(),
                    kind: ChunkKind::Diagram,
                    ref_key: None,
                    stale_ref: false,
                    body_markdown,
                    searchable_text,
                });
            }
        }

        Ok(chunks)
    }
}

fn compile_match_regex(pattern: Option<&str>) -> Result<Option<Regex>> {
    let Some(pattern) = pattern else {
        return Ok(None);
    };
    Regex::new(pattern)
        .map(Some)
        .map_err(|e| DomainError::InvalidData(format!("invalid regex in 'match': {}", e)))
}

fn write_legend_header(out: &mut String, pack: &Pack) {
    let title = pack
        .title
        .as_deref()
        .or(pack.name.as_ref().map(|n| n.as_str()))
        .unwrap_or("Untitled");
    let now = chrono::Utc::now();
    let freshness_state = FreshnessState::from_pack(pack, now);
    let _ = write!(out, "# Context pack: {}\n\n", title);
    let _ = writeln!(out, "- id: {}", pack.id);
    if let Some(name) = &pack.name {
        let _ = writeln!(out, "- name: {}", name);
    }
    let _ = writeln!(out, "- status: {}", pack.status);
    let _ = writeln!(out, "- revision: {}", pack.revision);
    let _ = writeln!(out, "- expires_at: {}", pack.expires_at.to_rfc3339());
    let _ = writeln!(out, "- ttl_remaining: {}", pack.ttl_remaining_human(now));
    let _ = writeln!(out, "- freshness_state: {}", freshness_state);
    if let Some(warning) = freshness_state.warning_text() {
        let _ = writeln!(out, "- warning: {}", warning);
    }
    if !pack.tags.is_empty() {
        let _ = writeln!(out, "- tags: {}", pack.tags.join(", "));
    }
    if let Some(brief) = &pack.brief {
        let _ = writeln!(out, "- brief: {}", brief);
    }
}

fn write_compact_handoff_summary(
    out: &mut String,
    pack: &Pack,
    filtered_chunks: &[RenderChunk],
    page_chunks: &[RenderChunk],
    has_more: bool,
) {
    let now = chrono::Utc::now();
    let freshness_state = FreshnessState::from_pack(pack, now);
    let objective = pack
        .title
        .as_deref()
        .or(pack.name.as_ref().map(|name| name.as_str()))
        .unwrap_or("Untitled")
        .to_string();
    let scope = compact_scope(pack);
    let risks = compact_risk_signals(pack, filtered_chunks, freshness_state);
    let gaps = compact_gap_signals(pack, has_more);
    let sections = compact_section_hints(pack);
    let refs_on_page = compact_ref_hints(page_chunks);

    out.push_str("\n## Handoff summary [handoff]\n");
    let _ = writeln!(out, "- objective: {}", objective);
    let _ = writeln!(out, "- scope: {}", scope);
    let _ = writeln!(
        out,
        "- verdict_status: status={}, freshness_state={}",
        pack.status, freshness_state
    );
    let _ = writeln!(
        out,
        "- freshness: expires_at={}, ttl_remaining={}",
        pack.expires_at.to_rfc3339(),
        pack.ttl_remaining_human(now)
    );
    out.push_str("- top_risks:\n");
    for risk in risks {
        let _ = writeln!(out, "  - {}", risk);
    }
    out.push_str("- top_gaps:\n");
    for gap in gaps {
        let _ = writeln!(out, "  - {}", gap);
    }
    out.push_str("- deep_nav_hints:\n");
    let _ = writeln!(
        out,
        "  - full_evidence: output get {{\"action\":\"get\",\"id\":\"{}\",\"mode\":\"full\"}}",
        pack.id
    );
    if has_more {
        out.push_str("  - continue_compact: call output get with LEGEND `next` cursor\n");
    }
    let _ = writeln!(
        out,
        "  - sections: {}",
        if sections.is_empty() {
            "none".to_string()
        } else {
            sections.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "  - refs_on_page: {}",
        if refs_on_page.is_empty() {
            "none".to_string()
        } else {
            refs_on_page.join(", ")
        }
    );
}

fn compact_scope(pack: &Pack) -> String {
    if let Some(brief) = &pack.brief {
        if !brief.trim().is_empty() {
            return brief.trim().to_string();
        }
    }

    let mut section_titles = pack
        .sections
        .iter()
        .map(|section| section.title.trim())
        .filter(|title| !title.is_empty())
        .take(COMPACT_NAV_HINT_LIMIT)
        .collect::<Vec<_>>();
    if section_titles.is_empty() {
        "scope is not explicitly documented in pack brief".to_string()
    } else {
        section_titles.dedup();
        format!("sections in scope: {}", section_titles.join(", "))
    }
}

fn compact_risk_signals(
    pack: &Pack,
    filtered_chunks: &[RenderChunk],
    freshness_state: FreshnessState,
) -> Vec<String> {
    let mut risks = Vec::new();

    if let Some(warning) = freshness_state.warning_text() {
        risks.push(format!("freshness: {}", warning));
    }

    let stale_ref_keys = filtered_chunks
        .iter()
        .filter(|chunk| chunk.stale_ref)
        .filter_map(|chunk| chunk.ref_key.as_deref())
        .take(COMPACT_SIGNAL_LIMIT)
        .collect::<Vec<_>>();
    if !stale_ref_keys.is_empty() {
        risks.push(format!("stale refs: {}", stale_ref_keys.join(", ")));
    }

    risks.extend(keyword_signals(
        pack,
        &[
            "risk", "risky", "blocker", "critical", "incident", "warning",
        ],
        COMPACT_SIGNAL_LIMIT.saturating_sub(risks.len()),
    ));

    if risks.is_empty() {
        risks.push("none explicitly signaled".to_string());
    }
    risks.truncate(COMPACT_SIGNAL_LIMIT);
    risks
}

fn compact_gap_signals(pack: &Pack, has_more: bool) -> Vec<String> {
    let mut gaps = keyword_signals(
        pack,
        &[
            "gap",
            "missing",
            "unknown",
            "todo",
            "tbd",
            "follow-up",
            "followup",
            "fixme",
        ],
        COMPACT_SIGNAL_LIMIT,
    );

    if has_more && gaps.len() < COMPACT_SIGNAL_LIMIT {
        gaps.push("compact page is partial; continue via LEGEND next cursor".to_string());
    }

    if gaps.is_empty() {
        gaps.push("none explicitly tagged".to_string());
    }

    gaps.truncate(COMPACT_SIGNAL_LIMIT);
    gaps
}

fn compact_section_hints(pack: &Pack) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut sections = Vec::new();
    for section in &pack.sections {
        let hint = format!("{}[{}]", section.title, section.key);
        if seen.insert(hint.clone()) {
            sections.push(hint);
        }
        if sections.len() >= COMPACT_NAV_HINT_LIMIT {
            break;
        }
    }
    sections
}

fn compact_ref_hints(page_chunks: &[RenderChunk]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut refs = Vec::new();
    for chunk in page_chunks {
        let Some(ref_key) = &chunk.ref_key else {
            continue;
        };
        if seen.insert(ref_key.clone()) {
            refs.push(ref_key.clone());
        }
        if refs.len() >= COMPACT_NAV_HINT_LIMIT {
            break;
        }
    }
    refs
}

fn keyword_signals(pack: &Pack, keywords: &[&str], limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }

    let mut seen_normalized = BTreeSet::new();
    let mut out = Vec::new();
    for candidate in pack_text_candidates(pack) {
        let normalized = candidate.to_lowercase();
        if !keywords.iter().any(|keyword| normalized.contains(keyword)) {
            continue;
        }
        if !seen_normalized.insert(normalized) {
            continue;
        }
        out.push(truncate_signal(candidate, 140));
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn pack_text_candidates(pack: &Pack) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(brief) = pack.brief.as_deref() {
        out.push(brief);
    }
    for section in &pack.sections {
        out.push(section.title.as_str());
        if let Some(description) = section.description.as_deref() {
            out.push(description);
        }
        for code_ref in &section.refs {
            out.push(code_ref.key.as_str());
            if let Some(title) = code_ref.title.as_deref() {
                out.push(title);
            }
            if let Some(why) = code_ref.why.as_deref() {
                out.push(why);
            }
        }
        for diagram in &section.diagrams {
            out.push(diagram.title.as_str());
            if let Some(why) = diagram.why.as_deref() {
                out.push(why);
            }
        }
    }
    out
}

fn truncate_signal(raw: &str, max_chars: usize) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut out = String::new();
    for ch in compact.chars().take(max_chars.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn invalid_cursor(reason: impl Into<String>) -> DomainError {
    DomainError::InvalidData(format!("invalid_cursor: {}", reason.into()))
}

fn request_fingerprint(
    mode: OutputMode,
    status_filter: Option<Status>,
    limit: Option<usize>,
    match_regex: Option<&str>,
) -> String {
    format!(
        "mode={}|status={}|limit={}|match={}",
        mode,
        status_filter
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        limit
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        match_regex.unwrap_or("-")
    )
}

fn encode_cursor_v1(cursor: &OutputCursorV1) -> Result<String> {
    let raw = serde_json::to_vec(cursor)
        .map_err(|e| invalid_cursor(format!("serialization error: {}", e)))?;
    Ok(format!("v1:{}", hex_encode(&raw)))
}

fn decode_cursor_v1(raw: &str) -> Result<OutputCursorV1> {
    let hex = raw
        .strip_prefix("v1:")
        .ok_or_else(|| invalid_cursor("unsupported version"))?;
    let bytes = hex_decode(hex).map_err(invalid_cursor)?;
    let cursor: OutputCursorV1 =
        serde_json::from_slice(&bytes).map_err(|_| invalid_cursor("malformed payload"))?;
    if cursor.v != 1 {
        return Err(invalid_cursor("unsupported version"));
    }
    Ok(cursor)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

fn hex_decode(raw: &str) -> std::result::Result<Vec<u8>, String> {
    if !raw.len().is_multiple_of(2) {
        return Err("hex length must be even".into());
    }

    let mut out = Vec::with_capacity(raw.len() / 2);
    let bytes = raw.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        let hi = decode_hex_digit(bytes[idx])
            .ok_or_else(|| format!("invalid hex digit '{}'", bytes[idx] as char))?;
        let lo = decode_hex_digit(bytes[idx + 1])
            .ok_or_else(|| format!("invalid hex digit '{}'", bytes[idx + 1] as char))?;
        out.push((hi << 4) | lo);
        idx += 2;
    }
    Ok(out)
}

fn decode_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
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
