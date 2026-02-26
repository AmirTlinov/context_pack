<h1 align="center">context-pack (MCP)</h1>

<p align="center">
  <b>Your agents spend fewer tokens on handoffs and deliver more accurate context.</b>
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

## The Problem

When agents hand off work to each other, they typically write free-form summaries in chat: "I found a bug in file X around line Y, it looks like Z." This wastes output tokens, loses precision, and forces the receiving agent to re-read source files to verify what was said.

The more agents collaborate, the worse this gets.

## The Solution

context-pack gives agents a shared, structured workspace. Instead of describing code in prose, an agent places anchors (file path + line range) into a pack. The server renders those anchors into real code excerpts. The agent then sends only `pack_id + short summary` in chat — the receiving agent opens the pack and gets the full picture: exact code, comments, diagrams, verdicts.

## Key Benefits

- **Fewer output tokens**: agents describe findings in a structured pack instead of prose, cutting handoff message size significantly.
- **Higher accuracy**: context is anchored to actual code lines, not paraphrased — no "trust me" summaries.
- **Less redundant work**: the receiving agent reads one pack instead of re-opening multiple source files.

---

## Quick Start

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | bash
```

**Windows (PowerShell):**

```powershell
iwr https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.ps1 -UseBasicParsing | iex
```

> Installers verify downloaded artifacts against `checksums.sha256` from the same release.

Then add the server to your MCP config:

**Codex `config.toml`:**

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

**Generic `mcp.json`:**

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

Restart your MCP client. Smoke-check with `input { "action": "list" }`.

---

## How It Works

```
Agent                    context-pack server           Orchestrator
  |                            |                            |
  |-- input write (anchors) -->|                            |
  |   { path, line range,      |                            |
  |     sections, comments }   |                            |
  |                            |-- renders code excerpts    |
  |-- output read ------------>|                            |
  |                            |-- returns rendered pack -->|
  |                            |                            |
  |-- chat: pack_id + summary ----------------------->|    |
                                                       |    |
                                              reads pack,   |
                                              gets full      |
                                              context       |
