use serde_json::{json, Value};
use std::collections::HashSet;

use crate::app::output_usecases::{OutputProfile, OutputReadRequest, OutputUseCases};
use crate::app::ports::FreshnessState;
use crate::domain::errors::DomainError;
use crate::domain::models::Pack;
use crate::domain::types::PackId;

use super::{freshness_opt, req_identifier, status_opt, str_opt, tool_text_success, usize_opt};

pub(super) async fn handle_output_tool(
    args: &Value,
    uc: &OutputUseCases,
) -> Result<Value, DomainError> {
    reject_output_format_param(args)?;

    let has_identity = args
        .get("id")
        .or_else(|| args.get("name"))
        .and_then(|v| v.as_str())
        .is_some();
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or(if has_identity { "read" } else { "list" });

    match action {
        "list" => {
            let status = status_opt(args, "status")?;
            let freshness = freshness_opt(args, "freshness")?;
            let query = str_opt(args, "query");
            let limit = usize_opt(args, "limit")?;
            let offset = usize_opt(args, "offset")?;
            let packs = uc
                .list_filtered_with_freshness(status, query, limit, offset, freshness)
                .await?;
            tool_text_success(format_pack_list_markdown(&packs))
        }
        "read" => {
            let ident = req_output_identifier(args)?;
            let request = build_output_get_request(args)?;
            let out_str = uc.get_rendered_with_request(&ident, request).await?;
            let out_str = append_selection_metadata(&ident, out_str);
            tool_text_success(out_str)
        }
        _ => Err(unsupported_output_action(action)),
    }
}

pub(super) fn reject_output_format_param(args: &Value) -> Result<(), DomainError> {
    if args.get("format").is_some() {
        return Err(DomainError::DetailedInvalidData {
            message: "'format' is not supported; output is always markdown".into(),
            details: json!({
                "tool": "output",
                "field": "format",
                "required_mode": "markdown",
                "replacement_hint": "omit the format field",
            }),
        });
    }
    Ok(())
}

fn format_pack_list_markdown(packs: &[Pack]) -> String {
    if packs.is_empty() {
        return "No context packs found.".to_string();
    }

    let mut out = String::from("# Context packs\n\n");
    let now = chrono::Utc::now();
    for pack in packs {
        let title = pack
            .title
            .as_deref()
            .or(pack.name.as_ref().map(|n| n.as_str()))
            .unwrap_or("Untitled");
        let ttl = pack.ttl_remaining_human(now);
        let freshness = FreshnessState::from_pack(pack, now);
        out.push_str(&format!(
            "- `{}` — {} (revision `{}`, ttl `{}`, freshness `{}`)",
            pack.id, title, pack.revision, ttl, freshness
        ));
        if let Some(warning) = freshness.warning_text() {
            out.push_str(&format!(" [warning: {}]", warning));
        }
        out.push('\n');
    }
    out
}

fn output_profile_opt(args: &Value) -> Result<Option<OutputProfile>, DomainError> {
    let Some(raw) = args.get("profile").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    Ok(Some(raw.parse::<OutputProfile>()?))
}

fn unsupported_output_action(action: &str) -> DomainError {
    if action == "get" {
        DomainError::DetailedInvalidData {
            message: "output action 'get' is not supported in v3; use action='read'".into(),
            details: json!({
                "tool": "output",
                "action": "unsupported",
                "requested_action": "get",
                "allowed_actions": ["list", "read"],
                "legacy_mapping": {"action": "read"},
            }),
        }
    } else {
        DomainError::DetailedInvalidData {
            message: format!(
                "unknown output action '{}'; allowed actions: list, read",
                action
            ),
            details: json!({
                "tool": "output",
                "action": "unknown",
                "requested_action": action,
                "allowed_actions": ["list", "read"],
            }),
        }
    }
}

fn build_output_get_request(args: &Value) -> Result<OutputReadRequest, DomainError> {
    let status_filter = status_opt(args, "status")?;
    let profile = output_profile_opt(args)?;
    let limit = usize_opt(args, "limit")?;
    let offset = usize_opt(args, "offset")?;
    reject_legacy_read_fields(args)?;
    let page_token = str_opt(args, "page_token");
    let contains = str_opt(args, "contains");

    Ok(OutputReadRequest {
        status_filter,
        profile,
        limit,
        offset,
        page_token,
        contains,
    })
}

