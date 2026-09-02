---
name: grid-assurance
description: Add, revise or discharge a claim row in docs/ASSURANCE.md, and pick the verification rung that actually evidences it. Load when a change makes a new guarantee, when a claim's evidence moved, or when check-assurance-links.sh is red.
---

# The assurance case

`docs/ASSURANCE.md` is grid's law: one row per claim, each naming the evidence
that makes it good and the wave it is due. The methodology is *High Assurance
Rust* — assurance is **derived**, never asserted, and a claim with no runnable
evidence is a comment.

`scripts/check-assurance-links.sh` fails CI when a row whose wave is at or
before `docs/CURRENT_WAVE` names evidence that does not exist. Future claims
are tracked, not enforced.

## Writing a row

```
| C42 | <statement> | <evidence> | <wave> |
```

- **The statement is falsifiable and specific.** "The walk is correct" is not
  a claim; "every walk terminates within fuel" is. Say what holds, what the
  boundary is, and what happens at the boundary — a claim that does not name
  its refusal case is half a claim. Read C25 and C31 for the shape: both spell
  out the over-budget behaviour, because that is where a reader's doubt is.
- **The evidence column names files and CI jobs, not prose.** A bare filename
  (`walk.rs`, `conformance.rs`) is matched against the tree; `ci:<job>` and
  `grep-gate` are the forms for evidence that is a CI job rather than a file.
  Composite evidence joins with `+`.
- **The wave is when it is due**: `4`, `4–5`, or `10+` for standing. A claim
  due now with no evidence fails the gate; that is the point.
- Claim ids are append-only. Revising a claim edits its row and says what
  changed; it never renumbers, because commit messages and comments point at
  ids.

## Picking the rung

Climb har-verify's ladder until the rung matches the risk, then stop. Grid's
rungs, and what each is already used for:

| Rung | Grid's use | Recipe |
| --- | --- | --- |
| clippy + the lint wall | the panic budget, the unsafe forbid | `just lint` |
| unit and integration tests | every behavioural claim | `just test` |
| proptest | algebraic properties: normalization, identity, verdicts | in the suite |
| `cargo fuzz` | anything parsing untrusted bytes — `normalize`, `references`, `segment` | `just fuzz <target>` |
| differential | incremental survey vs full, `grid check` sections vs the standalone verbs | in the suite |
| MIRI + ASan | the one unsafe block and the bytes it maps (C1) | `just miri` |
| kani | absence of a bound violation over all inputs — walk termination (C3) | `just verify` |
| fault injection | crash safety, evidenced by running the race, not by argument (C11) | `just soak` |

`just har` runs the assurance lane: the guardrails, MIRI and the proofs.

## Discharging a claim

1. Write the test first and **watch it fail for the claim's own reason**. A
   green test proves nothing until it has been seen red: revert the code, see
   the failure, restore. Two of yi's tests written the other way passed
   against broken code on the first try.
2. If the evidence is a gate rather than a test, prove the gate the same way —
   disable the gate, not the test, and watch the test fail because the gate is
   gone.
3. Land the row and the evidence in the same commit, so the link check is
   never green on a promise.
4. If the claim is about something the code *refuses* to do, the test asserts
   the refusal and its reason, not just a non-crash. A refusal that returns an
   empty answer instead of an error is the failure mode grid's own claims
   (C23, C25, C40, C42) are written to exclude.

## When a claim cannot be made good

Say so in the row and move the wave out, or delete the claim. Do not weaken
the statement to fit the evidence you have — that is how an assurance case
stops meaning anything, and it is invisible in a diff.
