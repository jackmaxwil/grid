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
# An optional tool is a binary on PATH, not a dependency: absent, it is announced and the
# gate stays green, so `just check` passes on a fresh clone — which is what README.md,
# docs/ASSURANCE.md and docs/TOOLING.md all say and what the code used to contradict by
# counting the absence as a failure. CI installs every one of them explicitly
# (.forgejo/workflows/grid-gate.yml), so the checks do not quietly stop running there.
optional() { # optional <binary> <install hint> <command...>
  local bin="$1" hint="$2"; shift 2
  if command -v "$bin" >/dev/null; then run "$@"; else echo "skip $bin ($hint)"; fi
}

run "$PY" scripts/guardrails/check_manifests.py
run "$PY" scripts/guardrails/check_boundaries.py
run "$PY" scripts/guardrails/check_commit_style.py
run "$PY" scripts/guardrails/check_commit_style.py --selfcheck
# The Row to Issue gate (.forgejo/workflows/grid-gate.yml, assurance job) only ever runs
# in CI, against the live forge; the tree's own check of it is this flag, over a fake
# transport that never leaves the process.
run "$PY" scripts/check_pr_metadata.py --selfcheck
run "$PY" scripts/guardrails/check_hygiene.py
run "$PY" scripts/guardrails/check_blob_size.py
run scripts/check-unsafe-confined.sh
run scripts/check-assurance-links.sh
# Prose is not exempt: grid's comments and design docs are most of what a reader gets.
# Config and the domain-word allowlist live in .codespellrc.
optional codespell "uv tool install codespell" codespell

# crates/wire is generated, so proving the tree still matches its generator needs the
# generator to run and rustfmt to normalize its output — a subprocess and a format pass per
# commit for a file set that only a contract edit moves. Cheap, but not free, and the whole
# point of the fast tier is that the hook is not felt.
if [ "$FAST" -eq 1 ]; then
  echo "skip wire_drift (--fast: it runs the generator and rustfmt)"
  echo "skip machete/deny (--fast: each wants a resolved dependency graph)"
else
  run "$PY" scripts/guardrails/check_wire_drift.py --selfcheck
  run cargo doc --workspace --no-deps --document-private-items -q
  optional cargo-machete "cargo install cargo-machete" cargo machete crates
  optional cargo-deny "cargo install cargo-deny" cargo deny check
fi

echo "----"
if [ "$FAILED" -eq 0 ]; then echo "guardrails: all green"; else echo "guardrails: $FAILED failing"; exit 1; fi
