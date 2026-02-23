use serde_json::{json, Value};

pub(super) fn tools_schema() -> Value {
    json!({
        "tools": [
            {
                "name": "input",
                "description": "Manage context packs: create/update/get/list, mutate sections/refs/diagrams, and update status/meta.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "Operation to perform",
                            "enum": [
                                "list", "create", "get",
                                "upsert_section", "delete_section",
                                "upsert_ref", "delete_ref",
                                "upsert_diagram",
                                "set_meta", "set_status",
                                "delete_pack",
                                "touch_ttl"
                            ]
                        },
                        "id": { "type": "string", "description": "Pack ID" },
                        "name": { "type": "string", "description": "Pack name (alternative to id)" },
                        "title": { "type": "string" },
                        "brief": { "type": "string", "description": "Short description of the pack" },
                        "tags": { "type": "array", "items": { "type": "string" } },
                        "ttl_minutes": { "type": "integer", "description": "TTL from now in minutes. Required for create; also used by touch_ttl(set)." },
                        "extend_minutes": { "type": "integer", "description": "Extend existing TTL by this many minutes (touch_ttl)." },
                        "expected_revision": { "type": "integer", "description": "Required for mutating actions to prevent lost updates." },
                        "status": { "type": "string", "enum": ["draft", "finalized"] },
                        "query": { "type": "string", "description": "Text search for list" },
                        "limit": { "type": "integer" },
                        "offset": { "type": "integer" },
                        "section_key": { "type": "string", "description": "^[a-z0-9][a-z0-9_-]{1,63}$" },
                        "section_title": { "type": "string" },
                        "section_description": { "type": "string" },
                        "section_order": { "type": "integer" },
                        "ref_key": { "type": "string", "description": "^[a-z0-9][a-z0-9_-]{1,63}$" },
                        "ref_title": { "type": "string" },
                        "ref_why": { "type": "string", "description": "Why this ref matters" },
                        "path": { "type": "string", "description": "Repo-relative file path" },
                        "line_start": { "type": "integer", "description": "1-indexed start line" },
                        "line_end": { "type": "integer", "description": "1-indexed end line" },
                        "group": { "type": "string", "description": "Group name for ref organization" },
                        "diagram_key": { "type": "string", "description": "^[a-z0-9][a-z0-9_-]{1,63}$" },
                        "mermaid": { "type": "string", "description": "Mermaid diagram source" },
                        "diagram_why": { "type": "string" }
                    }
                }
            },
            {
                "name": "output",
                "description": "Render a context pack with real code excerpts in Markdown, or list packs.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "get"]
                        },
                        "id": { "type": "string", "description": "Pack ID" },
                        "name": { "type": "string", "description": "Pack name" },
                        "status": {
                            "type": "string",
                            "enum": ["draft", "finalized"],
                            "description": "Optional status filter (for list and get)"
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["full", "compact"],
                            "description": "Render mode for output get (default: full)"
                        },
                        "query": { "type": "string", "description": "Optional text search for list" },
                        "cursor": { "type": "string", "description": "Opaque cursor returned by output get paging metadata" },
                        "match": { "type": "string", "description": "Regex filter applied to output get chunks" },
                        "limit": { "type": "integer" },
                        "offset": { "type": "integer" }
                    }
                }
            }
        ]
    })
}
