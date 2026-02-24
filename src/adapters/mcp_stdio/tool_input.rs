use serde_json::{json, Value};

use crate::app::input_usecases::{
    InputUseCases, SnapshotDiagram, SnapshotDocument, SnapshotRef, SnapshotSection, TouchTtlMode,
    WriteSnapshotRequest,
};
use crate::app::ports::FreshnessState;
use crate::domain::errors::DomainError;
use crate::domain::models::Pack;
use crate::domain::types::Status;

use super::{
    freshness_opt, pack_summary, req_identifier, req_u64, status_opt, str_opt, tool_success,
    u64_opt, usize_opt,
};

const INPUT_ALLOWED_ACTIONS: [&str; 5] = ["list", "get", "write", "ttl", "delete"];

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
            let ident = req_pack_identifier(args, "input", "get")?;
            let pack = uc.get(&ident).await?;
            tool_success("get", pack_with_freshness_metadata(pack)?)
        }
        "write" => handle_write_action(args, uc).await,
        "ttl" => {
            let ident = req_pack_identifier(args, "input", "ttl")?;
            let expected_revision = req_expected_revision(args)?;
            let ttl_minutes = u64_opt(args, "ttl_minutes")?;
            let extend_minutes = u64_opt(args, "extend_minutes")?;
            let mode = match (ttl_minutes, extend_minutes) {
                (Some(_), Some(_)) => {
                    return Err(DomainError::DetailedInvalidData {
                        message:
                            "input ttl requires exactly one of 'ttl_minutes' or 'extend_minutes'"
                                .into(),
                        details: json!({
                            "action": "ttl",
                            "required_fields": ["ttl_minutes", "extend_minutes"],
                            "required_mode": "exactly_one_of",
                            "provided_fields": [
                                "ttl_minutes",
                                "extend_minutes"
                            ],
                        }),
                    });
                }
                (Some(minutes), None) => TouchTtlMode::SetMinutes(minutes),
                (None, Some(minutes)) => TouchTtlMode::ExtendMinutes(minutes),
                (None, None) => {
                    return Err(DomainError::DetailedInvalidData {
                        message: "input ttl requires 'ttl_minutes' or 'extend_minutes'".into(),
                        details: json!({
                            "action": "ttl",
                            "required_fields": ["ttl_minutes", "extend_minutes"],
                            "required_mode": "exactly_one_of",
                        }),
                    });
                }
            };
            let pack = uc
                .touch_ttl_checked(&ident, expected_revision, mode)
                .await?;
            tool_success("ttl", serde_json::to_value(pack)?)
        }
        "delete" => {
            let ident = req_pack_identifier(args, "input", "delete")?;
            let deleted = uc.delete_pack_file(&ident).await?;
            tool_success(
                "delete",
                serde_json::json!({
                    "id": ident,
                    "deleted": deleted
                }),
            )
        }
        _ => Err(unsupported_input_action(action)),
    }
}

async fn handle_write_action(args: &Value, uc: &InputUseCases) -> Result<Value, DomainError> {
    reject_legacy_write_contract(args)?;
    let request = parse_write_snapshot_request(args)?;
    let pack = uc.write_snapshot(request).await?;
    tool_success("write", serde_json::to_value(pack)?)
}

fn reject_legacy_write_contract(args: &Value) -> Result<(), DomainError> {
    reject_legacy_write_field(args, "op", "document")?;
    reject_legacy_write_field(args, "snapshot", "document")?;

    const LEGACY_MUTATION_FIELDS: [&str; 18] = [
        "section_key",
        "section_title",
        "section_description",
        "section_order",
        "ref_key",
        "ref_title",
        "ref_why",
        "path",
        "line_start",
        "line_end",
        "group",
        "diagram_key",
        "mermaid",
        "diagram_why",
        "title",
        "brief",
        "tags",
        "ttl_minutes",
    ];

    for field in LEGACY_MUTATION_FIELDS {
        reject_legacy_write_field(args, field, "document")?;
    }

    Ok(())
}

fn reject_legacy_write_field(
    args: &Value,
    legacy_field: &str,
    supported_field: &str,
) -> Result<(), DomainError> {
    if args.get(legacy_field).is_some() {
        return Err(DomainError::DetailedInvalidData {
            message: format!(
                "'{}' is not supported for input write in v3; use '{}'",
                legacy_field, supported_field
            ),
            details: json!({
                "tool": "input",
                "action": "write",
                "unsupported_field": legacy_field,
                "supported_field": supported_field,
                "contract": "document_full_replace",
            }),
        });
    }
    Ok(())
}

