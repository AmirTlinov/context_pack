# context_pack v3 — Section templates (snapshot)

Ниже — шаблоны для `document.sections` в `input.write`.

## 1) Минимум для finalize

```json
[
  {
    "key": "scope",
    "title": "Scope",
    "description": "objective: <что проверяем>; in_scope: <границы>; out_of_scope: <что исключено>"
  },
  {
    "key": "findings",
    "title": "Findings",
    "description": "summary: <главный вывод>",
    "refs": [
      {
        "key": "finding_core_01",
        "path": "src/module/file.rs",
        "line_start": 10,
        "line_end": 34,
        "title": "Core finding anchor",
        "why": "Evidence for <claim>",
        "group": "module/flow"
      }
    ]
  },
  {
    "key": "qa",
    "title": "QA",
    "description": "verdict: pass; checks: <what was validated>; notes: <residual caveats>"
  }
]
```

> В `qa` должен присутствовать токен `verdict` (в title/description/ref/diagram).

---

## 2) Recommended extended layout

```json
[
  { "key": "scope", "title": "Scope", "description": "objective: ..." },
  { "key": "findings", "title": "Findings", "description": "summary: ..." },
  { "key": "risks", "title": "Risks", "description": "R1: ...; R2: ..." },
  { "key": "gaps", "title": "Gaps", "description": "missing evidence: ..." },
  { "key": "next_steps", "title": "Next steps", "description": "owner: ...; action: ..." },
  { "key": "qa", "title": "QA", "description": "verdict: pass|blocked; checks: ..." }
]
```

---

## 3) Reviewer-oriented findings block

```json
{
  "key": "findings",
  "title": "Findings",
  "description": "FND-auth-01: token fallback missing",
  "refs": [
    {
      "key": "auth_fallback_01",
      "path": "src/auth/guard.rs",
      "line_start": 41,
      "line_end": 74,
      "title": "FND-auth-01 anchor",
      "why": "Explains why unauthenticated branch exits early",
      "group": "auth/login"
    },
    {
      "key": "auth_fallback_02",
      "path": "src/auth/session.rs",
      "line_start": 10,
      "line_end": 39,
      "title": "FND-auth-01 cross-check",
      "why": "Shows expected behavior path",
      "group": "auth/session"
    }
  ]
}
```

---

## 4) QA section template (pass/blocked)

### PASS

```json
{
  "key": "qa",
  "title": "QA",
  "description": "verdict: pass; finalize_precheck: passed; stale_refs: none"
}
```

### BLOCKED

```json
{
  "key": "qa",
  "title": "QA",
  "description": "verdict: blocked; blocker: missing anchor for FND-auth-02; next: collect ref in src/auth/policy.rs"
}
```

---

## 5) Optional sections (when useful)

- `constraints` — внешние контракты/ограничения
- `decision_log` — принятые решения и причины
- `migration_plan` — поэтапная миграция
- `acceptance_checks` — чеклист проверок
- `handoff` — кого и с чем маршрутизировать дальше

Держи ключи короткими, стабильными, lowercase snake/kebab.
