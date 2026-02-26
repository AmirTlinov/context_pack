# context-pack — Technical Reference

This document is the reference documentation for developers integrating context-pack into agents, orchestrators, or tooling. It covers the complete tool contract, error schema, finalize rules, output profiles, paging, migration notes, and CI policy.

For installation and usage overview, see [README.md](README.md).

---

## Table of Contents

- [Tool contract](#tool-contract)
- [Error contract (v3)](#error-contract-v3)
- [Finalize checklist](#finalize-checklist-fail-closed)
- [Output read profiles](#output-read-profiles)
- [Paging contract](#paging-contract)
- [Deterministic read examples](#deterministic-read-examples)
- [Release notes / migration examples](#release-notes--migration-examples-58-62)
- [CI and coverage policy](#ci-and-coverage-policy-maintainers)

---

## Tool contract

- Pack id format: `pk_[a-z2-7]{8}`.
- `input` actions: `list`, `get`, `write`, `ttl`, `delete`.
- `input write` is **document-only** full-replace snapshot (`document` object); legacy `op` and granular mutation fields are rejected with guidance.
- Update writes require `id|name` + `expected_revision`; create writes omit both and allocate a new pack id.
- `validate_only=true` runs the same snapshot/finalize validations but does not persist changes.
- `ttl` accepts exactly one: `ttl_minutes` or `extend_minutes`.
- `output` actions: `list|read` (no extra tool/action sprawl).
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
- `contains` performs deterministic case-insensitive substring matching over rendered chunk text.
- `output` is always markdown (`format` is rejected).

---

## Error contract (v3)

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

---

## Finalize checklist (fail-closed)

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

---

## Output read profiles

| Profile | Default limit | Use case |
|---|---|---|
| `orchestrator` | 6 | Compact handoff-first page for routing decisions |
| `reviewer` | unlimited | Full evidence, complete code snippets, deep review |
| `executor` | higher than orchestrator | Actionable compact output for task execution |

Compact profiles (orchestrator, executor) include:
- objective/scope
- verdict/status
- top risks/gaps
- freshness metadata
- deep-nav hints
- ref metadata and stale markers (code fences omitted)

---

## Paging contract

- `limit` + (`offset` for first page, or `page_token` for continuation).
- Deterministic LEGEND fields: `has_more` + `next_page_token`.
- `page_token` is fail-closed (`invalid_page_token` in message, `invalid_data` code) on stale/mismatch state.
- `contains` performs deterministic case-insensitive substring matching over rendered chunk text.

In successful output LEGEND, inspect:
- `selected_by` (`exact_id` or name-based policy marker)
- `selected_revision`
- `selected_status`
- `profile` (effective read profile)
- `freshness_state` / `expires_at` / `ttl_remaining` (+ `warning` when stale risk is present)
- `next_page_token` (for paging continuation)

---

## Deterministic read examples

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

Filter by content substring:

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "contains": "TODO"
  }
}
```

Legacy `{ "action":"get", "id":"pk_abcd2345", "mode":"full", "cursor":"<cursor>", "match":"TODO" }` maps to:

```json
{
  "name": "output",
  "arguments": {
    "action": "read",
    "id": "pk_abcd2345",
    "profile": "reviewer",
    "page_token": "<page_token>",
    "contains": "TODO"
  }
}
```

---

## Release notes / migration examples (#58-#62)

Use this map when upgrading clients from pre-#58 behavior.

### #58 — freshness-state filters + stale-safe defaults

- Before: list/get consumers often had to infer staleness manually.
- After:
  - `input/output list` default hides expired packs (stale-safe),
  - `freshness=expired` explicitly surfaces stale packs,
  - stable metadata fields are present: `freshness_state`, `expires_at`, `ttl_remaining`.
- Migration:
  - default list: `{ "action":"list" }` (expired hidden),
  - explicit stale path: `{ "action":"list", "freshness":"expired" }`.

### #59 — deterministic `get(name=...)` resolution

- Before: name-based reads could be ambiguous without actionable routing context.
- After:
  - deterministic priority (`finalized` > latest `updated_at` > highest `revision`),
  - fail-closed ambiguity with `code=ambiguous` and `details.candidate_ids`,
  - success LEGEND includes `selected_by`, `selected_revision`, `selected_status`.

### #60 — fail-closed finalize checklist

- Before: finalize readiness could be under-specified in operator flows.
- After:
  - finalize requires `scope`, `findings`, `qa.verdict`,
  - stale/broken refs block finalize,
  - machine-readable `finalize_validation` details include `missing_sections`, `missing_fields`, `invalid_refs`.

### #61 — compact handoff-first default output

- Before (v2): `output get` default often returned full heavy markdown for routing decisions.
- After:
  - default `output read` is bounded compact handoff (`profile=orchestrator`, `limit=6`),
  - compact provides objective/scope/verdict/risks/gaps/deep-nav hints,
  - reviewer drill-down stays available via `profile=reviewer`.
- Migration:
  - `{ "action":"get", "id":"pk_abcd2345" }` -> `{ "action":"read", "id":"pk_abcd2345" }`
  - `{ "action":"get", "id":"pk_abcd2345", "mode":"full" }` -> `{ "action":"read", "id":"pk_abcd2345", "profile":"reviewer" }`

### #62 — actionable revision conflict diagnostics

- Before: conflict details were minimal (expected vs actual only).
- After:
  - `revision_conflict` details now include `expected_revision`, `current_revision` (`actual_revision` alias), `last_updated_at`, bounded `changed_section_keys`, `guidance`.
- Retry pattern:
  1. `input get` latest pack,
  2. merge intent against changed sections,
  3. retry with fresh `expected_revision`.

### #73 S3 — output read profiles + page_token

- Before (v2): clients used `output get` with `mode`, `cursor`, and regex `match`.
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
  - legacy `{ "action":"get", "id":"pk_abcd2345", "mode":"full", "cursor":"<cursor>", "match":"TODO" }` ->
    `{ "action":"read", "id":"pk_abcd2345", "profile":"reviewer", "page_token":"<page_token>", "contains":"TODO" }`.

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

Run the same gate locally:

```bash
scripts/check_coverage_baseline.sh
```