fn parse_write_snapshot_request(args: &Value) -> Result<WriteSnapshotRequest, DomainError> {
    let document = args
        .get("document")
        .ok_or_else(|| DomainError::DetailedInvalidData {
            message: "input write requires 'document'".into(),
            details: json!({
                "tool": "input",
                "action": "write",
                "required_fields": ["document"],
            }),
        })?;
    let document_obj = document
        .as_object()
        .ok_or_else(|| DomainError::DetailedInvalidData {
            message: "'document' must be an object".into(),
            details: json!({
                "tool": "input",
                "action": "write",
                "field": "document",
                "required_type": "object",
            }),
        })?;

    let identifier = str_opt(args, "id")
        .or_else(|| str_opt(args, "name"))
        .or_else(|| str_opt(args, "identifier"));
    let expected_revision = u64_opt(args, "expected_revision")?;

    if identifier.is_some() && expected_revision.is_none() {
        return Err(DomainError::DetailedInvalidData {
            message: "write update requires expected_revision".into(),
            details: json!({
                "tool": "input",
                "action": "write",
                "required_fields": ["expected_revision"],
                "guidance": [
                    "fetch latest revision with input.get before mutating"
                ],
            }),
        });
    }

    if identifier.is_none() && expected_revision.is_some() {
        return Err(DomainError::DetailedInvalidData {
            message: "expected_revision is only valid with id/name update writes".into(),
            details: json!({
                "tool": "input",
                "action": "write",
                "required_fields": ["id", "name"],
                "unsupported_field": "expected_revision",
            }),
        });
    }

    let sections = document_obj
        .get("sections")
        .and_then(Value::as_array)
        .ok_or_else(|| DomainError::DetailedInvalidData {
            message: "document.sections is required".into(),
            details: json!({
                "tool": "input",
                "action": "write",
                "required_fields": ["document.sections"],
            }),
        })?;

    let mut parsed_sections = Vec::with_capacity(sections.len());
    for section in sections {
        let section_obj = section
            .as_object()
            .ok_or_else(|| DomainError::InvalidData("section must be an object".into()))?;
        let refs = parse_document_refs(section_obj.get("refs"))?;
        let diagrams = parse_document_diagrams(section_obj.get("diagrams"))?;
        parsed_sections.push(SnapshotSection {
            key: req_document_str(section_obj, "key")?,
            title: req_document_str(section_obj, "title")?,
            description: document_opt_str(section_obj, "description"),
            refs,
            diagrams,
        });
    }

    let tags = parse_document_tags(document_obj.get("tags"))?;
    let ttl_minutes = document_obj
        .get("ttl_minutes")
        .map(document_u64)
        .transpose()?;

    Ok(WriteSnapshotRequest {
        identifier,
        expected_revision,
        validate_only: args
            .get("validate_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        document: SnapshotDocument {
            name: document_opt_str(document_obj, "name"),
            title: document_opt_str(document_obj, "title"),
            brief: document_opt_str(document_obj, "brief"),
            tags,
            ttl_minutes,
            status: parse_document_status(document_obj.get("status"))?,
            sections: parsed_sections,
        },
    })
}

fn parse_document_status(value: Option<&Value>) -> Result<Status, DomainError> {
    match value {
        None => Ok(Status::Draft),
        Some(raw) => {
            let text = raw.as_str().ok_or_else(|| {
                DomainError::InvalidData("document.status must be a string".into())
            })?;
            text.parse::<Status>()
        }
    }
}

fn parse_document_tags(raw: Option<&Value>) -> Result<Vec<String>, DomainError> {
    let Some(raw_tags) = raw else {
        return Ok(Vec::new());
    };

    let tags = raw_tags
        .as_array()
        .ok_or_else(|| DomainError::InvalidData("document.tags must be an array".into()))?;
    let mut parsed = Vec::with_capacity(tags.len());
    for tag in tags {
        let value = tag.as_str().ok_or_else(|| {
            DomainError::InvalidData("document.tags entries must be strings".into())
        })?;
        parsed.push(value.to_string());
    }
    Ok(parsed)
}

