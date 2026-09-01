# Agent loop: the honesty number, `grid test`, `grid check`, `grid inspect`

Status: **proposed**, 2026-09-01. Follows the drift plan
(`2026-09-01-drift.md`, implemented). Scope: what survived a pressure test
of grid's value to a coding agent on a Rust, Rust + Python, or Python
repository — the areas the test kept, at the minimum each needs. Authority:
ONTOLOGY.md for vocabulary, PORCELAIN.md for records and exit codes,
ASSURANCE.md for claims and dependency discipline, LANG_RS.md for the Rust
frontend's ceilings.

---

## Part I — Architecture overview

### What grid holds, and the one rule

Four things nobody else holds together, all already built:

1. **Content identity per definition** — `datum` (normalized AST), `sig`
   (what a caller can observe without the body), `shape` (name-erased
   rename key). A body edit, an interface edit and a rename are three
   different facts.
2. **Snapshots as reference sets** — the membership image: every datum the
   chain ever held, stored once, plus one sorted list per snapshot. "What
   changed between S and T" is a merge-diff; "unchanged" is an id
   comparison; neither loads a definition.
3. **Edges with an evidence grade** — `proven > observed > modeled >
   heuristic`. Proven and observed gate; modeled gates only when a human
   declared it; heuristic ranks and never gates.
4. **Runtime traces keyed to closures** — wakes (what a test run did),
   closure keys (the transitive identity a verdict depended on), and an
   append-only verdict log.

Every feature in this plan is a read over those four, joined. The plan
adds no fifth substrate and no dependency. The one rule that keeps the
design honest is the grade rule in (3), and this plan's pressure test found
two places the brainstorm had broken it — inferred wire boundaries and a
"flake proof" — both repaired below by demotion to heuristic and by renaming
the claim to what is actually proven.

### The honesty number

Every soundness claim below — test selection, attribution, unused code —
is a claim about the *charted* graph, and the chart is known incomplete.
Measured on 2026-09-01 with the drift-plan binary:

| repo | defs | proven edges | references declined | share of references |
|---|---|---|---|---|
| grid (this repo) | 1395 | 2535 | 2285 | 47% |
| yi (Rust + Python) | 4312 | 5648 | 10983 | 66% |

And the number **under-reports**. `lang-rs` extracts `self.m()` as a
reference and never extracts `x.m()` at all (LANG_RS.md non-goal: receiver
resolution needs types), so a Rust method call is not declined — it is
invisible. `lang-py` does extract `obj.m()` and declines it. The declined
count also conflates three populations with opposite meanings for an
analysis: a path into std or an external crate (harmless to intra-repo
soundness), a name a glob import would have supplied (a hole), and an
ambiguous callsign (a hole).

Consequence for the architecture: step 0 makes the incompleteness
*measured and split* before anything is built on it, and step 1 fills the
largest hole — dynamic dispatch and method calls — with **observed** edges
from per-test coverage, which is the evidence that already exists for
exactly the paths a test takes. Nothing in this plan pretends a static
closure is complete; every verb states its unchecked remainder, as
`grid drift` already does.

### Where the value lands, by language

A compiler owns types. On a Rust-only repository, `cargo check` already
catches a proven caller of a moved signature, and Rust test time is mostly
compile time. What survives for Rust is narrower than "game-changing" and
is true:

- **test selection at the test-binary level** — `cargo test --workspace`
  compiles and *links* every test binary; selecting binaries skips links,
  which is where Rust's test time goes;
- **callers the check did not compile** — `#[cfg(test)]` and feature-gated
  code that `cargo check` skipped, discovered sixty seconds later by
  `cargo test`;
- **attribution without rerunning** — closure ∩ membership diff, in
  milliseconds, at definition granularity;
- **semantic interference between worktrees** — a moved signature versus a
  caller in another branch, with zero textual overlap;
- **deserialize-compatibility across snapshots** — the persisted-bytes case
  the compiler cannot see because both ends share a struct;
- **plan adherence** — a moved interface nobody promised.

Where there is no compiler (Python, later TS), grid's graph *is* the type
checker, and every item above is worth more: per-definition test
selection, unused code, attribution, and the observed-edge fill all carry
the weight a compiler would otherwise carry.

### The agent-facing surface

Fourteen verbs plus a nautical vocabulary is a tax. The agent-facing
surface after this plan is three verbs and two hooks; everything else is a
component they compose:

```
grid test  [--trace] [--all] [-- <runner args>]     run what the change can reach; replay the rest
grid check [-- <cargo check args>]                   callers the compile did not compile
grid inspect [base] [head] [--track f] [--quick]     everything the change did that nobody checked
grid pack <pattern>... [--bodies N] [--sigs M] [--budget T]   context, with its horizon
```

Hooks (documented, not shipped as code — Claude Code hooks are the user's
configuration): PostToolUse → `grid inspect --quick` (drift + check, under
50ms); pre-commit → `grid inspect`; CI → `grid test --trace --all`, which
is what refreshes the traces local selection reads.

### Data flow

```
 survey ──► chart (segments, membership) ◄──── wake ingest (lcov | coverage.py JSON)
   │             │            │                        ▲
   │             │            └── closure keys ─► verdict log      per-test traces
   │             ▼                                    │                  ▲
   │        drift / leadline / scope                  │                  │
   │             │                                    ▼                  │
   └──► grid test ───── selection = inbound walk from changed set ∩ tests; replay hits; run misses ──┘
        grid check ──── proven callers ∩ cfg the check did not compile
        grid inspect ── drift + check + stale + weak + uncovered + clone + compat + adherence
        grid pack ───── bodies to hop N, sigs to hop M, budget, certificate
        grid suspects --closure ── closure(test) ∩ changed(since)
        grid interfere ── drift(A) ∩ touched(B), both ways, over two stores
```

The runner (cargo, nextest, pytest, coverage.py) stays outside grid. Grid
ingests what the runner produced and never instruments anything. That is
the same boundary `git ls-files` sits on today: discovery is the user's
tool, the chart is grid's.

---

## Part II — Feature set

One paragraph per area the pressure test kept, stating what ships and what
was deliberately not built.

