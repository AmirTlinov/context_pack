#!/usr/bin/env bash
set -euo pipefail

BASELINE_FILE="${COVERAGE_BASELINE_FILE:-.github/coverage-baseline.json}"
MIN_COVERAGE="${COVERAGE_MIN_LINES:-}"

if [[ ! -f "$BASELINE_FILE" ]]; then
  echo "error: coverage baseline file not found: $BASELINE_FILE" >&2
  exit 1
fi

if [[ ! -s "$BASELINE_FILE" ]]; then
  echo "error: coverage baseline file is empty: $BASELINE_FILE" >&2
  exit 1
fi

if [[ -z "$MIN_COVERAGE" ]]; then
  if ! command -v python3 >/dev/null 2>&1; then
    echo "error: python3 is required for baseline file parsing" >&2
    exit 1
  fi

  MIN_COVERAGE="$(python3 - "$BASELINE_FILE" <<'PY'
import json
import sys

baseline_path = sys.argv[1]

with open(baseline_path, 'r', encoding='utf-8') as fp:
    payload = json.load(fp)

baseline = payload.get('line_coverage_percent')
if baseline is None:
    raise SystemExit(1)

print(baseline)
PY
)"
fi

if [[ -z "$MIN_COVERAGE" ]]; then
  echo "error: baseline missing or invalid in '$BASELINE_FILE'." >&2
  echo "Expected key 'line_coverage_percent' with a numeric value." >&2
  exit 1
fi

echo "coverage policy: minimum TOTAL line coverage = ${MIN_COVERAGE}%"
echo "coverage baseline source: ${BASELINE_FILE}"

cargo llvm-cov --all-targets --all-features --fail-under-lines "$MIN_COVERAGE"
