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

## Dependency justification

Every dependency in `Cargo.lock` gets a line here saying why it is in the TCB.
Wave 0: no dependencies.
