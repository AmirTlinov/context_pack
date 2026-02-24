use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use mcp_context_pack::domain::{
    models::Pack,
    types::{PackId, PackName, Status},
};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

const MAX_FRAME_BYTES: usize = 1024 * 1024;

struct McpE2EClient {
    child: Child,
    stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout: BufReader<tokio::process::ChildStdout>,
}

impl McpE2EClient {
    async fn spawn(storage_root: &Path, source_root: &Path) -> Result<Self> {
        let bin_path = resolve_binary_path()?;

        let mut child = Command::new(bin_path)
            .env("CONTEXT_PACK_ROOT", storage_root)
            .env("CONTEXT_PACK_SOURCE_ROOT", source_root)
            .env("CONTEXT_PACK_LOG", "off")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("spawn MCP server")?;

        let stdin = tokio::io::BufWriter::new(
            child
                .stdin
                .take()
                .context("missing piped stdin for spawned server")?,
        );
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .context("missing piped stdout for spawned server")?,
        );

        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    async fn call(&mut self, request: Value) -> Result<Value> {
        self.send_raw_json(request).await?;
        self.read_response().await
    }

    async fn send_raw_json(&mut self, request: Value) -> Result<()> {
        let body = serde_json::to_vec(&request)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn send_raw_json_with_content_type(&mut self, request: Value) -> Result<()> {
        let body = serde_json::to_vec(&request)?;
        let header = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self) -> Result<Value> {
        read_mcp_response(&mut self.stdout).await
    }

    async fn stop(mut self) -> Result<()> {
        drop(self.stdin);
        self.child.kill().await.ok();
        self.child.wait().await.context("wait for server exit")?;
        Ok(())
    }
}

fn resolve_binary_path() -> Result<std::path::PathBuf> {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_mcp-context-pack") {
        return Ok(std::path::PathBuf::from(path));
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_mcp_context_pack") {
        return Ok(std::path::PathBuf::from(path));
    }

    let current_exe = std::env::current_exe().context("resolve current test executable path")?;
    let debug_dir = current_exe
        .parent()
        .and_then(|p| p.parent())
        .context("resolve target/debug directory")?;

    for candidate in ["mcp-context-pack", "mcp_context_pack"] {
        let path = debug_dir.join(candidate);
        if path.exists() {
            return Ok(path);
        }
    }

    anyhow::bail!(
        "missing mcp-context-pack binary in '{}' and CARGO_BIN_EXE_* env",
        debug_dir.display()
    )
}

fn parse_content_length(line: &str) -> Result<usize> {
    let mut parts = line.splitn(2, ':');
    let key = parts.next().map(|s| s.trim()).unwrap_or("");
    if !key.eq_ignore_ascii_case("content-length") {
        anyhow::bail!("unsupported transport header: {key}");
    }

    let value = parts
        .next()
        .map(str::trim)
        .context("missing content-length value")?;
    let len = value.parse::<usize>()?;
    if len > MAX_FRAME_BYTES {
        anyhow::bail!(
            "content-length {} exceeds allowed limit {}",
            len,
            MAX_FRAME_BYTES
        );
    }

    Ok(len)
}

async fn read_mcp_response<R>(reader: &mut BufReader<R>) -> Result<Value>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("unexpected EOF while reading header"));
        }
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]).trim().to_string();
        if trimmed.is_empty() {
            continue;
        }

        if !trimmed.to_ascii_lowercase().starts_with("content-length") {
            let value = serde_json::from_str::<Value>(&trimmed)?;
            return Ok(value);
        }

        let len = parse_content_length(&trimmed)?;
        loop {
            let mut tail = String::new();
            let n = reader.read_line(&mut tail).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("unexpected EOF while reading header tail"));
            }
            if tail.trim_end_matches(&['\r', '\n'][..]).trim().is_empty() {
                break;
            }
        }

        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;
        let value = serde_json::from_slice(&body)?;
        return Ok(value);
    }
}

fn parse_tool_payload(response: &Value) -> Result<Value> {
    let text = response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("text"))
        .and_then(Value::as_str)
        .context("missing text content payload")?;
    let payload: Value = serde_json::from_str(text)?;
    Ok(payload)
}

fn payload_pack_revision(payload: &Value) -> Result<u64> {
    payload
        .get("payload")
        .and_then(|v| v.get("revision"))
        .and_then(Value::as_u64)
        .context("missing payload.revision")
}

fn output_markdown(response: &Value) -> Result<&str> {
    response
        .get("result")
        .and_then(|r| r.get("content"))
        .and_then(|v| v.get(0))
        .and_then(|v| v.get("text"))
        .and_then(Value::as_str)
        .context("missing rendered markdown output")
}

fn legend_value(markdown: &str, key: &str) -> Option<String> {
    let prefix = format!("- {}: ", key);
    markdown
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::to_string))
}

