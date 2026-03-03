# context_pack v3 — Daily workflows

Цель: быстро передавать фактический контекст между ролями без повторного сканирования кода.

## A) Explorer → Orchestrator

1. **Создай/найди draft pack**
   - `input.list` (query/freshness)
   - при отсутствии: `input.write` create
2. **Собери фактуру в sections/refs**
   - цикл: `input.get(id)` → взять `revision` → `input.write(id, expected_revision, document)`
3. **Пройди finalize gate**
   - `input.write(validate_only=true, document.status=finalized)`
   - исправь ошибки (`missing_sections`, `missing_fields`, `invalid_refs`)
4. **Сделай finalize commit**
   - `input.write(..., document.status=finalized)`
5. **Сними handoff-view для оркестратора**
   - `output.read(id, profile=orchestrator)`
6. **Передай в чат строго**
   - `pack_id` (exact)
   - `status`
   - 1-line summary
   - next action

Шаблон сообщения:

`pack_id=pk_abcd2345 finalized; summary=auth fallback root cause confirmed; next=orchestrator route fix owner`

---

## B) Orchestrator → Executor

1. Orchestrator читает:
   - `output.read(id, profile=orchestrator)`
2. Выбирает executable slice и передаёт **тот же pack_id** executor'у.
3. Executor читает:
   - `output.read(id, profile=executor)`
   - при необходимости фокус: `contains="<token>"`
4. Если executor дополняет фактуру:
   - `input.get(id)` → `revision`
   - `input.write(id, expected_revision, updated document)`
5. Перед merge/close:
   - если статус должен быть finalized, снова пройти `validate_only` gate.

Практика: orchestration и execution не создают новый pack без причины; сначала переиспользуют текущий `pack_id`.

---

## C) Reviewer workflow

1. Reviewer читает полный evidence:
   - `output.read(id, profile=reviewer)`
2. Проверяет:
   - есть ли anchors для ключевых выводов
   - нет ли `stale ref` в рендере
   - есть ли `qa` c `verdict`
3. Для прицельного просмотра:
   - `output.read(id, profile=reviewer, contains="<finding-token>")`
4. Итог review:
   - `PASS` или `BLOCKED`
   - список обязательных доработок (если BLOCKED)
5. В completion/review comment обязательно:
   - `pack_id`
   - review URL
   - verdict

---

## Fast role map

- `profile=orchestrator`: компактный handoff-first обзор (default limit bounded)
- `profile=executor`: компактный actionable обзор
- `profile=reviewer`: полный markdown с evidence/snippets
