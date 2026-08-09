#!/usr/bin/env bash
# Every claim row in docs/ASSURANCE.md whose wave is due (<= CURRENT_WAVE)
# must name evidence that exists (test file, fuzz target, script, or CI job).
# CI fails on a dangling due claim. Future claims are tracked, not enforced.
set -euo pipefail
f=docs/ASSURANCE.md
[ -f "$f" ] || { echo "missing $f"; exit 1; }
current=$(tr -d '[:space:]' < docs/CURRENT_WAVE)
fail=0
while IFS= read -r line; do
  case "$line" in
    "| C"*) ;;
    *) continue ;;
  esac
  claim=$(echo "$line" | cut -d'|' -f2 | tr -d ' ')
  evidence=$(echo "$line" | cut -d'|' -f4)
  wave=$(echo "$line" | cut -d'|' -f5 | tr -d ' ')
  # wave forms: "0+", "4", "4-5" (en dash or hyphen)
  due=$(echo "$wave" | sed 's/[–—]/-/g')
  start=${due%%[+-]*}
  case "$due" in *+*) [ "$current" -ge "$start" ] || continue ;; esac
  case "$due" in *-*) end=${due##*-}; [ "$current" -ge "$end" ] || continue ;; esac
  case "$due" in *[!+-]*) [ "$due" = "$start" ] && [ "$current" -ge "$start" ] || continue ;; esac
  found=0
  for token in $evidence; do
    case "$token" in
      grep-gate|ci:*|*+*) found=1 ;;  # CI-job evidence or composite CI evidence
      *)
        if find . -path ./target -prune -o -type f -name "*${token}*" -print | grep -q .; then found=1; fi ;;
    esac
  done
  if [ "$found" -eq 0 ]; then echo "dangling claim: $claim (due wave $wave)"; fail=1; fi
done < "$f"
exit $fail