fn rendered_ref_keys(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("#### ")?;
            let (key, _) = rest.split_once(" [")?;
            if key.starts_with("ref-") {
                Some(key.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn make_named_pack_with(
    name: &str,
    status: Status,
    updated_at: chrono::DateTime<Utc>,
    revision: u64,
) -> Pack {
    let mut pack = Pack::new(PackId::new(), Some(PackName::new(name).unwrap()));
    pack.status = status;
    pack.updated_at = updated_at;
    pack.created_at = updated_at - Duration::minutes(1);
    pack.revision = revision.max(1);
    pack.expires_at = updated_at + Duration::hours(24);
    pack
}

fn write_pack_file(storage_root: &Path, pack: &Pack) -> Result<()> {
    let packs_dir = storage_root.join("packs");
    std::fs::create_dir_all(&packs_dir)?;
    let payload = serde_json::to_string(pack)?;
    std::fs::write(
        packs_dir.join(format!("{}.json", pack.id.as_str())),
        payload,
    )?;
    Ok(())
}

#[tokio::test]
async fn e2e_tool_call_roundtrip_with_real_stdio() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let source_path = source_root.join("auth.rs");
    tokio::fs::write(
        &source_path,
        "fn login() {\n    let token = \"ok\";\n}\nfn logout() {}\n",
    )
    .await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let initialize = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}))
            .await?;
        assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");

        let tools = client
            .call(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .await?;
        let listed: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .context("missing tools list")?
            .iter()
            .map(|tool| tool["name"].as_str().unwrap_or_default())
            .collect();
        assert!(listed.contains(&"input"));
        assert!(listed.contains(&"output"));
        let tools_array = tools["result"]["tools"]
            .as_array()
            .context("missing tool entries")?;
        let input_tool = tools_array
            .iter()
            .find(|tool| tool["name"] == "input")
            .context("missing input tool schema")?;
        let output_tool = tools_array
            .iter()
            .find(|tool| tool["name"] == "output")
            .context("missing output tool schema")?;
        assert_eq!(
            input_tool["inputSchema"]["properties"]["action"]["enum"],
            json!(["list", "get", "write", "ttl", "delete"])
        );
        assert_eq!(
            output_tool["inputSchema"]["properties"]["action"]["enum"],
            json!(["list", "read"])
        );

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"e2e-pack",
                        "title":"E2E pack",
                        "brief":"E2E integration test",
                        "ttl_minutes": 90
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let mut revision = payload_pack_revision(&created_payload)?;

        let upsert_scope = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"write","op":"upsert_section",
                        "expected_revision": revision,
                        "section_key":"scope",
                        "section_title":"Scope",
                        "section_description":"Auth flow scope"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&upsert_scope)?)?;

        let upsert_findings = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"write","op":"upsert_section",
                        "expected_revision": revision,
                        "section_key":"findings",
                        "section_title":"Findings",
                        "section_description":"Auth findings"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&upsert_findings)?)?;

        let upsert_ref = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"write","op":"upsert_ref",
                        "expected_revision": revision,
                        "section_key":"findings",
                        "ref_key":"auth_handler",
                        "ref_title":"login handler",
                        "ref_why":"Need actual snippet for login flow",
                        "path":"auth.rs",
                        "line_start":1,
                        "line_end":3
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&upsert_ref)?)?;

        let upsert_qa = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":7,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"write","op":"upsert_section",
                        "expected_revision": revision,
                        "section_key":"qa",
                        "section_title":"QA",
                        "section_description":"verdict: pass"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&upsert_qa)?)?;

        let finalized = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":8,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"write","op":"set_status",
                        "expected_revision": revision,
                        "status":"finalized"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&finalized)?)?;
        assert!(revision >= 6);

        let output = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":9,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "id": pack_id.clone()
                    }
                }
            }))
            .await?;
        let rendered_compact = output
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .context("missing rendered markdown output")?;
        assert!(rendered_compact.contains("[LEGEND]"));
        assert!(rendered_compact.contains("- mode: compact"));
        assert!(rendered_compact.contains("- paging: active"));
        assert!(rendered_compact.contains("- limit: 6"));
        assert!(rendered_compact.contains("## Handoff summary [handoff]"));
        assert!(rendered_compact.contains("login handler"));
        assert!(!rendered_compact.contains("token = \"ok\""));
        assert!(rendered_compact.contains("ttl_remaining"));

        let output_full = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":10,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "id": pack_id.clone(),
                        "mode":"full"
                    }
                }
            }))
            .await?;
        let rendered_full = output_full
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .context("missing full rendered markdown output")?;
        assert!(rendered_full.contains("token = \"ok\""));
        assert!(rendered_full.contains("```rust"));
        let listed = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":11,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"list",
                        "query":"e2e-pack"
                    }
                }
            }))
            .await?;
        let list_markdown = listed
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .context("missing list markdown output")?;
        assert!(list_markdown.contains("ttl"), "list must expose human ttl");

        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_input_delete_pack_is_deterministic() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let create = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"delete-pack",
                        "ttl_minutes": 60
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&create)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();

        let pack_path = storage_root.join("packs").join(format!("{}.json", pack_id));
        assert!(pack_path.exists(), "new pack file must exist before delete_pack");

        let deleted = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"delete",
                        "id": pack_id
                    }
                }
            }))
            .await?;
        let delete_payload = parse_tool_payload(&deleted)?;
        assert_eq!(delete_payload["payload"]["deleted"].as_bool(), Some(true));
        assert!(!pack_path.exists(), "delete_pack action should remove pack file");

        let list = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{ "action":"list" }
                }
            }))
            .await?;
        let list_payload = parse_tool_payload(&list)?;
        assert_eq!(list_payload["payload"]["count"].as_u64(), Some(0));

        let deleted_again = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"delete",
                        "id": pack_path.file_name().unwrap().to_string_lossy().trim_end_matches(".json")
                    }
                }
            }))
            .await?;
        let deleted_again_payload = parse_tool_payload(&deleted_again)?;
        assert_eq!(
            deleted_again_payload["payload"]["deleted"].as_bool(),
            Some(false)
        );
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_tool_error_contract_is_machine_readable() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let response = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"boom"
                    }
                }
            }))
            .await?;

        assert_eq!(response["result"]["isError"], true);
        let err_payload = parse_tool_payload(&response)?;
        assert_eq!(err_payload["error"], true);
        assert_eq!(err_payload["kind"], "validation");
        assert_eq!(err_payload["code"], "invalid_data");
        assert_eq!(err_payload["request_id"], 2);
        assert!(err_payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown input action"));
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_v3_rejects_legacy_actions_with_invalid_data() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let legacy_input = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"create",
                        "name":"legacy-cutover-check",
                        "ttl_minutes":30
                    }
                }
            }))
            .await?;
        assert_eq!(legacy_input["result"]["isError"], true);
        let legacy_input_payload = parse_tool_payload(&legacy_input)?;
        assert_eq!(legacy_input_payload["code"], "invalid_data");
        assert!(
            legacy_input_payload["message"]
                .as_str()
                .unwrap_or_default()
                .contains("not supported in v3"),
            "legacy input action must return explicit invalid_data"
        );
        assert!(
            legacy_input_payload["message"]
                .as_str()
                .unwrap_or_default()
                .contains("action='write'"),
            "legacy input action error must include v3 guidance"
        );

        let legacy_output = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"get",
                        "id":"pk_aaaaaaaa"
                    }
                }
            }))
            .await?;
        assert_eq!(legacy_output["result"]["isError"], true);
        let legacy_output_payload = parse_tool_payload(&legacy_output)?;
        assert_eq!(legacy_output_payload["code"], "invalid_data");
        assert!(
            legacy_output_payload["message"]
                .as_str()
                .unwrap_or_default()
                .contains("use action='read'"),
            "legacy output action must return explicit invalid_data guidance"
        );

        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_request_id_and_revision_conflict_contract() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":"init-1","method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"create-1",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"conflict-e2e",
                        "ttl_minutes": 120
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let stale_revision = payload_pack_revision(&created_payload)?;

        let updated = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"update-1",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id": pack_id.clone(),
                        "expected_revision": stale_revision,
                        "section_key":"sec",
                        "section_title":"Section"
                    }
                }
            }))
            .await?;
        let _next_revision = payload_pack_revision(&parse_tool_payload(&updated)?)?;

        let conflict = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"conflict-req",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"set_meta",
                        "id": pack_id.clone(),
                        "expected_revision": stale_revision,
                        "title":"new title"
                    }
                }
            }))
            .await?;

        assert_eq!(conflict["result"]["isError"], true);
        let err_payload = parse_tool_payload(&conflict)?;
        assert_eq!(err_payload["kind"], "conflict");
        assert_eq!(err_payload["code"], "revision_conflict");
        assert_eq!(err_payload["request_id"], "conflict-req");
        assert_eq!(
            err_payload["details"]["expected_revision"].as_u64(),
            Some(stale_revision)
        );
        let current_revision = err_payload["details"]["current_revision"]
            .as_u64()
            .context("missing current_revision details")?;
        assert!(
            current_revision > stale_revision,
            "conflict should include fresher current_revision"
        );
        assert_eq!(
            err_payload["details"]["actual_revision"].as_u64(),
            Some(current_revision),
            "actual_revision is kept as compatibility alias"
        );
        assert!(
            err_payload["details"]["last_updated_at"]
                .as_str()
                .unwrap_or_default()
                .contains('T'),
            "details must include last_updated_at timestamp"
        );
        let changed_keys = err_payload["details"]["changed_section_keys"]
            .as_array()
            .context("missing changed_section_keys")?;
        assert!(
            changed_keys.iter().any(|key| key.as_str() == Some("sec")),
            "changed_section_keys should include the modified section"
        );
        assert!(
            changed_keys.len() <= 12,
            "changed_section_keys should be bounded"
        );
        assert!(
            err_payload["details"]["guidance"]
                .as_str()
                .unwrap_or_default()
                .contains("re-read latest pack"),
            "guidance must explain retry workflow"
        );

        let reread = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"reread-1",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"get",
                        "id": pack_id.clone()
                    }
                }
            }))
            .await?;
        let latest_revision = payload_pack_revision(&parse_tool_payload(&reread)?)?;

        let retry = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"retry-1",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"set_meta",
                        "id": pack_id,
                        "expected_revision": latest_revision,
                        "title":"resolved-after-reread"
                    }
                }
            }))
            .await?;
        assert_ne!(retry["result"]["isError"], true);
        let retried_payload = parse_tool_payload(&retry)?;
        assert!(
            payload_pack_revision(&retried_payload)? > latest_revision,
            "retry with fresh revision should succeed and bump revision"
        );
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_finalize_validation_reports_missing_sections_and_invalid_refs() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;
    tokio::fs::write(source_root.join("short.rs"), "line1\nline2\n").await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"finalize-validation-e2e",
                        "ttl_minutes": 30
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let mut revision = payload_pack_revision(&created_payload)?;

        let scope = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id": pack_id.clone(),
                        "expected_revision": revision,
                        "section_key":"scope",
                        "section_title":"Scope",
                        "section_description":"scope text"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&scope)?)?;

        let findings = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id": pack_id.clone(),
                        "expected_revision": revision,
                        "section_key":"findings",
                        "section_title":"Findings",
                        "section_description":"finding text"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&findings)?)?;

        let stale_ref = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_ref",
                        "id": pack_id.clone(),
                        "expected_revision": revision,
                        "section_key":"findings",
                        "ref_key":"ref-one",
                        "path":"short.rs",
                        "line_start": 10,
                        "line_end": 10
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&stale_ref)?)?;

        let missing_qa = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"set_status",
                        "id": pack_id.clone(),
                        "expected_revision": revision,
                        "status":"finalized"
                    }
                }
            }))
            .await?;
        assert_eq!(missing_qa["result"]["isError"], true);
        let missing_qa_payload = parse_tool_payload(&missing_qa)?;
        assert_eq!(missing_qa_payload["code"], "finalize_validation");
        assert_eq!(
            missing_qa_payload["details"]["missing_sections"],
            json!(["qa"])
        );
        assert_eq!(missing_qa_payload["details"]["missing_fields"], json!([]));
        assert_eq!(missing_qa_payload["details"]["invalid_refs"], json!([]));

        let qa = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":7,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id": pack_id.clone(),
                        "expected_revision": revision,
                        "section_key":"qa",
                        "section_title":"QA",
                        "section_description":"verdict: fail"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&qa)?)?;

        let broken_ref = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":8,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"set_status",
                        "id": pack_id,
                        "expected_revision": revision,
                        "status":"finalized"
                    }
                }
            }))
            .await?;
        assert_eq!(broken_ref["result"]["isError"], true);
        let broken_ref_payload = parse_tool_payload(&broken_ref)?;
        assert_eq!(broken_ref_payload["code"], "finalize_validation");
        assert_eq!(broken_ref_payload["details"]["missing_sections"], json!([]));
        assert_eq!(broken_ref_payload["details"]["missing_fields"], json!([]));
        assert_eq!(
            broken_ref_payload["details"]["invalid_refs"][0]["section_key"],
            "findings"
        );
        assert_eq!(
            broken_ref_payload["details"]["invalid_refs"][0]["ref_key"],
            "ref-one"
        );
        assert_eq!(
            broken_ref_payload["details"]["invalid_refs"][0]["path"],
            "short.rs"
        );
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_write_snapshot_validate_only_precheck_and_finalize_commit() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;
    tokio::fs::write(source_root.join("auth.rs"), "line1\nline2\nline3\n").await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write",
                        "snapshot":{
                            "name":"snapshot-e2e",
                            "title":"Snapshot E2E",
                            "ttl_minutes":30,
                            "status":"draft",
                            "sections":[
                                {"key":"notes","title":"Notes","description":"draft content"}
                            ]
                        }
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let revision = payload_pack_revision(&created_payload)?;

        let precheck = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write",
                        "identifier": pack_id.clone(),
                        "expected_revision": revision,
                        "validate_only": true,
                        "snapshot":{
                            "name":"snapshot-e2e",
                            "title":"Finalize precheck",
                            "status":"finalized",
                            "sections":[
                                {"key":"scope","title":"Scope","description":"scope text"},
                                {"key":"findings","title":"Findings","description":"finding text","refs":[{"key":"ref-one","path":"auth.rs","line_start":1,"line_end":2}]}
                            ]
                        }
                    }
                }
            }))
            .await?;
        assert_eq!(precheck["result"]["isError"], true);
        let precheck_payload = parse_tool_payload(&precheck)?;
        assert_eq!(precheck_payload["code"], "finalize_validation");
        assert_eq!(precheck_payload["details"]["missing_sections"], json!(["qa"]));

        let reread = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"get",
                        "id": pack_id.clone()
                    }
                }
            }))
            .await?;
        let reread_payload = parse_tool_payload(&reread)?;
        assert_eq!(
            payload_pack_revision(&reread_payload)?,
            revision,
            "validate_only precheck must not persist pack state"
        );
        assert_eq!(reread_payload["payload"]["status"], "draft");

        let finalized = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write",
                        "identifier": pack_id.clone(),
                        "expected_revision": revision,
                        "snapshot":{
                            "name":"snapshot-e2e",
                            "title":"Finalize commit",
                            "status":"finalized",
                            "sections":[
                                {"key":"scope","title":"Scope","description":"scope text"},
                                {"key":"findings","title":"Findings","description":"finding text","refs":[{"key":"ref-one","path":"auth.rs","line_start":1,"line_end":2}]},
                                {"key":"qa","title":"QA","description":"verdict: pass"}
                            ]
                        }
                    }
                }
            }))
            .await?;
        let finalized_payload = parse_tool_payload(&finalized)?;
        assert_eq!(finalized_payload["payload"]["status"], "finalized");
        assert!(
            payload_pack_revision(&finalized_payload)? > revision,
            "persisted write should increment revision"
        );

        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_touch_ttl_requires_mode_field() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"ttl-mode-pack",
                        "ttl_minutes": 30
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let revision = payload_pack_revision(&created_payload)?;

        let response = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"ttl",
                        "id": pack_id,
                        "expected_revision": revision
                    }
                }
            }))
            .await?;

        assert_eq!(response["result"]["isError"], true);
        let err_payload = parse_tool_payload(&response)?;
        assert_eq!(err_payload["kind"], "validation");
        assert_eq!(err_payload["code"], "ttl_required");
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_create_requires_ttl_minutes() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let response = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"missing-ttl"
                    }
                }
            }))
            .await?;

        assert_eq!(response["result"]["isError"], true);
        let err_payload = parse_tool_payload(&response)?;
        assert_eq!(err_payload["kind"], "validation");
        assert_eq!(err_payload["code"], "invalid_data");
        assert!(err_payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("'ttl_minutes' is required"));
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_shutdown_exit_terminates_server() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let _ = client
        .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
        .await?;
    let shutdown = client
        .call(json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":{}}))
        .await?;
    assert_eq!(shutdown["id"], 2);

    // exit can be notification; server should terminate promptly
    client
        .send_raw_json(json!({"jsonrpc":"2.0","method":"exit","params":{}}))
        .await?;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let status = client.child.try_wait().context("query child status")?;
    assert!(status.is_some(), "server must exit after 'exit'");
    Ok(())
}

