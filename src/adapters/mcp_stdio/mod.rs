mod error_contract;
mod rpc;
mod schema;
mod tool_input;
mod tool_output;
mod transport;

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{BufReader, BufWriter};
use tokio::time::Duration;

use crate::app::input_usecases::InputUseCases;
use crate::app::output_usecases::OutputUseCases;
use crate::app::ports::FreshnessState;
use crate::domain::errors::DomainError;
use crate::domain::models::Pack;
use crate::domain::types::Status;

use error_contract::domain_error_response;
use rpc::{RpcEnvelope, RpcRequest};
use schema::tools_schema;
use tool_input::handle_input_tool;
use tool_output::handle_output_tool;
use transport::{read_next_message, write_response, TransportMode};

const MAX_FRAME_BYTES: usize = 10 * 1024 * 1024; // 10 MiB

fn parse_initialize_timeout_ms(raw: Option<&str>) -> Duration {
    const DEFAULT_SECS: u64 = 20;
    let parsed = raw
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0);
    match parsed {
        Some(ms) => Duration::from_millis(ms),
        None => Duration::from_secs(DEFAULT_SECS),
    }
}

fn initialize_timeout() -> Duration {
    let raw = std::env::var("CONTEXT_PACK_INITIALIZE_TIMEOUT_MS").ok();
    parse_initialize_timeout_ms(raw.as_deref())
}

pub async fn start_mcp_server(
    input_uc: Arc<InputUseCases>,
    output_uc: Arc<OutputUseCases>,
) -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut writer = BufWriter::new(stdout);
    let mut shutdown_requested = false;
    let init_timeout = initialize_timeout();
    let mut initialized = false;
    let init_deadline = tokio::time::Instant::now() + init_timeout;
    let mut response_mode: Option<TransportMode> = None;

    loop {
        let read_result = if initialized {
            read_next_message(&mut reader, MAX_FRAME_BYTES).await
        } else {
            let now = tokio::time::Instant::now();
            if now >= init_deadline {
                return Err(anyhow::anyhow!(
                    "no initialize received within {:?}; closing server",
                    init_timeout
                ));
            }
            match tokio::time::timeout(
                init_deadline.saturating_duration_since(now),
                read_next_message(&mut reader, MAX_FRAME_BYTES),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "no initialize received within {:?}; closing server",
                        init_timeout
                    ));
                }
            }
        };

        let (raw, mode) = match read_result {
            Ok(Some(msg)) => msg,
            Ok(None) => break, // EOF
            Err(e) => {
                tracing::warn!("frame read error: {}", e);
                continue;
            }
        };
        if response_mode.is_none() {
            response_mode = Some(mode);
        }

        if raw.trim().is_empty() {
            continue;
        }

        let req: RpcRequest = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                let envelope =
                    RpcEnvelope::rpc_error(Value::Null, -32700, format!("parse error: {}", e));
                write_response(&mut writer, &envelope, response_mode.unwrap_or(mode)).await?;
                continue;
            }
        };

        if req.method == "initialize" && !initialized {
            initialized = true;
        }

        let request_id = req.id.clone().unwrap_or(Value::Null);
        let is_notification = req.id.is_none();
        if req.method == "shutdown" {
            shutdown_requested = true;
            if !is_notification {
                let envelope = RpcEnvelope::success(request_id, json!(null));
                write_response(&mut writer, &envelope, response_mode.unwrap_or(mode)).await?;
            }
            continue;
        }

        if req.method == "exit" {
            if !is_notification {
                let envelope = RpcEnvelope::success(request_id, json!(null));
                write_response(&mut writer, &envelope, response_mode.unwrap_or(mode)).await?;
            }
            break;
        }

        if shutdown_requested {
            if !is_notification {
                let envelope = RpcEnvelope::rpc_error(
                    request_id,
                    -32000,
                    "server is shut down; only 'exit' is accepted",
                );
                write_response(&mut writer, &envelope, response_mode.unwrap_or(mode)).await?;
            }
            continue;
        }

        if let Some(envelope) = handle_request(&req, &input_uc, &output_uc).await {
            write_response(&mut writer, &envelope, response_mode.unwrap_or(mode)).await?;
        }
    }

    Ok(())
}

