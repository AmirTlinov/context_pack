# v3 Contract Cheat-Sheet (copy/paste)

Источник правды: [TECHNICAL.md](../../../../TECHNICAL.md).

## Allowed actions

- `input`: `list`, `get`, `write`, `ttl`, `delete`
- `output`: `list`, `read`

> Если видишь `create`, `upsert_*`, `set_*`, `output get`, `delete_pack`, `cursor`, `match`, `mode` — это legacy/v2 и должно быть заменено.

---

## 1) `input.list`

```json
{
  "name": "input",
  "arguments": {
    "action": "list",
    "freshness": "fresh",
    "limit": 20,
    "offset": 0
  }
}
```

`freshness` опционален: `fresh | expiring_soon | expired`.
По умолчанию expired скрыты.

---

## 2) `input.get`

```json
{
  "name": "input",
  "arguments": {
    "action": "get",
    "id": "pk_abcd2345"
  }
}
```

Используй перед любой мутацией, чтобы взять актуальный `revision`.

---

## 3) `input.write` — create (new pack)

> Create: **не** передаём `id/name` на верхнем уровне и **не** передаём `expected_revision`.

```json
{
  "name": "input",
  "arguments": {
    "action": "write",
    "document": {
      "name": "ctx-auth-login-20260303",
      "title": "Auth login regression",
      "brief": "Evidence pack for auth login investigation",
      "tags": ["auth", "incident", "v3"],
      "ttl_minutes": 180,
      "status": "draft",
      "sections": [
        {
          "key": "scope",
          "title": "Scope",
          "description": "objective: locate failing auth path and root cause"
        },
        {
          "key": "findings",
          "title": "Findings",
          "description": "Root-cause evidence",
          "refs": [
            {
              "key": "auth_guard_01",
              "path": "src/auth/guard.rs",
              "line_start": 41,
              "line_end": 74,
              "title": "Login guard early-return",
              "why": "Missing token fallback branch",
              "group": "auth/login"
            }
          ]
        },
        {
          "key": "qa",
          "title": "QA",
          "description": "verdict: pending"
        }
      ]
    }
  }
}
```

---

## 4) `input.write` — update existing pack

> Update: нужен `id` (или `name`) + `expected_revision`.
> Для детерминизма и handoff всегда используй `id`.

```json
{
  "name": "input",
  "arguments": {
    "action": "write",
    "id": "pk_abcd2345",
    "expected_revision": 7,
    "document": {
      "name": "ctx-auth-login-20260303",
      "title": "Auth login regression",
      "brief": "Evidence pack for auth login investigation",
      "tags": ["auth", "incident", "v3"],
      "ttl_minutes": 240,
      "status": "draft",
      "sections": [
        {
          "key": "scope",
          "title": "Scope",
          "description": "objective: locate failing auth path and root cause"
        },
        {
          "key": "findings",
          "title": "Findings",
          "description": "Updated findings after rerun",
          "refs": [
            {
              "key": "auth_guard_01",
              "path": "src/auth/guard.rs",
              "line_start": 41,
              "line_end": 74,
              "title": "Login guard early-return",
              "why": "Token fallback still missing",
              "group": "auth/login"
            },
            {
              "key": "session_flow_02",
              "path": "src/auth/session.rs",
              "line_start": 10,
              "line_end": 39,
              "title": "Session init path",
              "why": "Shows expected branch",
              "group": "auth/session"
            }
          ]
        },
        {
          "key": "qa",
          "title": "QA",
          "description": "verdict: pending"
        }
      ]
    }
  }
}
```

`input.write` — full-replace snapshot: если секцию не включить в `document.sections`, она исчезнет.

---

## 5) Finalize precheck (`validate_only=true`) — обязательный gate

```json
{
  "name": "input",
  "arguments": {
    "action": "write",
    "id": "pk_abcd2345",
    "expected_revision": 8,
    "validate_only": true,
    "document": {
      "name": "ctx-auth-login-20260303",
      "title": "Auth login regression",
      "brief": "Ready for finalize",
      "tags": ["auth", "incident", "v3"],
      "ttl_minutes": 240,
      "status": "finalized",
      "sections": [
        { "key": "scope", "title": "Scope", "description": "objective: ..." },
        { "key": "findings", "title": "Findings", "description": "evidence: ..." },
        { "key": "qa", "title": "QA", "description": "verdict: pass" }
      ]
    }
  }
}
```

---

## 6) Finalize commit (persist)

```json
{
  "name": "input",
  "arguments": {
    "action": "write",
    "id": "pk_abcd2345",
    "expected_revision": 8,
    "document": {
      "name": "ctx-auth-login-20260303",
      "title": "Auth login regression",
      "brief": "Ready for finalize",
      "tags": ["auth", "incident", "v3"],
      "ttl_minutes": 240,
      "status": "finalized",
      "sections": [
        { "key": "scope", "title": "Scope", "description": "objective: ..." },
        { "key": "findings", "title": "Findings", "description": "evidence: ..." },
        { "key": "qa", "title": "QA", "description": "verdict: pass" }
      ]
    }
  }
}
```

---

## 7) `input.ttl` (set/extend)

```json
{
  "name": "input",
  "arguments": {
    "action": "ttl",
    "id": "pk_abcd2345",
    "expected_revision": 9,
    "extend_minutes": 120
  }
}
```

Ровно одно из полей: `ttl_minutes` или `extend_minutes`.

---

## 8) `input.delete`

```json
{
  "name": "input",
  "arguments": {
    "action": "delete",
    "id": "pk_abcd2345"
  }
}
```

---

## 9) `output.list`

```json
{
  "name": "output",
  "arguments": {
    "action": "list",
    "freshness": "expired"
  }
}
```

---

## 10) `output.read` — orchestrator compact (default)

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "profile": "orchestrator"
  }
}
```

---

## 11) `output.read` — reviewer full evidence

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

---

## 12) `output.read` — executor + contains + paging

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "profile": "executor",
    "contains": "auth/login",
    "limit": 8
  }
}
```

Продолжение по `next_page_token` из LEGEND:

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "page_token": "<next_page_token_from_legend>"
  }
}
```

`contains` — case-insensitive substring filter (не regex).
