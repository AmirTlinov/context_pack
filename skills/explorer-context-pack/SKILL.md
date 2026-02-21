---
name: explorer-context-pack
description: "Операционный skill для реальных проектов: как агентам (explorer/deep_explorer/reviewer) собирать и передавать фактический контекст через mcp__context_pack (input/output), с ревизиями, TTL, QA gate и строгим handoff."
---

# Explorer Context Pack Skill (Project Mode)

Цель: передавать **фактический контекст из кода**, а не мнение. Вся «масса» — внутри context-pack, в чат — только `pack_id + summary`.

---

## Минимальный путь (для реальных задач)

1) `input(list)` → выбрать/создать pack (`create` с `ttl_minutes`).
2) `get` → взять `revision` → `upsert_section/ref/diagram` с `expected_revision`.
3) `output(get)` → самопроверка читаемости.
4) QA gate → `finalized` или `draft + gaps`.
5) Handoff в чат: `pack_id + summary <= 200 символов`.

---

## Hard rules (fail‑closed)

- Если агент исследует код/архитектуру/риски — skill обязателен.
- Канал данных: только `mcp__context_pack__input/output`.
- Никаких «простыней» в чат: все доказательства только в пакете.
- Summary: **1 строка, <=200 символов**, иначе результат невалиден.

---

## Когда читать reference‑файлы

- **actions.md** → точные обязательные поля по действиям.
- **qa-dod.md** → QA gate, finalized/draft, Critical/High требования.
- **recovery.md** → что делать при `revision_conflict/expired/not_found/stale_ref`.
- **role-modes.md** → Explorer / Deep / Reviewer + batch‑ритм.
- **examples.md** → каркасы секций для bug/review/feature задач.
- **troubleshooting.md** → частые DX‑проблемы (ttl, size‑limit, stale refs).

---

## DoD (минимум)

- Есть валидный `pack_id`.
- Summary в лимите и по делу.
- В пакете достаточно доказательств, чтобы оркестратор **не** повторял рескан кода.

Если не выполнено — `draft` или `BLOCKED` с причиной.
