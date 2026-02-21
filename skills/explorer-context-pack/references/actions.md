# actions.md — обязательные поля

| tool | action | обязательные поля |
|---|---|---|
| input | list | `action` |
| input | create | `action`, `ttl_minutes` |
| input | get | `action`, `id|name` |
| input | upsert_section | `action`, `id|name`, `expected_revision`, `section_key`, `section_title` |
| input | delete_section | `action`, `id|name`, `expected_revision`, `section_key` |
| input | upsert_ref | `action`, `id|name`, `expected_revision`, `section_key`, `ref_key`, `path`, `line_start`, `line_end` |
| input | delete_ref | `action`, `id|name`, `expected_revision`, `section_key`, `ref_key` |
| input | upsert_diagram | `action`, `id|name`, `expected_revision`, `section_key`, `diagram_key`, `title`, `mermaid` |
| input | set_meta | `action`, `id|name`, `expected_revision`, минимум одно: `title|brief|tags` |
| input | set_status | `action`, `id|name`, `expected_revision`, `status` |
| input | touch_ttl | `action`, `id|name`, `expected_revision`, ровно одно: `ttl_minutes` или `extend_minutes` |
| output | list | `action` |
| output | get | `action`, `id|name` |

Канонические поля (алиасы запрещены):
- `section_title`, `section_description`
- `ref_title`, `ref_why`
- `diagram_why`

`output` всегда markdown, `format` не передавать.
