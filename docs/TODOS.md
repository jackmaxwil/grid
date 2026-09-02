# TODOS — the journeys, their pressure test, and what is next

Written 2026-09-01, after the drift, naming, and agent-loop plans landed.
This file is the working list. A plan document is written when an item
here is ready to be a contract; this file says what is worth one and why.

## Who grid is for

Two users. A **coding agent** in its edit loop — the primary user, every
turn. A **human reviewer or maintainer** reading what the agent did — the
secondary user, once per change. CI is the agent's journey run one more
time with everything on.

## The core journeys

### Agent

**A1. Edit loop** — every task, every turn.

1. `grid scope <target> --bodies 1 --sigs 3 --budget T` — context with a
   stated horizon; read two hundred lines, not two thousand.
2. Edit (`grid edit apply`, or a plain write).
3. PostToolUse hook: `grid check --quick` — drift suspects and uncompiled
   callers; exit 3 stops the agent before it claims done.
4. `grid test` — run what the change reaches, replay the rest, name the
   changed definitions no test reaches and the tests whose verdict
   flipped under an unchanged closure.
5. Pre-commit: `grid check` — the full composite; a refactor certificate or
   a gate.

The point: the agent's claims ("only refactored", "tests cover it", "no
API change") become checked predicates instead of prose.

**A2. Plan → implement → gate** — multi-step tasks.

1. Plan mode writes the plan file: promised defs (with `interface`),
   promised edges. 2. `grid plan lint`. 3. Implement; `grid drift --plan`
shows the callers nobody promised. 4. `grid gate` — hold on an
unpromised interface move or a promise not made good.

**A3. A test went red.** `grid suspects <test> --closure <last-green>` —
changed definitions inside the test's closure, ranked by hops, declined
count stated. Milliseconds, no rerun, no bisect, no history.

**A4. Parallel agents.** `grid interfere <other-worktree>` before a merge
— a moved signature here against a new caller there, with no file in
common.

### Human

