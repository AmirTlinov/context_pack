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
3. Agent calls `output read`.
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
CONTEXT_PACK_EXPIRED_GRACE_SECONDS = "900"
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
        "CONTEXT_PACK_MAX_SOURCE_BYTES": "2097152",
        "CONTEXT_PACK_EXPIRED_GRACE_SECONDS": "900"
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
| `CONTEXT_PACK_EXPIRED_GRACE_SECONDS` | Grace window (seconds) for `expired` packs before purge/not_found (default `900`) |

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
- `output` actions stay `list|read` (no extra tool/action sprawl).
- `input list` and `output list` accept optional `freshness` filter:
  - `fresh`
  - `expiring_soon`
  - `expired`
- Default list behavior is stale-safe: expired packs are hidden unless `freshness=expired` is requested explicitly.
- Expired packs remain readable via `freshness=expired` for `CONTEXT_PACK_EXPIRED_GRACE_SECONDS` (default `900`) then are treated as unavailable.
- `output read` additive args: `profile(orchestrator|reviewer|executor)`, `limit`, `offset`, `page_token`, `contains` (case-insensitive substring).
- Default `output read` uses `profile=orchestrator`: **compact handoff-first page** bounded by default `limit=6`.
- `profile=reviewer` returns full evidence/snippets (deep review).
- `profile=executor` returns actionable compact output (higher default bound than orchestrator).
- Freshness metadata is normalized and stable in list/read surfaces:
  - `freshness_state`
  - `expires_at`
  - `ttl_remaining`
- Human-readable output (`output list|read`) adds concise warnings for `expiring_soon` and `expired`.
- Deterministic `output read(name=...)` resolution order:
  1. prefer `finalized` candidates over non-finalized;
  2. inside that status tier, pick latest `updated_at`;
  3. if still tied, pick highest `revision`;
  4. if still tied, fail closed with `ambiguous` + `details.candidate_ids`.
- Successful `output read` includes selection rationale in LEGEND:
  - `selected_by`
  - `selected_revision`
  - `selected_status`
- Compact profiles keep ref metadata and stale markers, but omit code fences for refs.
- Default orchestrator compact handoff is bounded (`limit=6` when omitted) and returns `next_page_token` for drill-down.
- Paging contract: `limit` + (`offset` first page or `page_token` continuation) with deterministic legend fields `has_more` + `next_page_token`.
- `page_token` is fail-closed (`invalid_page_token` in message, `invalid_data` code) on stale/mismatch state.
- `contains` performs deterministic case-insensitive substring matching over rendered chunk text.
- `output` is always markdown (`format` is rejected).

### Error contract (v3)

For v3 validation failures (`code=invalid_data`), payloads are normalized and stable:

- `kind`: always `validation`
- `code`: always `invalid_data`
- `details`: deterministic machine-readable guidance, including:
  - requested/legacy fields (`requested_action`, `requested_field`, `supported_field`, etc.),
  - allowed replacement sets (`allowed_actions`, `allowed_ops`),
  - and required inputs (`required_fields`, `mutually_interchangeable`).
- `message`: concise human-readable summary that mirrors the same intent as `details`.

Examples:

- `input`/`output` legacy action or field usage returns actionable guidance (`action='write'`, `use action='read'`, `unsupported_field` + `supported_field`).
- `input delete` and `output read` report required identifier keys explicitly (`id`/`name`).

### Finalize checklist (fail-closed)

Before setting `status=finalized`, ensure:
- `scope` section exists and has substance (description and/or refs/diagrams).
- `findings` section exists and has substance (description and/or refs/diagrams).
- `qa` section exists and contains a `verdict` field (for example: `verdict: pass`).
- all refs are resolvable (no stale/broken anchors).

If finalize validation fails, the error is returned as `finalize_validation` with structured details:
- `missing_sections`
- `missing_fields`
- `invalid_refs` (section/ref/path/line range/reason)

