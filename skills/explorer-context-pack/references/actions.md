# LEGACY notice: actions.md

Этот файл больше не описывает актуальный контракт.

Используй:

- `../../../.codex/skills/context-pack-repo-profile/references/00-v3-contract-cheatsheet.md`
- `../../../TECHNICAL.md`

## v2 -> v3 map

| Legacy (не использовать) | v3 replacement |
|---|---|
| `input create` | `input write` (create snapshot) |
| `upsert_section` / `upsert_ref` / `upsert_diagram` | `input write` (full document snapshot) |
| `set_meta` / `set_status` | `input write` |
| `touch_ttl` | `input ttl` |
| `output get` | `output read` |

Актуальные action'ы v3:

- `input`: `list|get|write|ttl|delete`
- `output`: `list|read`
