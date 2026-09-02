---
name: review
description: >
  Verify finished work against its acceptance criteria with a cold-context
  reviewer. Use before declaring a wave step or large task complete, when
  the user asks for a completion review, or when a plan's final
  verification step calls for a fresh look. Not a code-style review — this
  checks that the claimed end state is actually true.
---

# Review

Verification by the mind that built the thing inherits its blind spots: the
checks were made inside the scope of the work that produced them. A
reviewer with cold context and only the standard finds what the implementer
cannot.

## Brief a cold reviewer

Spawn a subagent whose prompt contains **only the standard, never your
implementation story**:

- the plan's steps with their acceptance criteria, verbatim
  (`docs/plans/<plan>.md`);
- the assurance claim rows the work is supposed to make good
  (`docs/ASSURANCE.md`), verbatim;
- the gate commands: `just check`, plus any deeper rung the claim's evidence
  column names (`just miri`, `just verify`, `just soak`);
- the original user requirements that matter, verbatim;
- how to report (see below).

Do not include what you did, how you did it, what was hard, or what you
believe works — the reviewer must derive expectations from the standard
and measure the workspace against them.

## What the reviewer does

For every acceptance item: identify the evidence that would prove it, then
inspect the actual current state — run the checks, run the tests, read the
files, execute the binary against a real tree. Judgments, in order of
strength:

1. **proven** — a command exited 0 or the artifact demonstrably matches.
2. **contradicted** — evidence shows the claim false (quote it verbatim).
3. **unverifiable** — the acceptance is too vague to measure; say what
   evidence would be needed.

Match verification scope to claim scope: a narrow check never supports a
broad claim. A test that has never been seen red is not evidence — if the
work claims a regression is fixed, the reviewer's strongest move is to
revert the fix, watch the test fail, and restore.

Grid's own verbs are the reviewer's cheapest instrument here: `grid check`
reports what changed against what was promised, `grid drift --tests` names
the tests that are green but prove nothing about the change, and
`grid scope` builds the context pack for a definition before reading whole
files.

## Report

One line per acceptance item: `step · verdict · decisive evidence
(command + exit, or file:line)`. Then a coverage line: what was not checked
and why. Contradicted and unverifiable items reopen the step — they never
narrow the claim.
