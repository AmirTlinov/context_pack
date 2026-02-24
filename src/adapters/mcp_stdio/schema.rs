use serde_json::{json, Value};

pub(super) fn tools_schema() -> Value {
    json!({
        "tools": [
            {
                "name": "input",
                "description": "Manage context packs with v3 actions: list/get/write/ttl/delete.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "Operation to perform",
                            "enum": ["list", "get", "write", "ttl", "delete"]
                        },
                        "id": { "type": "string", "description": "Pack ID" },
                        "name": { "type": "string", "description": "Pack name (alternative to id)" },
                        "ttl_minutes": { "type": "integer", "description": "TTL from now in minutes (action=ttl(set))." },
                        "extend_minutes": { "type": "integer", "description": "Extend existing TTL by this many minutes (action=ttl)." },
                        "expected_revision": { "type": "integer", "description": "Required for update writes and ttl actions." },
                        "validate_only": {
                            "type": "boolean",
                            "description": "When true, input.write validates document and returns diagnostics without persistence."
                        },
                        "document": {
                            "type": "object",
                            "description": "Full-replace snapshot payload for action=write.",
                            "properties": {
                                "name": { "type": "string", "description": "Optional pack name (new pack only, immutable for updates)." },
                                "title": { "type": "string" },
                                "brief": { "type": "string", "description": "Short description of the pack" },
                                "tags": { "type": "array", "items": { "type": "string" } },
                                "ttl_minutes": { "type": "integer", "description": "Optional TTL override from now in minutes." },
                                "status": { "type": "string", "enum": ["draft", "finalized"] },
                                "sections": {
                                    "type": "array",
                                    "description": "Full list of sections (each section can include refs and diagrams)."
                                }
                            }
                        },
                        "status": { "type": "string", "enum": ["draft", "finalized"] },
                        "freshness": {
                            "type": "string",
                            "enum": ["fresh", "expiring_soon", "expired"],
                            "description": "Optional list filter by freshness state."
                        },
                        "query": { "type": "string", "description": "Text search for list" },
                        "limit": { "type": "integer" },
                        "offset": { "type": "integer" }
                    }
                }
            },
            {
                "name": "output",
                "description": "Render v3 output actions: list/read.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "read"]
                        },
                        "id": { "type": "string", "description": "Pack ID" },
                        "name": { "type": "string", "description": "Pack name" },
                        "status": {
                            "type": "string",
                            "enum": ["draft", "finalized"],
                            "description": "Optional status filter (for list and read)"
                        },
                        "freshness": {
                            "type": "string",
                            "enum": ["fresh", "expiring_soon", "expired"],
                            "description": "Optional freshness filter for list."
                        },
                        "profile": {
                            "type": "string",
                            "enum": ["orchestrator", "reviewer", "executor"],
                            "description": "Read profile defaults: orchestrator (compact bounded), reviewer (full evidence), executor (actionable compact)."
                        },
                        "query": { "type": "string", "description": "Optional text search for list" },
                        "page_token": { "type": "string", "description": "Opaque page token returned by output read paging metadata." },
                        "contains": { "type": "string", "description": "Optional substring filter applied to rendered chunks." },
                        "limit": { "type": "integer" },
                        "offset": { "type": "integer" }
                    }
                }
            }
        ]
    })
}
