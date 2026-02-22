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

## Configuration (`config.toml` / `mcp.json`)

### Codex `config.toml`

```toml
[mcp_servers.context_pack]
command = "/absolute/path/to/mcp-context-pack"
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
      "command": "/absolute/path/to/mcp-context-pack",
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
| `command` | Absolute path to server binary |
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

1. Build:
   ```bash
   cargo build --release
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
- `input` actions: `list`, `create`, `get`, `upsert_section`, `delete_section`, `upsert_ref`, `delete_ref`, `upsert_diagram`, `set_meta`, `set_status`, `touch_ttl`.
- All mutating actions except `create` require `expected_revision`.
- `create` requires `ttl_minutes`.
- `touch_ttl` accepts exactly one: `ttl_minutes` or `extend_minutes`.
- `output` is always markdown (`format` is rejected).

---

## Troubleshooting

- `revision_conflict` → re-read pack (`get`) and retry with fresh `expected_revision`.
- `stale_ref` → update or delete outdated anchor.
- `not_found` → pack likely expired by TTL.
- `tool output too large` → split pack into smaller sections (rendered output is bounded).

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

## Настройка (`config.toml` / `mcp.json`)

### Codex `config.toml`

```toml
[mcp_servers.context_pack]
command = "/absolute/path/to/mcp-context-pack"
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
      "command": "/absolute/path/to/mcp-context-pack",
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
| `command` | Абсолютный путь к бинарнику сервера |
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

1. Соберите сервер:
   ```bash
   cargo build --release
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
- `input` actions: `list`, `create`, `get`, `upsert_section`, `delete_section`, `upsert_ref`, `delete_ref`, `upsert_diagram`, `set_meta`, `set_status`, `touch_ttl`.
- Все мутации, кроме `create`, требуют `expected_revision`.
- `create` требует `ttl_minutes`.
- `touch_ttl` принимает строго одно: `ttl_minutes` или `extend_minutes`.
- `output` всегда markdown (`format` отклоняется).

---

## Диагностика

- `revision_conflict` → перечитать пакет (`get`) и повторить мутацию с новым `expected_revision`.
- `stale_ref` → обновить или удалить устаревший якорь.
- `not_found` → пакет, скорее всего, истёк по TTL.
- `tool output too large` → разбить пакет на более мелкие секции.

</details>
