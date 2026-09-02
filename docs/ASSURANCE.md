# Grid assurance case

Claim → evidence, maintained from Wave 0. CI fails on a dangling claim
(`scripts/check-assurance-links.sh`). Methodology: *High Assurance Rust*.

| claim | statement | evidence | wave |
|---|---|---|---|
| C1 | no input to `chart-io` causes UB | segment.rs adversarial.rs + ci:chart-io-sanitizers | 4 |
| C2 | same (source, epoch) ⇒ same bytes | determinism.rs store.rs corpus.rs | 4–5 |
| C3 | every walk terminates within fuel | proofs.rs walk.rs | 9 |
| C4 | no cache hit without established closure | closure.rs conformance.rs | 7 |
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
| C17 | two traces fork at the first hop where their call order parts, and each trace's exclusive edges are itemized | diff.rs | 8 |

| C18 | a bridged hop the language does not establish is labeled `Grade::Modeled`, is deterministic, and is never upgraded to proven | query.rs walk.rs | 9 |
| C19 | `grid.query.walk` answers over the real socket with the same traversal the engine uses, not a second one | conformance.rs walk.rs | 9 |
| C20 | a review pack contains only paths that fulfill the request, plus the divergences that refused it | porcelain.rs conformance.rs | 9 |

| C21 | seeded retained spectra rank the injected culprit first by plain Ochiai, with datum identity as the deterministic tie-break | suspects.rs | 10 |
| C22 | seeded revision history finds the injected culprit deterministically and reports measured cache hits above one half of probes | bisect.rs | 10 |
| C23 | suspect ranking and bisect both refuse when retained production history is absent; fixture evidence is never presented as accumulated production evidence | wave10.rs conformance.rs bisect.rs | 10 |

