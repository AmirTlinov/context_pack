use serde_json::{json, Value};

use crate::app::input_usecases::{
    InputUseCases, TouchTtlMode, UpsertDiagramRequest, UpsertRefRequest,
};
use crate::app::ports::FreshnessState;
use crate::domain::errors::DomainError;
use crate::domain::models::Pack;

use super::{
    freshness_opt, pack_summary, req_identifier, req_status, req_str, req_u64, req_usize,
    status_opt, str_opt, tags_opt, tool_success, u64_opt, usize_opt,
};

pub(super) async fn handle_input_tool(
    args: &Value,
    uc: &InputUseCases,
) -> Result<Value, DomainError> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    match action {
        "list" => {
            let status = status_opt(args, "status")?;
            let freshness = freshness_opt(args, "freshness")?;
            let query = str_opt(args, "query");
            let limit = usize_opt(args, "limit")?;
            let offset = usize_opt(args, "offset")?;
            let packs = uc
                .list_with_freshness(status, query, limit, offset, freshness)
                .await?;
            let summaries: Vec<Value> = packs.iter().map(pack_summary).collect();
            tool_success(
                "list",
                json!({
                    "count": summaries.len(),
                    "packs": summaries
                }),
            )
        }
        "get" => {
            let ident = req_identifier(args)?;
            let pack = uc.get(&ident).await?;
            tool_success("get", pack_with_freshness_metadata(pack)?)
        }
        "write" => handle_write_action(args, uc).await,
        "ttl" => {
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let ttl_minutes = u64_opt(args, "ttl_minutes")?;
            let extend_minutes = u64_opt(args, "extend_minutes")?;
            let mode = match (ttl_minutes, extend_minutes) {
                (Some(_), Some(_)) => {
                    return Err(DomainError::InvalidData(
                        "provide either 'ttl_minutes' or 'extend_minutes', not both".into(),
                    ));
                }
                (Some(minutes), None) => TouchTtlMode::SetMinutes(minutes),
                (None, Some(minutes)) => TouchTtlMode::ExtendMinutes(minutes),
                (None, None) => {
                    return Err(DomainError::TtlRequired(
                        "'ttl_minutes' or 'extend_minutes' is required".into(),
                    ));
                }
            };
            let pack = uc
                .touch_ttl_checked(&ident, expected_revision, mode)
                .await?;
            tool_success("ttl", serde_json::to_value(pack)?)
        }
        "delete" => {
            let id = req_str(args, "id")?;
            let deleted = uc.delete_pack_file(&id).await?;
            tool_success(
                "delete",
                serde_json::json!({
                    "id": id,
                    "deleted": deleted
                }),
            )
        }
        _ => Err(unsupported_input_action(action)),
    }
}

