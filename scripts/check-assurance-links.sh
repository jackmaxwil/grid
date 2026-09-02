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
  # wave forms: "0+", "4", "4-5" (en dash or hyphen). One exclusive case, not three
  # overlapping ones: the third arm used to also match "N+" and its `[ "$due" = "$start" ]`
  # test was false there, so every standing claim — C5, C10, and all thirty "10+" rows —
  # skipped itself before the evidence was ever looked for.
  due=$(echo "$wave" | sed 's/[–—]/-/g')
  start=${due%%[+-]*}
  case "$due" in
    *+*) [ "$current" -ge "$start" ] || continue ;;
    *-*) end=${due##*-}; [ "$current" -ge "$end" ] || continue ;;
    *)   [ "$current" -ge "$start" ] || continue ;;
  esac
  # Every token, not one of them. A row naming four files was satisfied by any one
  # existing, so a claim could lose its actual evidence and stay green as long as a
  # sibling file survived — which is a link check that does not check the link.
  for token in $evidence; do
    case "$token" in
      grep-gate) continue ;;
      # `ci:<job>` names a CI job, and until now it was skipped before the
      # existence test — so C3 claimed `ci:kani` for a job that lived only in
      # .github/workflows/, on a remote whose push URL is `no_push`. A claim
      # whose evidence is a job that runs nowhere is a claim with no evidence.
      # The estate is Forgejo, so that is where a job has to exist.
      ci:*)
        job=${token#ci:}
        if ! grep -qE "^  ${job}:" .forgejo/workflows/*.yml; then
          echo "dangling claim: $claim (due wave $wave) — no CI job '$job' in .forgejo/workflows/"
          fail=1
        fi
        continue ;;
      *+*) continue ;;  # the + joining a composite evidence list
    esac
    if ! find . -path ./target -prune -o -type f -name "*${token}*" -print | grep -q .; then
      echo "dangling claim: $claim (due wave $wave) — nothing matches '$token'"
      fail=1
    fi
  done
done < "$f"
exit $fail