**Prerequisites (step 0).** A rev survey spawns one git process, not one
per file (yi: 18,398 source files, ~10s → one `cat-file --batch`). The
declined count splits into `external` / `internal` / `method`, and Rust
method calls are extracted as a reference kind the resolver always declines
and always counts. Nothing else in this plan is honest without both.

**Coverage as traces (step 1).** `grid wake ingest --lcov` exists and maps
executed lines onto definitions. This step adds coverage.py's JSON report
as a second input and the orchestration that produces *one wake per test*
— which is what selection, weak, uncovered, and attribution need. Tracing
cost is proportional to the tests that ran, and after step 2 only the
selected tests run.

**`grid test` (step 2).** Test inventory from the runner (`cargo nextest
list`, `pytest --collect-only`), selection by one inbound walk from the
changed set over every grade, replay from the verdict log where the closure
key still holds, run the rest, record every verdict. A test with no
retained trace is always run; a walk over budget runs everything and says
so. A verdict that disagrees with a prior one under the same key is
reported as `nondeterministic` — the fact, not a diagnosis. Changed
definitions no test reaches are `uncovered`, a warning.

**`grid check` (step 3).** Wraps `cargo check`, records the features and
target it ran with, and reports every proven caller of a changed definition
that sits under a `cfg` the check excluded — `cfg(test)` first, features
second. Decidable exclusions gate; an unknown predicate warns.

**Plan adherence (step 4).** A def promise may say whether the interface
moves. A moved signature no promise covers holds the gate with reason
`interface`; a body edit under a promise that said `interface: false` is
tolerated. Tracks already exist; this is one optional field.

**`grid pack` (step 5).** The context pack with bodies to one depth, sigs to
a further depth, a byte-derived token budget, and a certificate that names
the depth each tier is complete to. Budget is spent in whole layers, so the
certificate is never a lie. `--since <snapshot>` packs only what moved.

**Attribution (step 6).** `grid suspects <test> --closure <since>`: the
definitions in the test's closure that changed since `since`, ranked by hop
distance, with no run history required. When two traces of the test are
retained, the first divergent hop (`grid wake diff`, C17) is appended.

**Deserialize compatibility (step 7).** For a type deriving `Deserialize`
whose sig moved, a decidable check of the field-level diff against the
rules old bytes care about. Heuristic by default; a `persisted` invariant
promotes it to a gate at `modeled`, because a human declared that old
bytes exist.

**Interference (step 8).** `grid interfere <other-worktree>`: over two
stores against a common base, the definitions both touched, and each
side's drift suspects that land in the other's touched set. Semantic
overlap git's merge cannot see.

**`grid inspect` (step 9).** The composite: gates (drift, check,
adherence, verdict-stale, covenant, compat-when-declared) and a ranked tail
(weak, uncovered, clone-of, compat-when-not-declared). Exit is the max over
gates; a component that refuses refuses the whole. A refactor certificate
is one `done` record over the same data: interface-stable, tests replayed,
definitions uncovered.

**Not built, recorded as decisions:** dependency-upgrade suspects (Rust:
`cargo check` covers types and body diffs of dependencies are pure noise;
Python-first, separate plan), reachability/security reports (presence-only
is all a call graph can say; not led with), profiler ingestion (grid holds
counts, not time), doc drift (low precision), near-duplicate detection
(measure exact-shape hit rate first), V8 coverage (no TS frontend yet), a
nextest wrapper script for single-run per-test tracing (ceiling marked in
step 1), per-definition declined counts (a segment fact; step 0b's split is
per file and summed).

---

## Part III — Use cases

Each is a scenario, the commands, and what the records say. Exit codes are
the uniform ones (`0` pass, `1` internal, `2` bad input, `3` divergence,
`4` stale coordinate).

**1. Iterating on one function, Python monorepo, 20-minute suite.**
```
$ grid test
hit   @9c1e..  pass  2026-09-01T10:02:11Z           (× 884 replayed)
miss  @44a0..  closure-changed                       (× 12 selected)
miss  @b17f..  no-trace                              (× 3 never traced: run)
... pytest runs 15 tests ...
test  match  tests/test_parse.py::test_roundtrip  pass=pass
done  tests 15/899 ran · 884 replayed · 0 failed
```
Exit 0. Fifteen tests ran instead of 899 because 884 closures did not
move and each of those tests has a trace saying what it actually reached.

**2. Same, Rust workspace with 40 integration binaries.** Selection rolls
up to `(package, binary)` and the runner is invoked as
`cargo nextest run -p a --test x -p b --test y -E 'test(=…) + …'`, so 38
binaries are neither compiled nor linked. The saving is the link, not the
run.

**3. A green check that was not green.** Agent moves a signature, runs
`cargo check`, sees green, commits; `cargo test` fails in CI on a caller
under `mod tests`.
```
$ grid check
drift proven  crate.mod.tests.round_trips  <-calls-  crate.mod.parse  1233→4bc4  uncompiled:cfg(test)  src/mod.rs:212
```
Exit 3, before the commit. `grid check -- --tests` compiles it and the
record disappears.

**4. "I only refactored."** Agent claims a pure refactor.
```
$ grid inspect
done  refactor  interfaces held · 14 covering tests replayed pass · 2 changed defs uncovered
warn  crate.io.flush   uncovered
warn  crate.io.retry   uncovered
```
Exit 0 — nothing gates — but the two uncovered definitions are named,
which is the sentence the claim was missing.

**5. Test fails now, passed at the last snapshot, no history retained.**
```
$ grid suspects tests::parse_roundtrip --closure @a1b2
hit  crate.parse.header@e03f   closure  via 1 hop, datum moved
hit  crate.util.trim@77c1      closure  via 3 hops, datum moved
miss 41  references declined inside the closure (internal 6, method 35)
```
Exit 0. Two candidates out of 4312, in milliseconds, and the declined
count says how much of the closure the chart could not follow.

**6. Two agents in two worktrees.** A moved `key_event` signature in
worktree A; a new caller of it in worktree B; no file in common.
```
$ grid interfere ../wt-b
drift proven  yi_console.replay  <-calls-  yi_tui.drive.key_event  1233→4bc4  interfere:../wt-b  crates/console/src/replay.rs:40
```
Exit 3 before either merges.

