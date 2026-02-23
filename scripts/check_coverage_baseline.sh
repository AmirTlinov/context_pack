#!/usr/bin/env bash
set -euo pipefail

BASELINE_FILE="${COVERAGE_BASELINE_FILE:-.github/coverage-baseline.json}"
MIN_COVERAGE="${COVERAGE_MIN_LINES:-}"

read_json_threshold() {
  local baseline_path="$1"
  local raw

  if ! raw="$(python3 - "$baseline_path" <<'PY' 
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as fp:
    payload = json.load(fp)

if not isinstance(payload, dict):
    print("error: baseline JSON must be an object", file=sys.stderr)
    raise SystemExit(1)

baseline = payload.get("line_coverage_percent")
if baseline is None:
    print("error: missing key 'line_coverage_percent'", file=sys.stderr)
    raise SystemExit(2)

if isinstance(baseline, bool):
    print("error: 'line_coverage_percent' must be numeric, got boolean", file=sys.stderr)
    raise SystemExit(3)

try:
    float(baseline)
except (TypeError, ValueError):
    print("error: 'line_coverage_percent' must be numeric", file=sys.stderr)
    raise SystemExit(4)

print(baseline)
PY
)"; then
    return 1
  fi

  printf '%s\n' "$raw"
}

validate_threshold() {
  local source="$1"
  local raw="$2"
  local validated

  if ! validated="$(python3 - "$raw" "$source" <<'PY' 
import math
import sys

raw = sys.argv[1].strip()
source = sys.argv[2]

try:
    value = float(raw)
except ValueError:
    print(f"error: coverage threshold from {source} is not numeric: {raw!r}", file=sys.stderr)
    raise SystemExit(2)

if not math.isfinite(value):
    print(f"error: coverage threshold from {source} is not finite: {raw!r}", file=sys.stderr)
    raise SystemExit(3)

if value <= 0 or value > 100:
    print(f"error: coverage threshold from {source} must satisfy 0 < x <= 100, got {raw!r}", file=sys.stderr)
    raise SystemExit(4)

print(f"{value:g}")
PY
)"; then
    return 1
  fi

  printf '%s\n' "$validated"
}

if [[ ! -f "$BASELINE_FILE" ]]; then
  echo "error: coverage baseline file not found: $BASELINE_FILE" >&2
  exit 1
fi

if [[ ! -s "$BASELINE_FILE" ]]; then
  echo "error: coverage baseline file is empty: $BASELINE_FILE" >&2
  exit 1
fi

if [[ -z "$MIN_COVERAGE" ]]; then
  if ! threshold_raw="$(read_json_threshold "$BASELINE_FILE")"; then
    echo "error: failed to read coverage baseline from '$BASELINE_FILE'." >&2
    exit 1
  fi
  source_desc="baseline file ($BASELINE_FILE)"
else
  threshold_raw="$MIN_COVERAGE"
  source_desc="environment variable COVERAGE_MIN_LINES"
fi

if ! MIN_COVERAGE="$(validate_threshold "$source_desc" "$threshold_raw")"; then
  echo "error: invalid coverage threshold validation failed for $source_desc." >&2
  exit 1
fi

echo "coverage policy: minimum TOTAL line coverage = ${MIN_COVERAGE}%"
echo "coverage threshold source: $source_desc"

cargo llvm-cov --all-targets --all-features --fail-under-lines "$MIN_COVERAGE"