async fn handle_request(
    request: &RpcRequest,
    input_uc: &InputUseCases,
    output_uc: &OutputUseCases,
) -> Option<RpcEnvelope> {
    let id = request.id.clone().unwrap_or(Value::Null);
    let is_notification = request.id.is_none();
    let params = request.params.clone().unwrap_or(Value::Null);

    let envelope = match request.method.as_str() {
        "initialize" => RpcEnvelope::success(
            id.clone(),
            json!({
                "protocolVersion": initialize_protocol_version(request.params.as_ref()),
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": {
                    "name": "context-pack",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "ping" => RpcEnvelope::success(id.clone(), json!({})),
        "notifications/initialized" | "initialized" => {
            RpcEnvelope::success(id.clone(), json!(null))
        }
        "tools/list" => RpcEnvelope::success(id.clone(), tools_schema()),
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            if !args.is_object() {
                RpcEnvelope::rpc_error(id.clone(), -32602, "tool arguments must be an object")
            } else {
                match tool_name {
                    "input" => match handle_input_tool(&args, input_uc).await {
                        Ok(v) => RpcEnvelope::success(id.clone(), v),
                        Err(e) => domain_error_response(id.clone(), &e),
                    },
                    "output" => match handle_output_tool(&args, output_uc).await {
                        Ok(v) => RpcEnvelope::success(id.clone(), v),
                        Err(e) => domain_error_response(id.clone(), &e),
                    },
                    _ => RpcEnvelope::rpc_error(
                        id.clone(),
                        -32602,
                        format!("unknown tool '{}'", tool_name),
                    ),
                }
            }
        }
        _ => RpcEnvelope::rpc_error(
            id.clone(),
            -32601,
            format!("method not found: '{}'", request.method),
        ),
    };

    if is_notification {
        None
    } else {
        Some(envelope)
    }
}

fn initialize_protocol_version(request_params: Option<&Value>) -> &str {
    request_params
        .and_then(|value| value.get("protocolVersion"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("2025-06-18")
}

pub(super) fn to_json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{\"error\":true}".to_string())
}

pub(super) fn tool_success(action: &str, payload: Value) -> Result<Value, DomainError> {
    let content = json!({
        "action": action,
        "payload": payload
    });
    let text = serde_json::to_string(&content)?;
    if text.len() > MAX_FRAME_BYTES {
        return Err(DomainError::InvalidData(format!(
            "tool output too large: {} bytes (max {})",
            text.len(),
            MAX_FRAME_BYTES
        )));
    }
    Ok(json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    }))
}

pub(super) fn tool_text_success(text: String) -> Result<Value, DomainError> {
    if text.len() > MAX_FRAME_BYTES {
        return Err(DomainError::InvalidData(format!(
            "tool output too large: {} bytes (max {})",
            text.len(),
            MAX_FRAME_BYTES
        )));
    }
    Ok(json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    }))
}

pub(super) fn pack_summary(pack: &Pack) -> Value {
    let now = chrono::Utc::now();
    let ttl_remaining_human = pack.ttl_remaining_human(now);
    let freshness_state = FreshnessState::from_pack(pack, now);
    json!({
        "id": pack.id,
        "name": pack.name,
        "title": pack.title,
        "status": pack.status,
        "revision": pack.revision,
        "updated_at": pack.updated_at,
        "expires_at": pack.expires_at,
        "ttl_remaining_seconds": pack.ttl_remaining_seconds(now),
        "ttl_remaining_human": ttl_remaining_human.clone(),
        "ttl_remaining": ttl_remaining_human,
        "freshness_state": freshness_state
    })
}