**7. Plan mode → track → gate.** The plan says `parse`'s interface moves and
nothing else's does. Implementation also moved `render`'s.
```
$ grid gate plan.json
hold  plan-42  @8ae0143
def   ~  crate.view.render  5d1c→a90e  sig   unaccounted: interface
```
Exit 3. `grid drift --track plan.json` on the same track suppresses the
promised callers and lists the rest.

**8. Context for an edit, with a budget.**
```
$ grid pack crate.parse.header --bodies 1 --sigs 3 --budget 3000
## crate.parse.header  src/parse.rs:88   (body)
...
## crate.parse.read_u16  src/parse.rs:60   (body, hop 1)
...
## sig  crate.io.Reader.fill  src/io.rs:41   (hop 2)
pub fn fill(&mut self, _: &mut [u8]) -> io::Result<usize>
...
# horizon: bodies complete through 1 hop, sigs through 3, ≈2.8k tokens (bytes/4)
```
A document on stdout, the pack on `--json`. The horizon line is what makes
a bounded answer usable.

**9. A config struct that old files were written by.** A field added
without `#[serde(default)]` to a type declared `persisted`.
```
$ grid inspect
inv   violated  persisted:crate.cfg.Config   field `retries` required; bytes from @a1b2 lack it
```
Exit 3, `modeled`, because the declaration is what makes old bytes a fact.
Without the declaration the same finding is a `warn` and exits 0.

**10. The same test twice, different verdict, nothing changed.**
```
$ grid test
warn  tests.net.test_reconnect  nondeterministic  same closure key, same env: pass 10:02, fail 10:14
```
The claim is exactly "not the charted code". A race is code; grid does not
say flake.

---

## Standing rules for every step (the repo's own, restated)

- **Additive wire evolution only.** New fields are `Option` +
  `skip_serializing_if`; new records are new structs; enum variants extend,
  never repurpose. The one segment epoch bump is declared in step 3 and
  batches every new per-definition fact this plan needs, so the store is
  dropped once, not three times. `Segment::parse` keeps reading every epoch
  ever shipped, pinned by a committed fixture.
- **No new dependencies.** coverage.py JSON and `cargo nextest list`'s
  JSON are `serde_json`, already in the TCB; lcov is the existing line
  scan; everything else is walks over data grid owns.
- **Runners stay outside.** Grid spawns `cargo`, `pytest`, `coverage` the
  way it spawns `git`, and ingests their output. Grid instruments nothing
  and ships no plugin.
- **Panic budget zero, HAR posture.** New index math `get`-guarded or
  saturating; no `unwrap` outside tests; every walk budgeted, **rejected
  never truncated**.
- **Every step lands with its conservatism test**, seen red before the fix
  is claimed, and every commit message records the mutation that made it
  red.
- **Evidence grades are load-bearing.** Proven and observed gate. Modeled
  gates only under a declaration (covenant, `persisted` invariant).
  Heuristic — clone-of, weak, uncovered, undeclared compat, closure
  attribution — ranks and never moves the exit code.
- **Perf ledger holds or is corrected in writing.** Every commit message
  re-measures its row (the ledger below) and states the number; a budget
  that does not hold is stated, never rounded.

Sequencing (each step independently shippable, gates green between):

```
0. prerequisites      rev-survey in one process; declined split + method-call counting
1. traces             coverage.py JSON ingestion; one wake per test (recipes for `grid test --trace`)
2. grid test          inventory, selection, replay, record, nondeterministic, uncovered   (needs 1)
3. grid check         cfg facts (segment epoch 4), cargo check wrapper, uncompiled callers
4. plan adherence     `interface` on a def promise                                        (small)
5. grid pack          tiers, budget, certificate, --since
6. attribution        `grid suspects --closure`                                            (needs 0a)
7. compat             Deserialize rules + `persisted` invariant                            (needs 0a)
8. interference       `grid interfere`                                                     (needs 0a)
9. grid inspect       composition, clone-of, refactor certificate, orphans with traces     (needs 2, 3, 4, 7)
10. surface           README agent loop, hooks, PORCELAIN/ONTOLOGY/ASSURANCE closure       (needs 9)
```

---

## Part IV — Technical implementation plan

### 0. Prerequisites

#### 0a. A rev survey spawns one git process

**Problem.** `files::sources_at` runs `git show rev:path` per file
([files.rs:453](../../crates/survey/src/files.rs)). yi has 18,398 tracked
source files; `grid survey HEAD~1` took ~10s there, and `grid drift`'s
default base is a rev, so the flagship verb inherited it. Steps 6, 7, 8
and 9 all survey at revs.

**Design.** One `git cat-file --batch` child: write `rev:path\n` per file
to its stdin, read `<oid> blob <size>\n<bytes>\n` per answer from its
stdout, in order. Missing or non-blob answers (`… missing`) skip the file
and are counted, as an unreadable blob is today. Manifests read through
the same child. Ignore filtering (`ignored`) stays as it is; the listing
stays `ls-tree`. Two spawns per rev survey, total.

`SurveyError::Git` keeps its meaning. No wire change.

**Tests.**
- Ground truth: a rev survey of a fixture repo produces the same snapshot
  id before and after (C2 makes byte identity a one-line assertion).
- Differential: for every tracked file, the blob read through the batch
  equals `git show`'s, over a fixture with a non-UTF-8 blob, a symlink,
  and a file added only in the rev.
- Conservatism: a path the listing names that the rev cannot serve is
  skipped and counted, never fabricated as empty.

**Perf.** yi `grid survey HEAD~1`: ~10s → under 1s (the survey itself is
~270ms). Grid: no measurable change (92 files).

#### 0b. The declined count says what it declined

**Problem.** One number for three populations, and Rust method calls are
not in it at all.

**Design.**
- `grid_datum::RefKind` gains `MethodCall` — `x.m(...)` with the receiver
  path and method name, extracted by `lang-rs` (`visit_expr_method_call`)
  and by `lang-py` for attribute calls that are not `self.` (today filed
  as `Call`; the variant is additive and `Call` keeps meaning what it
  means for `f(...)`). Both resolvers decline a `MethodCall`
  unconditionally: no types, no edge, same rule as today, now counted.
