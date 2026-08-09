#!/usr/bin/env bash
# Every claim row in docs/ASSURANCE.md must name at least one evidence target
# that exists (test file, fuzz target, script, or CI job). CI fails on a
# dangling claim.
set -euo pipefail
f=docs/ASSURANCE.md
[ -f "$f" ] || { echo "missing $f"; exit 1; }
fail=0
while IFS= read -r line; do
  case "$line" in
    "| C"*) ;;
    *) continue ;;
  esac
  claim=$(echo "$line" | cut -d'|' -f2 | tr -d ' ')
  evidence=$(echo "$line" | cut -d'|' -f4)
  found=0
  for token in $evidence; do
    case "$token" in
      grep-gate|ci:*) found=1 ;;  # CI-job evidence
      *)
        if find . -path ./target -prune -o -type f -name "*${token}*" -print | grep -q .; then found=1; fi ;;
    esac
  done
  if [ "$found" -eq 0 ]; then echo "dangling claim: $claim"; fail=1; fi
done < "$f"
exit $fail