#[tokio::test]
async fn e2e_notification_without_id_produces_no_response() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        client
            .send_raw_json(json!({
                "jsonrpc":"2.0",
                "method":"notifications/initialized",
                "params": {}
            }))
            .await?;

        let ping = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"ping",
                "params": {}
            }))
            .await?;

        assert_eq!(ping["id"], 2);
        assert!(ping["result"].is_object());
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_output_get_supports_mode_match_paging_and_cursor() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;
    tokio::fs::write(
        source_root.join("flow.rs"),
        "fn a() { let token = \"TOKEN_01\"; }\nfn b() { let token = \"TOKEN_02\"; }\nfn c() { let token = \"TOKEN_03\"; }\n",
    )
    .await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"paging-e2e-pack",
                        "ttl_minutes": 60
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let mut revision = payload_pack_revision(&created_payload)?;

        let section = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id": pack_id.clone(),
                        "expected_revision": revision,
                        "section_key":"flow",
                        "section_title":"Flow"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&section)?)?;

        for (idx, line_no) in [(1_u32, 1_u32), (2, 2), (3, 3)] {
            let updated = client
                .call(json!({
                    "jsonrpc":"2.0",
                    "id": format!("ref-{}", idx),
                    "method":"tools/call",
                    "params":{
                        "name":"input",
                        "arguments":{
                            "action":"write","op":"upsert_ref",
                            "id": pack_id.clone(),
                            "expected_revision": revision,
                            "section_key":"flow",
                            "ref_key": format!("ref-0{}", idx),
                            "path":"flow.rs",
                            "line_start": line_no,
                            "line_end": line_no
                        }
                    }
                }))
                .await?;
            revision = payload_pack_revision(&parse_tool_payload(&updated)?)?;
        }
        assert!(revision >= 5);

        let page1 = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":7,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": pack_id.clone(),
                        "mode":"compact",
                        "match":"TOKEN_0[12]",
                        "limit":1
                    }
                }
            }))
            .await?;
        let markdown1 = output_markdown(&page1)?;
        assert!(!markdown1.contains("```rust"));
        assert_eq!(legend_value(markdown1, "has_more").as_deref(), Some("true"));
        let next = legend_value(markdown1, "next")
            .filter(|value| value != "null")
            .context("missing next cursor on first page")?;
        assert_eq!(rendered_ref_keys(markdown1), vec!["ref-01"]);

        let page2 = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":8,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": pack_id,
                        "cursor": next
                    }
                }
            }))
            .await?;
        let markdown2 = output_markdown(&page2)?;
        assert_eq!(
            legend_value(markdown2, "has_more").as_deref(),
            Some("false")
        );
        assert_eq!(legend_value(markdown2, "next").as_deref(), Some("null"));
        assert_eq!(rendered_ref_keys(markdown2), vec!["ref-02"]);
        assert!(!markdown2.contains("ref-03"));

        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_output_get_default_compact_is_bounded_and_full_preserved() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let source_body = (1_u32..=15_u32)
        .map(|idx| format!("fn step_{idx:02}() {{ let token = \"TOKEN_{idx:02}\"; }}"))
        .collect::<Vec<_>>()
        .join("\n");
    tokio::fs::write(source_root.join("bounded.rs"), format!("{source_body}\n")).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"bounded-default-pack",
                        "title":"Bounded default pack",
                        "brief":"Routing handoff for bounded compact output",
                        "ttl_minutes": 60
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let mut revision = payload_pack_revision(&created_payload)?;

        let section = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id": pack_id.clone(),
                        "expected_revision": revision,
                        "section_key":"routing",
                        "section_title":"Routing"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&section)?)?;

        for idx in 1_u32..=15_u32 {
            let updated = client
                .call(json!({
                    "jsonrpc":"2.0",
                    "id": format!("bounded-ref-{}", idx),
                    "method":"tools/call",
                    "params":{
                        "name":"input",
                        "arguments":{
                            "action":"write","op":"upsert_ref",
                            "id": pack_id.clone(),
                            "expected_revision": revision,
                            "section_key":"routing",
                            "ref_key": format!("ref-{idx:02}"),
                            "path":"bounded.rs",
                            "line_start": idx,
                            "line_end": idx
                        }
                    }
                }))
                .await?;
            revision = payload_pack_revision(&parse_tool_payload(&updated)?)?;
        }
        assert!(revision >= 17);

        let compact_default = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": pack_id.clone()
                    }
                }
            }))
            .await?;
        let compact_markdown = output_markdown(&compact_default)?;
        assert!(compact_markdown.contains("- mode: compact"));
        assert!(compact_markdown.contains("- paging: active"));
        assert!(compact_markdown.contains("- limit: 6"));
        assert_eq!(
            legend_value(compact_markdown, "has_more").as_deref(),
            Some("true")
        );
        assert!(compact_markdown.contains("## Handoff summary [handoff]"));
        assert!(compact_markdown.contains("- objective:"));
        assert!(compact_markdown.contains("- scope:"));
        assert!(compact_markdown.contains("- verdict_status:"));
        assert!(compact_markdown.contains("- top_risks:"));
        assert!(compact_markdown.contains("- top_gaps:"));
        assert!(compact_markdown.contains("- deep_nav_hints:"));
        assert!(!compact_markdown.contains("```rust"));
        assert_eq!(
            rendered_ref_keys(compact_markdown).len(),
            6,
            "default compact handoff must be bounded by default page size"
        );
        let next_cursor = legend_value(compact_markdown, "next")
            .filter(|value| value != "null")
            .context("default compact page should expose next cursor for remainder")?;

        let compact_page_two = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": pack_id.clone(),
                        "cursor": next_cursor
                    }
                }
            }))
            .await?;
        let compact_page_two_markdown = output_markdown(&compact_page_two)?;
        assert_eq!(
            rendered_ref_keys(compact_page_two_markdown),
            vec![
                "ref-07".to_string(),
                "ref-08".to_string(),
                "ref-09".to_string(),
                "ref-10".to_string(),
                "ref-11".to_string(),
                "ref-12".to_string()
            ]
        );
        assert_eq!(
            legend_value(compact_page_two_markdown, "has_more").as_deref(),
            Some("true")
        );
        let next_cursor_page_two = legend_value(compact_page_two_markdown, "next")
            .filter(|value| value != "null")
            .context("second compact page should still expose next cursor")?;

        let compact_page_three = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": pack_id.clone(),
                        "cursor": next_cursor_page_two
                    }
                }
            }))
            .await?;
        let compact_page_three_markdown = output_markdown(&compact_page_three)?;
        assert_eq!(
            rendered_ref_keys(compact_page_three_markdown),
            vec![
                "ref-13".to_string(),
                "ref-14".to_string(),
                "ref-15".to_string()
            ]
        );
        assert_eq!(
            legend_value(compact_page_three_markdown, "next").as_deref(),
            Some("null")
        );

        let full = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":7,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": pack_id,
                        "mode":"full"
                    }
                }
            }))
            .await?;
        let full_markdown = output_markdown(&full)?;
        assert!(full_markdown.contains("```rust"));
        assert!(
            compact_markdown.len() < full_markdown.len(),
            "compact default should be materially smaller than full output"
        );

        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_output_get_invalid_regex_returns_validation_error() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"regex-e2e-pack",
                        "ttl_minutes": 60
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();

        let response = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": pack_id,
                        "match":"[broken"
                    }
                }
            }))
            .await?;

        assert_eq!(response["result"]["isError"], true);
        let err_payload = parse_tool_payload(&response)?;
        assert_eq!(err_payload["kind"], "validation");
        assert_eq!(err_payload["code"], "invalid_data");
        assert!(err_payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid regex"));
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_output_rejects_format_parameter() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"format-pack",
                        "ttl_minutes": 30
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();

        let response = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "id": pack_id,
                        "format":"json"
                    }
                }
            }))
            .await?;

        assert_eq!(response["result"]["isError"], true);
        let err_payload = parse_tool_payload(&response)?;
        assert_eq!(err_payload["kind"], "validation");
        assert_eq!(err_payload["code"], "invalid_data");
        assert!(err_payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("always markdown"));
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_shutdown_notification_has_no_side_effects() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let _ = client
        .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
        .await?;
    let _ = client
        .call(json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":{}}))
        .await?;

    client
        .send_raw_json(json!({
            "jsonrpc":"2.0",
            "method":"tools/call",
            "params":{
                "name":"input",
                "arguments":{
                    "action":"write","op":"create",
                    "name":"should-not-exist",
                    "ttl_minutes": 30
                }
            }
        }))
        .await?;

    let blocked = client
        .call(json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{
                "name":"input",
                "arguments":{"action":"list"}
            }
        }))
        .await?;
    assert_eq!(blocked["error"]["code"], -32000);

    client
        .send_raw_json(json!({"jsonrpc":"2.0","method":"exit","params":{}}))
        .await?;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let packs_dir = storage_root.join("packs");
    if packs_dir.exists() {
        let mut entries = tokio::fs::read_dir(&packs_dir).await?;
        let mut has_pack_files = false;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                has_pack_files = true;
                break;
            }
        }
        assert!(
            !has_pack_files,
            "no pack .json files should be created after shutdown notification"
        );
    }
    Ok(())
}