fn parse_document_refs(raw: Option<&Value>) -> Result<Vec<SnapshotRef>, DomainError> {
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
            key: req_document_str(obj, "key")?,
            path: req_document_str(obj, "path")?,
            line_start: req_document_usize(obj, "line_start")?,
            line_end: req_document_usize(obj, "line_end")?,
            title: document_opt_str(obj, "title"),
            why: document_opt_str(obj, "why"),
            group: document_opt_str(obj, "group"),
        });
    }
    Ok(out)
}

fn parse_document_diagrams(raw: Option<&Value>) -> Result<Vec<SnapshotDiagram>, DomainError> {
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
            key: req_document_str(obj, "key")?,
            title: req_document_str(obj, "title")?,
            mermaid: req_document_str(obj, "mermaid")?,
            why: document_opt_str(obj, "why"),
        });
    }
    Ok(out)
}

fn req_document_str(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, DomainError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| DomainError::InvalidData(format!("document.{} is required", key)))
}

fn req_document_usize(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<usize, DomainError> {
    let value = obj
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| DomainError::InvalidData(format!("document.{} is required", key)))?;
    usize::try_from(value)
        .map_err(|_| DomainError::InvalidData(format!("document.{} is out of range", key)))
}

fn document_u64(value: &Value) -> Result<u64, DomainError> {
    value
        .as_u64()
        .ok_or_else(|| DomainError::InvalidData("document.ttl_minutes must be an integer".into()))
}

fn document_opt_str(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_string())
}

fn req_pack_identifier(args: &Value, tool: &str, action: &str) -> Result<String, DomainError> {
    req_identifier(args).map_err(|err| match err {
        DomainError::InvalidData(_) => DomainError::DetailedInvalidData {
            message: format!("{} {} requires 'id' or 'name'", tool, action),
            details: json!({
                "tool": tool,
                "action": action,
                "required_fields": ["id", "name"],
                "mutually_interchangeable": ["id", "name"]
            }),
        },
        other => other,
    })
}

fn req_expected_revision(args: &Value) -> Result<u64, DomainError> {
    req_u64(args, "expected_revision").map_err(|err| match err {
        DomainError::InvalidData(_) => DomainError::DetailedInvalidData {
            message: "write/ttl actions require expected_revision".into(),
            details: json!({
                "tool": "input",
                "action": "revision_guard",
                "required_fields": ["expected_revision"],
                "guidance": [
                    "fetch latest revision with input.get before mutating"
                ]
            }),
        },
        other => other,
    })
}

fn unsupported_input_action(action: &str) -> DomainError {
    match action {
        "create" | "upsert_section" | "delete_section" | "upsert_ref" | "delete_ref"
        | "upsert_diagram" | "set_meta" | "set_status" => DomainError::DetailedInvalidData {
            message: format!(
                "input action '{}' is not supported in v3; use action='write' with document",
                action
            ),
            details: json!({
                "tool": "input",
                "action": "unsupported",
                "requested_action": action,
                "allowed_actions": INPUT_ALLOWED_ACTIONS,
                "legacy_mapping": {
                    "action": "write",
                    "required_fields": ["document"],
                    "contract": "document_full_replace",
                },
            }),
        },
        "touch_ttl" => DomainError::DetailedInvalidData {
            message: "input action 'touch_ttl' is not supported in v3; use action='ttl'".into(),
            details: json!({
                "tool": "input",
                "action": "unsupported",
                "requested_action": "touch_ttl",
                "allowed_actions": INPUT_ALLOWED_ACTIONS,
                "legacy_mapping": { "action": "ttl" },
            }),
        },
        "delete_pack" => DomainError::DetailedInvalidData {
            message: "input action 'delete_pack' is not supported in v3; use action='delete'"
                .into(),
            details: json!({
                "tool": "input",
                "action": "unsupported",
                "requested_action": "delete_pack",
                "allowed_actions": INPUT_ALLOWED_ACTIONS,
                "legacy_mapping": { "action": "delete" },
            }),
        },
        _ => DomainError::DetailedInvalidData {
            message: format!(
                "unknown input action '{}'; allowed actions: list, get, write, ttl, delete",
                action
            ),
            details: json!({
                "tool": "input",
                "action": "unknown",
                "requested_action": action,
                "allowed_actions": INPUT_ALLOWED_ACTIONS,
            }),
        },
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