pub(super) fn req_identifier(args: &Value) -> Result<String, DomainError> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    id.or(name).map(str::to_string).ok_or_else(|| {
        DomainError::InvalidData("'id' or 'name' is required for this action".into())
    })
}

pub(super) fn str_opt(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn usize_opt(args: &Value, key: &str) -> Result<Option<usize>, DomainError> {
    let Some(raw) = args.get(key).and_then(|v| v.as_u64()) else {
        return Ok(None);
    };
    let parsed = usize::try_from(raw)
        .map_err(|_| DomainError::InvalidData(format!("'{}' is out of range", key)))?;
    Ok(Some(parsed))
}

pub(super) fn req_u64(args: &Value, key: &str) -> Result<u64, DomainError> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| DomainError::InvalidData(format!("'{}' is required", key)))
}

pub(super) fn u64_opt(args: &Value, key: &str) -> Result<Option<u64>, DomainError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| DomainError::InvalidData(format!("'{}' must be an integer", key)))
}

pub(super) fn status_opt(args: &Value, key: &str) -> Result<Option<Status>, DomainError> {
    let Some(raw) = str_opt(args, key) else {
        return Ok(None);
    };
    Ok(Some(raw.parse::<Status>()?))
}

pub(super) fn freshness_opt(
    args: &Value,
    key: &str,
) -> Result<Option<FreshnessState>, DomainError> {
    let Some(raw) = str_opt(args, key) else {
        return Ok(None);
    };
    Ok(Some(raw.parse::<FreshnessState>()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    async fn parse_msg(input: &[u8]) -> anyhow::Result<Option<String>> {
        let mut reader = BufReader::new(input);
        Ok(read_next_message(&mut reader, MAX_FRAME_BYTES)
            .await?
            .map(|(msg, _)| msg))
    }

    fn extract_content_text(envelope: &RpcEnvelope) -> String {
        envelope
            .result
            .as_ref()
            .and_then(|r| r.get("content"))
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string()
    }

    #[tokio::test]
    async fn test_plain_json_line_is_accepted() {
        let input = b"{\"jsonrpc\":\"2.0\"}\n";
        let msg = parse_msg(input).await.unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
    }

    #[tokio::test]
    async fn test_plain_json_without_newline_is_accepted() {
        let input = b"{\"jsonrpc\":\"2.0\"}";
        let msg = parse_msg(input).await.unwrap().unwrap();
        let parsed: Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
    }

    #[tokio::test]
    async fn test_plain_multiline_json_is_rejected() {
        let input = b"{\n\"jsonrpc\":\"2.0\",\n\"method\":\"ping\"\n}\n";
        let err = parse_msg(input).await.unwrap_err();
        assert!(err.to_string().contains("invalid JSON message"), "{}", err);
    }

    #[tokio::test]
    async fn test_json_line_batch_messages_are_parsed_sequentially() {
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1}\n{\"jsonrpc\":\"2.0\",\"id\":2}\n";
        let mut reader = BufReader::new(input.as_slice());

        let first = read_next_message(&mut reader, MAX_FRAME_BYTES)
            .await
            .unwrap()
            .unwrap();
        let second = read_next_message(&mut reader, MAX_FRAME_BYTES)
            .await
            .unwrap()
            .unwrap();

        let first_json: Value = serde_json::from_str(&first.0).unwrap();
        let second_json: Value = serde_json::from_str(&second.0).unwrap();
        assert_eq!(first_json["id"], 1);
        assert_eq!(second_json["id"], 2);
        assert_eq!(first.1, TransportMode::JsonLine);
        assert_eq!(second.1, TransportMode::JsonLine);
    }

    #[tokio::test]
    async fn test_content_length_framing() {
        let body = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}";
        let frame = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut input = frame.into_bytes();
        input.extend_from_slice(body);
        let msg = parse_msg(&input).await.unwrap().unwrap();
        assert!(msg.contains("tools/list"));
    }

    #[tokio::test]
    async fn test_content_length_header_case_insensitive() {
        let body = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
        let frame = format!("content-length: {}\n\n", body.len());
        let mut input = frame.into_bytes();
        input.extend_from_slice(body);
        let msg = parse_msg(&input).await.unwrap().unwrap();
        assert!(msg.contains("\"method\":\"ping\""));
    }

    #[tokio::test]
    async fn test_content_type_then_content_length_is_accepted() {
        let body = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
        let frame = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let mut input = frame.into_bytes();
        input.extend_from_slice(body);
        let msg = parse_msg(&input).await.unwrap().unwrap();
        assert!(msg.contains("\"method\":\"ping\""));
    }

    #[tokio::test]
    async fn test_transport_mode_detection() {
        let framed_body = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
        let framed_header = format!("Content-Length: {}\r\n\r\n", framed_body.len());
        let mut framed_input = framed_header.into_bytes();
        framed_input.extend_from_slice(framed_body);
        let mut framed_reader = BufReader::new(framed_input.as_slice());
        let (_, mode1) = read_next_message(&mut framed_reader, MAX_FRAME_BYTES)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mode1, TransportMode::Framed);

        let mut json_reader = BufReader::new(b"{\"jsonrpc\":\"2.0\"}\n".as_slice());
        let (_, mode2) = read_next_message(&mut json_reader, MAX_FRAME_BYTES)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(mode2, TransportMode::JsonLine);
    }

    #[tokio::test]
    async fn test_frame_size_limit() {
        let huge_len = MAX_FRAME_BYTES + 1;
        let frame = format!("Content-Length: {}\r\n\r\n", huge_len);
        let mut reader = BufReader::new(frame.as_bytes());
        let result = read_next_message(&mut reader, MAX_FRAME_BYTES).await;
        assert!(result.is_err(), "should reject frames exceeding max size");
    }

    #[tokio::test]
    async fn test_oversized_header_line_is_rejected() {
        // Simulate a framed message whose first header line exceeds max_frame_bytes.
        // This exercises the read_until size check in read_line_from_first_byte,
        // which previously had no bound and could cause OOM.
        let tiny_max: usize = 64;
        // Build a header line longer than tiny_max, with no newline so read_until
        // must buffer the whole thing before returning.
        let huge_header = format!("X-Huge: {}\n", "A".repeat(tiny_max + 1));
        let mut reader = BufReader::new(huge_header.as_bytes());
        let result = read_next_message(&mut reader, tiny_max).await;
        assert!(result.is_err(), "should reject oversized header lines");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("incoming frame too large") || msg.contains("too large"),
            "unexpected error: {}",
            msg
        );
    }

    #[tokio::test]
    async fn test_oversized_json_line_is_rejected() {
        // A JSON line that exceeds max_frame_bytes must be rejected before processing.
        let tiny_max: usize = 20;
        // A valid-looking JSON object that is longer than tiny_max.
        let huge_json = format!(
            "{{{}}}\n",
            "\"k\":\"".to_string() + &"v".repeat(tiny_max) + "\""
        );
        let mut reader = BufReader::new(huge_json.as_bytes());
        let result = read_next_message(&mut reader, tiny_max).await;
        assert!(result.is_err(), "should reject oversized JSON line");
    }

    #[test]
    fn test_domain_error_contract_is_strict_json() {
        let envelope =
            domain_error_response(Value::from(42), &DomainError::InvalidData("x".into()));
        let text = extract_content_text(&envelope);
        let parsed: Value = serde_json::from_str(&text).expect("must be valid JSON");
        assert_eq!(parsed["error"], true);
        assert_eq!(parsed["kind"], "validation");
        assert_eq!(parsed["code"], "invalid_data");
        assert_eq!(parsed["request_id"], 42);
    }

    #[test]
    fn test_domain_error_contract_for_detailed_invalid_data() {
        let envelope = domain_error_response(
            Value::from("detailed"),
            &DomainError::DetailedInvalidData {
                message: "bad request".into(),
                details: json!({
                    "required_fields": ["id"],
                    "code_hint": "missing_id"
                }),
            },
        );
        let text = extract_content_text(&envelope);
        let parsed: Value = serde_json::from_str(&text).expect("must be valid JSON");
        assert_eq!(parsed["error"], true);
        assert_eq!(parsed["kind"], "validation");
        assert_eq!(parsed["code"], "invalid_data");
        assert_eq!(parsed["message"], "bad request");
        assert_eq!(parsed["details"]["required_fields"], json!(["id"]));
        assert_eq!(parsed["details"]["code_hint"], "missing_id");
    }

    #[test]
    fn test_domain_error_contract_keeps_string_request_id() {
        let envelope = domain_error_response(
            Value::from("req-42"),
            &DomainError::RevisionConflict {
                expected: 3,
                actual: 4,
            },
        );
        let text = extract_content_text(&envelope);
        let parsed: Value = serde_json::from_str(&text).expect("must be valid JSON");
        assert_eq!(parsed["request_id"], "req-42");
        assert_eq!(parsed["kind"], "conflict");
        assert_eq!(parsed["code"], "revision_conflict");
        assert_eq!(parsed["details"]["expected_revision"], 3);
        assert_eq!(parsed["details"]["actual_revision"], 4);
    }

    #[test]
    fn test_domain_error_contract_for_migration_required() {
        let envelope = domain_error_response(
            Value::from(9),
            &DomainError::MigrationRequired("schema mismatch".into()),
        );
        let text = extract_content_text(&envelope);
        let parsed: Value = serde_json::from_str(&text).expect("must be valid JSON");
        assert_eq!(parsed["kind"], "migration_required");
        assert_eq!(parsed["code"], "migration_required");
    }

    #[test]
    fn test_domain_error_contract_for_pack_id_conflict() {
        let envelope = domain_error_response(
            Value::from(1),
            &DomainError::PackIdConflict("pk_abcd1234".into()),
        );
        let text = extract_content_text(&envelope);
        let parsed: Value = serde_json::from_str(&text).expect("must be valid JSON");
        assert_eq!(parsed["kind"], "conflict");
        assert_eq!(parsed["code"], "pack_id_conflict");
    }

    #[test]
    fn test_domain_error_contract_for_deserialize_error() {
        let envelope =
            domain_error_response(Value::from(1), &DomainError::Deserialize("bad json".into()));
        let text = extract_content_text(&envelope);
        let parsed: Value = serde_json::from_str(&text).expect("must be valid JSON");
        assert_eq!(parsed["kind"], "deserialize_error");
        assert_eq!(parsed["code"], "deserialize_error");
    }

    #[test]
    fn test_output_format_parameter_is_rejected() {
        let args = json!({ "format": "json" });
        let err =
            crate::adapters::mcp_stdio::tool_output::reject_output_format_param(&args).unwrap_err();
        assert!(matches!(err, DomainError::DetailedInvalidData { .. }));
    }

    #[test]
    fn test_tool_success_rejects_oversized_payload() {
        let huge = "x".repeat(MAX_FRAME_BYTES + 1);
        let err = tool_success("get", json!({ "blob": huge })).unwrap_err();
        assert!(matches!(err, DomainError::InvalidData(_)));
        assert!(err.to_string().contains("tool output too large"));
    }

    #[test]
    fn test_parse_initialize_timeout_ms() {
        assert_eq!(parse_initialize_timeout_ms(None), Duration::from_secs(20));
        assert_eq!(
            parse_initialize_timeout_ms(Some("1500")),
            Duration::from_millis(1500)
        );
        assert_eq!(
            parse_initialize_timeout_ms(Some("  250  ")),
            Duration::from_millis(250)
        );
        assert_eq!(
            parse_initialize_timeout_ms(Some("0")),
            Duration::from_secs(20)
        );
        assert_eq!(
            parse_initialize_timeout_ms(Some("invalid")),
            Duration::from_secs(20)
        );
    }
}