#[tokio::test]
async fn e2e_concurrent_create_same_name_rejects_one_process() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client1 = McpE2EClient::spawn(&storage_root, &source_root).await?;
    let mut client2 = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let _ = client1
        .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
        .await?;
    let _ = client2
        .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
        .await?;

    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let b1 = barrier.clone();
    let b2 = barrier.clone();

    let create_req = json!({
        "jsonrpc":"2.0",
        "id":2,
        "method":"tools/call",
        "params":{
            "name":"input",
            "arguments":{
                "action":"write","op":"create",
                "name":"same-name",
                "ttl_minutes": 30
            }
        }
    });

    let create_req_1 = create_req.clone();
    let t1 = tokio::spawn(async move {
        b1.wait().await;
        let resp = client1.call(create_req_1).await?;
        client1.stop().await?;
        Ok::<Value, anyhow::Error>(resp)
    });
    let t2 = tokio::spawn(async move {
        b2.wait().await;
        let resp = client2.call(create_req).await?;
        client2.stop().await?;
        Ok::<Value, anyhow::Error>(resp)
    });

    let r1 = t1.await??;
    let r2 = t2.await??;

    let responses = vec![r1, r2];
    let mut success_count = 0usize;
    let mut conflict_count = 0usize;
    for response in responses {
        let is_error = response["result"]["isError"].as_bool().unwrap_or(false);
        if !is_error {
            success_count += 1;
            continue;
        }
        let err = parse_tool_payload(&response)?;
        if err["kind"] == "conflict" && err["code"] == "conflict" {
            conflict_count += 1;
        }
    }

    assert_eq!(success_count, 1, "exactly one create should succeed");
    assert_eq!(conflict_count, 1, "second create must return conflict");
    Ok(())
}

