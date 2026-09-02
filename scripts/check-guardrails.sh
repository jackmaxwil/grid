#!/usr/bin/env bash
# The gate entrypoint, ported from yi's scripts/guardrails/check_guardrails.sh. Ordered
# most-legible-failure-first, one failure count at the end, one exit code. `--fast` drops
# everything that needs a build or a cargo tool so the pre-commit hook stays under a couple
# of seconds; pre-push and CI run the whole thing.
set -u
cd "$(dirname "$0")/.."
FAST=0
[ "${1:-}" = "--fast" ] && FAST=1
PY="${PY:-$(command -v python3.13 || command -v python3.12 || command -v python3.11 || command -v python3)}"
"$PY" -c "import tomllib" 2>/dev/null || { echo "FAIL: $PY lacks tomllib (need python >= 3.11)"; exit 1; }
FAILED=0
run() { "$@" || FAILED=$((FAILED + 1)); }

run "$PY" scripts/guardrails/check_manifests.py
run "$PY" scripts/guardrails/check_boundaries.py
run "$PY" scripts/guardrails/check_commit_style.py
run "$PY" scripts/guardrails/check_commit_style.py --selfcheck
run "$PY" scripts/guardrails/check_hygiene.py
run "$PY" scripts/guardrails/check_blob_size.py
run scripts/check-unsafe-confined.sh
run scripts/check-assurance-links.sh
# Prose is not exempt: grid's comments and design docs are most of what a reader gets.
# Config and the domain-word allowlist live in .codespellrc.
if command -v codespell >/dev/null; then run codespell; else echo "FAIL codespell (uv tool install codespell)"; FAILED=$((FAILED+1)); fi

# crates/wire is generated, so proving the tree still matches its generator needs the
# generator to run and rustfmt to normalize its output. Both are cheap, but the check
# rewrites crates/wire/src in place and restores it, which is not something a commit hook
# should be doing under a partially staged tree.
if [ "$FAST" -eq 1 ]; then
  echo "skip wire_drift (--fast: it regenerates crates/wire/src in place)"
  echo "skip machete/deny (--fast: each wants a resolved dependency graph)"
else
  run "$PY" scripts/guardrails/check_wire_drift.py --selfcheck
  run cargo doc --workspace --no-deps --document-private-items -q
  if command -v cargo-machete >/dev/null; then run cargo machete crates; else echo "FAIL machete (cargo install cargo-machete)"; FAILED=$((FAILED+1)); fi
  if command -v cargo-deny >/dev/null; then run cargo deny check; else echo "FAIL deny (cargo install cargo-deny)"; FAILED=$((FAILED+1)); fi
fi

echo "----"
if [ "$FAILED" -eq 0 ]; then echo "guardrails: all green"; else echo "guardrails: $FAILED failing"; exit 1; fi
