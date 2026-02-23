---
name: context-pack-repo-profile
description: "Проектный профиль по работе с context_pack для mcp/context_pack: инварианты, recovery и operator checklist."
---

# Context Pack Repo Profile (mcp/context_pack)

## Что это
Норматив для всех расследований/ревью, где используется `mcp__context_pack` в этом репозитории.

---

## Runtime invariants (в рамках SSOT этого репозитория)

1) **Coverage baseline policy**
- Baseline хранится в `.github/coverage-baseline.json`.
- Значение baseline: `line_coverage_percent`.
- Любая смена policy должна пройти `scripts/check_coverage_baseline.sh` и быть зафиксирована в PR.
- Порог строгий: `0 < x <= 100`.

2) **Oversize guard**
- Лимит размера pack-файла по умолчанию — 512 KiB (`CONTEXT_PACK_MAX_PACK_BYTES`, по умолчанию 524288).
- Oversized payload должен быть rejected до persistence (create/save) с `invalid_data`.
- Не допускается persistence oversized pack, который ломает последующий list/get.

3) **Unreadable / malformed isolation**
- Коррумпированный, unreadable или oversized pack на диске не должен «блокировать» все операции.
- Перечисляющие пути (`input`/`output` list и enumerate) должны удалять такие pack-файлы и продолжать с валидными.
- TTL purge должен очищать такие файлы.

4) **delete_pack recovery**
- Для точечного восстановления используйте `input(delete_pack)`.
- Точка восстановления:
  1. локализовать проблемный `pack_id` (list/output);
  2. `input` `action: delete_pack` с `id`;
  3. верифицировать, что оставшиеся healthy packs читаются.

---

## Operational discipline

- Применять deterministic read protocol для больших пакетов:
  - `mode`: `full | compact`;
  - `limit` + `offset` для первой страницы;
  - `cursor`/`next` для продолжения.
- Regex-фильтр `match` обязателен только при необходимости сузить выборку и всегда проверять корректность.
- Ключевой handoff: `pack_id + <=200 символов summary`.

---

## Operator checklist

Для каждой задачи с контекстным сканированием:
1. `input(list)` (по возможности через query) и/или `output(list)` → зафиксировать стартовый набор
2. `input(create)` или `input(get)` → получить `revision`.
3. Вносить факт-секции (`upsert_section`, `upsert_ref`) с anchors.
4. Подготовить обязательные артефакты для release/мержа:
   - `pack_id` (finalized),
   - `review`-comment URL,
   - `review` verdict (`PASS`/`BLOCKED`),
   - `@codex` статус: optional/non-blocking (информационный).
5. На рисках High/Critical требуются минимум:
   - два anchors **или**
   - один anchor + независимая контрпроверка.
6. Перед финишем: `output(get)` и QA gate.
7. Set status:
   - `finalized` при полной воспроизводимости;
   - иначе `draft` + `gaps`.
8. Перед закрытием issue добавить `PR`, `pack_id`, AC mapping.

---

## Review handoff protocol (required by issue-55 contract)
- Default review path: dedicated `review` agent requested from PR via `@review review` (required).
- Review comment URL and `PASS|BLOCKED` verdict must be captured and posted in completion report.
- `@codex review` is optional/secondary and never blocking; if unavailable, document fallback explicitly.
- Review agent loop runs on finalized context pack (`pack_id`) as primary evidence.

## AC mapping шаблон для delivery-комментария в issue

- `global skills` обновлён — да/нет.
- `repo-local profile` добавлен — да/нет.
- `global review loop` соблюдён (обязательный `review` loop + `pack_id` + `review comment` URL + `PASS/BLOCKED`).
- `@codex` учтён как optional/non-blocking — да/нет.
- Указаны новые output/get параметры (`compact|full`, `limit|offset|cursor|next`, `match`).
- Указан обязательный review маршрут: `review` агент (required), `@codex` optional, review verdict captured, review comment URL + `pack_id` в completion.
- QA handoff дисциплина соблюдена (`pack_id + summary`).
- Тест-гейт выполнен: `cargo fmt --check && cargo test --all-targets --all-features`.