#[tokio::test]
async fn e2e_initialize_with_content_type_header() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    client
        .send_raw_json_with_content_type(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await?;
    let init = client.read_response().await?;
    assert_eq!(init["id"], 1);
    assert!(init["result"]["serverInfo"]["name"].is_string());

    client.stop().await?;
    Ok(())
}

#[tokio::test]
async fn e2e_initialize_accepts_unframed_json_message() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let bin_path = resolve_binary_path()?;
    let mut child = Command::new(bin_path)
        .env("CONTEXT_PACK_ROOT", &storage_root)
        .env("CONTEXT_PACK_SOURCE_ROOT", &source_root)
        .env("CONTEXT_PACK_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("spawn MCP server")?;

    let mut stdin = tokio::io::BufWriter::new(
        child
            .stdin
            .take()
            .context("missing piped stdin for spawned server")?,
    );
    let mut stdout = BufReader::new(
        child
            .stdout
            .take()
            .context("missing piped stdout for spawned server")?,
    );

    stdin
        .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;

    let init = read_mcp_response(&mut stdout).await?;
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "context-pack");

    child.kill().await.ok();
    child.wait().await.context("wait for server exit")?;
    Ok(())
}

#[tokio::test]
async fn e2e_unframed_json_line_batch_is_processed_sequentially() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let bin_path = resolve_binary_path()?;
    let mut child = Command::new(bin_path)
        .env("CONTEXT_PACK_ROOT", &storage_root)
        .env("CONTEXT_PACK_SOURCE_ROOT", &source_root)
        .env("CONTEXT_PACK_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("spawn MCP server")?;

    let mut stdin = tokio::io::BufWriter::new(
        child
            .stdin
            .take()
            .context("missing piped stdin for spawned server")?,
    );
    let mut stdout = BufReader::new(
        child
            .stdout
            .take()
            .context("missing piped stdout for spawned server")?,
    );

    stdin
        .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;

    let init = read_mcp_response(&mut stdout).await?;
    assert_eq!(init["id"], 1);

    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":2,"method":"ping"}
{"jsonrpc":"2.0","id":3,"method":"ping"}
"#,
        )
        .await?;
    stdin.flush().await?;

    let ping1 = read_mcp_response(&mut stdout).await?;
    let ping2 = read_mcp_response(&mut stdout).await?;
    assert_eq!(ping1["id"], 2);
    assert_eq!(ping2["id"], 3);

    child.kill().await.ok();
    child.wait().await.context("wait for server exit")?;
    Ok(())
}

