# context-pack (MCP)

> **EN:** Turn agent research into **high-signal context packs** (real code excerpts + comments + diagrams), not vague opinions.  
> **RU:** Превращает исследование агентов в **плотные контекст-пакеты** (реальные вырезки кода + комментарии + диаграммы), а не расплывчатые мнения.

---

## 1) Why this MCP is useful / Чем этот MCP полезен

### EN
Without `context-pack`, explorer-like agents often return summaries with references, and the orchestrator must re-open files to verify reality.  
With `context-pack`, agents place anchors (`path + line range`) and the server renders those anchors into real Markdown code excerpts.

**Result:**
- fewer output tokens in agent-to-orchestrator handoff (cheaper runs);
- higher factual quality and completeness of context;
- less duplicate investigation by the orchestrator.

### RU
Без `context-pack` исследовательские агенты обычно передают пересказ + ссылки, и оркестратор вынужден заново лезть в код и проверять факты.  
С `context-pack` агент ставит якоря (`path + диапазон строк`), а сервер превращает их в реальные вырезки кода в Markdown.

**Итог:**
- меньше output-токенов при передаче результатов (дешевле);
- выше качество и полнота фактического контекста;
- оркестратору не нужно повторно делать ту же разведку.

---

## 2) Quick flow / Быстрый сценарий

1. Agent uses `input` to create/update a pack and add anchors.
2. Agent uses `output get` to render full Markdown with real excerpts.
3. Agent returns only `pack_id + short summary` to orchestrator.
4. Orchestrator reads one “meaty” pack instead of re-reading many files.

---

## 3) Build and run / Сборка и запуск

```bash
cargo build --release
./target/release/mcp-context-pack
```

Recommended env:

```bash
CONTEXT_PACK_ROOT=/tmp/context-pack \
CONTEXT_PACK_SOURCE_ROOT="$PWD" \
CONTEXT_PACK_LOG="mcp_context_pack=info" \
CONTEXT_PACK_INITIALIZE_TIMEOUT_MS=20000 \
./target/release/mcp-context-pack
```

> `CONTEXT_PACK_ROOT` is the storage root. Packs are stored in `{CONTEXT_PACK_ROOT}/packs/*.md`.

---

## 4) Configuration for clients / Параметры для клиентов

## 4.1 Codex `config.toml`

```toml
[mcp_servers.context_pack]
command = "/absolute/path/to/mcp-context-pack"
args = []

[mcp_servers.context_pack.env]
CONTEXT_PACK_ROOT = "/absolute/path/to/context-pack-data"
CONTEXT_PACK_SOURCE_ROOT = "__SESSION_CWD__"
CONTEXT_PACK_LOG = "mcp_context_pack=info"
CONTEXT_PACK_INITIALIZE_TIMEOUT_MS = "20000"
```

## 4.2 Generic `mcp.json`

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
        "CONTEXT_PACK_INITIALIZE_TIMEOUT_MS": "20000"
      }
    }
  }
}
```

## 4.3 What each parameter means / Что означает каждый параметр

| Parameter | EN | RU |
|---|---|---|
| `command` | Path to server binary. | Путь к бинарнику сервера. |
| `args` | Optional CLI args (usually empty). | Опциональные аргументы CLI (обычно пусто). |
| `CONTEXT_PACK_ROOT` | Storage root for packs. Actual files: `{root}/packs/*.md`. | Корень хранилища пакетов. Фактические файлы: `{root}/packs/*.md`. |
| `CONTEXT_PACK_SOURCE_ROOT` | Source tree root used to resolve anchors into code excerpts. `__SESSION_CWD__`, `session_cwd`, `cwd`, `.` mean current session directory. | Корень исходников для превращения якорей в цитаты кода. `__SESSION_CWD__`, `session_cwd`, `cwd`, `.` означают текущую директорию сессии. |
| `CONTEXT_PACK_LOG` | Log filter (stderr). | Уровень/фильтр логов (stderr). |
| `CONTEXT_PACK_INITIALIZE_TIMEOUT_MS` | How long server waits for first MCP `initialize` before exit. | Сколько сервер ждёт первый MCP `initialize` перед завершением. |

---

## 5) Tools and contract / Инструменты и контракт

Pack id format: `pk_[a-z2-7]{8}` (example: `pk_f2ireb33`).

### `input`
- default action (if omitted): `list`
- actions: `list`, `create`, `get`, `upsert_section`, `delete_section`, `upsert_ref`, `delete_ref`, `upsert_diagram`, `set_meta`, `set_status`, `touch_ttl`

Important rules:
- all mutating actions except `create` require `expected_revision`;
- `create` requires `ttl_minutes`;
- `touch_ttl` requires exactly one: `ttl_minutes` **or** `extend_minutes`;
- canonical fields only (legacy aliases are rejected):
  - `section_title`, `section_description`
  - `ref_title`, `ref_why`
  - `diagram_why`

### `output`
- default action: `list` (or `get` if `id/name` is provided)
- `get` renders Markdown with real code excerpts
- output format is always Markdown (`format` param is rejected)

### Success/Error shape

- successful `input`: JSON payload in `result.content[0].text`
- successful `output`: Markdown in `result.content[0].text`
- tool error: `isError=true` + strict JSON in `text`:

```json
{
  "error": true,
  "kind": "validation|not_found|conflict|invalid_state|stale_ref|io_error|migration_required",
  "code": "invalid_data|ttl_required|not_found|revision_conflict|...",
  "message": "...",
  "request_id": 123,
  "details": {}
}
```

---

## 6) Step-by-step install for agents / Пошаговая установка для агентов

### EN
1. Build server:
   ```bash
   cargo build --release
   ```
2. Choose stable storage directory and source root policy.
3. Add `context_pack` server block to your MCP client config (`config.toml` or `mcp.json`).
4. Restart MCP client/session.
5. Smoke test:
   - call `input` with `{ "action": "list" }`
   - call `output` with `{ "action": "list" }`
6. For explorer/reviewer agents: enforce handoff style `pack_id + short summary`; keep full evidence in context-pack.

### RU
1. Соберите сервер:
   ```bash
   cargo build --release
   ```
2. Выберите стабильный путь для хранилища и политику source root.
3. Добавьте блок `context_pack` в конфиг MCP-клиента (`config.toml` или `mcp.json`).
4. Перезапустите MCP-клиент/сессию.
5. Smoke-проверка:
   - вызовите `input` с `{ "action": "list" }`
   - вызовите `output` с `{ "action": "list" }`
6. Для explorer/reviewer агентов: в чат отправлять только `pack_id + короткий summary`, все доказательства хранить внутри context-pack.

---

## 7) Troubleshooting / Диагностика

- `revision_conflict`  
  EN: read latest pack (`get`) and retry with new `expected_revision`.  
  RU: перечитайте пакет (`get`) и повторите мутацию с новым `expected_revision`.

- `stale_ref`  
  EN: file/lines moved; update or delete stale ref.  
  RU: файл/строки изменились; обновите или удалите устаревший ref.

- `not_found`  
  EN: pack may have expired by TTL.  
  RU: пакет мог истечь по TTL.

- `tool output too large`  
  EN: rendered output exceeded size limit (~1 MiB). Split pack into smaller sections.  
  RU: итоговый output превысил лимит (~1 MiB). Разбейте пакет на более мелкие секции.