- `survey::Resolved.declined` stays as the sum; three counters are added
  beside it: `declined_external` (the path's head is `std`/`core`/`alloc`,
  a package named in `Cargo.lock`, or a Python top-level module not rooted
  in the tree), `declined_method`, and `declined_internal` (everything
  else: glob-supplied names, ambiguous callsigns, unresolved re-exports).
  Nothing that reads `declined` changes meaning.
- Wire: `DriftUnchecked` gains `unresolved_external`, `unresolved_method`,
  `unresolved_internal` (all `Option<u32>`, `skip_serializing_if`);
  `Snapshot` (the `grid.survey.run` result) gains the same three under an
  optional `declined` object.
- Porcelain: `grid survey` closes with one `miss` record when the sum is
  nonzero — `miss 2285  references declined (external 1740, method 0,
  internal 545)` — the existing tag, the existing "what could not be
  checked" meaning. `grid drift`'s closing `miss` gains the same split in
  its message. No new record.

**Tests.**
- Ground truth (per language): a fixture with one call into `std`, one
  method call on a typed local, one name supplied only by a glob import,
  one call to a sibling definition → edges 1, counts (1, 1, 1).
- Conservatism (seen red first): a method call on a receiver whose type
  is syntactically obvious (`Foo::new().m()`) still yields **no edge** and
  is counted as `method` — types are `modeled` and this plan emits none.
  Red today because the call is not counted at all.
- Property: `declined == external + method + internal` for every file in
  the corpus.

**Perf.** Extraction of method calls adds one visitor arm; survey on grid
re-measured (expected within noise of ~114ms).

**Numbers to state in the commit.** grid and yi, all three counters. The
expectation: on Rust the `method` counter will dominate once it exists,
which is the honest picture step 1 is built for.

---

### 1. Coverage as per-test traces

**Problem.** Selection, weak, uncovered and attribution all need *one wake
per test*. `grid wake ingest --lcov <report> --test <name>` already maps
executed lines to definitions ([lcov.rs](../../crates/grid/src/lcov.rs))
and retains one wake per call; what is missing is a second input format
for Python and the orchestration that produces one report per test.

**Design.**
- `grid wake ingest --coverage-json <file> --test <name>`: coverage.py's
  JSON report (`files.<path>.executed_lines`) through the same
  line-to-definition mapping `lcov::wake_of` uses; the parser is
  `serde_json` into two typed structs. Same C16 accounting (unmapped lines
  and unknown files counted, never dropped). The wake id is
  `wake:cov:<test>`; the lcov path keeps `wake:lcov:<test>`.
- `Wake` gains `snapshot: Option<SnapshotId>` (additive): the snapshot the
  trace was mapped against, so decay grading and attribution can pin the
  chart without resolving `git_rev` through git. `git_rev` stays.
- Per-test orchestration lives in `grid test --trace` (step 2); this step
  ships the ingestion and a documented recipe per runner:

  Rust — `cargo llvm-cov nextest --lcov --output-path <f> -E 'test(=<name>)'`
  per test, then `grid wake ingest --lcov <f> --test <name>`. Per-test
  reports need one export per test because a single profile cannot be
  attributed to tests that shared a process; nextest runs one process per
  test, which is what makes the loop correct at all. `cargo llvm-cov` is
  the user's toolchain (it is not installed on the machine this plan was
  written on; nextest is) — `grid test --trace` says so and runs untraced
  when it is absent.
  `ponytail:` one nextest run with a wrapper script keying
  `LLVM_PROFILE_FILE` by test name would make this a single invocation;
  that is nextest configuration, not grid code, and lands when the loop
  is measured too slow for CI.

  Python — `pytest --cov --cov-context=test`, then per test
  `coverage json --contexts='<node id>' -o <f>` and
  `grid wake ingest --coverage-json <f> --test <node id>`. No rerun per
  test: the contexts are already in coverage.py's data file.

- Wake volume: one JSON file per test under `.grid/wakes`; `retained()`
  reads all of them per query. `ponytail:` O(wakes) per read; a
  membership-style packed index lands when a repo holds more than ~10k
  retained wakes.

**Tests.**
- Ground truth: a coverage.py report fixture over the Python fixture repo
  → the same observations `--lcov` produces for the same lines
  (differential between the two ingestion paths — they share the mapping
  and nothing else).
- Conservatism: a report naming a file the chart does not hold, and lines
  outside any definition, are counted and reported; a malformed report is
  bad input (exit 2), never an empty wake.
- Property: ingesting the same report twice retains one wake with the same
  bytes (idempotence, C2's shape).

**Perf.** Ingestion of a 4312-def report under 50ms (one `BTreeMap` per
path and a linear scan of lines).

---

### 2. `grid test`

**Problem.** `cargo test` and `pytest` run everything; grid holds enough
to say which tests a change can reach, to replay verdicts whose closure
did not move, and to name what no test reaches — with the runner's own
inventory and the runner's own execution.

**Design.**

```
grid test [base] [--trace] [--all] [--no-replay] [--json] [-- <runner args>]
```

1. **Inventory** — the runner's, never grid's guess: `cargo nextest list
   --message-format json` (Rust; `cargo test -- --list` as the fallback
   when nextest is absent, binary granularity only) and `pytest
   --collect-only -q` (Python). Each test id maps to a callsign by the
   frontend's spelling (`crate::mod::tests::t` → `crate.mod.tests.t`;
   `tests/test_x.py::TestY::test_z` → `tests.test_x.TestY.test_z`) and
   resolves by `grid.resolve`'s rule to a datum. A test the chart cannot
   resolve is **always run** and listed as `miss <id>  unresolved`.
2. **Changed set** `C` — every definition whose datum moved between `base`
   (default: the last snapshot a `grid test` recorded against, else
   `HEAD`) and a worktree survey. Datum, not sig: tests care about bodies.
3. **Selection** — one inbound walk from `C` over **every** grade
   (`Kind::Test` semantics: proven, observed, modeled, heuristic), fuel
   `grid_cache::FUEL`. A test is selected iff the walk reaches its datum.
   A walk that runs aground selects **every test** and says why on stderr
   (rejection, never a shorter list). A test with **no retained trace** is
   selected regardless of the walk — the static closure alone does not
   license a skip, because the honesty number says it is incomplete.
