use serde_json::{json, Value};

use crate::app::input_usecases::{
    InputUseCases, SnapshotDiagram, SnapshotDocument, SnapshotRef, SnapshotSection, TouchTtlMode,
    UpsertDiagramRequest, UpsertRefRequest, WriteSnapshotRequest,
};
use crate::app::ports::FreshnessState;
use crate::domain::errors::DomainError;
use crate::domain::models::Pack;
use crate::domain::types::Status;

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
    if args.get("snapshot").is_some() {
        let pack = uc
            .write_snapshot(parse_write_snapshot_request(args)?)
            .await?;
        return tool_success("write", serde_json::to_value(pack)?);
    }

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

fn parse_write_snapshot_request(args: &Value) -> Result<WriteSnapshotRequest, DomainError> {
    let snapshot = args.get("snapshot").ok_or_else(|| {
        DomainError::InvalidData("snapshot is required for action='write'".into())
    })?;
    let snapshot_obj = snapshot
        .as_object()
        .ok_or_else(|| DomainError::InvalidData("snapshot must be an object".into()))?;

    let status = match snapshot_obj.get("status").and_then(Value::as_str) {
        Some(raw) => raw.parse::<Status>()?,
        None => Status::Draft,
    };
    let sections = snapshot_obj
        .get("sections")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::InvalidData("snapshot.sections is required".into()))?;

    let mut parsed_sections = Vec::with_capacity(sections.len());
    for section in sections {
        let section_obj = section
            .as_object()
            .ok_or_else(|| DomainError::InvalidData("section must be an object".into()))?;
        let refs = parse_snapshot_refs(section_obj.get("refs"))?;
        let diagrams = parse_snapshot_diagrams(section_obj.get("diagrams"))?;
        parsed_sections.push(SnapshotSection {
            key: req_snapshot_str(section_obj, "key")?,
            title: req_snapshot_str(section_obj, "title")?,
            description: snapshot_opt_str(section_obj, "description"),
            refs,
            diagrams,
        });
    }

    let tags = match snapshot_obj.get("tags") {
        Some(raw_tags) => {
            let tags = raw_tags
                .as_array()
                .ok_or_else(|| DomainError::InvalidData("snapshot.tags must be an array".into()))?;
            let mut parsed = Vec::with_capacity(tags.len());
            for tag in tags {
                let value = tag.as_str().ok_or_else(|| {
                    DomainError::InvalidData("snapshot.tags entries must be strings".into())
                })?;
                parsed.push(value.to_string());
            }
            parsed
        }
        None => Vec::new(),
    };

    let ttl_minutes = snapshot_obj
        .get("ttl_minutes")
        .map(snapshot_u64)
        .transpose()?;

    Ok(WriteSnapshotRequest {
        identifier: str_opt(args, "identifier").or_else(|| str_opt(args, "id")),
        expected_revision: u64_opt(args, "expected_revision")?,
        validate_only: args
            .get("validate_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        snapshot: SnapshotDocument {
            name: snapshot_opt_str(snapshot_obj, "name"),
            title: snapshot_opt_str(snapshot_obj, "title"),
            brief: snapshot_opt_str(snapshot_obj, "brief"),
            tags,
            ttl_minutes,
            status,
            sections: parsed_sections,
        },
    })
}

fn parse_snapshot_refs(raw: Option<&Value>) -> Result<Vec<SnapshotRef>, DomainError> {
    let Some(raw_refs) = raw else {
        return Ok(Vec::new());
    };
    let refs = raw_refs
        .as_array()
        .ok_or_else(|| DomainError::InvalidData("section.refs must be an array".into()))?;
    let mut out = Vec::with_capacity(refs.len());
    for value in refs {
        let obj = value
            .as_object()
            .ok_or_else(|| DomainError::InvalidData("ref must be an object".into()))?;
        out.push(SnapshotRef {
            key: req_snapshot_str(obj, "key")?,
            path: req_snapshot_str(obj, "path")?,
            line_start: req_snapshot_usize(obj, "line_start")?,
            line_end: req_snapshot_usize(obj, "line_end")?,
            title: snapshot_opt_str(obj, "title"),
            why: snapshot_opt_str(obj, "why"),
            group: snapshot_opt_str(obj, "group"),
        });
    }
    Ok(out)
}

fn parse_snapshot_diagrams(raw: Option<&Value>) -> Result<Vec<SnapshotDiagram>, DomainError> {
    let Some(raw_diagrams) = raw else {
        return Ok(Vec::new());
    };
    let diagrams = raw_diagrams
        .as_array()
        .ok_or_else(|| DomainError::InvalidData("section.diagrams must be an array".into()))?;
    let mut out = Vec::with_capacity(diagrams.len());
    for value in diagrams {
        let obj = value
            .as_object()
            .ok_or_else(|| DomainError::InvalidData("diagram must be an object".into()))?;
        out.push(SnapshotDiagram {
            key: req_snapshot_str(obj, "key")?,
            title: req_snapshot_str(obj, "title")?,
            mermaid: req_snapshot_str(obj, "mermaid")?,
            why: snapshot_opt_str(obj, "why"),
        });
    }
    Ok(out)
}

fn req_snapshot_str(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, DomainError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| DomainError::InvalidData(format!("snapshot.{} is required", key)))
}

fn req_snapshot_usize(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<usize, DomainError> {
    let value = obj
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| DomainError::InvalidData(format!("snapshot.{} is required", key)))?;
    usize::try_from(value)
        .map_err(|_| DomainError::InvalidData(format!("snapshot.{} is out of range", key)))
}

fn snapshot_u64(value: &Value) -> Result<u64, DomainError> {
    value
        .as_u64()
        .ok_or_else(|| DomainError::InvalidData("snapshot.ttl_minutes must be an integer".into()))
}

fn snapshot_opt_str(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}