#[tokio::test]
async fn e2e_server_exits_if_initialize_never_arrives() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let bin_path = resolve_binary_path()?;
    let mut child = Command::new(bin_path)
        .env("CONTEXT_PACK_ROOT", &storage_root)
        .env("CONTEXT_PACK_SOURCE_ROOT", &source_root)
        .env("CONTEXT_PACK_LOG", "off")
        .env("CONTEXT_PACK_INITIALIZE_TIMEOUT_MS", "250")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("spawn MCP server")?;

    let status = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait())
        .await
        .context("wait timeout for child exit")?
        .context("wait for child status")?;
    assert!(
        status.code().is_some(),
        "server must exit if initialize is not received within timeout"
    );
    Ok(())
}

#[tokio::test]
async fn e2e_input_rejects_legacy_alias_fields() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"legacy-alias-pack",
                        "ttl_minutes": 30
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing created pack id")?
            .to_string();
        let revision = payload_pack_revision(&created_payload)?;

        let response = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id": pack_id,
                        "expected_revision": revision,
                        "section_key":"flow",
                        "title":"Legacy alias"
                    }
                }
            }))
            .await?;

        assert_eq!(response["result"]["isError"], true);
        let err_payload = parse_tool_payload(&response)?;
        assert_eq!(err_payload["kind"], "validation");
        assert_eq!(err_payload["code"], "invalid_data");
        assert!(err_payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("'title' is not supported"));
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_schema_mismatch_reports_migration_required() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(storage_root.join("packs")).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    tokio::fs::write(
        storage_root.join("packs").join("pk_aaaaaaaa.json"),
        r#"{"schema_version":1,"id":"pk_aaaaaaaa","name":"old-pack","title":null,"brief":null,"status":"draft","tags":[],"sections":[],"revision":1,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","expires_at":"2099-01-01T00:00:00Z"}"#,
    )
    .await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let response = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{"action":"list"}
                }
            }))
            .await?;

        assert_eq!(response["result"]["isError"], true);
        let err_payload = parse_tool_payload(&response)?;
        assert_eq!(err_payload["kind"], "migration_required");
        assert_eq!(err_payload["code"], "migration_required");
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_output_get_name_resolution_metadata_and_ambiguity_candidates() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let now = Utc::now();
    let selected = make_named_pack_with("name-resolve-pack", Status::Finalized, now, 12);
    let older_finalized = make_named_pack_with(
        "name-resolve-pack",
        Status::Finalized,
        now - Duration::minutes(15),
        4,
    );
    let newer_draft = make_named_pack_with(
        "name-resolve-pack",
        Status::Draft,
        now + Duration::minutes(1),
        100,
    );

    let tie_time = now + Duration::minutes(5);
    let ambiguous_a = make_named_pack_with("ambiguous-e2e-pack", Status::Finalized, tie_time, 9);
    let ambiguous_b = make_named_pack_with("ambiguous-e2e-pack", Status::Finalized, tie_time, 9);

    write_pack_file(&storage_root, &selected)?;
    write_pack_file(&storage_root, &older_finalized)?;
    write_pack_file(&storage_root, &newer_draft)?;
    write_pack_file(&storage_root, &ambiguous_a)?;
    write_pack_file(&storage_root, &ambiguous_b)?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let by_name = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "name":"name-resolve-pack"
                    }
                }
            }))
            .await?;
        let markdown_name = output_markdown(&by_name)?;
        assert_eq!(
            legend_value(markdown_name, "id").as_deref(),
            Some(selected.id.as_str())
        );
        assert_eq!(
            legend_value(markdown_name, "selected_by").as_deref(),
            Some("name_latest_finalized_updated_at_then_revision")
        );
        assert_eq!(
            legend_value(markdown_name, "selected_revision").as_deref(),
            Some("12")
        );
        assert_eq!(
            legend_value(markdown_name, "selected_status").as_deref(),
            Some("finalized")
        );

        let by_id = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id": selected.id.as_str()
                    }
                }
            }))
            .await?;
        let markdown_id = output_markdown(&by_id)?;
        assert_eq!(
            legend_value(markdown_id, "selected_by").as_deref(),
            Some("exact_id")
        );
        assert_eq!(
            legend_value(markdown_id, "selected_revision").as_deref(),
            Some("12")
        );
        assert_eq!(
            legend_value(markdown_id, "selected_status").as_deref(),
            Some("finalized")
        );

        let ambiguous = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "name":"ambiguous-e2e-pack"
                    }
                }
            }))
            .await?;
        assert_eq!(ambiguous["result"]["isError"], true);
        let err_payload = parse_tool_payload(&ambiguous)?;
        assert_eq!(err_payload["code"], "ambiguous");
        let mut actual_candidates = err_payload["details"]["candidate_ids"]
            .as_array()
            .context("missing candidate_ids details")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect::<Vec<_>>();
        actual_candidates.sort();
        let mut expected_candidates = vec![
            ambiguous_a.id.as_str().to_string(),
            ambiguous_b.id.as_str().to_string(),
        ];
        expected_candidates.sort();
        assert_eq!(actual_candidates, expected_candidates);
        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_freshness_metadata_filters_and_warnings() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .await?;

        let create_pack = |request_id: i64, name: &str| {
            json!({
                "jsonrpc":"2.0",
                "id":request_id,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":name,
                        "ttl_minutes":120
                    }
                }
            })
        };

        let created_fresh = parse_tool_payload(&client.call(create_pack(2, "fresh-pack")).await?)?;
        let created_expiring =
            parse_tool_payload(&client.call(create_pack(3, "expiring-pack")).await?)?;
        let created_expired =
            parse_tool_payload(&client.call(create_pack(4, "expired-pack")).await?)?;

        let fresh_id = created_fresh["payload"]["id"]
            .as_str()
            .context("missing fresh id")?
            .to_string();
        let expiring_id = created_expiring["payload"]["id"]
            .as_str()
            .context("missing expiring id")?
            .to_string();
        let expired_id = created_expired["payload"]["id"]
            .as_str()
            .context("missing expired id")?
            .to_string();

        let patch_expiry = |pack_id: &str, expires_at: chrono::DateTime<Utc>| -> Result<()> {
            let path = storage_root.join("packs").join(format!("{}.json", pack_id));
            let raw = std::fs::read_to_string(&path)?;
            let mut value: Value = serde_json::from_str(&raw)?;
            value["expires_at"] = Value::String(expires_at.to_rfc3339());
            std::fs::write(path, serde_json::to_string(&value)?)?;
            Ok(())
        };

        patch_expiry(&expiring_id, Utc::now() + Duration::seconds(60))?;
        patch_expiry(&expired_id, Utc::now() - Duration::seconds(5))?;

        let input_list_default = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{"action":"list"}
                }
            }))
            .await?;
        let input_list_payload = parse_tool_payload(&input_list_default)?;
        let packs = input_list_payload["payload"]["packs"]
            .as_array()
            .context("missing input list packs")?;
        assert!(
            packs
                .iter()
                .all(|pack| pack.get("freshness_state").is_some()),
            "input list must include normalized freshness_state"
        );
        assert!(
            packs.iter().all(|pack| pack.get("ttl_remaining").is_some()),
            "input list must include stable ttl_remaining field"
        );
        assert!(
            packs
                .iter()
                .all(|pack| pack.get("id").and_then(Value::as_str) != Some(expired_id.as_str())),
            "default stale-safe list must hide expired packs"
        );
        assert!(
            packs
                .iter()
                .any(|pack| pack.get("id").and_then(Value::as_str) == Some(fresh_id.as_str())),
            "fresh pack should remain visible in stale-safe default list"
        );

        let input_list_expired = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"list",
                        "freshness":"expired"
                    }
                }
            }))
            .await?;
        let expired_payload = parse_tool_payload(&input_list_expired)?;
        let expired_packs = expired_payload["payload"]["packs"]
            .as_array()
            .context("missing expired filter packs")?;
        assert_eq!(
            expired_packs.len(),
            1,
            "expired filter should isolate stale pack"
        );
        assert_eq!(expired_packs[0]["id"], Value::String(expired_id.clone()));
        assert_eq!(
            expired_packs[0]["freshness_state"],
            Value::String("expired".into())
        );

        let input_get = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":7,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"get",
                        "id":expiring_id.as_str()
                    }
                }
            }))
            .await?;
        let input_get_payload = parse_tool_payload(&input_get)?;
        assert_eq!(
            input_get_payload["payload"]["freshness_state"],
            Value::String("expiring_soon".into())
        );
        assert!(
            input_get_payload["payload"].get("ttl_remaining").is_some()
                && input_get_payload["payload"].get("expires_at").is_some(),
            "input get must expose stable freshness summary fields"
        );

        let output_get = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":8,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id":expiring_id.as_str()
                    }
                }
            }))
            .await?;
        let output_markdown_expiring = output_markdown(&output_get)?;
        assert_eq!(
            legend_value(output_markdown_expiring, "freshness_state").as_deref(),
            Some("expiring_soon")
        );
        assert!(
            legend_value(output_markdown_expiring, "warning")
                .as_deref()
                .is_some_and(|warning| warning.contains("expiring soon")),
            "output get legend must include expiring warning"
        );

        let output_list_expired = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":9,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"list",
                        "freshness":"expired"
                    }
                }
            }))
            .await?;
        let expired_markdown = output_markdown(&output_list_expired)?;
        assert!(
            expired_markdown.contains(&expired_id),
            "output list freshness=expired must surface expired pack"
        );
        assert!(
            expired_markdown.contains("warning: expired"),
            "human-readable output must warn about expired packs"
        );

        Ok(())
    }
    .await;

    client.stop().await?;
    result
}