Draft workflow remains flexible: these checks are enforced only on finalize transition.

### Release notes / migration examples (#58-#62)

Use this quick map when upgrading clients from pre-#58 behavior.

1) **#58 freshness-state filters + stale-safe defaults**
- Before: list/get consumers often had to infer staleness manually.
- After:
  - `input/output list` default hides expired packs (stale-safe),
  - `freshness=expired` explicitly surfaces stale packs,
  - stable metadata fields are present: `freshness_state`, `expires_at`, `ttl_remaining`.
- Example:
  - default list: `{ "action":"list" }` (expired hidden),
  - explicit stale path: `{ "action":"list", "freshness":"expired" }`.

2) **#59 deterministic `get(name=...)` resolution**
- Before: name-based reads could be ambiguous without actionable routing context.
- After:
  - deterministic priority (`finalized` > latest `updated_at` > highest `revision`),
  - fail-closed ambiguity with `code=ambiguous` and `details.candidate_ids`,
  - success LEGEND includes `selected_by`, `selected_revision`, `selected_status`.

3) **#60 fail-closed finalize checklist**
- Before: finalize readiness could be under-specified in operator flows.
- After:
  - finalize requires `scope`, `findings`, `qa.verdict`,
  - stale/broken refs block finalize,
  - machine-readable `finalize_validation` details include
    `missing_sections`, `missing_fields`, `invalid_refs`.

4) **#61 compact handoff-first default output**
- Before: `output get` default often returned full heavy markdown for routing decisions.
- After:
  - default `output get` is bounded compact handoff (`mode=compact`, `limit=6`),
  - compact provides objective/scope/verdict/risks/gaps/deep-nav hints,
  - reviewer drill-down stays available via `mode=full`.

5) **#62 actionable revision conflict diagnostics**
- Before: conflict details were minimal (expected vs actual only).
- After:
  - `revision_conflict` details now include
    `expected_revision`, `current_revision` (`actual_revision` alias),
    `last_updated_at`, bounded `changed_section_keys`, `guidance`.
- Retry pattern:
  1. `input get` latest pack,
  2. merge intent against changed sections,
  3. retry with fresh `expected_revision`.

6) **#73 S3 output read profiles + page_token**
- Before: clients used `output get` with `mode`, `cursor`, and regex `match`.
- After:
  - output contract is `action=read` with profile routing:
    - `orchestrator` (default compact bounded),
    - `reviewer` (full evidence),
    - `executor` (actionable compact),
  - `page_token` replaces cursor continuation,
  - `contains` replaces regex complexity for deterministic substring filtering.
- Migration examples:
  - `{ "action":"read", "id":"pk_abcd2345" }`,
  - `{ "action":"read", "id":"pk_abcd2345", "profile":"reviewer" }`,
  - `{ "action":"read", "id":"pk_abcd2345", "page_token":"<token>" }`.

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
- `revision_conflict` now includes actionable diagnostics in `details`:
  - `expected_revision`
  - `current_revision` (compat alias: `actual_revision`)
  - `last_updated_at`
  - `changed_section_keys` (bounded list)
  - `guidance` (operator next steps)
- conflict-handling playbook:
  1. re-read latest pack (`input get`) to fetch current revision and state;
  2. merge your pending intent against latest sections listed in `changed_section_keys`;
  3. retry mutation with `expected_revision=current_revision` from reread.
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

Compact handoff-first read (default, bounded):

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345"
  }
}
```

Full drill-down read (complete snippets for review):

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "profile": "reviewer"
  }
}
```

Executor compact read (actionable, higher compact bound):

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "profile": "executor"
  }
}
```

Continue paging using LEGEND `next_page_token`:

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "page_token": "<next_page_token-from-legend>"
  }
}
```

List only expired packs (explicit stale surfacing):

```json
{
  "name": "output",
  "arguments": {
    "action": "list",
    "freshness": "expired"
  }
}
```