async fn handle_write_action(args: &Value, uc: &InputUseCases) -> Result<Value, DomainError> {
    let op = req_str(args, "op")?;
    match op.as_str() {
        "create" => {
            let name = str_opt(args, "name");
            let title = str_opt(args, "title");
            let brief = str_opt(args, "brief");
            let tags = tags_opt(args)?;
            let ttl_minutes = req_u64(args, "ttl_minutes")?;
            let pack = uc
                .create_with_tags_ttl(name, title, brief, tags, ttl_minutes)
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        "upsert_section" => {
            reject_legacy_alias(args, "title", "section_title")?;
            reject_legacy_alias(args, "description", "section_description")?;
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let section_key = req_str(args, "section_key")?;
            let title = req_str(args, "section_title")?;
            let description = str_opt(args, "section_description");
            let order = usize_opt(args, "section_order")?;
            let pack = uc
                .upsert_section_checked(
                    &ident,
                    &section_key,
                    title,
                    description,
                    order,
                    expected_revision,
                )
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        "delete_section" => {
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let section_key = req_str(args, "section_key")?;
            let pack = uc
                .delete_section_checked(&ident, &section_key, expected_revision)
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        "upsert_ref" => {
            reject_legacy_alias(args, "title", "ref_title")?;
            reject_legacy_alias(args, "why", "ref_why")?;
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let req = UpsertRefRequest {
                section_key: req_str(args, "section_key")?,
                ref_key: req_str(args, "ref_key")?,
                path: req_str(args, "path")?,
                line_start: req_usize(args, "line_start")?,
                line_end: req_usize(args, "line_end")?,
                title: str_opt(args, "ref_title"),
                why: str_opt(args, "ref_why"),
                group: str_opt(args, "group"),
            };
            let pack = uc
                .upsert_ref_checked(&ident, req, expected_revision)
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        "delete_ref" => {
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let section_key = req_str(args, "section_key")?;
            let ref_key = req_str(args, "ref_key")?;
            let pack = uc
                .delete_ref_checked(&ident, &section_key, &ref_key, expected_revision)
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        "upsert_diagram" => {
            reject_legacy_alias(args, "why", "diagram_why")?;
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let request = UpsertDiagramRequest {
                section_key: req_str(args, "section_key")?,
                diagram_key: req_str(args, "diagram_key")?,
                title: req_str(args, "title")?,
                mermaid: req_str(args, "mermaid")?,
                why: str_opt(args, "diagram_why"),
            };
            let pack = uc
                .upsert_diagram_checked(&ident, request, expected_revision)
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        "set_meta" => {
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let title = str_opt(args, "title");
            let brief = str_opt(args, "brief");
            let tags = tags_opt(args)?;
            let pack = uc
                .set_meta_checked(&ident, title, brief, tags, expected_revision)
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        "set_status" => {
            let ident = req_identifier(args)?;
            let expected_revision = req_u64(args, "expected_revision")?;
            let status = req_status(args, "status")?;
            let pack = uc
                .set_status_checked(&ident, status, expected_revision)
                .await?;
            tool_success("write", serde_json::to_value(pack)?)
        }
        _ => Err(DomainError::InvalidData(format!(
            "unknown write op '{}'; allowed ops: create, upsert_section, delete_section, upsert_ref, delete_ref, upsert_diagram, set_meta, set_status",
            op
        ))),
    }
}

fn unsupported_input_action(action: &str) -> DomainError {
    let hint = match action {
        "create" | "upsert_section" | "delete_section" | "upsert_ref" | "delete_ref"
        | "upsert_diagram" | "set_meta" | "set_status" => {
            Some(format!("use action='write' with op='{}'", action))
        }
        "touch_ttl" => Some("use action='ttl'".to_string()),
        "delete_pack" => Some("use action='delete'".to_string()),
        _ => None,
    };

    if let Some(hint) = hint {
        DomainError::InvalidData(format!(
            "input action '{}' is not supported in v3; {}",
            action, hint
        ))
    } else {
        DomainError::InvalidData(format!(
            "unknown input action '{}'; allowed actions: list, get, write, ttl, delete",
            action
        ))
    }
}

fn pack_with_freshness_metadata(pack: Pack) -> Result<Value, DomainError> {
    let now = chrono::Utc::now();
    let ttl_remaining_seconds = pack.ttl_remaining_seconds(now);
    let ttl_remaining_human = pack.ttl_remaining_human(now);
    let freshness_state = FreshnessState::from_ttl_seconds(ttl_remaining_seconds);
    let mut payload = serde_json::to_value(pack)?;
    let object = payload.as_object_mut().ok_or_else(|| {
        DomainError::InvalidData("internal error: expected pack payload object".into())
    })?;

    object.insert(
        "ttl_remaining_seconds".to_string(),
        Value::from(ttl_remaining_seconds),
    );
    object.insert(
        "ttl_remaining_human".to_string(),
        Value::String(ttl_remaining_human.clone()),
    );
    object.insert(
        "ttl_remaining".to_string(),
        Value::String(ttl_remaining_human),
    );
    object.insert(
        "freshness_state".to_string(),
        serde_json::to_value(freshness_state)?,
    );
    Ok(payload)
}

fn reject_legacy_alias(
    args: &Value,
    legacy_key: &str,
    canonical_key: &str,
) -> Result<(), DomainError> {
    if args.get(legacy_key).is_some() {
        return Err(DomainError::InvalidData(format!(
            "'{}' is not supported; use '{}' instead",
            legacy_key, canonical_key
        )));
    }
    Ok(())
}