4. **Replay** — for each test not selected, `grid.cache.check` with
   `Kind::Test` and this run's `env_hash`; a hit is replayed as a `hit`
   record; a miss (key moved, or never recorded) joins the run set.
5. **Run** — Rust: selected tests roll up to `(package, binary)` and the
   runner is invoked once as `cargo nextest run -p … --test … -E 'test(=…)
   + …'` (binary-level restriction is what skips links); Python: `pytest
   <node ids>`. Runner output passes through to stderr; grid's records
   stay on stdout. With `--trace`, each selected test runs under the
   step-1 recipe and its wake is ingested afterwards, so traces are
   refreshed exactly for the tests that ran.
6. **Record** — every verdict through `grid.cache.record` with the wake
   id when traced. A record whose key already holds the **opposite**
   verdict is reported: `warn <test>  nondeterministic  same closure key,
   same env: pass <t1>, fail <t2>` — detected at record time from the
   last entry, no history table needed. The word is *nondeterministic*,
   never *flake*: the closure proves the charted code did not change, and
   a race is code.
7. **Uncovered** — definitions in `C` the selection walk never connected
   to any test: `warn <def>  uncovered`. Heuristic; never gates.

`env_hash` = blake3 over: toolchain (`rustc -Vv` / `python --version`),
lockfile bytes (`Cargo.lock` / the resolved requirements file when one
exists), the runner args, the target triple, and whether the run was
traced (an instrumented binary is a different environment). Stated in
`--json` so a verdict can be re-keyed later.

**Wire (additive).** `grid.test.plan` — selection as data, no running:

```rust
pub struct TestPlanParams { pub base: Option<SnapshotOrRev>, pub env_hash: String,
                            pub tests: Vec<TestId> }          // the runner's inventory
pub struct TestPlan {
    pub base: SnapshotId, pub head: SnapshotId,
    pub selected: Vec<TestSelection>,      // test, datum, why: reached | no_trace | unresolved | aground
    pub replayed: Vec<CacheHit>,
    pub uncovered: Vec<DefRef>,
    pub unchecked: DriftUnchecked,         // the step-0 split
}
```
The CLI does the spawning, as `survey` spawns git; the daemon answers the
plan and records verdicts through the existing `grid.cache.record`.

**Porcelain.** Existing tags only: `hit` (replayed verdicts), `miss`
(selected, with `closure-changed` / `no-trace` / `unresolved` / `aground`
as the reason), `test` (outcomes, `expected=actual`), `warn`
(`nondeterministic`, `uncovered`), and a closing `done  tests R/N ran ·
H replayed · F failed`. Exit `3` when any run test failed, `1` when the
runner could not be spawned, `0` otherwise. `--all` skips selection and
replay and runs the inventory (the CI form).

**Tests.**
- Ground truth: a fixture repo (both languages) with three tests, traced;
  change one definition → exactly its test is selected; a fourth test
  with no trace is selected every time.
- Differential (rung 5): over a seeded sequence of edits, for every test
  the plan skipped, the recorded verdict at base equals the verdict a full
  run at head produces — the selection is compared against running
  everything, edit by edit.
- Conservatism (seen red first): a test whose only trace is decayed
  (`stale-trace`) is selected, not skipped; a walk over fuel selects every
  test; a changed definition with no inbound path is `uncovered` and the
  exit code does not move for it.
- Property: `plan(S, S)` selects only the untraced and unresolved tests;
  selection is deterministic for a snapshot pair; adding a trace never
  *adds* a selected test.

**Perf.** `grid.test.plan` on grid under 20ms warm (one inbound walk over
~2.5k edges plus one cache check per test). The run is the runner's.

---

### 3. `grid check`

**Problem.** `cargo check` does not compile `#[cfg(test)]` code, nor code
behind features it was not given. Grid's chart is text, so it holds those
callers. The gap between "green check" and "red test" is decidable from the
chart plus the flags the check ran with.

**Design.**
- **Segment epoch 3 → 4** (the one bump in this plan, declared here):
  every definition gains a `cfg` fact — the conjunction of every `#[cfg]`
  predicate on the item and on each enclosing module, as written,
  interned in a per-segment string table (`Section::DefCfgs`: one `u32`
  index per def + a table; `cfg_attr` recorded as its own predicate text
  and evaluated as *unknown*). Epoch-3 images answer `cfg` with `None`.
  The epoch-3 fixture is committed before `FORMAT_EPOCH` flips; the
  superseded-store recovery from the drift plan handles the flip.
- `grid check [-- <cargo check args>]` runs `cargo check` with the given
  args (default: none), records `{features, all_features, no_default,
  tests, target}` from the args plus `[features].default` from each
  manifest, and evaluates each changed definition's proven callers' `cfg`s
  against that record. A predicate that evaluates to *false* under the
  record means the caller was not compiled: `drift proven <caller>
  <-calls- <changed> <sig> uncompiled:<cfg> <file:line>`, exit `3`. A
  predicate the evaluator cannot decide (`cfg_attr`, a custom cfg, a
  `target_*` the record lacks) is `warn <caller>  cfg unknown: <text>`,
  exit unchanged — an unproven exclusion does not gate.
- The evaluator: `all`, `any`, `not`, `test`, `feature = "…"`,
  `unix`/`windows`/`target_os`/`target_family`/`target_arch`, against the
  host triple when no `--target` was given. ~80 lines, no dependency.
- Python: `grid check` prints one hint — there is no compile to wrap —
  and exits `0`. Said, not silent.
- The changed set is the same `C` as `grid drift` (sig moved or unknown);
  a body-only edit has no uncompiled-caller problem.

**Wire (additive).** `grid.check` → `CheckReport { compiled_with:
CheckRecord, uncompiled: Vec<DriftSuspect>, unknown: Vec<DriftWarning> }`;
`DriftSuspect.reason` carries `uncompiled:<cfg>`; `DefRef` gains
`cfg: Option<String>`.

**Tests.**
- Ground truth: `#[cfg(test)] mod tests { fn t() { changed() } }` → one
  `drift` record after a plain check; none after `-- --tests`. A
  `#[cfg(feature = "tui")]` caller → a record without the feature, none
  with `--features tui`, none with `--all-features`.
