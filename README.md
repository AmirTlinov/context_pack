<h1 align="center">context-pack (MCP)</h1>

<p align="center">
  <b>High-signal context handoff for multi-agent coding</b><br/>
  Real code excerpts + comments + diagrams instead of opinion-only summaries.
</p>

<p align="center">
  <a href="#en">EN</a>
  &nbsp;•&nbsp;
  <a href="#ru">RU</a>
</p>

---

<a id="en"></a>

<details open>
<summary><b>EN</b></summary>

## What this MCP solves

Without `context-pack`, explorer/reviewer agents often send short opinions with refs, and the orchestrator must re-open files and re-verify facts.

With `context-pack`, agents place anchors (`path + line range`) via `input`, and `output` renders those anchors into real Markdown code excerpts.

### Why it matters

- **Lower cost**: fewer output tokens in handoff.
- **Higher quality**: factual, inspectable context instead of “trust me” summaries.
- **Less duplicate work**: orchestrator reads one “meaty” pack, not multiple source files.

---

## How it works (30 seconds)

1. Agent builds/updates a pack with `input` actions.
2. Agent adds sections, code anchors, comments, diagrams.
3. Agent calls `output get`.
4. Orchestrator receives `pack_id + short summary`, then opens one complete markdown pack.

---

## Install from GitHub Releases (recommended)

### One-line install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | bash
```

Pin a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | CONTEXT_PACK_VERSION=v0.1.0 bash
```

Install to a custom directory:

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | CONTEXT_PACK_INSTALL_DIR="$HOME/bin" bash
```

### One-line install (Windows PowerShell)

```powershell
iwr https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.ps1 -UseBasicParsing | iex
```

> Installers verify downloaded artifacts against `checksums.sha256` from the same release.

### Homebrew (macOS/Linux)

```bash
brew install --formula https://github.com/AmirTlinov/context_pack/releases/latest/download/mcp-context-pack.rb
```

### Scoop (Windows)

```powershell
scoop install https://github.com/AmirTlinov/context_pack/releases/latest/download/mcp-context-pack.json
```

### Manual install

1. Open **GitHub Releases** and download the archive for your OS/CPU.
2. Unpack `mcp-context-pack` (`.exe` on Windows).
3. Put it into your PATH (for example `~/.local/bin` on Linux/macOS).

> Release artifacts are published on each tag `v*` via `.github/workflows/release.yml`.
> Maintainers: release playbook is in `RELEASE.md`.

---

## Configuration (`config.toml` / `mcp.json`)

### Codex `config.toml`

```toml
[mcp_servers.context_pack]
command = "mcp-context-pack"
args = []