**H1. Review the agent's PR.** `grid check <base> <head> --plan f --why
--json` — interface moves, unpromised work, uncompiled callers, stale
verdicts, weak tests, uncovered changes, clones, compat findings,
covenant fires. Read that first, the diff second.

**H2. "Can I change this?"** `grid uses X` and `grid scope X --sigs 2`,
then stub the new signature and `grid drift`. Decide with numbers.

**H3. The compiler-blind cases.** Declare once — `grid covenant A B --via
protocol` across a process or language boundary, a `persisted` invariant
for a serialized type — and A1/H1 gate on it forever.

**H4. Orient in an unfamiliar repo.** `grid scope <entry> --bodies 1
--sigs 2`, `grid explain <any word>`, and `grid scope --since <snapshot>`
on return.

### CI

`grid test --trace --all` (refreshes the traces local selection reads),
`grid check --compile -- --all-targets`, `grid gate <plan>`.

### Not core

Kept, not led with: `watch`, `bisect`, `hotspots`, `orphans --traced`,
`log`, the Ochiai mode of `suspects`, `trace diff`. Future products, not
this one: binary analysis, a package corpus, vulnerability reachability.

A1 is most of the value. If an agent learns only `scope`, `check`, `test`
and one hook line, grid is used every turn; the rest is discovered through
exit 3 and `grid explain`.

## Pressure test, round two

Each journey was run against the tree as it stands (2026-09-01, 1813 defs,
3419 proven edges, 11,985 declined). What broke or leaked, and the item it
produced.

**A1.3 — the hook is noisy.** Every `grid check --quick` prints the same
`miss 11985 unresolved references unchecked (…)` and, until a compile
record exists, `warn - no compile record since @…`. On a PostToolUse hook
that fires per edit, that is two lines of unchanged text into the agent's
context every turn. The honest counts are right; the delivery is wrong.
→ T-01, T-02.

**A1.4 — `grid test` is inert without traces.** On this repo every test is
selected (`391 no-trace`, `34 unresolved`) because no trace exists and
`cargo llvm-cov` is not installed. Until someone runs `--trace --all`
once, `grid test` is `cargo nextest run` plus bookkeeping. The plan knew
this; the first-run experience does not say it loudly enough. → T-03,
T-04, T-05.

**A1 — drift is blind to method calls.** The honesty number is now true
and it is ugly: 8,852 of 11,985 declined references on this repo are
`x.m()`. A moved signature reached only through a method call produces no
suspect. On Rust `cargo check` covers it; on Python nothing does. This is
the single largest gap between grid's promise and its evidence. → T-06.

**A1 — every `check` says `miss 391 unattributed entries`.** Verdicts that
`grid test` recorded without a trace have no trace id, so `--tests`
cannot attribute them, forever. The count only grows. → T-07.

**A2 — authoring a plan is JSON by hand.** `grid plan new` exists; an agent
still has to write callsign patterns and promise kinds. The journey is
right; the on-ramp is steep. → T-08.

**A3 — `--closure <since>` needs a snapshot the agent must remember.**
The verdict log already knows the last snapshot each test passed at.
→ T-09.

**A4 — interference needs both stores at the same epoch.** Against the
sibling worktree it refused correctly (epoch 3, superseded). Refusal is
right; the hint should name the command. → T-10.

**H1 — the review pack is records, not a document.** A reviewer on GitHub
wants a comment, not thirty porcelain lines. → T-11.

**H3 — `persisted` is hand-edited JSON.** `grid covenant` has a verb;
`persisted` does not. → T-12.

**CI — traces stay in CI.** `.grid/traces` is per machine. CI refreshes
them; the local store never sees them unless someone copies the
directory. → T-13.

**Cross-cutting — the survey inside every verb.** ~94ms of every `check`
is the worktree survey a hook must re-run to see the edit. The budgets
that miss (`--quick` 141ms vs 50, full 204ms vs 200) miss for that reason
alone. → T-14.

**Untested live paths.** `grid test --trace` on Rust (needs `cargo
llvm-cov`), the pytest inventory and pytest-cov path, `--coverage-json`
against a real coverage.py report (fixture only). They are implemented to
the recipes and have not run for real. → T-15.

**Platform.** The daemon is `std::os::unix::net`; Windows is out and not
stated anywhere. → T-16.

## Next steps, exhaustive

Priority: **P0** blocks a core journey; **P1** makes a core journey
pleasant; **P2** extends; **P3** research. Size: S (a day), M (days), L (a
plan).

### Honesty number and the resolver

- **T-06 — done** (`docs/plans/2026-09-01-method-calls.md`, 30f41e0 +
  3819612): `x.m()` resolves on a receiver whose type is written or
  syntactically inferred, by rustc's own lookup order — proven where the
  written form matches the inherent method's at the first step, modeled
  otherwise, refused with a reason wherever a visible trait, a bounded
  impl, a wrapper or no inherent method stands in the way; rustc's own
  binding, run on the fixture, agrees with every emitted edge. Measured,
  same tree, previous binary against this one: this repo `method` 9,549
  → 7,599 (64 refused), `external` 2,534 → 4,303, proven edges 3,648 →
  3,769, modeled 0 → 71; yi `method` 23,871 → 18,146 (338 refused),
  `external` 10,062 → 14,670, proven 6,876 → 7,159, modeled 0 → 519.
  Python: annotated parameters resolve modeled; zero on yi. The "cut by
  half" target was not reachable by any syntactic rule (R-1); the
  residual needs a type checker (T-45).
- **T-17 — done** (6b71bcb + 6fc7855): segment epoch 5 stores the declined
  split per definition (four saturating `u16`), summing to the survey's
  counters by construction; an older snapshot answers unknown, never
  zero. The consumers — `suspects --closure`, `check`, `scope --bodies`,
  `grid uses` — do not read the column yet (T-46).
- **T-18 — done** (faa5629): `self::` and `super::` inside an inline
  module anchor against the module; one wrong proven edge on the fixture
  is gone, and ~110 test-module references on this repo (4 on yi) that
  `use super::x` had left `internal` are proven.
- **T-19 (P2, M) Macro-generated callers.** Unreachable from source; see
  the binary-side idea under Future — the compiled artifact names macro
  output the survey declined.
- **T-20 — done** (2a7c8a7): Python builtins and Rust prelude names left
  unbound count as `external`; `internal` on this repo went 1460 → 740.

### Agent-loop ergonomics

- **T-01 (P0, S) Hook delivery.** Document the exact PostToolUse wrapper:
  silent on exit 0, and on exit 3 re-run with output to stderr and exit 2
  so Claude Code feeds it back. One shell line in README; no new flag.
- **T-02 (P1, S) Unchanged counts do not repeat.** `check --quick` prints
  the `unchecked` line and the no-compile-record warning only when they
  changed since the last check in this store (a one-line state file).
  Stated in PORCELAIN as a `--quick` property; the full form always
  prints them. (done eb946ff)
- **T-03 (P0, S) First-run banner for `grid test`.** When zero traces are
  retained: one stderr line naming the fact and the two commands (install
  `cargo-llvm-cov` / `pytest-cov`; run `grid test --trace --all` once).
- **T-04 (P1, S) `just traces` recipe.** Install check plus the CI form.
- **T-05 (P1, M) Single-run per-test tracing on Rust.** A nextest wrapper
  script keyed by test name sets `LLVM_PROFILE_FILE`, making `--trace`
  one invocation instead of one per test. Configuration shipped as a
  documented template, not code.
- **T-07 (P0, S) Attribute `grid test` verdicts without a trace.** Record
  a synthetic trace id (`test:<inventory id>`) so `stale.rs` can attribute
  them; the `unattributed` count becomes what it should be — verdicts
  recorded by something other than `grid test`.
- **T-08 (P1, M) Plan authoring on-ramp.** `grid plan new --from-diff
  <base>` drafts promises from a working diff (defs touched, edges added)
  for the agent to prune; `grid plan new --defs a.b c.d` for the tiny
  case. The gate stays the judge.
- **T-09 (P1, S) Default `since` for `suspects --closure`.** The last
  snapshot at which the test recorded `pass` (from the verdict log); say
  which one was used. (done 6dfb18c)
- **T-10 (P1, S) Refusal hints name the command.** `interfere` on a
  superseded other store says `run grid survey in <root>`; every
  superseded-store refusal does the same. (done 545faae)
- **T-11 (P1, S) `grid check --format md`.** One markdown document for a
  PR comment: gates, then tail, then the certificate. Porcelain stays the
  record form; `--json` the wire form. (done 7702ec7)
- **T-12 (P1, S) `grid invariant persisted <pattern>`** (and `--list`),
  the `covenant` verb's shape, writing `.grid/invariants.json`. (done a558a9e)
- **T-21 (P1, S) `scope` default depths.** With `--bodies` alone, `--sigs`
  defaults to 3; on a hub the hop-1 body layer rarely fits a small budget
  and demotes to sigs silently-but-stated. Consider `--bodies 0 --sigs 2`
  as the documented "orient" preset and `--bodies 1 --sigs 3` as "edit". (done af265fe)
- **T-22 (P2, S) `grid explain` for exit codes and record tags in
  context** (`grid explain miss`, `grid explain 3`).

### Traces across machines

- **T-13 (P1, M) Trace transport.** `grid trace export <dir>` / `grid
  trace import <dir>` (content-addressed files, idempotent), so CI's
  `.grid/traces` can travel as an artifact and local selection reads it.
  Decay grading already handles a trace from a different snapshot.
- **T-23 (P2, M) Packed trace index.** `retained()` scans one file per
  trace; past ~10k traces a membership-style packed image (marked
  `ponytail:` today).

### Performance

- **T-14 (P1, L) Incremental worktree survey by file event.** The survey
  inside `check --quick` is the whole budget miss. `grid watch` already
  holds a session with per-file re-survey; a hook that hands `check` the
  edited path (`--touched <file>`) could re-survey that file only and
  reuse the session's chart. Phases 2/3 are O(tree) by claim C13 (edges
  are never cached); the upgrade is a resolver that invalidates by
  reference — a survey-architecture change with its own plan.
- **T-24 (P1, S) `check` opens the store once.** Sections currently
  compose verbs that each open the store and resolve the base rev
  (~13ms each, one git spawn for a rev). Pass the opened chart down. (done e27b8c5)
- **T-25 (P2, S) `suspects --closure` by rev spawns `rev-parse`.** Resolve
  through the store's rev label first (as `check` does); 15.8ms → ~4ms.
- **T-26 (P2, S) Body reads in `scope --bodies`.** Memoized per file
  already; a hub with 200 hop-1 callers across 60 files is 60 reads —
  fine; state the ceiling.

### Correctness warts

- **T-27 (P1, S) `grid test` runner output is captured then replayed.**
  Stream it (`ponytail:` today); a 12s nextest run with no output looks
  hung.
- **T-28 (P1, S) `unresolved` tests from `proptest!` and other macro-
  defined tests.** Always run — correct — but noisy (34 on this repo).
  Name the macro in the `miss` reason so the reader knows why.
- **T-29 (P2, S) `cargo test -- --list` fallback compiles the whole
  workspace.** Say so on stderr when nextest is absent and suggest it.
- **T-30 (P2, M) Compat: tuple-struct members and enum variant payloads**
  are skipped (`ponytail:`); a removed payload field is a compat break.
- **T-31 (P2, S) `check --compile` `examples/` not modeled** for cfg
  joining (marked).

### Untested-live paths and platform

- **T-15 (P0, S) Run the recipes for real once.** Install `cargo-llvm-cov`
  and `pytest-cov` on one machine, run `grid test --trace --all` on this
  repo and on a Python fixture, ingest, then `grid test` again and watch
  replay happen. Record the numbers in README.
- **T-16 (P2, S) State the platform.** macOS and Linux; Windows needs a
  named-pipe daemon or a `--no-daemon` mode. Say so in README.
- **T-32 (P2, M) Python end-to-end on a real repo.** yi's Python side has
  been surveyed; run A1 there with pytest and coverage.py.

### Documentation and tooling

- **T-33 (P1, S) README rewrite** around the journeys above (done with
  this file), with the stale numbers and epochs corrected.
- **T-34 (P1, M) Repo tooling port from yi** — CI, lint config, guards
  (landed on `claude/grid-tooling`). Comment enforcement and the env-var
  and dependency-count budgets were ported and then deleted before merge;
  `docs/TOOLING.md` records each with the reason. The vendored `har-*`
  skills went the same way: the doctrine is read from
  `~/Development/har-skills/skills/`, never copied here.
- **T-35 (P1, S) `docs/PORCELAIN.md` text-mode section** covers `scope
  --bodies`, `explain` and `check --format md`, under the stated rule that
  `--json` is always the record form. (done 7702ec7)
- **T-36 (P2, S) A walkthrough doc** — one real session transcript of A1
  on this repo, records and exit codes shown.
- **T-37 (P1, S) A Kani job on Forgejo.** C3 ("every walk terminates
  within fuel") has real harnesses in `crates/walk/src/proofs.rs` and
  `crates/chart/src/proofs.rs` and a `just verify` that runs them, and no
  CI job anywhere that ever has. The evidence column says so now rather
  than claiming a `ci:kani` that lived on the estate with `no_push`.
  Adding the job means a `kani-verifier` install and a runner willing to
  spend the minutes; until it exists, C3's evidence is a person who ran
  the proofs.
- **T-38 (P2, S) `errors.rs` is outside `check_wire_drift.py`'s reach.**
  `gen-wire.py` writes `crates/wire/src/lib.rs` and `methods.rs` only, so
  a hand edit to the hand-written `errors.rs` beside them is invisible to
  the drift check. C44 now says which files it covers instead of implying
  the directory. Either generate the error enum from the contract's error
  codes as well, or leave it hand-written and let the row stand — but not
  a check that quietly means less than its name.
- **T-40 (P2, S) `cargo-fuzz` is still compiled from source in CI.**
  `taiki-e/install-action` was tried for it and the `fuzz-build` job went red
  in 18 s, while the same action installed `just`, `cargo-deny`,
  `cargo-machete` and `cargo-nextest` green on four other jobs of the same
  run. This Forgejo build serves no job logs over its API, so the failure
  could not be attributed and the step went back to `cargo install cargo-fuzz
  --locked`, which has always worked here. Retry with the log in hand; the
  prize is a few minutes off every pull request.
- **T-39 (P2, S) The pre-commit hook gates the checkout, not the index.**
  `just precommit` runs over the working tree, so a partial `git add` is
  judged against changes that are not being committed. Left as it is
  deliberately (a stash-based hook is worse where every worktree shares
  one stash stack), recorded in `docs/TOOLING.md`, and worth revisiting
  only with a `git worktree`-based staging copy rather than a stash.

### Future products (each its own plan; none started)

- **T-40 (P2, L) TypeScript/JavaScript frontend** on `swc`, datum over the
  type-erased AST so `.ts` source and transpiled bundles agree; types in
  the sig; stack-graph push/pop resolution with grid's acceptance policy
  (`docs/research/ANALYSIS.md`).
- **T-41 (P3, L) Binary frontend (Mach-O, arm64) via spawned platform
  tools** — functions from `LC_FUNCTION_STARTS`, address-erased datums,
  direct-call edges, `links_against` imports, DWARF bridge to source
  datums; the four fates (materialized / inlined / folded / dropped);
  build diff; reproducible-build certificate; size attribution. Cheapest
  experiment first: hash `target/release/grid`'s functions across two
  builds and see whether datums are stable on this toolchain.
- **T-42 (P3, L) Package corpus.** Charts of registry packages at
  versions as a shared, content-addressed store; shape index across it.
  Enables vendored-code recognition, attestation of installed
  dependencies, and function-level vulnerability reachability (advisory
  names a function; grid answers reachable / unreachable with a horizon /
  cannot say with the declined share).
- **T-43 (P3, M) Datum-keyed knowledge.** Notes, invariants, review
  findings keyed by datum rather than `file:line`, surviving rename, move
  and vendoring; export/import with the store.
- **T-44 (P3, L) Non-code frontends.** SQL schema, OpenAPI/protobuf,
  Terraform — definitions and references, same drift and gate.

### From the method-calls plan's cut line (`docs/plans/2026-09-01-method-calls.md`)

- **T-45 (P1, L) A type checker for the seventy percent.** The `method`
  residual (7,599 here, 18,146 on yi) has no syntactic type; only
  rust-analyzer as a sidecar or in-process reaches it. Gated on R-5.
- **T-46 (P1, S) Consumers of the per-definition declined split.**
  `DefRef.declined` and `declined_within` on the wire; `suspects
  --closure`'s `miss` summed over the closure's definitions (`miss ? …
  (pre-epoch-5 snapshot)` when unknown, never `0`); `check`'s and
  `drift`'s second clause `… of which N inside the changed definitions`
  and `DriftUnchecked.unresolved_method_refused`; `scope --bodies`'
  horizon clause; `grid uses X`'s refused hint. The column is stored
  (epoch 5); each consumer is one join.
- **T-47 (P2, M) Trait-method resolution by impl search.** Picking among
  `Foo@A.m`, `Foo@B.m` and a blanket impl needs the trait in scope and
  the receiver's bounds; declined with reason `trait candidate` today.
- **T-48 (P2, M) Autoderef through `Box`/`Rc`/`Arc`/`Option`.** The
  wrapper's own methods and impls sit earlier in the lookup chain; counted
  `external` today, honestly.
- **T-49 (P2, S) Cross-file chain following, Python constructor locals,
  and a standing `resolve: modeled-receivers` declaration.** Extraction
  follows a written return in the same file only (`MethodFacts.ret` holds
  every method's return in phase 3 — the upgrade); Python locals bound by
  a constructor and the standing declaration land with the first Python
  repository whose annotated receivers resolve, which yi is not (zero).
- **T-50 (P3, M) A `std` signature table.** Would turn part of the
  seventy percent into `external` by name (`v.iter().map(..)` is
  `Iterator::map`), not into edges, against a moving standard library.
- **T-51 (P2, S) `SelfCall` under the typed-receiver rule.** Pre-existing
  (review of PR #22, finding 8): `self.m()` resolves proven to the
  inherent `m` with no receiver-form or trait-candidate check, while `let
  s = self; s.m()` now refuses where a trait candidate or a form mismatch
  stands in the way; `self.clone()` beside an inherent `fn clone(&mut
  self)` and `#[derive(Clone)]` is the case. One rule for both.