```

1. The agent writes a pack via `input` — sections, code anchors (file + line range), comments, diagrams.
2. The agent calls `output read` to get the rendered markdown.
3. The agent sends `pack_id + short summary` in chat — nothing more.
4. The orchestrator opens the pack and gets complete, factual context.

---

## Configuration

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

> Set `CONTEXT_PACK_ROOT` to a **root directory**, not to `.../packs`.
>
> Storage format is JSON (`packs/*.json`). Legacy markdown packs are not supported.

---

## Reading a Pack

Compact handoff read (default — bounded, orchestrator profile):

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

For the full tool contract, paging, profiles, error codes, and migration notes — see [TECHNICAL.md](TECHNICAL.md).

---

## Troubleshooting

- `revision_conflict` — re-read the pack (`input get`) to get the current revision, then retry with `expected_revision` set to the value from the re-read.
- `stale_ref` — update or remove the outdated anchor.
- `not_found` — pack has likely expired by TTL.
- `tool output too large` — split the pack into smaller sections.
- `ambiguous` — name matched multiple packs; use exact `id` from `details.candidate_ids`.
- Corrupted or oversized pack files are removed automatically during list operations. To remove a specific pack: `input { "action": "delete_pack", "id": "<pack_id>" }`.

---

<details>
<summary>All install methods</summary>

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

### Build from source

```bash
cargo build --release
# binary: target/release/mcp-context-pack
```

> Release artifacts are published on each tag `v*` via `.github/workflows/release.yml`.
> Maintainers: release playbook is in `RELEASE.md`.

</details>

</details>

---

<a id="ru"></a>

<details>
<summary><b>RU</b></summary>

## Проблема

Когда агенты передают работу друг другу, они обычно пишут пересказы в чат: "Я нашёл баг в файле X примерно на строке Y, выглядит как Z." Это тратит output-токены, теряет точность и заставляет получающего агента заново открывать исходники, чтобы проверить сказанное.

Чем больше агентов в цепочке — тем больше потери.

## Решение

context-pack даёт агентам общее структурированное рабочее пространство. Вместо того чтобы описывать код словами, агент ставит якоря (путь к файлу + диапазон строк) в пакет. Сервер превращает якоря в реальные вырезки кода. Агент отправляет в чат только `pack_id + короткий summary` — получающий агент открывает пакет и видит полную картину: точный код, комментарии, диаграммы, вердикты.

## Ключевые преимущества

- **Меньше output-токенов**: агенты фиксируют находки в структурированном пакете вместо прозы — размер handoff-сообщений значительно сокращается.
- **Выше точность**: контекст привязан к конкретным строкам кода, а не к пересказу — никаких "доверься мне".
- **Меньше дублирующей работы**: получающий агент читает один пакет, а не заново открывает несколько исходных файлов.

---

## Быстрый старт

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.sh | bash
```

**Windows (PowerShell):**

```powershell
iwr https://raw.githubusercontent.com/AmirTlinov/context_pack/main/scripts/install.ps1 -UseBasicParsing | iex
```

> Инсталлеры проверяют скачанный архив по `checksums.sha256` из того же релиза.

Затем добавьте сервер в ваш MCP-конфиг:

**Codex `config.toml`:**

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

**Универсальный `mcp.json`:**

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

Перезапустите MCP-клиент. Smoke-проверка: `input { "action": "list" }`.

---

## Как это работает

```
Агент                    context-pack сервер           Оркестратор
  |                            |                            |
  |-- input write (якоря) ---->|                            |
  |   { путь, диапазон строк,  |                            |
  |     секции, комментарии }  |                            |
  |                            |-- рендерит вырезки кода    |
  |-- output read ------------>|                            |
  |                            |-- отдаёт отрендеренный  -->|
  |                            |   пакет                    |
  |-- чат: pack_id + summary ----------------------->|     |
                                                      |     |
                                           читает пакет,    |
                                           получает полный   |
                                           контекст         |
```

1. Агент записывает пакет через `input` — секции, якоря кода (файл + диапазон строк), комментарии, диаграммы.
2. Агент вызывает `output read` и получает отрендеренный markdown.
3. Агент отправляет в чат `pack_id + короткий summary` — и только это.
4. Оркестратор открывает пакет и получает полный, фактический контекст.

---

## Настройка

### Справка по параметрам

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

> Задавайте `CONTEXT_PACK_ROOT` как **корневую папку**, не как `.../packs`.
>
> Формат хранения — JSON (`packs/*.json`). Старые markdown-пакеты не поддерживаются.

---

## Чтение пакета

Compact handoff-чтение (по умолчанию — bounded, профиль orchestrator):

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345"
  }
}
```

Полный drill-down для ревью (полные snippets):

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

Полный контракт инструментов, постраничное чтение, профили, коды ошибок и примеры миграции — в [TECHNICAL.md](TECHNICAL.md).

---

## Диагностика

- `revision_conflict` — перечитайте пакет (`input get`), получите текущий revision, повторите мутацию с `expected_revision` из перечитанного пакета.
- `stale_ref` — обновите или удалите устаревший якорь.
- `not_found` — пакет, скорее всего, истёк по TTL.
- `tool output too large` — разбейте пакет на более мелкие секции.
- `ambiguous` — имя совпало с несколькими пакетами; используйте точный `id` из `details.candidate_ids`.
- Повреждённые или oversized-файлы пакетов удаляются автоматически при операциях list. Для точечного удаления: `input { "action": "delete_pack", "id": "<pack_id>" }`.

---

<details>
<summary>Все способы установки</summary>

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

### Сборка из исходников

```bash
cargo build --release
# бинарник: target/release/mcp-context-pack
```

> Release-артефакты публикуются на каждый тег `v*` через `.github/workflows/release.yml`.
> Для сопровождающих: сценарий релиза описан в `RELEASE.md`.

</details>

</details>
