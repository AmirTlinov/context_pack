use anyhow::{Context, Result};
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

        let created = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "action":"create",
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

        let upsert_section = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":4,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"upsert_section",
                        "expected_revision": revision,
                        "section_key":"flow",
                        "section_title":"Flow section",
                        "section_description":"Auth flow anchor"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&upsert_section)?)?;

        let upsert_ref = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":5,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"upsert_ref",
                        "expected_revision": revision,
                        "section_key":"flow",
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

        let finalized = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":6,
                "method":"tools/call",
                "params":{
                    "name":"input",
                    "arguments":{
                        "id": pack_id.clone(),
                        "action":"set_status",
                        "expected_revision": revision,
                        "status":"finalized"
                    }
                }
            }))
            .await?;
        revision = payload_pack_revision(&parse_tool_payload(&finalized)?)?;
        assert!(revision >= 4);

        let output = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":7,
                "method":"tools/call",
                "params":{
                    "name":"output",
                    "arguments":{
                        "id": pack_id.clone()
                    }
                }
            }))
            .await?;
        let rendered = output
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("text"))
            .and_then(Value::as_str)
            .context("missing rendered markdown output")?;
        assert!(rendered.contains("[LEGEND]"));
        assert!(rendered.contains("login handler"));
        assert!(rendered.contains("token = \"ok\""));
        assert!(rendered.contains("ttl_remaining"));

        let listed = client
            .call(json!({
                "jsonrpc":"2.0",
                "id":8,
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
                        "action":"create",
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
                        "action":"upsert_section",
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
                        "action":"set_meta",
                        "id": pack_id,
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
        assert!(
            err_payload["details"]["actual_revision"]
                .as_u64()
                .unwrap_or(0)
                > stale_revision
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
                        "action":"create",
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
                        "action":"touch_ttl",
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
                        "action":"create",
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
                        "action":"create",
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
                    "action":"create",
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
        assert!(
            entries.next_entry().await?.is_none(),
            "no packs should be created after shutdown notification"
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
                "action":"create",
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

    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let status = child.try_wait().context("query child status")?;
    assert!(
        status.is_some(),
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
                        "action":"create",
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
                        "action":"upsert_section",
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
        storage_root.join("packs").join("pk_aaaaaaaa.md"),
        r#"---
schema_version: 1
id: pk_aaaaaaaa
name: old-pack
title: null
brief: null
status: draft
tags: []
sections: []
revision: 1
created_at: 2026-01-01T00:00:00Z
updated_at: 2026-01-01T00:00:00Z
expires_at: 2099-01-01T00:00:00Z
---
"#,
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
