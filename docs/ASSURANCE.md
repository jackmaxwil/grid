# Grid assurance case

Claim → evidence, maintained from Wave 0. CI fails on a dangling claim
(`scripts/check-assurance-links.sh`). Methodology: *High Assurance Rust*.

| claim | statement | evidence | wave |
|---|---|---|---|
| C1 | no input to `chart-io` causes UB | loader fuzz + MIRI + ASan | 4 |
| C2 | same (source, epoch) ⇒ same bytes | determinism property + incremental≡full | 4–5 |
| C3 | every walk terminates within fuel | Kani fuel-decrease harness | 9 |
| C4 | no cache hit without established closure | adversarial closure proptest | 7 |
| C5 | two writers only (survey, edit.apply); unsafe confined to `chart-io` | grep-gate | 0+ |
| C6 | a datum ignores formatting, comments and local renaming; `span_hash` does not | properties.rs determinism.rs | 2 |
| C7 | the normalizer never panics on arbitrary bytes and never disagrees with itself | normalize.rs | 2+ |

## Dependency justification

Every dependency in `Cargo.lock` gets a line here saying why it is in the TCB.

- Wave 0: no dependencies.
- Wave 1 (`wire`): `serde`, `serde_json` — the external vocabulary is boring
  serde by design (engine plan); the wire is the only place JSON is parsed.
- Wave 2 (`datum`): `blake3` — the hash the `Datum` schema names, so it is
  contract, not choice. `rustpython-parser` — a pure-Rust Python parser, no
  Python runtime and no C in the build; a parser must exist somewhere and this
  one keeps `forbid(unsafe_code)` intact outside `chart-io`. `proptest`
  (dev-only) — the property harness C6 is evidenced by.

Normalization stability is a compatibility surface (implementation plan,
Risks): the canonical bytes are prefixed `grid-datum/v1`, so a parser upgrade
that moves normalization is a declared epoch bump, never a silent re-hash.