[mcp_servers.context_pack.env]
CONTEXT_PACK_ROOT = "/absolute/path/to/context-pack-data"
CONTEXT_PACK_SOURCE_ROOT = "__SESSION_CWD__"
CONTEXT_PACK_LOG = "mcp_context_pack=info"
CONTEXT_PACK_INITIALIZE_TIMEOUT_MS = "20000"
CONTEXT_PACK_MAX_PACK_BYTES = "524288"
CONTEXT_PACK_MAX_SOURCE_BYTES = "2097152"
```

### Generic `mcp.json`

```json
{
  "mcpServers": {
    "context_pack": {
      "command": "mcp-context-pack",
      "args": [],
      "env": {
        "CONTEXT_PACK_ROOT": "/absolute/path/to/context-pack-data",
        "CONTEXT_PACK_SOURCE_ROOT": "__SESSION_CWD__",
        "CONTEXT_PACK_LOG": "mcp_context_pack=info",
        "CONTEXT_PACK_INITIALIZE_TIMEOUT_MS": "20000",
        "CONTEXT_PACK_MAX_PACK_BYTES": "524288",
        "CONTEXT_PACK_MAX_SOURCE_BYTES": "2097152"
      }
    }
  }
}
```

### Parameter reference

| Parameter | Meaning |
|---|---|
| `command` | Binary path or executable name in `PATH` (recommended: `mcp-context-pack`) |
| `args` | Optional CLI args (usually `[]`) |
| `CONTEXT_PACK_ROOT` | Storage root (`{root}/packs/*.json`) |
| `CONTEXT_PACK_SOURCE_ROOT` | Source root used to resolve anchors into code excerpts (`__SESSION_CWD__`, `session_cwd`, `cwd`, `.` = current session dir) |
| `CONTEXT_PACK_LOG` | Log filter (stderr) |
| `CONTEXT_PACK_INITIALIZE_TIMEOUT_MS` | Wait timeout for first MCP `initialize` |
| `CONTEXT_PACK_MAX_PACK_BYTES` | Max bytes per stored pack file |
| `CONTEXT_PACK_MAX_SOURCE_BYTES` | Max bytes per source file when rendering excerpts |

> Recommendation: set `CONTEXT_PACK_ROOT` to a **root directory**, not to `.../packs`.
>
> Breaking note: storage format is JSON (`packs/*.json`). Legacy markdown packs are not supported.

---

## Install for agents (step-by-step)

1. Install `mcp-context-pack`:
   - from Releases (recommended), or
   - from source:
     ```bash
     cargo build --release
     # binary: target/release/mcp-context-pack
     ```
2. Choose stable dirs:
   - storage root (`CONTEXT_PACK_ROOT`)
   - source root policy (`CONTEXT_PACK_SOURCE_ROOT`)
3. Add the server config (`config.toml` or `mcp.json`).
4. Restart MCP client/session.
5. Smoke-check:
   - `input` with `{ "action": "list" }`
   - `output` with `{ "action": "list" }`
6. Enforce handoff discipline:
   - chat response: `pack_id + short summary`
   - full evidence: only in context-pack.

---

## Tool contract (short)

- Pack id format: `pk_[a-z2-7]{8}`.
- `input` actions: `list`, `create`, `get`, `upsert_section`, `delete_section`, `upsert_ref`, `delete_ref`, `upsert_diagram`, `set_meta`, `set_status`, `touch_ttl`, `delete_pack`.
- All mutating actions except `create` and `delete_pack` require `expected_revision`.
- `create` requires `ttl_minutes`.
- `touch_ttl` accepts exactly one: `ttl_minutes` or `extend_minutes`.
- `output` actions stay `list|get` (no extra tool/action sprawl).
- `output get` additive args: `mode(full|compact)`, `limit`, `offset`, `cursor`, `match` (regex).
- Deterministic `output get(name=...)` resolution order:
  1. prefer `finalized` candidates over non-finalized;
  2. inside that status tier, pick latest `updated_at`;
  3. if still tied, pick highest `revision`;
  4. if still tied, fail closed with `ambiguous` + `details.candidate_ids`.
- Successful `output get` includes selection rationale in LEGEND:
  - `selected_by`
  - `selected_revision`
  - `selected_status`
- Backward compatibility: `output get` without paging args keeps legacy full markdown shape.
- `mode=compact` keeps ref metadata and stale markers, but omits code fences for refs.
- Paging contract: `limit` + (`offset` first page or `cursor` continuation) with deterministic legend fields `has_more` + `next`.
- Cursor is fail-closed (`invalid_cursor` in message, `invalid_data` code) on stale/mismatch state.
- `match` invalid regex returns validation error (`invalid_data`) with explicit reason.
- `output` is always markdown (`format` is rejected).

---

## CI and coverage policy (maintainers)

Repository quality gates in CI enforce:

- `cargo test`
- `cargo clippy --all-targets --all-features -- -D warnings`
- coverage baseline policy (no silent regressions)

Coverage is checked by:

1. collecting `llvm-cov` coverage for all targets and features;
2. reading the TOTAL line-coverage value from the machine-readable report;
3. failing only when current line coverage drops below the configured baseline.
4. requiring a strict threshold: `0 < threshold <= 100` (negative / zero / above-100 thresholds are rejected).

The baseline is reviewable and stored in:

- `.github/coverage-baseline.json`

To update policy intentionally, change `line_coverage_percent` in that file (must stay within `0 < x <= 100`) and document the rationale in the PR.  
Do **not** bypass coverage with `--fail-under` disable flags.

You can run the same gate locally with:

```bash
scripts/check_coverage_baseline.sh
```

## Troubleshooting

- `revision_conflict` → re-read pack (`get`) and retry with fresh `expected_revision`.
- `stale_ref` → update or delete outdated anchor.
- `not_found` → pack likely expired by TTL.
- `tool output too large` → split pack into smaller sections (rendered output is bounded).
- name ambiguity is fail-closed:
  - error code: `ambiguous`
  - details: `candidate_ids` list for explicit operator choice/retry by exact id.
- malformed / oversized / unreadable pack files are handled deterministically:
  - List path: `input`/`output` actions that enumerate packs remove malformed/oversized pack files and proceed.
  - Purge path: TTL purge also removes malformed/oversized files and keeps healthy packs intact.
  - `delete_pack`: call `input` with `{ "action": "delete_pack", "id": "<pack_id>" }` when targeting a known corrupted pack.
  3. Re-run your normal query (`output`/`list`) to confirm the remaining healthy packs.

### Deterministic read examples

Exact id read:

```json
{
  "name": "output",
  "arguments": {
    "action": "get",
    "id": "pk_abcd2345"
  }
}
```

Name-based read (deterministic/fail-closed):

```json
{
  "name": "output",
  "arguments": {
    "action": "get",
    "name": "auth-handoff-pack"
  }
}
```

In successful output LEGEND, inspect:
- `selected_by` (`exact_id` or name-based policy marker)
- `selected_revision`
- `selected_status`

</details>

---

<a id="ru"></a>

<details>
<summary><b>RU</b></summary>

## Что решает этот MCP

Без `context-pack` explorer/reviewer-агенты часто отдают краткое мнение с рефами, а оркестратор вынужден заново открывать файлы и проверять факты.

С `context-pack` агент ставит якоря (`path + диапазон строк`) через `input`, а `output` превращает их в реальные markdown-вырезки кода.

### Почему это важно

- **Дешевле**: меньше output-токенов в handoff.
- **Качественнее**: фактический, проверяемый контекст вместо пересказа.
- **Быстрее**: оркестратор читает один “мясной” пакет, а не заново роется в коде.

---

## Как это работает (30 секунд)

1. Агент создаёт/обновляет пакет через `input`.
2. Добавляет секции, якоря кода, комментарии, диаграммы.
3. Вызывает `output get`.
4. Оркестратор получает `pack_id + короткий summary` и открывает один полный markdown-пакет.

---

## Установка из GitHub Releases (рекомендуется)

### Установка одной командой (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | bash
```

Зафиксировать конкретную версию:

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | CONTEXT_PACK_VERSION=v0.1.0 bash
```

Установить в нестандартную директорию:

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | CONTEXT_PACK_INSTALL_DIR="$HOME/bin" bash
```

### Установка одной командой (Windows PowerShell)

```powershell
iwr https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.ps1 -UseBasicParsing | iex
```

> Инсталлеры проверяют скачанный архив по `checksums.sha256` из того же релиза.

### Homebrew (macOS/Linux)

```bash
brew install --formula https://github.com/AmirTlinov/context_pack/releases/latest/download/mcp-context-pack.rb
```

### Scoop (Windows)

```powershell
scoop install https://github.com/AmirTlinov/context_pack/releases/latest/download/mcp-context-pack.json
```

### Ручная установка

1. Откройте **GitHub Releases** и скачайте архив под вашу ОС/архитектуру.
2. Распакуйте `mcp-context-pack` (`.exe` на Windows).
3. Положите бинарник в `PATH` (например, `~/.local/bin` на Linux/macOS).

> Release-артефакты публикуются на каждый тег `v*` через `.github/workflows/release.yml`.
> Для сопровождающих: сценарий релиза описан в `RELEASE.md`.

---

## Настройка (`config.toml` / `mcp.json`)

### Codex `config.toml`

```toml
[mcp_servers.context_pack]
command = "mcp-context-pack"
args = []

[mcp_servers.context_pack.env]
CONTEXT_PACK_ROOT = "/absolute/path/to/context-pack-data"
CONTEXT_PACK_SOURCE_ROOT = "__SESSION_CWD__"
CONTEXT_PACK_LOG = "mcp_context_pack=info"
CONTEXT_PACK_INITIALIZE_TIMEOUT_MS = "20000"
CONTEXT_PACK_MAX_PACK_BYTES = "524288"
CONTEXT_PACK_MAX_SOURCE_BYTES = "2097152"
```

### Универсальный `mcp.json`

```json
{
  "mcpServers": {
    "context_pack": {
      "command": "mcp-context-pack",
      "args": [],
      "env": {
        "CONTEXT_PACK_ROOT": "/absolute/path/to/context-pack-data",
        "CONTEXT_PACK_SOURCE_ROOT": "__SESSION_CWD__",
        "CONTEXT_PACK_LOG": "mcp_context_pack=info",
        "CONTEXT_PACK_INITIALIZE_TIMEOUT_MS": "20000",
        "CONTEXT_PACK_MAX_PACK_BYTES": "524288",
        "CONTEXT_PACK_MAX_SOURCE_BYTES": "2097152"
      }
    }
  }
}
```

### Что означает каждый параметр

| Параметр | Назначение |
|---|---|
| `command` | Путь к бинарнику или имя команды в `PATH` (рекомендуется: `mcp-context-pack`) |
| `args` | Опциональные аргументы CLI (обычно `[]`) |
| `CONTEXT_PACK_ROOT` | Корень хранилища (`{root}/packs/*.json`) |
| `CONTEXT_PACK_SOURCE_ROOT` | Корень исходников для превращения якорей в вырезки (`__SESSION_CWD__`, `session_cwd`, `cwd`, `.` = текущая директория сессии) |
| `CONTEXT_PACK_LOG` | Фильтр логов (stderr) |
| `CONTEXT_PACK_INITIALIZE_TIMEOUT_MS` | Таймаут ожидания первого MCP `initialize` |
| `CONTEXT_PACK_MAX_PACK_BYTES` | Максимальный размер файла пакета в байтах |
| `CONTEXT_PACK_MAX_SOURCE_BYTES` | Максимальный размер исходного файла при рендеринге вырезок |

> Рекомендация: задавайте `CONTEXT_PACK_ROOT` как **корневую папку**, не как `.../packs`.
>
> Важно: формат хранения — JSON (`packs/*.json`). Старые markdown-пакеты не поддерживаются.

---

## Пошаговая установка для агентов

1. Установите `mcp-context-pack`:
   - из Releases (рекомендуется), или
   - соберите из исходников:
     ```bash
     cargo build --release
     # бинарник: target/release/mcp-context-pack
     ```
2. Выберите стабильные пути:
   - корень хранилища (`CONTEXT_PACK_ROOT`)
   - политика source root (`CONTEXT_PACK_SOURCE_ROOT`)
3. Добавьте блок сервера в `config.toml` или `mcp.json`.
4. Перезапустите MCP-клиент/сессию.
5. Smoke-проверка:
   - `input` с `{ "action": "list" }`
   - `output` с `{ "action": "list" }`
6. Зафиксируйте формат handoff:
   - в чат: `pack_id + короткий summary`
   - все доказательства: внутри context-pack.

---

## Краткий контракт инструментов

- Формат id: `pk_[a-z2-7]{8}`.
- `input` actions: `list`, `create`, `get`, `upsert_section`, `delete_section`, `upsert_ref`, `delete_ref`, `upsert_diagram`, `set_meta`, `set_status`, `touch_ttl`, `delete_pack`.
- Все мутации, кроме `create` и `delete_pack`, требуют `expected_revision`.
- `create` требует `ttl_minutes`.
- `touch_ttl` принимает строго одно: `ttl_minutes` или `extend_minutes`.
- `output` остаётся с actions только `list|get` (без разрастания API).
- Дополнительные аргументы `output get`: `mode(full|compact)`, `limit`, `offset`, `cursor`, `match` (regex).
- Детерминированное разрешение `output get(name=...)`:
  1. приоритет `finalized` над не-finalized;
  2. внутри выбранного статуса — максимальный `updated_at`;
  3. при равенстве — максимальный `revision`;
  4. если ничья осталась — fail-closed: `ambiguous` + `details.candidate_ids`.
- Успешный `output get` добавляет в LEGEND метаданные выбора:
  - `selected_by`
  - `selected_revision`
  - `selected_status`
- Обратная совместимость: `output get` без paging-аргументов возвращает прежний full-markdown формат.
- `mode=compact` сохраняет метаданные ref и stale-маркеры, но убирает code fences у ref.
- Paging-контракт: `limit` + (`offset` для первой страницы или `cursor` для продолжения) с детерминированными `has_more` и `next` в LEGEND.
- Cursor fail-closed: при stale/mismatch возвращается `invalid_data` с семантикой `invalid_cursor` в message.
- Невалидный regex в `match` возвращает validation error (`invalid_data`) с явной причиной.
- `output` всегда markdown (`format` отклоняется).

---

## Диагностика

- `revision_conflict` → перечитать пакет (`get`) и повторить мутацию с новым `expected_revision`.
- `stale_ref` → обновить или удалить устаревший якорь.
- `not_found` → пакет, скорее всего, истёк по TTL.
- `tool output too large` → разбить пакет на более мелкие секции.
- неоднозначный выбор по `name` fail-closed:
  - код ошибки: `ambiguous`
  - `details.candidate_ids`: список кандидатов для явного выбора по точному `id`.
- есть повреждённый/oversized/unreadable pack:
  - `input/output list` автоматически очищают такие pack-файлы при перечислении;
  - `purge_expired` удаляет такие файлы при background/оперативной очистке TTL;
  - для точечного удаления: `input` с `{ "action": "delete_pack", "id": "<pack_id>" }`.
  1. Найдите `pack_id` проблемного файла.
  2. Вызовите `input` с `{ "action": "delete_pack", "id": "<pack_id>" }`.
  3. Проверьте `output`/`list`, что здоровые пакеты по-прежнему доступны.

### Примеры детерминированного чтения

Чтение по точному id:

```json
{
  "name": "output",
  "arguments": {
    "action": "get",
    "id": "pk_abcd2345"
  }
}
```

Чтение по name (детерминированно/fail-closed):

```json
{
  "name": "output",
  "arguments": {
    "action": "get",
    "name": "auth-handoff-pack"
  }
}
```

В успешном LEGEND проверяйте:
- `selected_by` (`exact_id` или маркер name-политики)
- `selected_revision`
- `selected_status`

</details>
