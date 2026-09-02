---
name: grid-wire
description: Change grid's external vocabulary — the OpenRPC contract, the generated grid-wire crate, and the porcelain records — without breaking a caller. Load before editing docs/wire-v1.openrpc.json, anything under crates/wire/src, or a porcelain column.
---

# Changing the wire

`crates/wire/src` is **generated**. `docs/wire-v1.openrpc.json` is the source
of truth and `scripts/gen-wire.py` writes the Rust from it.
`check_wire_drift.py` regenerates into a scratch copy, formats it, and fails
the gate unless the committed tree is byte-identical — so a hand edit to a
generated file is caught, and so is a contract change whose regeneration was
never run.

## The loop

1. Edit `docs/wire-v1.openrpc.json`. Add the example alongside the schema:
   `crates/wire/tests/conformance.rs` round-trips every example through the
   generated types byte-canonically, so an example is the test.
2. `just wire` — runs the generator and `cargo fmt`. Never edit
   `crates/wire/src` directly, and never format only the generated files.
3. `cargo test -p grid-wire`, then `just check`. The conformance suite in
   `crates/grid/tests/conformance.rs` answers over the real socket, not
   in-process: a method the daemon claims must actually be reachable (C10).
4. Update `docs/ONTOLOGY.md` if the change introduces a word. `grid explain`
   is generated from ONTOLOGY plus the contract at build time, so a new noun
   with no ontology row explains itself badly.
5. Update `docs/PORCELAIN.md` if a stdout column moves. Porcelain is a
   contract too — scripts parse it — and its version is in the document.

## What is allowed

Additive evolution only.

| Change | Verdict |
| --- | --- |
| New optional field, or one with a default | allowed |
| New method, new error code, new enum variant a reader tolerates | allowed |
| Renaming a field | **no** — a rename is a break; add the new name and keep the old |
| Repurposing a field's meaning | **no** — the worst kind of break, because it type-checks |
| Removing a field or method | **no** without a version bump and a migration |
| Tightening a type (optional → required, wider → narrower) | **no** — an old caller now fails |

`deny_unknown_fields` is deliberate on the wire: an unknown key is an error,
never a silent drop. That is what makes the additive rule enforceable, and it
is why removing a field is worse here than in a tolerant schema.

## Fixtures and epochs

- A committed fixture is never edited or deleted; add a new one beside it. The
  old ones are what prove an old caller still works.
- The segment format is the other compatibility surface and it plays by a
  different rule than the wire: `Segment::parse` keeps reading every epoch
  ever shipped, answering columns an older image does not declare with `None`
  — never a fabricated value — pinned by one fixture per epoch. A manifest
  from a past epoch is dropped and named, because the prescribed recovery
  there is a re-survey.
- Normalization is its own declared epoch (`grid-datum/v1`). A parser upgrade
  that moves canonical bytes is a bump, never a silent re-hash: every identity
  in every store depends on it.

## When it is a claim

A wire change that adds a guarantee adds or revises a row in
`docs/ASSURANCE.md`, in the same change, naming the test file that proves it.
`scripts/check-assurance-links.sh` fails on a due claim whose evidence does
not exist, so the row and the test land together or not at all.
