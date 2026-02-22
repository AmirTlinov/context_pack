use serde_json::{json, Value};

use crate::adapters::mcp_stdio::rpc::RpcEnvelope;
use crate::adapters::mcp_stdio::to_json_text;
use crate::domain::errors::DomainError;

pub(super) fn domain_error_response(id: Value, err: &DomainError) -> RpcEnvelope {
    let (kind, code, details) = match err {
        DomainError::InvalidData(_) => ("validation", "invalid_data", Value::Null),
        DomainError::TtlRequired(_) => ("validation", "ttl_required", Value::Null),
        DomainError::NotFound(_) => ("not_found", "not_found", Value::Null),
        DomainError::Conflict(_) => ("conflict", "conflict", Value::Null),
        DomainError::RevisionConflict { expected, actual } => (
            "conflict",
            "revision_conflict",
            json!({
                "expected_revision": expected,
                "actual_revision": actual
            }),
        ),
        DomainError::InvalidState(_) => ("invalid_state", "invalid_state", Value::Null),
        DomainError::StaleRef(_) => ("stale_ref", "stale_ref", Value::Null),
        DomainError::Io(_) => ("io_error", "io_error", Value::Null),
        DomainError::Deserialize(_) => ("deserialize_error", "deserialize_error", Value::Null),
        DomainError::MigrationRequired(_) => {
            ("migration_required", "migration_required", Value::Null)
        }
    };

    let mut payload = json!({
        "error": true,
        "kind": kind,
        "code": code,
        "message": err.to_string(),
        "request_id": id
    });
    if !details.is_null() {
        payload["details"] = details;
    }
    let text = to_json_text(&payload);

    // MCP convention: tool-level errors are returned inside result + isError=true
    RpcEnvelope::success(
        id,
        json!({
            "content": [{ "type": "text", "text": text }],
            "isError": true
        }),
    )
}