| C24 | a definition's `sig` moves when and only when a caller could see the change; a snapshot that predates the sig column answers *unknown*, never *unchanged* | sig.rs sig_delta.rs epochs.rs | 10+ |
| C25 | a drift suspect is a dependent whose own datum did not move, reached by an edge at or above the query's floor; an inbound walk over budget refuses the whole answer, and every reference the survey declined is counted in the report, never silently absent | drift.rs conformance.rs | 10+ |
| C26 | an observed edge is graded `observed` only while both its endpoints are interned in the chart asked about; any endpoint that cannot be verified demotes it to `heuristic` with reason `stale-trace`, and the retained trace is never rewritten | decay.rs drift.rs | 10+ |
| C27 | a covenant fires only where one declared side changed and the other held, is symmetric in the two patterns, gates at `modeled` because a human declared it, and warns rather than silently passing when a pattern matches nothing at drift time | drift.rs covenant.rs | 10+ |
| C28 | a test's recorded verdict is reported fresh only when its closure key, recomputed at head, is one the cache holds; a closure that cannot be established is stale, and a verdict no retained trace claims is counted rather than checked | stale.rs drift.rs | 10+ |
| C29 | a membership diff of two snapshots names exactly the definitions the delta path does; an image extended one snapshot at a time is byte-identical to one rebuilt from the whole chain; a snapshot the chain never held is a typed refusal, never an empty diff | membership.rs | 10+ |
| C30 | a weak-test finding requires all four facts — the definition's datum moved, a live trace covers it, the test's own datum held, and a pass is on record at both endpoints — is graded heuristic, and never moves the exit code; coverage from a decayed trace yields no finding and is counted | weak.rs | 10+ |
| C31 | an aground walk's `complete_within` is a layer it fully expanded, so the answer truncated to that depth equals the unbudgeted walk's under the same filter; a walk whose first layer did not finish certifies nothing | certificate.rs walk.rs | 10+ |
| C32 | a watch session's summed deltas equal a fresh `grid drift` after every prefix of an event sequence; an edit and its revert net to zero; an unparseable file event emits no deltas and is counted, and so does every later event until the file parses again | watch.rs | 10+ |
| C33 | a rev survey reads every blob it needs through one `git cat-file --batch` child, byte-identical to `git show` per path; a path the listing names that the rev cannot serve is skipped, never fabricated as empty, and never shifts a later answer onto the wrong file | rev_blobs.rs determinism.rs | 10+ |
| C34 | every declined reference is counted in exactly one of external, method, internal, macro, and the four sum to the declined total; a method call yields no edge whatever its receiver looks like, a reference an import binds is still an edge, a Rust prelude name or a Python builtin left unbound is external, never internal, and a macro invocation inside a definition is counted once as `macro` with nothing inside it ever bound or guessed | declined_split.rs refs.rs | 10+ |
| C35 | lcov and coverage.py reports describing the same executed lines produce the same observations and the same resolved hops, because they share one line-to-definition mapping; a file the chart does not hold and a line inside no definition are counted and reported; a report that does not parse is bad input, never a trace claiming the test reached nothing; and re-ingesting a report retains one trace, byte for byte | coverage.rs porcelain.rs | 10+ |
| C36 | `grid test` skips a test only when its closure key at head is on record in this environment *and* a retained trace still describing code base holds watched its reach *and* every changed definition is one some retained trace lists (or is itself an inventory test) — a change no trace has ever listed makes every test `unwatched` and nothing replays across it; a walk over fuel selects every test rather than a shorter list; a test with no live trace, no name in the chart, or no verdict on record is always run; over a seeded edit sequence every skipped test's recorded verdict equals a full run at head; and `uncovered` and `nondeterministic` are reported and never move the exit code | testplan.rs testrun.rs conformance.rs | 10+ |
| C37 | a definition's `cfg` is the conjunction of every `#[cfg]` that conditions it, including the declaration of its module and its cargo target directory; `grid check --compile` gates only on a proven dependent whose `cfg` the compile record decidably excludes, warns on every predicate it cannot decide, and a snapshot that predates the cfg column answers *unknown*, never *compiled* | cfg.rs compile.rs epochs.rs adversarial.rs | 10+ |
| C38 | a def promise's `interface: true` is made good only by a sig move or an unknown transition (conservative), `interface: false` only by a sig that provably held — never by unknown — and a sig move under a `false` promise, or matching no promise permitting one, is `unaccounted` with reason `interface`, a subset of `defs_unaccounted` held once; `interface: None` changes no verdict a plan without the field would have reached; `grid plan lint` rejects `interface` on an `add`/`remove` promise | verdict.rs lint.rs porcelain.rs | 10+ |
| C39 | `grid scope` without `--bodies`/`--sigs` emits the bytes it emitted before the tiers existed; with them, the definition set the pack certifies at each hop is the set `grid scope --depth k` names, the budget is spent in whole hop layers so no body is ever truncated and no layer half-packed, a body layer too large for the budget is demoted to its sigs before it is dropped, a larger budget's pack covers every definition a smaller one's did, and the horizon line is emitted even when nothing fit | pack.rs porcelain.rs conformance.rs | 10+ |
| C40 | `grid suspects --closure` names exactly the definitions the test's closure at head reaches whose datum moved since the named snapshot — the same set an independent `grid scope` at full depth intersected with `grid diff` names — each with the hop distance that ranked it; a closure that runs past its fuel refuses the whole answer and a snapshot the retained chain never held is a typed refusal, neither ever an empty ranking; a bare `--closure` resolves to the last snapshot the verdict log says that test passed at and refuses when no such record exists, again never an empty ranking; every row carries the callsign and `file:line` the chart gives its datum, so it can be handed to `grid explain` as printed, and carries no `score` — the porcelain prints `closure` and the JSON prints `hops`, never a number a consumer would read as confidence; and the mode is heuristic, so it never moves the exit code | attribution.rs suspects.rs | 10+ |
| C41 | a Deserialize compatibility finding is one of six decidable rules over two field lists reconstructed from source at both snapshots; a type whose interface held produces none, and `#[serde(default)]` on the added field silences the rule it tripped; a derive the source does not spell out — behind a `cfg_attr`, from a macro, or under a serde attribute the reader cannot parse — is counted `compat unchecked` and never declared compatible; and a finding moves the exit code only where a standing `persisted` invariant declared that old bytes exist | compat.rs serde_shape.rs | 10+ |
| C42 | `grid interfere` names a definition only where one side's drift suspect lands in the other side's edits since their common base, or where both sides moved the same definition; the report is symmetric in the two roots up to the mine/theirs labels; the other root's store is opened read-only, never surveyed and never created; and a store that cannot be opened, is superseded, or does not hold the base refuses with exit 2 and its reason, never with an empty answer; a dirty worktree survey is labelled with the rev it was checked out from and never answers a lookup for that rev, and a base surveyed on demand is retained without becoming the live epoch; and every refusal a survey would fix names the command and the tree to run it in — the *other* worktree where that is the store that refused — while a refusal a survey would not fix offers none | interfere.rs conformance.rs store.rs compile.rs check.rs | 10+ |
| C43 | `grid check` is its components: each section of `check --json` is the standalone verb's own `--json` (drift, `drift --tests`'s stale and weak, `drift --compat`'s findings, `grid gate`'s verdict), never a re-implementation; the exit is the maximum over the gating sections and no ranked finding moves it; a section that refuses — an unreadable trace store, a walk over fuel — refuses the whole check with exit 1 and an empty stdout, never a report with the section missing; clone-of names exactly the definitions added since base whose shape equals an existing definition's; the refactor certificate is one `done` record over the same data and never says "behavior-preserving"; and one check surveys head, then opens the store *once* and diffs the two endpoints *once*, so every section answers about the tree that survey charted rather than about the epoch a store opened earlier would still be holding; `--format md` renders the same `CheckResult` and the same exit code as one document, and a section that refuses emits no document at all | check.rs conformance.rs | 10+ |
| C44 | a method-call edge is emitted only from a receiver whose type the file states or grid infers from a constructor, literal, or written return; it is proven only where the written receiver form matches the inherent method's at the first lookup step of a plain impl, modeled otherwise, and declined with a stated reason wherever any trait grid can see supplies the method, the impl is bounded or instantiated, the type is a wrapper, or the receiver is a closure or pattern binding; rustc's own binding, observed by running the fixture, agrees with every emitted edge; a receiver typed outside the tree counts `external` unless the tree implements its own trait for that type | receivers.rs refs.rs declined_split.rs | 10+ |
| C45 | every definition's declined split (segment epoch 5) sums to the survey's repo-wide counters, column by column, with module-level references in neither; `refused ≤ method` per definition; a counter saturates at `u16::MAX` and the survey reports it, never wraps; and a pre-epoch-5 snapshot answers *unknown*, never zero | epochs.rs adversarial.rs declined_split.rs | 10+ |
| C46 | `self::` and `super::` inside an inline module anchor against that module in `use` declarations and paths — `self::x` is the module's `x`, `super::x` climbs one inline module per `super` and then the file's own module path; a shadowed name resolves to the inline module's item, never the file's; a file with no inline module extracts byte-identically to before | refs.rs rust_edges.rs determinism.rs | 10+ |
| C50 | a tracked file a frontend claims that its parser rejects, or that is not UTF-8, is charted as nothing and named by the survey (`unparseable` / `unreadable`, on stdout and in the wire snapshot) — never as a file with nothing in it; a definition absent at head only because its file was not read is held out of the drift walk and reported as a `warn`, never as a suspect and never as exit 3; and an incremental survey answers the same as a cold one for such a file | unread.rs drift.rs watch.rs | 10+ |
| C51 | an empty answer is an answer about a chart: a read verb with no published snapshot, an `--under` prefix no charted definition sits below, and a `scope` pattern matching nothing all refuse with exit 2, the reason, and an empty stdout, and a `--budget` with no tier to bind on is refused rather than ignored; `--json` renders each of those as the same refusal and never as an empty success | refusals.rs porcelain.rs | 10+ |
| C47 | every file `scripts/gen-wire.py` writes — `crates/wire/src/lib.rs` and `methods.rs` — is byte-identical to what it writes from `docs/wire-v1.openrpc.json` and `rustfmt` then normalizes, so a hand edit to a generated file and a contract change whose regeneration was never run both fail the gate rather than shipping as a wire the daemon and its callers disagree about; `errors.rs` is hand-written and outside the generator's reach, and this row says so rather than claiming a coverage it does not have; the check regenerates into a scratch tree and never writes the working tree at all, and its own `--selfcheck` renames a wire method to prove the comparison is not blind | check_wire_drift.py | 1+ |
| C48 | N publishers into one store at once — the CLI beside the daemon, parallel surveys in one tree — leave a store that opens, reopens every snapshot published, and holds every one of them in the membership image; the publish window is one exclusive `flock` and every temp name is per process and per call | concurrent.rs | 11+ |
| C49 | a damaged chart — bytes that contradict the manifest — is refused by every read verb with one exit code (2) and the command and tree that recover it, is never served, and is dropped and re-surveyed cold by `grid survey`, which says what it dropped | damaged_store.rs store.rs | 11+ |