- **T-52 (P2, S) A field's declared type resolved in the struct's module.**
  `x.f.m()` with the struct in another file reads the field's written
  type off the index and resolves it as a path from the *caller's*
  module: a caller importing a different type of the same name binds
  wrongly, one importing nothing declines. Needs the struct's module
  beside its callsign on `Symbol`; the extractor side is done (`x.f` on
  a base written as `other::Foo` already goes to the index).

### Research questions (no code until answered)

- **R-1 — answered** (`docs/plans/2026-09-01-method-calls.md`, Part I):
  no syntactic rule halves the count. 70% of Rust method calls have no
  type any syntax in the file states, 21% are calls on `std`/external
  types (now `external`), 6–9% have a charted receiver of which a little
  over half resolve. Measured after landing: `method` −20% on this repo,
  −24% on yi; the rest is a type checker's (R-5).
- **R-5** Does a rust-analyzer sidecar (LSP `textDocument/definition` per
  call site, or `ra_ap_*` in-process) answer this repo's ~7,600 remaining
  `method` call sites under a second, cold? The only route to the other
  seventy percent, and a dependency decision the size of `syn` again.
- **R-2** Is a per-file incremental resolver (T-14) sound under C13, or
  does it need the SCC-shared closure work from ANALYSIS.md?
