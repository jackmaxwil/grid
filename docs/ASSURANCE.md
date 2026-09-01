# Grid assurance case

Claim → evidence, maintained from Wave 0. CI fails on a dangling claim
(`scripts/check-assurance-links.sh`). Methodology: *High Assurance Rust*.

| claim | statement | evidence | wave |
|---|---|---|---|
| C1 | no input to `chart-io` causes UB | segment.rs adversarial.rs + ci:miri+asan | 4 |
| C2 | same (source, epoch) ⇒ same bytes | determinism.rs store.rs corpus.rs + incremental≡full | 4–5 |
| C3 | every walk terminates within fuel | proofs.rs walk.rs + ci:kani | 9 |
| C4 | no cache hit without established closure | closure.rs (cache) conformance.rs | 7 |
| C5 | two writers only (survey, edit.apply); unsafe confined to `chart-io` | check-unsafe-confined.sh | 0+ |
| C6 | a datum ignores formatting, comments and local renaming; `span_hash` does not | properties.rs determinism.rs | 2 |
| C7 | the normalizer never panics on arbitrary bytes and never disagrees with itself | normalize.rs | 2+ |

| C8 | the gate holds if and only if some itemized divergence exists | verdict.rs | 3 |
| C9 | a promise pattern matches a callsign only in name-space; an unnamed datum makes no promise good | pattern.rs verdict.rs | 3 |
| C10 | every wire method the daemon claims is answered over the real socket, not only in-process | conformance.rs | 1+ |

| C11 | a store reopens valid after `kill -9` at any instant | crash_safety.rs | 4 |
| C12 | CSR indexing is total: a parsed segment answers every ordinal, in bounds | proofs.rs | 4 |

| C13 | an incremental survey and a full one produce the same segment bytes | determinism.rs corpus.rs | 5 |
| C14 | every recorded edge is one the language's own binding rules establish; an unresolvable or ambiguous name yields no edge | refs.rs determinism.rs | 5 |
| C15 | an edit addressed at a stale coordinate conflicts and writes nothing; it never lands against moved code | edit_apply.rs | 6 |

| C16 | an observed edge exists only where both endpoints resolve; every endpoint that does not resolve is returned, never dropped | ingest.rs | 8 |
| C17 | two wakes fork at the first hop where their call order parts, and each trace's exclusive edges are itemized | diff.rs | 8 |

| C18 | a bridged hop the language does not establish is labeled `Zoc::Modeled`, is deterministic, and is never upgraded to proven | query.rs walk.rs | 9 |
| C19 | `grid.query.walk` answers over the real socket with the same traversal the engine uses, not a second one | conformance.rs walk.rs | 9 |
| C20 | a review pack contains only paths that fulfill the request, plus the divergences that refused it | porcelain.rs conformance.rs | 9 |

| C21 | seeded retained spectra rank the injected culprit first by plain Ochiai, with datum identity as the deterministic tie-break | suspects.rs | 10 |
| C22 | seeded revision history finds the injected culprit deterministically and reports measured cache hits above one half of probes | bisect.rs | 10 |
| C23 | suspect ranking and bisect both refuse when retained production history is absent; fixture evidence is never presented as accumulated production evidence | wave10.rs conformance.rs bisect.rs | 10 |

| C24 | a definition's `sig` moves when and only when a caller could see the change; a snapshot that predates the sig column answers *unknown*, never *unchanged* | sig.rs sig_delta.rs epochs.rs | 10+ |
| C25 | a drift suspect is a dependent whose own datum did not move, reached by an edge at or above the query's floor; an inbound walk over budget refuses the whole answer, and every reference the survey declined is counted in the report, never silently absent | drift.rs conformance.rs | 10+ |
| C26 | an observed edge is graded `observed` only while both its endpoints are interned in the chart asked about; any endpoint that cannot be verified demotes it to `heuristic` with reason `stale-trace`, and the retained wake is never rewritten | decay.rs drift.rs | 10+ |
| C27 | a covenant fires only where one declared side changed and the other held, is symmetric in the two patterns, gates at `modeled` because a human declared it, and warns rather than silently passing when a pattern matches nothing at drift time | drift.rs covenant.rs | 10+ |
| C28 | a test's recorded verdict is reported fresh only when its closure key, recomputed at head, is one the cache holds; a closure that cannot be established is stale, and a verdict no retained wake claims is counted rather than checked | stale.rs drift.rs | 10+ |


## Dependency justification

Every dependency in `Cargo.lock` gets a line here saying why it is in the TCB.

- Wave 0: no dependencies.
- Wave 1 (`wire`): `serde`, `serde_json` — the external vocabulary is boring
  serde by design (engine plan); the wire is the only place JSON is parsed.