## What CI does and does not run

An evidence token of the form `ci:<job>` names a job in
`.forgejo/workflows/grid-gate.yml`, and `scripts/check-assurance-links.sh`
resolves it there. Forgejo is the only estate: the GitHub remote's push URL is
`no_push`, so a job that exists only in a `.github/` workflow runs nowhere and
is not evidence.

**C3 has no CI evidence and its row no longer claims any.** The Kani harnesses
in `crates/walk/src/proofs.rs` and `crates/chart/src/proofs.rs` are real and
`just verify` runs them, but no CI job anywhere has ever run Kani for grid —
the `proofs` job that claimed to was written into `.github/workflows/`, on the
estate that never runs. Until a Forgejo job exists, C3's evidence is the
harness and the developer who ran it, which is what the row now says. Adding
that job is a row in `docs/TODOS.md`, not a line of prose here.

## Maintained views

Every view a watch session maintains declares its class before it lands. The
classification is DBSP's and it is a cost model, not a taxonomy: it says what
state maintenance needs, and therefore what a session must hold.

| view | class | state it must hold | why |
|---|---|---|---|
| sig re-hash (a file's interface identities) | linear | none | a linear operator *is* its own incremental form: delta in, delta out. |
| drift suspects | bilinear | the edge table (the base snapshot's resident segments — already the integral of every edge delta) and the changed-sig set since base | a join needs the integrals of both inputs: `Δ(a⋈b) = Δa⋈Δb + a⋈Δb + Δa⋈b`. Nothing else is needed, and nothing else is held. |
| verdict closures | recursive | **declared not maintained** | the nested-stream case: correct to incrementalise and expensive to. The budgeted invalidate-and-rewalk in `grid cache` stays the call. |

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
- Wave 8 (`trace`): no new dependencies. A trace is ingested through
  `grid-chart`'s own resolver (the `grid.resolve` rule, so a trace names
  datums the same way every other verb does), retention is one `std::fs`
  file per trace keyed by `blake3` — already in the TCB, and hashing the id
  means no trace id is also a path — and the diff is `BTreeMap`/`BTreeSet`
  ordering, not a graph library.
- Wave 9 (`walk` in `grid`): no new packages. `grid` gains a path
  dependency on the workspace's existing `grid-walk` crate so the daemon
  runs the *same* traversal the engine is proven on (C3) rather than a second
  implementation the proofs would not cover; `Cargo.lock` gains nothing.
  Modeled bridge hops are static tables in `grid-walk`, not an inference
  engine: a table is auditable and deterministic, and it is why C18 can say
  a modeled hop never becomes proven.

The segment format is the other compatibility surface: images are prefixed
`GRIDSEG\0` and carry a `FORMAT_EPOCH`, and the manifest carries the same
epoch. A format change is a refusal to load old bytes (`BadEpoch`) and a
re-survey, never a reinterpretation — the same declared-epoch discipline the
normalizer follows. The two halves of that rule differ by design:
`Segment::parse` keeps reading *every* epoch ever shipped, answering the
columns an older image does not declare with `None` (never a fabricated
value), pinned by a committed fixture per epoch (epochs 1–4 as of epoch 5,
which added the per-definition declined split); a manifest from a past epoch
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

- Wave 3 (`gate`, `grid`): `serde_json` — already in the TCB from Wave 1; the
  daemon speaks JSON-RPC, which is the wire's own format, so no second codec
  enters. `proptest` (dev-only) — the property harness C8/C9 are evidenced by.
  `tempfile` (dev-only) — the CLI and socket tests need a throwaway git repo;
  a test that runs against the developer's own tree is not a test. No runtime
  dependency was added: the daemon uses `std::os::unix::net`, and the CLI
  parses its own arguments, because the surface is nine verbs and a flag.

Normalization stability is a compatibility surface (implementation plan,
Risks): the canonical bytes are prefixed `grid-datum/v1`, so a parser upgrade
that moves normalization is a declared epoch bump, never a silent re-hash.

- Agent loop (steps 0–10): no new packages; `grid` gained path dependencies
  on `grid-lang-rs` (step 3, the cfg evaluator beside the sig canonicalizer)
  and `blake3` (step 2, `env_hash`), both already in the TCB.
- Method calls (steps 4, 1, 2, 3): no new packages. The receiver facts are
  `syn` visitors the tree already has, the resolver arm is lookups over
  `DefIndex` and a static table of prelude method names, and the epoch-5
  column is parsed by `grid-chart`'s validate-then-accept path. The one
  test that runs a compiler (`receivers.rs`, rustc as the oracle) runs the
  `cargo` that is running the test.

- Tooling (`docs/TOOLING.md`): **no packages at all, runtime or dev.** The
  harness ported from yi adds only binaries that live on `PATH` and are
  announced when absent — `codespell`, `cargo-deny`, `cargo-machete`,
  `cargo-nextest`, `cargo-fuzz`, `kani` — plus Python scripts that import the
  standard library and nothing else. A dev-dependency would still be a package
  in `Cargo.lock`, and the lockfile is the thing this rule protects: `git diff`
  on it is empty across the whole port. `deny.toml`'s named ban list is this
  paragraph made mechanical, with `wildcards = "deny"` and
  `multiple-versions = "deny"` covering what the list did not think to name; a
  package that gets past both still arrives as a `Cargo.lock` line in the diff
  of the pull request that adds it.