fn reject_legacy_read_fields(args: &Value) -> Result<(), DomainError> {
    if args.get("mode").is_some() {
        return Err(DomainError::DetailedInvalidData {
            message:
                "'mode' is not supported in v3 read; use 'profile' (orchestrator|reviewer|executor)"
                    .into(),
            details: json!({
                "tool": "output",
                "action": "read",
                "unsupported_field": "mode",
                "supported_fields": [
                    "profile",
                    "limit",
                    "offset",
                    "page_token",
                    "contains",
                    "id",
                    "name",
                    "status"
                ],
            }),
        });
    }
    if args.get("cursor").is_some() {
        return Err(DomainError::DetailedInvalidData {
            message: "'cursor' is not supported; use 'page_token'".into(),
            details: json!({
                "tool": "output",
                "action": "read",
                "unsupported_field": "cursor",
                "supported_field": "page_token",
            }),
        });
    }
    if args.get("match").is_some() {
        return Err(DomainError::DetailedInvalidData {
            message: "'match' is not supported; use 'contains' substring filter".into(),
            details: json!({
                "tool": "output",
                "action": "read",
                "unsupported_field": "match",
                "supported_field": "contains",
            }),
        });
    }
    Ok(())
}

fn append_selection_metadata(identifier: &str, markdown: String) -> String {
    let selected_status = legend_value(&markdown, "status").unwrap_or_else(|| "unknown".into());
    let selected_revision = legend_value(&markdown, "revision").unwrap_or_else(|| "unknown".into());
    let selected_by = if PackId::parse(identifier.trim()).is_ok() {
        "exact_id"
    } else if selected_status == "finalized" {
        "name_latest_finalized_updated_at_then_revision"
    } else {
        "name_latest_draft_updated_at_then_revision"
    };

    let selection_metadata = [
        ("selected_by", selected_by.to_string()),
        ("selected_revision", selected_revision),
        ("selected_status", selected_status),
    ];
    overwrite_legend_metadata(markdown, &selection_metadata)
}

fn overwrite_legend_metadata(markdown: String, metadata: &[(&str, String)]) -> String {
    let Some((legend_start, legend_end)) = legend_bounds(&markdown) else {
        return append_metadata_without_legend(markdown, metadata);
    };

    let mut seen_keys = HashSet::new();
    let mut rewritten_legend_lines = Vec::new();

    for line in markdown[legend_start..legend_end].lines() {
        if let Some((key, _)) = parse_legend_line(line) {
            if let Some(value) = metadata_value(metadata, key) {
                if seen_keys.insert(key.to_string()) {
                    rewritten_legend_lines.push(format!("- {}: {}", key, value));
                }
                continue;
            }
        }
        rewritten_legend_lines.push(line.to_string());
    }

    for (key, value) in metadata {
        if seen_keys.insert((*key).to_string()) {
            rewritten_legend_lines.push(format!("- {}: {}", key, value));
        }
    }

    let mut out = String::with_capacity(markdown.len() + 96);
    out.push_str(&markdown[..legend_start]);
    for line in rewritten_legend_lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(&markdown[legend_end..]);
    out
}

fn append_metadata_without_legend(mut markdown: String, metadata: &[(&str, String)]) -> String {
    if !markdown.ends_with('\n') {
        markdown.push('\n');
    }
    for (key, value) in metadata {
        markdown.push_str("- ");
        markdown.push_str(key);
        markdown.push_str(": ");
        markdown.push_str(value);
        markdown.push('\n');
    }
    markdown
}

fn req_output_identifier(args: &Value) -> Result<String, DomainError> {
    req_identifier(args).map_err(|err| match err {
        DomainError::InvalidData(_) => DomainError::DetailedInvalidData {
            message: "output read requires 'id' or 'name'".into(),
            details: json!({
                "tool": "output",
                "action": "read",
                "required_fields": ["id", "name"],
                "mutually_interchangeable": ["id", "name"],
            }),
        },
        other => other,
    })
}

fn metadata_value<'a>(metadata: &'a [(&str, String)], key: &str) -> Option<&'a str> {
    metadata.iter().find_map(|(meta_key, value)| {
        if *meta_key == key {
            Some(value.as_str())
        } else {
            None
        }
    })
}

fn legend_bounds(markdown: &str) -> Option<(usize, usize)> {
    let marker = markdown.find("[LEGEND]")?;
    let mut legend_start = marker + "[LEGEND]".len();
    if markdown[legend_start..].starts_with('\n') {
        legend_start += 1;
    }

    let legend_end = markdown[legend_start..]
        .find("\n[CONTENT]\n")
        .map(|offset| legend_start + offset)
        .unwrap_or(markdown.len());
    Some((legend_start, legend_end))
}

fn legend_lines(markdown: &str) -> Option<&str> {
    let (legend_start, legend_end) = legend_bounds(markdown)?;
    Some(&markdown[legend_start..legend_end])
}

