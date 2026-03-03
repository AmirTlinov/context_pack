---
name: explorer-context-pack
description: "context_pack v3 skill pack для multi-agent handoff через MCP (input/output): контракт, daily workflows, finalize/TTL, recovery, templates."
---

# context_pack (v3) — Skill pack (distributable)

Этот skill лежит в `skills/` (не в скрытых папках), чтобы пользователи могли
скачать репозиторий и установить/подключить его к своим агентам.

SSOT: [`TECHNICAL.md`](../../TECHNICAL.md) (контракт, ошибки, finalize, paging, migration).

## Hard contract (v3)

- `input` actions: `list|get|write|ttl|delete`
- `output` actions: `list|read`
- `input.write` — только `document` full-replace snapshot (legacy v2 `create/upsert_*/set_*` запрещены)
- update (`input.write` / `input.ttl`) требует `expected_revision`
- finalize: сначала `input.write(validate_only=true, document.status=finalized)`, потом persist
- handoff между агентами — по **точному `pack_id`** (`pk_[a-z2-7]{8}`), не по `name`
- `output.read`: `profile=orchestrator|reviewer|executor`, pagination: `page_token`, filter: `contains`

## 60-секундный daily path

1. Найди/создай pack: `input.list` → при необходимости `input.write` (create).
2. Перед мутацией: `input.get(id)` → возьми `revision`.
3. Запиши полный snapshot: `input.write(id, expected_revision, document)`.
4. Перед finalize: `validate_only=true`.
5. Finalize commit (persist).
6. Handoff-view: `output.read(id=<pack_id>, profile=orchestrator)`.
7. В чат: `pack_id + 1-line summary + next action`.

## Router по references (progressive disclosure)

Открывай только нужное:

1. [`references/00-v3-contract-cheatsheet.md`](references/00-v3-contract-cheatsheet.md) — copy/paste JSON
2. [`references/10-daily-workflows.md`](references/10-daily-workflows.md) — role workflows
3. [`references/20-finalize-qa-freshness.md`](references/20-finalize-qa-freshness.md) — finalize/TTL/freshness
4. [`references/30-recovery-playbook.md`](references/30-recovery-playbook.md) — recovery
5. [`references/40-section-templates.md`](references/40-section-templates.md) — section templates
6. [`references/50-naming-conventions.md`](references/50-naming-conventions.md) — naming/contains discipline
7. [`references/60-anti-patterns.md`](references/60-anti-patterns.md) — анти-паттерны

## Minimal handoff protocol (в чат)

- `pack_id` (exact)
- `status` (`draft|finalized`)
- что читать (`profile=orchestrator|reviewer|executor`)
- 1-line summary
- next action