- Conservatism (seen red first): a caller under `cfg_attr(…)` or a custom
  `cfg(kani)` is a `warn`, not a `drift` record, and the exit is `0`.
- Epoch: the epoch-3 fixture parses; its `cfg` answers `None`, and a check
  against it warns `cfg unknown: pre-epoch-4 snapshot` for every caller
  rather than declaring anything compiled.
- Property: `check` with `--all-features -- --tests` on a repo with no
  custom cfgs emits nothing.

**Perf.** cfg extraction adds an interned string per def; survey
re-measured. The check itself is `cargo check`'s time plus under 20ms.

---

### 4. Plan adherence: `interface` on a def promise

**Problem.** A track already holds the gate on unpromised work
(`defs_unaccounted`, xte) at datum granularity. A plan says which
*interfaces* move; a body edit inside a promised definition is normal, a
moved signature nobody promised is the thing to hold on.

**Design.** `TrackDefPromise` gains `interface: Option<bool>` (additive).
Semantics in `grid_xte::verdict`:
- `Some(true)`: made good only by a `DefChange` with `sig_changed ==
  Some(true)` (or `None` — unknown is treated as moved, conservative);
- `Some(false)`: made good by a change with `sig_changed == Some(false)`;
  a sig move under this promise is `unaccounted` with reason `interface`;
- `None`: today's behavior, either.
A def whose sig moved and matches no promise with `interface` `true` or
`None` is `unaccounted` with reason `interface`, which holds the gate.
`grid track lint` rejects `interface` on an `add`/`remove` promise (bad
input, exit 2). `grid drift --track` already suppresses promised callers.

**Wire (additive).** The field, plus a second list beside the existing
`defs_unaccounted` naming the changes held for this reason:
`XteVerdict.interface_unaccounted: Option<Vec<DefChange>>` (a member of
both lists is held once, with reason `interface`).

**Porcelain.** `def ~ <callsign> <old→new> sig  unaccounted: interface` as
a trailing column on the existing `def` record inside gate output.

**Tests.** Ground truth for the three values; conservatism (seen red
first): a pre-epoch-3 snapshot on either side makes `sig_changed` `None`,
and a `Some(false)` promise then **holds** (unknown is a move) rather than
passing; property: adding `interface: None` to every promise leaves every
existing verdict fixture byte-identical.

**Perf.** Zero.

---

### 5. `grid pack`

**Problem.** `grid scope` returns names, edges, rules and churn, and
deliberately no bodies. An agent's next call is to read the bodies, one by
one, with no idea how much of the neighbourhood it has. A pack with a
budget and a certificate closes that loop in one call.

**Design.**

```
grid pack <pattern>... [--bodies N] [--sigs M] [--budget T] [--since <snapshot>] [--json]
```

