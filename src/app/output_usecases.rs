use std::collections::BTreeSet;
use std::fmt::{self, Write as FmtWrite};
use std::str::FromStr;
use std::sync::Arc;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputProfile {
    #[default]
    Orchestrator,
    Reviewer,
    Executor,
}

impl fmt::Display for OutputProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OutputProfile::Orchestrator => write!(f, "orchestrator"),
            OutputProfile::Reviewer => write!(f, "reviewer"),
            OutputProfile::Executor => write!(f, "executor"),
        }
    }
}

impl FromStr for OutputProfile {
    type Err = DomainError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim() {
            "orchestrator" => Ok(Self::Orchestrator),
            "reviewer" => Ok(Self::Reviewer),
            "executor" => Ok(Self::Executor),
            other => Err(DomainError::InvalidData(format!(
                "'profile' must be one of: orchestrator, reviewer, executor (got '{}')",
                other
            ))),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OutputReadRequest {
    pub status_filter: Option<Status>,
    pub profile: Option<OutputProfile>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub page_token: Option<String>,
    pub contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OutputPageTokenV1 {
    v: u8,
    pack_id: String,
    revision: u64,
    next_offset: usize,
    fingerprint: String,
    profile: OutputProfile,
    status_filter: Option<Status>,
    limit: Option<usize>,
    contains: Option<String>,
}

#[derive(Debug, Clone)]
struct EffectiveReadArgs {
    status_filter: Option<Status>,
    profile: OutputProfile,
    mode: OutputMode,
    limit: Option<usize>,
    start_offset: usize,
    contains: Option<String>,
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
            OutputReadRequest {
                status_filter,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn get_rendered_with_request(
        &self,
        identifier: &str,
        request: OutputReadRequest,
    ) -> Result<String> {
        let pack = self.resolve(identifier).await?;
        let args = self.resolve_effective_read_args(&pack, request)?;

        if let Some(required) = args.status_filter {
            if pack.status != required {
                return Err(DomainError::InvalidState(format!(
                    "pack status is '{}', expected '{}'",
                    pack.status, required
                )));
            }
        }

        self.render_pack_advanced(&pack, &args).await
    }

    fn resolve_effective_read_args(
        &self,
        pack: &Pack,
        request: OutputReadRequest,
    ) -> Result<EffectiveReadArgs> {
        if request.page_token.is_some() && request.offset.is_some() {
            return Err(invalid_page_token(
                "provide either 'offset' or 'page_token', not both",
            ));
        }

        let default_profile = request.profile.unwrap_or_default();
        let default_mode = profile_mode(default_profile);
        let contains = normalize_contains(request.contains);
        let paging_requested =
            request.limit.is_some() || request.offset.is_some() || request.page_token.is_some();

        match request.page_token {
            Some(raw_page_token) => {
                let token = decode_page_token_v1(&raw_page_token)?;
                if token.pack_id != pack.id.as_str() {
                    return Err(invalid_page_token("pack id mismatch"));
                }
                if token.revision != pack.revision {
                    return Err(invalid_page_token("pack revision changed"));
                }

                let effective_profile = request.profile.unwrap_or(token.profile);
                let effective_mode = profile_mode(effective_profile);
                let effective_status = request.status_filter.or(token.status_filter);
                let effective_limit = request
                    .limit
                    .or(token.limit)
                    .or_else(|| profile_default_limit(effective_profile));
                let effective_contains = contains.or(token.contains);

                if let Some(limit) = effective_limit {
                    if limit == 0 {
                        return Err(DomainError::InvalidData(
                            "'limit' must be >= 1 when paging is active".into(),
                        ));
                    }
                }

                let fingerprint = request_fingerprint(
                    effective_profile,
                    effective_mode,
                    effective_status,
                    effective_limit,
                    effective_contains.as_deref(),
                );
                if token.fingerprint != fingerprint {
                    return Err(invalid_page_token("request fingerprint mismatch"));
                }

                Ok(EffectiveReadArgs {
                    status_filter: effective_status,
                    profile: effective_profile,
                    mode: effective_mode,
                    limit: effective_limit,
                    start_offset: token.next_offset,
                    contains: effective_contains,
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
                let effective_limit = request
                    .limit
                    .or_else(|| profile_default_limit(default_profile));
                let paging_active = paging_requested || effective_limit.is_some();
                let fingerprint = request_fingerprint(
                    default_profile,
                    default_mode,
                    request.status_filter,
                    effective_limit,
                    contains.as_deref(),
                );
                Ok(EffectiveReadArgs {
                    status_filter: request.status_filter,
                    profile: default_profile,
                    mode: default_mode,
                    limit: effective_limit,
                    start_offset: request.offset.unwrap_or(0),
                    contains,
                    paging_active,
                    fingerprint,
                })
            }
        }
    }

    async fn render_pack_advanced(&self, pack: &Pack, args: &EffectiveReadArgs) -> Result<String> {
        let mut chunks = self.collect_chunks(pack, args.mode).await?;

        if let Some(contains) = args.contains.as_deref() {
            let needle = contains.to_lowercase();
            chunks.retain(|chunk| chunk.searchable_text.to_lowercase().contains(&needle));
        }

        let total_chunks = chunks.len();
        let start = args.start_offset.min(total_chunks);
        let end = match args.limit {
            Some(limit) => start.saturating_add(limit).min(total_chunks),
            None => total_chunks,
        };
        let page_chunks = &chunks[start..end];

        let has_more = end < total_chunks;
        let next_page_token = if args.paging_active && has_more {
            Some(encode_page_token_v1(&OutputPageTokenV1 {
                v: 1,
                pack_id: pack.id.as_str().to_string(),
                revision: pack.revision,
                next_offset: end,
                fingerprint: args.fingerprint.clone(),
                profile: args.profile,
                status_filter: args.status_filter,
                limit: args.limit,
                contains: args.contains.clone(),
            })?)
        } else {
            None
        };

        let mut out = String::with_capacity(2048);
        out.push_str("[LEGEND]\n");
        write_legend_header(&mut out, pack);
        let _ = writeln!(out, "- profile: {}", args.profile);
        if args.mode == OutputMode::Compact {
            let _ = writeln!(out, "- mode: compact");
        }
        if let Some(contains) = &args.contains {
            let _ = writeln!(out, "- contains: {}", contains);
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
            let _ = writeln!(
                out,
                "- next_page_token: {}",
                next_page_token.as_deref().unwrap_or("null")
            );
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
        "  - full_evidence: output read {{\"action\":\"read\",\"id\":\"{}\",\"profile\":\"reviewer\"}}",
        pack.id
    );
    if has_more {
        out.push_str("  - continue_compact: call output read with LEGEND `next_page_token`\n");
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
        gaps.push("compact page is partial; continue via LEGEND next_page_token".to_string());
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

fn invalid_page_token(reason: impl Into<String>) -> DomainError {
    DomainError::InvalidData(format!("invalid_page_token: {}", reason.into()))
}

fn request_fingerprint(
    profile: OutputProfile,
    mode: OutputMode,
    status_filter: Option<Status>,
    limit: Option<usize>,
    contains: Option<&str>,
) -> String {
    format!(
        "profile={}|mode={}|status={}|limit={}|contains={}",
        profile,
        mode,
        status_filter
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        limit
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string()),
        contains.unwrap_or("-")
    )
}

fn encode_page_token_v1(page_token: &OutputPageTokenV1) -> Result<String> {
    let raw = serde_json::to_vec(page_token)
        .map_err(|e| invalid_page_token(format!("serialization error: {}", e)))?;
    Ok(format!("v1:{}", hex_encode(&raw)))
}

fn decode_page_token_v1(raw: &str) -> Result<OutputPageTokenV1> {
    let hex = raw
        .strip_prefix("v1:")
        .ok_or_else(|| invalid_page_token("unsupported version"))?;
    let bytes = hex_decode(hex).map_err(invalid_page_token)?;
    let page_token: OutputPageTokenV1 =
        serde_json::from_slice(&bytes).map_err(|_| invalid_page_token("malformed payload"))?;
    if page_token.v != 1 {
        return Err(invalid_page_token("unsupported version"));
    }
    Ok(page_token)
}

fn profile_mode(profile: OutputProfile) -> OutputMode {
    match profile {
        OutputProfile::Reviewer => OutputMode::Full,
        OutputProfile::Orchestrator | OutputProfile::Executor => OutputMode::Compact,
    }
}

fn profile_default_limit(profile: OutputProfile) -> Option<usize> {
    match profile {
        OutputProfile::Orchestrator => Some(6),
        OutputProfile::Executor => Some(12),
        OutputProfile::Reviewer => None,
    }
}

fn normalize_contains(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
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
