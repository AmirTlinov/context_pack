# Recovery playbook (v3)

Глобальный retry budget: максимум 3 повтора на один тип сбоя, затем `BLOCKED` с причиной и evidence.

## 1) `revision_conflict`

**Сигнал**

- `code=revision_conflict`
- в details есть `expected_revision`, `current_revision`, `last_updated_at`, `changed_section_keys`, `guidance`

**Что делать**

1. `input.get(id)` — получить актуальный `revision`
2. Смерджить намерение с изменениями из `changed_section_keys`
3. Повторить `input.write`/`input.ttl` с новым `expected_revision=current_revision`
4. Если конфликт повторяется >3 раз — `BLOCKED` и эскалация владельцу pack

---

## 2) `ambiguous` (обычно при чтении по `name`)

**Сигнал**

- `code=ambiguous`
- в details: `candidate_ids`

**Что делать**

1. Возьми `candidate_ids` из ошибки
2. Повтори `output.read` по точному `id` (не по name)
3. Зафиксируй выбранный `pack_id` в handoff
4. Дальше работай только с этим `id`

Профилактика: межагентный handoff всегда по exact `pack_id`.

---

## 3) `expired` / `expiring_soon`

### `expiring_soon`

1. `input.get(id)` → взять `revision`
2. `input.ttl(id, expected_revision, extend_minutes=...)`
3. Проверить обновлённый `freshness_state`

### `expired`

1. `input.list(freshness=expired)` или `output.list(freshness=expired)`
2. Если pack ещё в grace window — быстро снять нужные данные через `output.read(id)`
3. Если not_found/после purge — создать новый pack через `input.write` create
4. В новом pack в `qa`/`scope` явно указать связь: `replaces: <old_pack_id>`

---

## 4) `stale_ref`

**Сигнал**

- предупреждение `> stale ref: ...` в output
- либо `finalize_validation` с `invalid_refs`

**Что делать**

1. Исправить `path`/`line_start`/`line_end` в snapshot
2. Если anchor больше не релевантен — удалить ref из `document`
3. Повторить `input.write` (draft)
4. Повторить finalize precheck (`validate_only=true`)

---

## 5) Быстрый triage-алгоритм

1. Классифицируй ошибку (`validation|conflict|not_found|stale_ref|invalid_state`)
2. Восстанови консистентность (`get revision`, `exact id`, `freshness check`)
3. Повтори только один шаг пайплайна, не весь workflow
4. Зафиксируй итог в `qa.description` (что сломалось/как восстановили)
