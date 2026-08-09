# Grid assurance case

Claim → evidence, maintained from Wave 0. CI fails on a dangling claim
(`scripts/check-assurance-links.sh`). Methodology: *High Assurance Rust*.

| claim | statement | evidence | wave |
|---|---|---|---|
| C1 | no input to `chart-io` causes UB | segment.rs adversarial.rs + ci:miri+asan | 4 |
| C2 | same (source, epoch) ⇒ same bytes | determinism.rs store.rs + incremental≡full | 4–5 |
| C3 | every walk terminates within fuel | Kani fuel-decrease harness | 9 |
| C4 | no cache hit without established closure | adversarial closure proptest | 7 |
| C5 | two writers only (survey, edit.apply); unsafe confined to `chart-io` | check-unsafe-confined.sh | 0+ |
| C6 | a datum ignores formatting, comments and local renaming; `span_hash` does not | properties.rs determinism.rs | 2 |
| C7 | the normalizer never panics on arbitrary bytes and never disagrees with itself | normalize.rs | 2+ |

| C8 | the gate holds if and only if some itemized divergence exists | verdict.rs | 3 |
| C9 | a promise pattern matches a callsign only in name-space; an unnamed datum makes no promise good | pattern.rs verdict.rs | 3 |
| C10 | every wire method the daemon claims is answered over the real socket, not only in-process | conformance.rs | 1+ |

| C11 | a store reopens valid after `kill -9` at any instant | crash_safety.rs | 4 |
| C12 | CSR indexing is total: a parsed segment answers every ordinal, in bounds | proofs.rs | 4 |


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
- Wave 4 (`chart`, `chart-io`): `memmap2` — the mapping primitive itself; the
  alternative is raw `libc::mmap`, i.e. more unsafe, not less. `arc-swap` — a
  lock-free pointer swap so a publication cannot make a reader block or observe
  a torn epoch; hand-rolling this is exactly the code that should not be
  hand-rolled. `tempfile` (dev-only) — throwaway store directories for the
  fault-injection loop. `chart` itself pulls only `blake3` (checksums, snapshot
  ids) and `grid-wire` (the vocabulary it stores), so the crate an adversarial
  image reaches is `forbid(unsafe_code)` with a two-dependency TCB.

The segment format is the other compatibility surface: images are prefixed
`GRIDSEG\0` and carry a `FORMAT_EPOCH`, and the manifest carries the same
epoch. A format change is a refusal to load old bytes (`BadEpoch`) and a
re-survey, never a reinterpretation — the same declared-epoch discipline the
normalizer follows.

## The one unsafe block

`chart-io::map_file` calls `Mmap::map`. Grid contains no other unsafe, enforced
by `scripts/check-unsafe-confined.sh` (C5), which also refuses any *other*
crate that tries to relax the workspace `forbid(unsafe_code)`. The safety
argument is not "we are careful": published segment files are written to a temp
name and atomically renamed, are content-addressed so a rewrite would be
byte-identical, and are never opened for writing again — so the mapped bytes
are immutable for the life of the mapping. External tampering is out of model
and is caught anyway by the manifest checksum before any byte is interpreted.
Interpretation happens in `grid-chart`, which is safe code, so hostile bytes
reach validation and nothing else (C1).

- Wave 3 (`xte`, `grid`): `serde_json` — already in the TCB from Wave 1; the
  daemon speaks JSON-RPC, which is the wire's own format, so no second codec
  enters. `proptest` (dev-only) — the property harness C8/C9 are evidenced by.
  `tempfile` (dev-only) — the CLI and socket tests need a throwaway git repo;
  a test that runs against the developer's own tree is not a test. No runtime
  dependency was added: the daemon uses `std::os::unix::net`, and the CLI
  parses its own arguments, because the surface is nine verbs and a flag.

Normalization stability is a compatibility surface (implementation plan,
Risks): the canonical bytes are prefixed `grid-datum/v1`, so a parser upgrade
that moves normalization is a declared epoch bump, never a silent re-hash.
