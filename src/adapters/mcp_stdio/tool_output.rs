use serde_json::Value;

use crate::app::output_usecases::{OutputGetRequest, OutputMode, OutputUseCases};
use crate::domain::errors::DomainError;
use crate::domain::models::Pack;

use super::{req_identifier, status_opt, str_opt, tool_text_success, usize_opt};

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
        .unwrap_or(if has_identity { "get" } else { "list" });

    match action {
        "list" => {
            let status = status_opt(args, "status")?;
            let query = str_opt(args, "query");
            let limit = usize_opt(args, "limit")?;
            let offset = usize_opt(args, "offset")?;
            let packs = uc.list_filtered(status, query, limit, offset).await?;
            tool_text_success(format_pack_list_markdown(&packs))
        }
        "get" => {
            let ident = req_identifier(args)?;
            let request = OutputGetRequest {
                status_filter: status_opt(args, "status")?,
                mode: output_mode_opt(args)?,
                limit: usize_opt(args, "limit")?,
                offset: usize_opt(args, "offset")?,
                cursor: str_opt(args, "cursor"),
                match_regex: str_opt(args, "match"),
            };
            let out_str = uc.get_rendered_with_request(&ident, request).await?;
            tool_text_success(out_str)
        }
        _ => Err(DomainError::InvalidData(format!(
            "unknown output action '{}'",
            action
        ))),
    }
}

pub(super) fn reject_output_format_param(args: &Value) -> Result<(), DomainError> {
    if args.get("format").is_some() {
        return Err(DomainError::InvalidData(
            "'format' is not supported; output is always markdown".into(),
        ));
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
        out.push_str(&format!(
            "- `{}` — {} (revision `{}`, ttl `{}`)\n",
            pack.id, title, pack.revision, ttl
        ));
    }
    out
}

fn output_mode_opt(args: &Value) -> Result<Option<OutputMode>, DomainError> {
    let Some(raw) = args.get("mode").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    Ok(Some(raw.parse::<OutputMode>()?))
}
