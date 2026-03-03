# context_pack v3 — Finalize / QA gate + Freshness/TTL discipline

## 1) Finalize gate (fail-closed)

Перед `status=finalized` обязательно:

- есть секция `scope` и в ней есть content (description/ref/diagram)
- есть секция `findings` и в ней есть content
- есть секция `qa`, содержащая `verdict` (например, `verdict: pass`)
- все refs резолвятся (нет stale/broken anchors)

Если нет — ошибка `code=finalize_validation` с деталями:

- `missing_sections`
- `missing_fields`
- `invalid_refs`

## 2) Обязательная последовательность finalize

1. `input.get(id)` → взять актуальный `revision`
2. `input.write(validate_only=true, document.status=finalized)`
3. Исправить найденные проблемы
4. `input.get(id)` снова (если были правки)
5. `input.write(..., document.status=finalized)` (persist)
6. `output.read(id, profile=orchestrator)` — sanity check handoff

`validate_only=true` не должен менять persisted state и revision.

---

## 3) Freshness/TTL discipline (ежедневно)

### Freshness states

- `fresh`
- `expiring_soon` (<= 15 минут до TTL deadline)
- `expired`

### Базовые правила

- На create всегда ставь осмысленный `document.ttl_minutes`.
- Перед handoff/ревью проверяй `freshness_state` через `output.read` LEGEND.
- Если `expiring_soon` и пак нужен дальше — продли TTL через `input.ttl`.
- `expired` по умолчанию скрыт в `list`; для диагностики используй `freshness=expired`.

### Продление TTL (без потери контекста)

1. `input.get(id)` → взять `revision`
2. `input.ttl(id, expected_revision, extend_minutes=...)`
3. Проверить `freshness_state` через `input.get`/`output.read`

---

## 4) Expired grace window

- `CONTEXT_PACK_EXPIRED_GRACE_SECONDS` (default `900`)
- Внутри grace expired pack ещё может читаться по exact `id` (`input.get` / `output.read`).
- Чтобы увидеть expired packs в list, используй `freshness=expired` (иначе expired скрыты по умолчанию).
- После grace: ожидаем purge/not_found (не рассчитывай на восстановление старого id)

---

## 5) QA quick checklist перед handoff

- [ ] `status` ожидаемый (`draft|finalized`)
- [ ] `scope/findings/qa` присутствуют и содержательны
- [ ] `qa` содержит `verdict`
- [ ] нет stale refs
- [ ] `freshness_state != expired` (или explicitly acknowledged)
- [ ] handoff содержит exact `pack_id`