- Layers by hop from the roots (in and out edges, `scope`'s walk). Bodies
  for hops `0..=N` (default 1), sig text for hops `N+1..=M` (default 3).
  Sig text is rendered by the frontend from the worktree source at pack
  time (`lang-rs`: the erased signature tokens `sig::of_fn` already
  builds; `lang-py`: the sig encoding's `def` line) — the sig section's
  own inclusion rule says this is exactly what a caller needs.
- Budget `T` in approximate tokens (`bytes / 4`, stated as `≈`; grid
  cannot run a tokenizer). Spent in **whole layers**, farthest first:
  when a layer does not fit, the whole layer is dropped and the
  certificate stops at the previous one. Never a partial layer — a
  certificate over a partial layer is the lie `complete_within` exists to
  prevent (C31).
- Output is a **document** on stdout (`## callsign  file:line  (body|sig,
  hop k)` headers, then the text), closed by one `# horizon:` line naming
  both depths and the estimate. PORCELAIN.md gains a "text-mode verbs"
  paragraph: `grid pack` and `grid explain` emit prose, and `--json` is
  the record form. `--json` extends `ScopePack` additively: `bodies:
  Option<Vec<PackText>>`, `sigs: Option<Vec<PackText>>`, `horizon:
  Option<PackHorizon { bodies_complete_within, sigs_complete_within,
  approx_tokens }>`.
- `--since <snapshot>`: keep only definitions whose datum or sig moved
  since — the delta-map a returning agent reads instead of the repo.
- Docs, ranking, hub maps: not built. The pack is ordered by hop, then
  callsign; anything smarter is heuristic and lands when measured wanted.

**Tests.**
- Ground truth: a chain fixture, `--bodies 1 --sigs 3`: bodies for hops
  0–1, sigs for 2–3, nothing beyond, horizon `1`/`3`.
- Property: a bigger budget's pack is a superset of a smaller one's
  (monotone); the set of definitions at each certified depth equals
  `grid scope --depth k`'s (differential against the existing walk).
- Conservatism (seen red first): a budget too small for the roots' own
  bodies emits the roots' sigs only and a horizon of `bodies: none, sigs:
  0` — never a truncated body, never a missing horizon line.

**Perf.** Under 30ms on grid for a 3-hop pack (the walk is `scope`'s;
rendering is a file read per body).

---

### 6. Attribution: `grid suspects <test> --closure <since>`

**Problem.** `grid suspects` ranks culprits by Ochiai over retained
spectra and refuses without history (C23). Most failures happen with no
history: the test passed at the last snapshot, fails now. The closure and
the membership diff answer that case in milliseconds.

**Design.** `--closure <snapshot|rev>` is a second evidence mode on the
same verb (the existing mode is untouched; the refusal it makes stays a
refusal):
- candidates = `closure(test datum, Kind::Test, at head)` ∩ the `Removed`
  half of `membership.changed_between(since, head)` — the definitions the
  test could reach whose bodies moved;
- ranked by hop distance from the test (leadline walk depth), then datum;
- when two retained wakes of the test exist (one at or before `since`),
  `grid_wake::diff`'s first divergence is appended as a `fork` record —
  the existing C17 machinery;
- the closing `miss` carries the step-0 split *restricted to the closure*
  when the survey can say it (per-file counts summed over the closure's
  files), else the repo-wide split — the reader must know how much of the
  closure the chart could not follow.

Grade: **heuristic** — a candidate is a definition that changed and is
reachable, not a proven cause. Rows are `hit` records with `closure` as
the verdict column and `via k hops` as the evidence text; exit `0`
always. A closure walk over fuel refuses (exit `1`, C4's rule). An
unknown `since` is bad input (exit `2`).

**Wire (additive).** `SuspectsParams.closure_since: Option<SnapshotOrRev>`;
`SuspectsRankedItem.hops: Option<u64>`.

**Tests.** Ground truth: a fixture where the culprit is 2 hops away and a
changed definition outside the closure is not listed; differential: the
candidate set equals `grid scope <test>` at full depth ∩ `grid diff since
head`, computed independently in the test; conservatism (seen red first):
a closure over fuel refuses the whole answer, and a `since` the chain never
held is a typed refusal, not an empty ranking.

**Perf.** Under 15ms on grid (one closure walk, one merge-diff).

---

### 7. Deserialize compatibility, and the `persisted` invariant

**Problem.** Within one repository both ends of a serde boundary share the
struct and the compiler forces them to move together. The case it cannot
see is **old bytes**: a config file, a cache row, a queued message written
by the previous snapshot. The drift plan's covenant covers a declared
*pair*; this is the single-type case, and the pressure test demoted the
brainstorm's "every derive is a boundary" to what is actually decidable.

**Design.**
- For each definition in `C` (sig moved) that is a `struct`/`enum` with a
  `Deserialize` derive at either snapshot: reconstruct the field/variant
  list with serde attributes from source on both sides (rev survey, step
  0a; `syn` parse of the one item) and apply the rules old bytes care
  about:
  - a field added without `default`/`default = …`/`skip_deserializing`
    and the type is not `Option` → **required field added**;
  - a field renamed (shape/name mismatch) without `alias` → **field
    renamed**;
  - a field's type changed → **type changed**;
  - a variant removed from an enum → **variant removed**;
  - `deny_unknown_fields` present and a field removed → **field removed
    under deny_unknown_fields**;
  - `rename_all` / `tag` / `untagged` changed → **representation changed**.
  Anything else (a removed field without `deny_unknown_fields`, an added
  `Option` field) is compatible and produces nothing.
- Grade **heuristic** by default: `warn <type>  compat: <rule>` — old
  bytes are not known to exist.
- `.grid/invariants.json` gains an `InvariantRule::Persisted` rule over a
  `CallsignPattern` (additive enum variant; `standing.rs` pattern). A
  finding on a type matching a `persisted` pattern is `inv violated
  persisted:<type>  <rule>` at **modeled**, exit `3` — the declaration is
  the human fact that old bytes exist.
- Rust only in this step; the sig already carries serde attributes, so a
  changed attribute is already in `C`. Python (`dataclass`, pydantic) has
  no attribute vocabulary to read yet — recorded, not built.

**Wire (additive).** `CompatFinding { def: DefRef, rule: String, declared:
bool }` on `InspectReport` (step 9); `InvariantRule` gains the
`persisted` variant.

**Tests.** Ground truth per rule (six fixtures, each the smallest struct
that trips exactly one rule); property: a type whose sig held produces
nothing, and adding `#[serde(default)]` to the tripping field silences the
first rule; conservatism (seen red first): a type whose derive is under a
`cfg` or comes from a macro the survey cannot see is skipped and counted
(`miss 1  compat unchecked (derive not visible)`), never declared
compatible; undeclared findings never move the exit code.

**Perf.** Two item parses per changed `Deserialize` type; negligible.

---

### 8. Interference: `grid interfere <other-worktree>`

**Problem.** Parallel sessions in worktrees (this plan was written in one)
merge on text. Grid can see the semantic overlap: a moved signature here,
a caller of it there, no file in common.

**Design.**

```
grid interfere <other-root> [--base <rev>] [--json]
```
- Base = `--base` or the merge base of the two heads; each store must hold
  a snapshot at base (surveyed at rev on demand, step 0a — a second store
  is opened read-only at `<other-root>/.grid`, never shared, never
  written).
- `touched(X)` = membership diff base → X's worktree snapshot;
  `drift(X)` = `grid drift base X` in X's store.
- Findings: `touched(A) ∩ touched(B)` (both edited the same definition — a
  textual conflict git will also find, listed for completeness with reason
  `both-touched`); `drift(A).suspects ∩ touched(B)` and the converse
  (reason `interfere:<other-root>`), each a `drift` record with its
  proven/observed grade. Exit `3` when any finding gates, `0` when none,
  `2` when either store cannot be opened, is superseded, or lacks the base
  — **never** "no interference" for a store grid could not read.

**Wire (additive).** `grid.interfere` → `InterfereReport { base, mine,
theirs, both_touched: Vec<DefRef>, suspects: Vec<DriftSuspect>, unchecked:
DriftUnchecked }`.

**Tests.** Ground truth: two tempdir worktrees from one fixture repo, a sig
move in A and a new caller in B → one record, exit 3; no overlap → no
records, exit 0; conservatism (seen red first): B's store superseded (an
older epoch) refuses with exit 2 and the reason, rather than reporting an
empty answer; property: the report is symmetric in the two roots up to the
`mine`/`theirs` labels.

**Perf.** Under 100ms with both stores warm (two membership diffs, two
inbound walks).

---

### 9. `grid inspect`

**Problem.** Each finding above is a verb; an agent needs one call after
an edit and one exit code. `grid review <track>` is the *gate's* pack and
keeps its meaning; `inspect` is the track-optional composite.

**Design.**

```
grid inspect [base] [head] [--track <file>] [--quick] [--json]
```
- **Gates** (any → exit `3`): drift suspects, `check` findings when a
  check record from this worktree exists and is newer than the last
  survey, adherence when `--track`, verdict-stale (`drift --tests`),
  covenant fires, compat findings on `persisted` types.
- **Ranked tail** (never moves the exit): weak-for, uncovered (from the
  last `grid test` plan, or recomputed when traces exist), clone-of
  (definitions **added** since base whose `shape` equals an existing
  definition's — the `Added` half of the membership diff joined against
  the shape column; exact match only, `warn <new>  clone-of <existing>`),
  undeclared compat findings.
- **Refactor certificate**: one `done` record when no gate fired —
  `done  refactor  interfaces held · <n> covering tests replayed pass ·
  <m> changed defs uncovered`; the phrase "behavior-preserving" appears
  nowhere.
- `--quick`: drift + check only, no wake or verdict-log reads; the
  PostToolUse form, budgeted under 50ms.
- Composition rule: every section is the corresponding verb's own result
  (`InspectReport` embeds `DriftReport`, `CheckReport`, `XteVerdict`,
  `Vec<StaleTest>`, `Vec<WeakTest>`, `Vec<CompatFinding>`, …), not a
  re-implementation; a section that **refuses** (aground, unknown
  snapshot, superseded store) refuses the whole inspect with that reason
  (exit `1`/`2`), never an inspect with a missing section.
- `grid orphans` gains a `--traced` flag: an `orphan` record also carries
  `wake 0` when no retained trace hits the definition either, which is the
  workspace-level unused-`pub` question rustc cannot ask (it checks per
  crate). Heuristic; `pub` in a published crate stays a maybe.

**Wire (additive).** `grid.inspect` → `InspectReport`; `CloneFinding`;
the orphans result gains `traces: Option<u64>` per item (a new struct if
the item is a bare `DefRef` today — never a repurposed column).

**Porcelain.** Sections print in gate-first order with the existing
records (`drift`, `def`, `inv`, `miss`, `warn`, `done`); the version bumps
to 1.7.0 for the new columns and the text-mode paragraph.

**Tests.** Differential: each section of `inspect --json` is
byte-identical to the standalone verb's `--json`; exit is the max over
gates on a fixture that trips two of them; conservatism (seen red first):
an aground drift section makes inspect exit `1` with the reason, not a
report without a drift section; clone-of on a fixture with two
name-erased-identical functions and one near-duplicate finds exactly the
pair.

**Perf.** `--quick` under 50ms on grid; full under 200ms.

---

### 10. Surface

README's "why an agent wants it" is rewritten around the loop above
(locate → `pack`; edit → `edit apply`; verify → `test`, `check`,
`inspect`), with the two hook lines, and the honest sentence about Rust.
ONTOLOGY.md gains `trace` (a per-test wake), `selection`, `replay`,
`uncompiled`, `pack`, `horizon`, `interference`, `persisted`, `clone-of`,
`nondeterministic`. PORCELAIN.md gains the text-mode paragraph and the new
columns. ASSURANCE.md gains C33–C42 (one per step 0b through 9) and the
dependency line "this plan: no new packages". CURRENT_WAVE → 11.

---

## Verification posture (har-verify ladder, applied)

- Rung 1 everywhere: every step's fixtures are ground-truth pairs and every
  conservatism assertion is a named test seen red first, with the mutation
  recorded in the commit message.
- Rung 3: declined sum identity (0b); ingestion idempotence (1); selection
  determinism and monotonicity (2); `check` silence under `--all-features
  -- --tests` (3); adherence fixtures byte-identical under `interface:
  None` (4); pack monotone in budget (5); interference symmetry (8).
- Rung 5 (differential): batch blob read vs `git show` (0a); lcov vs
  coverage.py ingestion over the same lines (1); **selection vs running
  everything over a seeded edit sequence** (2); pack layers vs `scope`
  (5); closure attribution vs an independent `scope ∩ diff` (6); each
  `inspect` section vs its verb (9).
- Fuzz/miri/loom/kani: the epoch-4 segment section is parsed by
  `grid-chart`'s validate-then-accept path and joins the adversarial
  corpus (C1); nothing else here parses untrusted bytes, owns `unsafe`, or
  adds concurrency. coverage.py JSON and nextest's JSON are serde into
  typed structs — the same trust posture as tracks and covenants.

## Perf ledger

Budgets, to be measured and stated in each commit (Apple M4 Max, release,
grid at 1395 defs / 2535 edges unless a row names yi).

| surface | budget | why it should hold |
|---|---|---|
| `grid survey <rev>`, yi | < 1s (from ~10s) | two git spawns instead of ~18,400 |
| `grid survey` with method-call refs and cfg facts | within noise of ~114ms | one visitor arm; one interned string per def |
| segment size, epoch 4 | +4 B/def + cfg table | one index per def; predicates repeat |
| coverage ingestion, 4312 defs | < 50ms | one map per path, linear over lines |
| `grid.test.plan` | < 20ms warm | one inbound walk + one cache check per test |
| `grid check` over `cargo check` | < 20ms | cfg evaluation per proven caller of `C` |
| `grid pack`, 3 hops | < 30ms | `scope`'s walk + one file read per body |
| `grid suspects --closure` | < 15ms | one closure walk + one merge-diff |
| `grid interfere`, both stores warm | < 100ms | two diffs, two walks, one extra store open |
| `grid inspect --quick` / full | < 50ms / < 200ms | drift + check / plus wake and verdict reads |

Two rows from the drift plan stay corrected, not restored: `grid survey`
is ~114ms, not under 50ms, and a watch event is O(tree). Neither is touched
here.

## Cut line

Recorded so each is a decision, not an omission:

- **Dependency-upgrade suspects.** Rust: `cargo update && cargo check`
  gives the type-level answer and body diffs of a dependency are noise by
  construction. Python-first, needs cross-tree resolution into
  `site-packages`; its own plan.
- **Reachability / security reports.** A call graph certifies presence,
  never absence, and grid has no data flow; a curated-sink report is a
  later, small step once the honesty number is smaller.
- **Profiler ingestion.** Grid holds counts; every profiler format is a
  parser and a promise grid cannot keep.
- **Doc drift.** Low precision; a doc hash is a segment fact that can ride
  a later epoch bump with something that earns it.
- **Near-duplicate detection.** Exact `shape` first; measure the hit rate
  on grid and yi before writing a similarity algorithm.
- **V8 coverage.** Meaningless without the TS frontend; lands with it.
- **A nextest wrapper script** for single-run per-test tracing:
  configuration, not grid; when the loop is measured too slow.
- **Per-definition declined counts.** A segment fact; step 0b's split is
  per file and summed, which is enough for every consumer here.
- **`grid map`.** Hub ranking by in-degree finds `Error::new`; `pack
  --since` is the delta-map that was actually wanted.