In successful output LEGEND, inspect:
- `selected_by` (`exact_id` or name-based policy marker)
- `selected_revision`
- `selected_status`
- `profile` (effective read profile)
- `freshness_state` / `expires_at` / `ttl_remaining` (+ `warning` when stale risk is present)
- `next_page_token` (for paging continuation)

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
        "CONTEXT_PACK_MAX_SOURCE_BYTES": "2097152",
        "CONTEXT_PACK_EXPIRED_GRACE_SECONDS": "900"
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
| `CONTEXT_PACK_EXPIRED_GRACE_SECONDS` | Сколько секунд истекший pack остаётся доступен как `expired` перед purge/not_found (по умолчанию `900`) |

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
- `input list` и `output list` принимают опциональный фильтр `freshness`:
  - `fresh`
  - `expiring_soon`
  - `expired`
- Поведение list по умолчанию stale-safe: expired-пакеты скрыты, пока явно не запрошен `freshness=expired`.
- Истёкшие пакеты видны через `freshness=expired` в течение окна `CONTEXT_PACK_EXPIRED_GRACE_SECONDS` (по умолчанию `900`), после чего считаются отсутствующими.
- Дополнительные аргументы `output get`: `mode(full|compact)`, `limit`, `offset`, `cursor`, `match` (regex).
- По умолчанию `output get` отдаёт **compact handoff-first страницу** (`mode=compact`, bounded `limit=6`), чтобы оркестратор принимал решение без перегруза.
- Compact handoff включает: objective/scope, verdict/status, top risks/gaps, freshness и deep-nav hints.
- Для полного ревью используйте `mode=full` (полные refs/anchors/snippets).
- В list/get добавлена нормализованная freshness-мета в стабильной форме:
  - `freshness_state`
  - `expires_at`
  - `ttl_remaining`
- В человекочитаемом выводе (`output list|get`) добавляются краткие предупреждения для `expiring_soon` и `expired`.
- Детерминированное разрешение `output get(name=...)`:
  1. приоритет `finalized` над не-finalized;
  2. внутри выбранного статуса — максимальный `updated_at`;
  3. при равенстве — максимальный `revision`;
  4. если ничья осталась — fail-closed: `ambiguous` + `details.candidate_ids`.
- Успешный `output get` добавляет в LEGEND метаданные выбора:
  - `selected_by`
  - `selected_revision`
  - `selected_status`
- `mode=compact` сохраняет метаданные ref и stale-маркеры, но убирает code fences у ref.
- По умолчанию compact handoff bounded (`limit=6`, если не указан) и отдаёт `next` для drill-down.
- Paging-контракт: `limit` + (`offset` для первой страницы или `cursor` для продолжения) с детерминированными `has_more` и `next` в LEGEND.
- Cursor fail-closed: при stale/mismatch возвращается `invalid_data` с семантикой `invalid_cursor` в message.
- Невалидный regex в `match` возвращает validation error (`invalid_data`) с явной причиной.
- `output` всегда markdown (`format` отклоняется).

### Finalize checklist (fail-closed)

Перед установкой `status=finalized` убедитесь:
- есть секция `scope` с содержимым (description и/или refs/diagrams);
- есть секция `findings` с содержимым (description и/или refs/diagrams);
- есть секция `qa`, где присутствует поле `verdict` (например: `verdict: pass`);
- все refs резолвятся (нет stale/broken anchors).

Если валидация финализации не проходит, возвращается `finalize_validation` со структурированными деталями:
- `missing_sections`
- `missing_fields`
- `invalid_refs` (section/ref/path/line range/reason)

Черновой workflow остаётся гибким: эти проверки применяются только при переходе в finalized.

### Release notes / migration-примеры (#58-#62)

Краткая карта миграции для клиентов, которые обновляются с поведения до #58.