fn parse_legend_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("- ")?;
    let (key, value) = rest.split_once(':')?;
    Some((key.trim(), value.trim_start()))
}

fn legend_value(markdown: &str, key: &str) -> Option<String> {
    legend_lines(markdown)?.lines().find_map(|line| {
        let (line_key, value) = parse_legend_line(line)?;
        if line_key == key {
            Some(value.to_string())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::app::output_usecases::OutputProfile;

    use super::{
        append_selection_metadata, build_output_get_request, legend_lines, legend_value,
        reject_legacy_read_fields,
    };

    #[test]
    fn selected_metadata_remains_present_when_content_contains_marker_substrings() {
        let markdown = r#"[LEGEND]
- id: pk_aaaaaaaa
- status: finalized
- revision: 12

[CONTENT]

## Section [sec]

- selected_by: fake-marker
- selected_revision: fake-marker
- selected_status: fake-marker
"#;

        let rendered = append_selection_metadata("named-pack", markdown.to_string());
        let legend = legend_lines(&rendered).expect("legend block should exist");

        assert_eq!(
            legend_value(&rendered, "selected_by").as_deref(),
            Some("name_latest_finalized_updated_at_then_revision")
        );
        assert_eq!(
            legend_value(&rendered, "selected_revision").as_deref(),
            Some("12")
        );
        assert_eq!(
            legend_value(&rendered, "selected_status").as_deref(),
            Some("finalized")
        );
        assert_eq!(legend.matches("- selected_by: ").count(), 1);
        assert_eq!(legend.matches("- selected_revision: ").count(), 1);
        assert_eq!(legend.matches("- selected_status: ").count(), 1);
        assert!(
            rendered.contains("- selected_by: fake-marker"),
            "content marker text must remain untouched"
        );
    }

    #[test]
    fn selected_metadata_is_deterministically_overwritten_in_legend() {
        let markdown = r#"[LEGEND]
- id: pk_aaaaaaaa
- status: draft
- revision: 4
- selected_by: stale-a
- selected_by: stale-b
- selected_revision: 1
- selected_status: finalized

[CONTENT]
noop
"#;

        let rendered = append_selection_metadata("pk_aaaaaaaa", markdown.to_string());
        let legend = legend_lines(&rendered).expect("legend block should exist");

        assert_eq!(legend.matches("- selected_by: ").count(), 1);
        assert_eq!(legend.matches("- selected_revision: ").count(), 1);
        assert_eq!(legend.matches("- selected_status: ").count(), 1);
        assert_eq!(
            legend_value(&rendered, "selected_by").as_deref(),
            Some("exact_id")
        );
        assert_eq!(
            legend_value(&rendered, "selected_revision").as_deref(),
            Some("4")
        );
        assert_eq!(
            legend_value(&rendered, "selected_status").as_deref(),
            Some("draft")
        );
    }

    #[test]
    fn default_output_read_uses_orchestrator_profile() {
        let request = build_output_get_request(&json!({ "id": "pk_aaaaaaaa" }))
            .expect("default request must parse");
        assert_eq!(request.profile, None);
        assert_eq!(request.limit, None);
    }

    #[test]
    fn explicit_profile_is_parsed() {
        let request = build_output_get_request(&json!({
            "id": "pk_aaaaaaaa",
            "profile": "reviewer"
        }))
        .expect("explicit profile request must parse");
        assert_eq!(request.profile, Some(OutputProfile::Reviewer));
        assert_eq!(request.limit, None);
    }

    #[test]
    fn page_token_is_forwarded_without_profile_override() {
        let request = build_output_get_request(&json!({
            "id": "pk_aaaaaaaa",
            "page_token": "v1:deadbeef"
        }))
        .expect("page_token request must parse");
        assert_eq!(request.profile, None);
        assert_eq!(request.limit, None);
        assert_eq!(request.page_token.as_deref(), Some("v1:deadbeef"));
    }

    #[test]
    fn legacy_read_fields_are_rejected() {
        let mode = reject_legacy_read_fields(&json!({"mode":"full"}))
            .expect_err("mode should be rejected");
        assert!(mode.to_string().contains("'mode' is not supported"));

        let cursor = reject_legacy_read_fields(&json!({"cursor":"v1:x"}))
            .expect_err("cursor should be rejected");
        assert!(cursor.to_string().contains("'cursor' is not supported"));

        let r#match = reject_legacy_read_fields(&json!({"match":"foo"}))
            .expect_err("match should be rejected");
        assert!(r#match.to_string().contains("'match' is not supported"));
    }
}
