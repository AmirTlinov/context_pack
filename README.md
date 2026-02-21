# context-pack — MCP Context Pack Server

A standalone MCP server that lets AI agents build rich, curated **context packs** — structured bundles of code references, descriptions, and Mermaid diagrams — so that other agents or humans can consume them as a complete, self-contained context.

## Concept

Instead of writing large diffs or copying code around, an agent uses `input` to plant **anchors** (refs) pointing to exact lines in source files. When `output` is called, those anchors are expanded into real code excerpts, producing a full context document ready for immediate use.

## Tools

Pack ID format: `pk_[a-z2-7]{8}` (example: `pk_f2ireb33`).

### `input` — Build a context pack

Call without arguments to list existing packs:
```json
{ "name": "input" }
```

All mutations use an `action` field:

| action | description |
|---|---|
| `create` | Create a new draft pack |
| `get` | Fetch pack metadata |
| `list` | List packs (with optional `status`, `query`, `limit`, `offset`) |
| `upsert_section` | Add or update a section |
| `delete_section` | Remove a section |
| `upsert_ref` | Add or update a code reference anchor |
| `delete_ref` | Remove a reference |
| `upsert_diagram` | Add or update a Mermaid diagram |
| `set_meta` | Update title, brief, tags |
| `set_status` | Transition `draft` to `finalized` or back |
| `touch_ttl` | Set or extend pack TTL (`ttl_minutes` or `extend_minutes`) |

Canonical `input` fields (no aliases):
- `upsert_section`: use `section_title` / `section_description` (do not use `title` / `description`)
- `upsert_ref`: use `ref_title`, `ref_why` (do not use `title`/`why`)
- `upsert_diagram`: use `diagram_why` for rationale (do not use `why`)

### Mutation concurrency contract

All mutating actions except `create` require `expected_revision`.
If the stored revision differs, the tool returns `kind=conflict`, `code=revision_conflict`.

### TTL contract

- `create` requires `ttl_minutes`
- `touch_ttl` requires exactly one field:
  - `ttl_minutes` (set absolute TTL from now), or
  - `extend_minutes` (extend current TTL)
- expired packs are purged automatically on repository reads/listing.

### `output` — Render a context pack

Call without arguments to list packs:
```json
{ "name": "output" }
```

Call with `id` or `name` to get a fully rendered pack with real code excerpts:
```json
{ "name": "output", "arguments": { "name": "my-pack" } }
```

Optional list filters: `status`, `query`, `limit`, `offset`.
Optional `status` filter for `get`: `{ "status": "finalized" }` to require a specific state.
`output` is always Markdown; `format` parameter is unsupported.

## Tool response contract

`input` successful calls return `result.content[0].text` as strict JSON:

```json
{
  "action": "upsert_ref",
  "payload": { "...": "..." }
}
```

`output` successful calls return plain Markdown in `result.content[0].text` (no JSON envelope).

Tool failures return `isError: true` and strict machine-readable JSON in `text`:

```json
{
  "error": true,
  "kind": "validation|not_found|conflict|invalid_state|stale_ref|io_error|migration_required",
  "code": "invalid_data|ttl_required|not_found|revision_conflict|...",
  "message": "...",
  "request_id": 123,
  "details": { "...": "..." }
}
```

## MCP transport contract

- Preferred request transport is stdio framing: `Content-Length: N\r\n\r\n{json}`.
- Standard extra headers like `Content-Type` are accepted before `Content-Length`.
- For compatibility, server also accepts JSON messages without framing (JSON object/array; trailing newline is optional).
- Response mode follows the first incoming message transport:
  - framed request -> framed responses,
  - unframed JSON request -> newline-delimited JSON responses.

## Rendered output format

```
[LEGEND]
# Context pack: {title}
- id: ...
- name: ...
- status: finalized
- revision: 5
- expires_at: 2026-02-20T12:00:00Z
- ttl_remaining: 32m
- tags: auth, api
- brief: Authentication flow

[CONTENT]
## Section Title [section-key]
Section description

### group: core
#### ref-key [section-key]
**Ref Title**
- path: src/auth/handler.rs
- lines: 42-67
- why: Main entry point for token validation

```rust
  42: pub async fn validate_token(token: &str) -> Result<Claims> {
  ...
  67: }
```
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `CONTEXT_PACK_ROOT` | `.agents/mcp/context_pack` | Storage directory |
| `CONTEXT_PACK_SOURCE_ROOT` | CWD | Repository root for resolving file paths. Special values `__SESSION_CWD__`, `session_cwd`, `cwd`, `.` force current working directory. |
| `CONTEXT_PACK_LOG` | `mcp_context_pack=info` | Log level (stderr) |
| `CONTEXT_PACK_INITIALIZE_TIMEOUT_MS` | `20000` | Max wait for first `initialize` request before server exits (prevents orphan MCP processes). |

## Storage

Packs are stored as YAML frontmatter files:
```
{CONTEXT_PACK_ROOT}/packs/{pack-id}.md
```

Storage integrity policy is fail-closed:
- malformed pack files cause list/get to return `io_error` (they are never silently skipped).
- schema version mismatches return `migration_required`.

Lock files are expected implementation details:
- `.create.lock` for global create-name uniqueness
- `{pack-id}.lock` for per-pack revision-safe writes

## Build and run

```bash
cargo build --release
CONTEXT_PACK_ROOT=/tmp/packs cargo run
```

## Verification (production gates)

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Architecture

```
Domain (pure):   errors, types, models (invariants, FSM)
       down
Ports:           PackRepositoryPort, CodeExcerptPort
       down
App (usecases):  InputUseCases, OutputUseCases
       down
Adapters:        MarkdownStorageAdapter, CodeExcerptFsAdapter, mcp_stdio
```

Hexagonal architecture: domain has zero external dependencies.