1) **#58 freshness-state фильтры + stale-safe дефолты**
- До: клиентам часто приходилось вручную вычислять «просроченность» пакетов.
- После:
  - `input/output list` по умолчанию скрывают expired-пакеты (stale-safe),
  - `freshness=expired` явно показывает stale-пакеты,
  - стабильные поля свежести: `freshness_state`, `expires_at`, `ttl_remaining`.
- Пример:
  - дефолтный список: `{ "action":"list" }` (expired скрыты),
  - явный stale-path: `{ "action":"list", "freshness":"expired" }`.

2) **#59 детерминированное `get(name=...)`**
- До: выбор по имени мог быть неоднозначным без маршрутизирующих подсказок.
- После:
  - детерминированный приоритет (`finalized` > свежий `updated_at` > высокий `revision`),
  - fail-closed неоднозначность: `code=ambiguous` + `details.candidate_ids`,
  - в успешном LEGEND: `selected_by`, `selected_revision`, `selected_status`.

3) **#60 fail-closed finalize checklist**
- До: критерии готовности к finalize могли трактоваться неоднозначно.
- После:
  - finalize требует `scope`, `findings`, `qa.verdict`,
  - stale/broken refs блокируют finalize,
  - `finalize_validation` возвращает
    `missing_sections`, `missing_fields`, `invalid_refs`.

4) **#61 compact handoff-first дефолтный output**
- До: `output get` по умолчанию часто возвращал тяжёлый full markdown даже для routing.
- После:
  - дефолтный `output get` — bounded compact handoff (`mode=compact`, `limit=6`),
  - compact даёт objective/scope/verdict/risks/gaps/deep-nav hints,
  - reviewer drill-down остаётся через `mode=full`.

5) **#62 диагностируемые revision-конфликты**
- До: детали конфликта были минимальными (только expected/actual).
- После:
  - `revision_conflict` содержит
    `expected_revision`, `current_revision` (`actual_revision` alias),
    `last_updated_at`, ограниченный `changed_section_keys`, `guidance`.
- Retry-паттерн:
  1. перечитать пакет через `input get`,
  2. сверить намерение с изменившимися секциями,
  3. повторить мутацию с новым `expected_revision`.

---

## Диагностика

- `revision_conflict` → перечитать пакет (`get`) и повторить мутацию с новым `expected_revision`.
- `revision_conflict` теперь возвращает диагностику в `details`:
  - `expected_revision`
  - `current_revision` (совместимый алиас: `actual_revision`)
  - `last_updated_at`
  - `changed_section_keys` (ограниченный список)
  - `guidance` (подсказка оператору по следующим шагам)
- playbook при конфликте:
  1. перечитать актуальный пакет (`input get`), получить свежий `revision` и состояние;
  2. сверить/слить своё намерение с секциями из `changed_section_keys`;
  3. повторить мутацию с `expected_revision=current_revision` из перечитанного пакета.
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

Compact handoff-first чтение (по умолчанию, bounded):

```json
{
  "name": "output",
  "arguments": {
    "action": "get",
    "id": "pk_abcd2345"
  }
}
```

Полный drill-down для ревью (полные snippets):

```json
{
  "name": "output",
  "arguments": {
    "action": "get",
    "id": "pk_abcd2345",
    "mode": "full"
  }
}
```

Продолжение compact paging через LEGEND `next`:

```json
{
  "name": "output",
  "arguments": {
    "action": "get",
    "id": "pk_abcd2345",
    "cursor": "<next-from-legend>"
  }
}
```

Показать только expired-пакеты (явное surfacing stale):

```json
{
  "name": "output",
  "arguments": {
    "action": "list",
    "freshness": "expired"
  }
}
```

В успешном LEGEND проверяйте:
- `selected_by` (`exact_id` или маркер name-политики)
- `selected_revision`
- `selected_status`
- `freshness_state` / `expires_at` / `ttl_remaining` (+ `warning`, когда есть риск stale)
- `next` (для продолжения compact handoff paging)

</details>
