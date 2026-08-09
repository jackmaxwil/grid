#!/usr/bin/env bash
# C5 evidence: unsafe code exists in `chart-io` and nowhere else.
#
# Matches unsafe as a *code* construct (`unsafe {`, `unsafe fn`, `unsafe impl`,
# `unsafe trait`), not the word — the crates that must never contain unsafe are
# exactly the ones whose docs explain why they do not.
set -euo pipefail
pattern='(^|[^[:alnum:]_])unsafe[[:space:]]*(\{|fn|impl|trait|extern)'
if grep -rnE "$pattern" crates --include='*.rs' | grep -v '^crates/chart-io/'; then
  echo "unsafe code outside chart-io"; exit 1
fi
# The inverse guard: chart-io is the only crate allowed to relax the workspace
# forbid, so nobody can quietly opt out somewhere else.
if grep -rn 'unsafe_code' crates --include='Cargo.toml' | grep -v '^crates/chart-io/'; then
  echo "a crate other than chart-io overrides the unsafe_code lint"; exit 1
fi
echo "unsafe confined to chart-io"
