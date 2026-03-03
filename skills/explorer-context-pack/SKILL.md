---
name: explorer-context-pack
description: "LEGACY (v2) skill. Deprecated for this repository. Use .codex/skills/context-pack-repo-profile (v3)."
---

# ⚠ explorer-context-pack — LEGACY (deprecated)

Этот skill-пакет сохранён только для исторического контекста и **не должен использоваться**
для ежедневной работы в `mcp/context_pack`.

Причина: содержимое было написано под v2 (`create/upsert_*/output get`) и противоречит
каноническому v3 контракту.

## Что использовать вместо него

- Canonical repo-local skill:
  - [`../../.codex/skills/context-pack-repo-profile/SKILL.md`](../../.codex/skills/context-pack-repo-profile/SKILL.md)
- Основной контракт:
  - [`../../TECHNICAL.md`](../../TECHNICAL.md)

## Быстрый migration map (v2 -> v3)

- `input.create` -> `input.write(document=...)` (create)
- `upsert_section/ref/diagram` -> `input.write(document full snapshot)`
- `set_status` -> `input.write(document.status=...)`
- `touch_ttl` -> `input.ttl(ttl_minutes|extend_minutes, expected_revision)`
- `output.get` -> `output.read`

Если у тебя в промпте/шаблоне встречается v2-глагол — перепиши на v3 перед выполнением.