- Wave 2 (`datum`, since split into `datum` + `lang-py`): `blake3` — the hash
  the `Datum` schema names, so it is contract, not choice. `rustpython-parser`
  — a pure-Rust Python parser, no Python runtime and no C in the build; a
  parser must exist somewhere and this one keeps `forbid(unsafe_code)` intact
  outside `chart-io`. The parser now lives only in `grid-lang-py`; `grid-datum`
  is the parser-free identity vocabulary every frontend emits, so adding a
  language adds a `grid-lang-*` crate, never a parser above one. `proptest`
  (dev-only) — the property harness C6 is evidenced by.
- `lang-rs` (Rust frontend): `syn` (`full` + `visit`) — a pure-Rust parser
  for the entire language, no C, same `forbid(unsafe_code)` posture as
  rustpython; `proc-macro2` with `span-locations`, whose `Span::byte_range`
  is exact outside a proc-macro context, the only place a survey runs;
  `quote` for the `ToTokens` canonical printing. All three are one-author
  (dtolnay) MIT/Apache staples already vetted by most of crates.io.
- Wave 5 (`survey`): no new dependencies. Extraction reuses `datum`'s parser
  (one parse of a file serves identity and references alike), discovery is
  `git ls-files` rather than a walker crate so the repo's own ignore rules are
  the only policy, and the incremental key is `blake3`, already in the TCB.
- Wave 4 (`chart`, `chart-io`): `memmap2` — the mapping primitive itself; the
  alternative is raw `libc::mmap`, i.e. more unsafe, not less. `arc-swap` — a
  lock-free pointer swap so a publication cannot make a reader block or observe
  a torn epoch; hand-rolling this is exactly the code that should not be
  hand-rolled. `tempfile` (dev-only) — throwaway store directories for the
  fault-injection loop. `chart` itself pulls only `blake3` (checksums, snapshot
  ids) and `grid-wire` (the vocabulary it stores), so the crate an adversarial
  image reaches is `forbid(unsafe_code)` with a two-dependency TCB.
- Wave 7 (`cache`): no new dependencies. `blake3` freezes the key (the same
  hash the rest of the engine is identified by), `grid-chart` supplies the
  closure walk's graph, and the verdict log is `std::fs` — an append-only line
  file needs no database, and a database would be state a partial write could
  corrupt.

- Wave 10 (`grid`): no new dependencies. Seeded, test-local retained history
  makes the ranking, bisection, and cache-measurement regressions repeatable;
  it is algorithm evidence only, not weeks of production CI history. Stage 4+
  promotion remains gated until real retained history exists and measures the
  production cache-hit threshold.

- Wave 6 (`grid` agent verbs): no new dependencies. The write path reuses
  `grid-lang-py`'s parser — an edit's new identity is computed by the same
  normalization the survey would compute, so a post-edit survey can only
  agree — and file discovery stays `git ls-files` via `grid-survey`.
  `serde_json` becomes a *build* dependency of `grid` (the explain corpus is
  generated from the wire contract at build time); it has been in the TCB
  since Wave 1, so the lockfile gains no package.
- Wave 8 (`wake`): no new dependencies. A wake is ingested through
  `grid-chart`'s own resolver (the `grid.resolve` rule, so a trace names
  datums the same way every other verb does), retention is one `std::fs`
  file per wake keyed by `blake3` — already in the TCB, and hashing the id
  means no wake id is also a path — and the diff is `BTreeMap`/`BTreeSet`
  ordering, not a graph library.
- Wave 9 (`leadline` in `grid`): no new packages. `grid` gains a path
  dependency on the workspace's existing `grid-leadline` crate so the daemon
  runs the *same* traversal the engine is proven on (C3) rather than a second
  implementation the proofs would not cover; `Cargo.lock` gains nothing.
  Modeled bridge hops are static tables in `grid-leadline`, not an inference
  engine: a table is auditable and deterministic, and it is why C18 can say
  a modeled hop never becomes proven.

The segment format is the other compatibility surface: images are prefixed
`GRIDSEG\0` and carry a `FORMAT_EPOCH`, and the manifest carries the same
epoch. A format change is a refusal to load old bytes (`BadEpoch`) and a
re-survey, never a reinterpretation — the same declared-epoch discipline the
normalizer follows. The two halves of that rule differ by design:
`Segment::parse` keeps reading *every* epoch ever shipped, answering the
columns an older image does not declare with `None` (never a fabricated
value), pinned by a committed fixture per epoch; a manifest from a past epoch
is instead dropped, so `Store::open` yields the empty store and names the
epoch it superseded (`Store::superseded`) — the recovery the rule prescribes
is a re-survey, and a hard error there would take the re-survey down with
everything else.

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