- **R-3** Are address-erased function datums stable across rebuilds on
  the same toolchain (T-41's gate)?
- **R-4** Do `.ts` source and Bun/esbuild bundle output agree under a
  type-erased datum (T-40's gate)?

## From the 2026-09-01 review

Findings of the high-effort review of `claude/grid-agent-loop` that were
not fixed in 5a258a3..2a7c8a7, one line each, with the reviewer's
location and severity. Nothing here gates; each is a wart with a named
fix.

- **F10 (medium)** `crates/grid/src/rpc.rs` `renderables` / `pack_text` —
  a definition with no location, no `line_end`, an unreadable file, an
  empty slice or no sig text is dropped from its layer while the horizon
  still certifies "bodies complete through k". Count drops per layer and
  emit `## <callsign> - (body unavailable, hop k)`, or lower the certified
  depth to the last layer with zero drops.
- **F11 (medium)** `rpc.rs` `body_at` — bodies are worktree lines at the
  live snapshot's locations with no survey and no re-hash; after an edit
  above a definition the text under `## callsign` is another definition's.
  Survey first (~92ms) and say so in the horizon line, or re-hash the
  slice through the frontend and mark a mismatch.
- **F12 (medium)** `crates/grid/src/check.rs:147` — `--quick` runs drift
  at the proven floor, so the hook drops observed-grade suspects, which
  gate; the plan's Status says "stays plain drift byte for byte" and is
  wrong, PORCELAIN/ONTOLOGY are right. Either drop the floor override
  (measure: `observed_inbound` is the only extra store read) or keep it
  and state the loss where the hook is documented.
- **F14 (medium, two of four)** `crates/grid/src/compat.rs:165-172` — a
  field replaced by one with `#[serde(default)]` or an `Option` type is
  still "renamed without alias"; `crates/lang-rs/src/serde_shape.rs:170`
  — `#[cfg_attr(feature = "x", serde(deny_unknown_fields))]` is neither
  `Hidden` nor read (treat a `cfg_attr` text containing `serde` as
  `Hidden`). Container `default` and `flatten` were fixed in 2a7c8a7.
- **F18 (medium)** `crates/wire/tests/conformance.rs` — only examples
  round-trip; no example carries `cfg`, `interface_unaccounted`,
  `nondeterministic`, `compat`, `declined`, `traces`, `compat_unchecked`,
  `unresolved_*`, `persisted`, `bodies`/`sigs`/`horizon`, `clones`,
  `refactor`, `stale`, `weak`, so a rename is invisible. One test walking
  `components.schemas.*.properties` against a populated instance per type;
  and `closure_since`'s description says "base-side half" where the code
  is the head-side `Added` half.
- **F19 (medium/low)** `check.rs:139-215`, `compile.rs:265-273`,
  `rpc.rs:305-360` — one `grid check` computes the base/head diff three
  times, opens the store three times, reads traces and the verdict log
  twice; pass `(&ChartDelta, &[Changed], base, head)` into
  `compile::report` and lift the trace/cache reads into one `Loaded`.
  (T-24 is the same item from the other side.)
- **F20 (low)** `crates/grid/src/interfere.rs:131-140` — `unresolved_refs:
  None` prints `miss ? unresolved references uncounted (head was not
  surveyed)` though both heads were surveyed; carry the two `Ran`s'
  declined splits or print nothing.
- **F22 (low)** `hit`'s second column is a `{:.6}` float or the word
  `closure` (`porcelain.rs` `suspects`): a column-meaning change shipped
  as 2.6.0 under PORCELAIN's own major rule; `suspects_closure` in
  `main.rs` prints a hand-padded `miss` outside `porcelain.rs`; ASSURANCE
  C36/C35/C40/C41 name `src/` modules as evidence and C37 "cargo target
  directory", C42 "does not hold the base" (now covered by interfere.rs),
  C36 `not-recorded`, C41 "from a macro" have no test of their own;
  ONTOLOGY rows `trace`, `check`, `refactor`, `closure`, `uncompiled`,
  `compat`, `horizon`, `interference`, `selected` exceed the file's own
  three-line rule.
- **F23 (low)** `crates/survey/src/files.rs:559` sizes an allocation from
  a `cat-file` header (`take(size+1).read_to_end` and check the length);
  `:509/:538` discard the child's exit status with stderr nulled, so a git
  without `--batch -z` surfaces as "answers ran out". `main.rs:1317`
  `observations.len() as u64`; `check.rs:697` byte-slices a hash string;
  `crates/chart/src/lib.rs:565` `idx as usize` and no check that cfg-table
  entry 0 is the empty predicate.
- **F24 (low)** the survey-level cfg join (`inherited_cfgs`, `mod x;`
  declarations and `tests/` dirs — `crates/survey/src/lib.rs:277-304`)
  has no test: passing `""` keeps every suite green. `files.rs:564`'s
  non-blob branch is unpinned. 25 of this repo's tests are `unresolved`
  (every `proptest!`-generated one): always run, never selected (T-28).
- **Cut list, skipped** — `runner.rs` `Rust::CargoTest` fallback (kept,
  as the reviewer said); `CheckParams.compile` stays on the wire,
  documented "Ignored".
- **Flake** — `crates/chart-io/tests/crash_safety.rs:59` tripped its own
  "never observed a published epoch" guard once under a full parallel
  suite; 3/3 green alone. The harness's kill window is time-based.

## From the 2026-09-02 review of `claude/grid-p1-check` (forge PR #23)

F1–F7 and F10 were fixed on that branch. What was left, and why.

- **P-F4b (medium)** `crates/grid/src/standing.rs` `save` and
  `crates/grid/src/covenant.rs` `save` publish by rename now, but neither
  takes a lock, so two concurrent `grid invariant` (or `grid covenant`)
  calls read-modify-write the whole file and one declaration is lost with
  no error. Left out of the atomicity fix on purpose: a different failure
  with a different remedy (an `O_EXCL` lock file beside the declaration, or
  a compare-and-swap on the bytes read). Rare today — both verbs are
  hand-run — and it becomes real the moment an agent loop writes them.
- **P-F8 (low)** `crates/grid/src/suspects.rs` `last_pass` breaks a tie on
  `trace_id` lexicographically because `recorded_at` is RFC 3339 at second
  resolution (`cache/src/lib.rs:276`). Two passes recorded in the same
  second against different snapshots resolve to whichever trace id sorts
  higher, which may be the older snapshot, while the stderr line says "the
  last snapshot ... recorded a pass at". Deterministic, and it names the
  snapshot it chose, so the answer stays checkable; the honest fix is
  sub-second resolution on `recorded_at`, which is a cache record change
  and an epoch question rather than a tie-break tweak.
- **P-F9 (low)** `standing::save` re-serializes with `to_string_pretty`, so
  a compactly authored `.grid/invariants.json` is reformatted on the first
  successful write. `deny_unknown_fields` on `Invariant` and `EdgeRule`
  (`wire/src/lib.rs:327,338`) means nothing is dropped, so the claim holds
  semantically; a558a9e's body said "survives a `grid invariant` call byte
  for byte", which is stronger than what
  `compat.rs::an_invariant_over_nothing_is_refused_and_hand_authored_rules_survive`
  asserts — bytes on the refusal path, ids on the write path. Wording debt,
  not a defect: the fix is to say "survives" and leave the assertion where
  it is.
- **P-plan (medium)** the `self-gate` CI job runs `grid gate
  .grid/plans/*.json` and fails a source-changing pull request that carries
  none. The ignore rules no longer forbid one, but no branch has written one
  yet, so the job still fails on any pull request touching `.rs`/`.py`/
  `.pyi`. Either every such branch carries a plan, or the job's guard grows
  a case for a branch that predates the convention.

## Recorded ceilings (from `ponytail:` markers in the tree)

The per-test `cargo llvm-cov` loop; `Store::retained` O(traces); bytes/4
as the token estimate; one memoized file read per body; `TokenStream::
to_string` spacing for Rust sigs; `--since` resolving without a survey;
runner output captured not streamed; `cargo test --list` names-only
fallback; `uncovered` recomputed per check; the membership image's
O(chain) rewrite; the watch session's O(tree) phases; `examples/` not
cfg-joined; compat's tuple-struct and payload skips. Each names its
upgrade path where it sits.