#[tokio::test]
async fn e2e_multi_agent_handoff_compact_full_and_stale_path() -> Result<()> {
    let dir = tempdir()?;
    let storage_root = dir.path().join("storage");
    let source_root = dir.path().join("source");
    tokio::fs::create_dir_all(&storage_root).await?;
    tokio::fs::create_dir_all(&source_root).await?;
    tokio::fs::write(
        source_root.join("handoff.rs"),
        "fn auth() {\n  let evidence_token = \"E2E\";\n  let reviewer_hint = true;\n}\n",
    )
    .await?;

    let mut client = McpE2EClient::spawn(&storage_root, &source_root).await?;

    let result: Result<()> = async {
        let _ = client
            .call(json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{}}))
            .await?;

        // Explorer publishes pack with finalizable minimum evidence.
        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"create-main-pack",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"multi-agent-handoff-pack",
                        "title":"Multi-agent handoff",
                        "brief":"Explorer to orchestrator/reviewer",
                        "ttl_minutes":120
                    }
                }
            }))
            .await?;
        let created_payload = parse_tool_payload(&created)?;
        let pack_id = created_payload["payload"]["id"]
            .as_str()
            .context("missing primary pack id")?
            .to_string();
        let mut revision = payload_pack_revision(&created_payload)?;

        let scope = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"scope",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id":pack_id.clone(),
                        "expected_revision":revision,
                        "section_key":"scope",
                        "section_title":"Scope",
                        "section_description":"explorer coverage"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&scope)?)?;

        let findings = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"findings",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id":pack_id.clone(),
                        "expected_revision":revision,
                        "section_key":"findings",
                        "section_title":"Findings",
                        "section_description":"auth evidence"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&findings)?)?;

        let finding_ref = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"finding-ref",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_ref",
                        "id":pack_id.clone(),
                        "expected_revision":revision,
                        "section_key":"findings",
                        "ref_key":"auth-ref",
                        "path":"handoff.rs",
                        "line_start":1,
                        "line_end":4
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&finding_ref)?)?;

        let qa = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"qa",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"upsert_section",
                        "id":pack_id.clone(),
                        "expected_revision":revision,
                        "section_key":"qa",
                        "section_title":"QA",
                        "section_description":"verdict: pass"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&qa)?)?;

        let finalized = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"finalize",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"set_status",
                        "id":pack_id.clone(),
                        "expected_revision":revision,
                        "status":"finalized"
                    }
                }
            }))
            .await?;
        let finalized_payload = parse_tool_payload(&finalized)?;
        assert_eq!(
            finalized_payload["payload"]["status"],
            Value::String("finalized".into())
        );

        // Orchestrator consumes compact handoff for routing decisions.
        let compact = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"orchestrator-read",
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id":pack_id.clone()
                    }
                }
            }))
            .await?;
        let compact_markdown = output_markdown(&compact)?;
        assert!(compact_markdown.contains("- mode: compact"));
        assert!(compact_markdown.contains("## Handoff summary [handoff]"));
        assert!(compact_markdown.contains("- deep_nav_hints:"));
        assert!(
            !compact_markdown.contains("```rust"),
            "compact routing view should avoid full code snippets"
        );

        // Reviewer consumes full evidence.
        let full = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"reviewer-read",
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"read",
                        "id":pack_id.clone(),
                        "mode":"full"
                    }
                }
            }))
            .await?;
        let full_markdown = output_markdown(&full)?;
        assert!(full_markdown.contains("```rust"));
        assert!(full_markdown.contains("evidence_token = \"E2E\""));

        // Explicit stale-pack path handling.
        let stale_created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"create-stale-pack",
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"write","op":"create",
                        "name":"multi-agent-stale-pack",
                        "ttl_minutes":120
                    }
                }
            }))
            .await?;
        let stale_payload = parse_tool_payload(&stale_created)?;
        let stale_id = stale_payload["payload"]["id"]
            .as_str()
            .context("missing stale pack id")?
            .to_string();

        let stale_path = storage_root.join("packs").join(format!("{stale_id}.json"));
        let stale_raw = std::fs::read_to_string(&stale_path)?;
        let mut stale_json: Value = serde_json::from_str(&stale_raw)?;
        stale_json["expires_at"] = Value::String((Utc::now() - Duration::seconds(5)).to_rfc3339());
        std::fs::write(stale_path, serde_json::to_string(&stale_json)?)?;

        let output_default = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"stale-default-list",
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{"action":"list"}
                }
            }))
            .await?;
        let default_markdown = output_markdown(&output_default)?;
        assert!(
            !default_markdown.contains(&stale_id),
            "stale-safe default list should hide expired pack"
        );

        let output_expired = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":"stale-expired-list",
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "action":"list",
                        "freshness":"expired"
                    }
                }
            }))
            .await?;
        let expired_markdown = output_markdown(&output_expired)?;
        assert!(
            expired_markdown.contains(&stale_id),
            "explicit freshness=expired list should surface stale pack"
        );
        assert!(
            expired_markdown.contains("warning: expired"),
            "stale path should be explicit in human-readable output"
        );

        Ok(())
    }
    .await;

    client.stop().await?;
    result
}
