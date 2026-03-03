---
name: context-pack-repo-profile
description: "Проектный профиль по работе с context_pack v3 для mcp/context_pack: инварианты, recovery и operator checklist."
---

# Context Pack Repo Profile (mcp/context_pack, v3)

SSOT для этого репозитория: [TECHNICAL.md](../../../TECHNICAL.md) + v3 реализация в `src/adapters/mcp_stdio/*`, `src/app/*`.

Используй этот skill для ежедневной работы через `mcp__context_pack` (explore, orchestration, execution, review).

## Hard contract (не обсуждается)

- `input` actions: `list`, `get`, `write`, `ttl`, `delete`
- `output` actions: `list`, `read`
- `input.write` только через `document` full-replace snapshot (legacy `upsert_*`, `set_*`, `op` запрещены)
- update (`input.write` / `input.ttl`) требует `expected_revision`
- finalize всегда через precheck: `input.write(validate_only=true, document.status=finalized)`
- handoff между агентами — только с **точным `pack_id`** (`pk_[a-z2-7]{8}`), не по `name`
- `output.read` routing: `profile=orchestrator|reviewer|executor`, pagination: `page_token`, фильтр: `contains`

## 60-секундный daily path

1. Найди/проверь pack: `input.list` / `input.get(id)`.
2. Перед мутацией получи свежий `revision` (`input.get`).
3. Запиши **полный** snapshot через `input.write`.
4. Перед finalize: `validate_only=true`.
5. Коммит finalize (без `validate_only`).
6. Считай handoff-view: `output.read(id=<pack_id>, profile=orchestrator)`.
7. Передай дальше: `pack_id + 1-line summary + next action`.

## Router по references (progressive disclosure)

Открывай только нужный файл:

1. [`references/00-v3-contract-cheatsheet.md`](references/00-v3-contract-cheatsheet.md)
   - copy-paste JSON для всех action'ов v3
2. [`references/10-daily-workflows.md`](references/10-daily-workflows.md)
   - потоки Explorer→Orchestrator, Orchestrator→Executor, Reviewer
3. [`references/20-finalize-qa-freshness.md`](references/20-finalize-qa-freshness.md)
   - finalize gate, `validate_only`, TTL/freshness дисциплина
4. [`references/30-recovery-playbook.md`](references/30-recovery-playbook.md)
   - recovery: `revision_conflict`, `ambiguous`, `expired/expiring_soon`, `stale_ref`
5. [`references/40-section-templates.md`](references/40-section-templates.md)
   - каркасы `scope/findings/qa` + optional sections
6. [`references/50-naming-conventions.md`](references/50-naming-conventions.md)
   - naming для `section_key/ref_key/group` + contains/search discipline
7. [`references/60-anti-patterns.md`](references/60-anti-patterns.md)
   - чего не делать (v2 legacy, partial snapshot, name-handoff и т.д.)

## Repo invariants (операционные)

- Expired grace window: `CONTEXT_PACK_EXPIRED_GRACE_SECONDS` (default `900`)
- Oversize guard: `CONTEXT_PACK_MAX_PACK_BYTES` (default `524288`)
- Corrupted/oversized файлы не должны ломать `list/read`; они изолируются/очищаются
- CI quality baseline:
  - `.github/coverage-baseline.json`
  - `scripts/check_coverage_baseline.sh`

## Минимальный handoff protocol

В completion/report обязательно:

- `pack_id` (точный)
- статус пакета (`draft|finalized`)
- что читать следующим (`profile=orchestrator|reviewer|executor`)
- при ревью: `review URL + verdict (PASS|BLOCKED)`

Пример 1 строки:

`pack_id=pk_abcd2345 finalized; summary=auth guard root cause verified; next=executor apply fix plan (profile=executor)`

## Legacy note

Папка `skills/explorer-context-pack/*` в этом репо переведена в LEGACY-режим.
Для актуальной работы используйте только текущий профиль и его references.
