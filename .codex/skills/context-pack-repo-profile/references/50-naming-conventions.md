# Naming conventions (`section_key`, `ref_key`, `group`)

Цель: стабилизировать структуру пакета и сделать фильтрацию/навигацию предсказуемой.

## 1) `section_key` / `ref_key` / `diagram.key` format

Валидатор ключей: `^[a-z0-9][a-z0-9_-]{1,63}$`

Практика:

- только lowercase
- длина 2..64
- без пробелов/кириллицы/точек
- стабильные ключи (не переименовывай без необходимости)

Примеры:

- section: `scope`, `findings`, `qa`, `next_steps`
- ref: `auth_guard_01`, `session_flow_02`, `risk_token_expiry_01`
- diagram: `flow_login_01`

---

## 2) Рекомендуемая семантика ключей

### Section keys

- `scope`, `findings`, `qa` — baseline для finalize
- optional: `risks`, `gaps`, `constraints`, `next_steps`, `acceptance_checks`

### Ref keys

Шаблон: `<area>_<topic>_<nn>`

- `auth_fallback_01`
- `storage_purge_02`
- `api_contract_03`

### Group names (`ref.group`)

`group` — произвольная строка для рендера групп (`### group: ...`).

Рекомендуемый шаблон: `<domain>/<subsystem>`

- `auth/login`
- `auth/session`
- `storage/purge`
- `mcp/output`

---

## 3) Как сделать `contains` действительно полезным

Важно: server-side `contains` ориентирован на отрендеренный chunk-текст (title/why/path/snippet/diagram text).

Поэтому:

1. Ключевой токен дублируй в `ref.title` или `ref.why`.
2. Для findings используй стабильные маркеры вида `FND-auth-01`.
3. Дублируй маркер в `qa.description` для итогового поиска.
4. Не рассчитывай только на `section_key/ref_key` как единственный search token.

Пример:

- `ref_key = auth_fallback_01`
- `ref.title = "FND-auth-01 anchor"`
- `ref.why = "FND-auth-01 confirms missing fallback"`

Тогда `contains="FND-auth-01"` работает детерминированно.

---

## 4) Handoff naming discipline

- В handoff всегда писать exact `pack_id`.
- Не передавать только `name`, если есть несколько одноимённых пакетов.
- Если pack recreated после expiry, явно указывать связь:
  - `replaces: pk_old1234`
