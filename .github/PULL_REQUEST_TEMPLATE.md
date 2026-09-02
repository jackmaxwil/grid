<!-- Cold-reader narrative. Every section is prose a reviewer who was not in the session can
     follow. No checkboxes: gate proof lives in the CI status checks alone, where it cannot
     be ticked by hand. Delete a section only if it is truly empty, and say so in Summary
     when that matters. -->

## Summary
<!-- What changed and why, in a few sentences a cold reader can follow without the diff
     open. -->

## User outcomes
<!-- What someone running `grid` can do, see, or rely on after this that they could not
     before. "Nothing user-visible" is a valid answer — say why the change exists anyway. -->

## Claims touched
<!-- The docs/ASSURANCE.md rows this change adds, revises, or discharges: claim id,
     statement, wave. A new claim names its evidence file in the same PR, or
     check-assurance-links.sh fails. "None" if none. -->

## Evidence
<!-- For each claim above and for the behavior itself: which test, fuzz target, proof
     harness or CI job proves it, and how it was seen red. A regression test run against
     the unfixed code, a gate disabled to watch its test fail — a green test proves nothing
     until it has failed for the right reason. -->

## Wire and format changes
<!-- Edits to docs/wire-v1.openrpc.json, the regenerated crates/wire diff, porcelain
     records, FORMAT_EPOCH movement, fixtures added (never edited). Wire evolution is
     additive: new fields optional or defaulted, never renamed, never repurposed.
     "None" if none. -->

## Files edited
<!-- A map, not a list: group by crate, one line each on why that area was touched. -->

## Dependencies
<!-- "None" is the expected answer. A new runtime crate needs its line in ASSURANCE.md's
     dependency justification, its deny.toml consequence, and the argument for why std or
     something already in the tree does not do the job. -->

## Assurance notes
<!-- Ratchets moved and why (a baseline edit lands in its own commit), guardrail debt paid
     off or taken on, decisions revised. -->
