# Incremental survey by touched file (T-14, answering R-2)

Status: **planned**, 2026-09-01. Written after the agent-loop plan landed
(`docs/plans/2026-09-01-agent-loop.md`, whose shape this follows) and
after the survey's phases were timed one by one — the timing is in Part I
and it reorders the work. Every step is one commit with the gates green
(`cargo fmt --all --check`, `cargo clippy --workspace --all-targets` under
pedantic deny, `cargo test --workspace`, `cargo deny check`,
`scripts/check-assurance-links.sh`, `scripts/check-unsafe-confined.sh`),
its conservatism test seen red first with the mutation named in the commit
message, its perf row re-measured and stated, and its deviations from this
document listed. No new dependencies: the daemon is `std::os::unix::net`
as it is today, the client is the same socket the conformance tests
already speak, and the invalidation is a set intersection.

The subjects are TODOS.md item **T-14** (incremental worktree survey by
file event) and research question **R-2** (is a per-file incremental
resolver sound under C13, or does it need the SCC-shared closure work).
R-2 is answered in Part I: it is sound, under one condition the naive
design does not meet, and the SCC work is a different problem.

---

## Part I — Architecture overview

### The budget, and what actually costs

`grid check --quick` is the PostToolUse hook. Its budget is 50ms; it
measures 120–140ms on this repo today (README: ~141ms). The agent-loop
plan attributed the miss to "the worktree survey a hook must re-run"
(~94ms of it), and TODOS.md's T-14 proposed re-surveying only the edited
file. Before designing that, the survey was timed phase by phase
(2026-09-01, Apple M4 Max, release, per-phase `Instant`s in-process; not
committed). This repo, 114 source files, 1,813 defs; yi, 384 files, 4,312
defs:

| phase | this repo | yi | scales with |
|---|---:|---:|---|
| discovery — `git ls-files`, read every file, blake3 each | 20–26ms | 28ms | tree |
| 1 — extract (parse to defs + refs), **cold cache** | 38–43ms | 80ms | tree — in a fresh process |
| 1 — extract with every file a cache hit | 10ms* | 3ms | touched (+ a clone per hit) |
| 2 — index (`DefIndex` over every file) | 6.3ms | 13ms | tree |
| 3 — resolve (every reference of every file) | 6.5ms | 20.5ms | tree |
| segment build (`grid_chart::build`) | 1.0ms | 2.7ms | tree, linear |
| segment parse-back (counts) | 0.2ms | 0.5ms | tree, linear |
| publish (segment write + membership fold) | 12ms (11–42) | 15ms (15–36) | chain length |
| `grid drift` between two already-published ids, proven floor | < 5ms | — | changed set |
| `check --quick`, whole, warm store | 120–140ms | ~430ms† | |

\* the rev survey inside `check` — cache hits still clone `FileFacts`.
† yi's `check --quick` includes a base rev survey the store did not yet hold.

Three facts follow, and they reorder T-14:

1. **Phase 1 is O(touched) only inside a process that keeps the cache.**
   `grid_survey::Cache` is a `HashMap` in memory; every CLI invocation
   starts with it empty and parses the whole tree. The hook's process is
   always fresh. So the claim "the per-file incremental key already makes
   phase 1 O(touched)" is true of `grid watch` and false of the hook — and
   parsing is the largest single cost (38–43ms here, 80ms on yi).
2. **Discovery is O(tree) and is paid before anything.** Reading and
   hashing every tracked file is 20–28ms; `watch::Session` holds the file
   list across events precisely to skip it.
3. **Phases 2 and 3 together are 13ms here and 34ms on yi.** They are
   O(tree) by construction (C13: an edge is a fact about two files, so
   edges are recomputed from per-file facts every time) — and they are the
   *smaller* half of the problem. An incremental resolver alone would
   leave `check --quick` at ~110ms on this repo.

So the daemon is not this plan's cut line; it is its substrate. A resident
process that holds discovery, the parse cache, the index and the previous
resolution is what turns every row above into O(touched) — and once it
exists, the incremental resolver is the 13/34ms that takes the budget from
"nearly" to "holds". The publish step (12–15ms, dominated by the
membership fold) is the third cost and gets its own row.

### The question R-2 asks, and the answer

R-2: is per-file incremental resolution sound under C13 — does re-resolving
only some files after an edit produce the *byte-identical* segment a full
survey produces — or does it need the SCC-shared closure work from
`docs/research/ANALYSIS.md`?

**It is sound, and the SCC work is unrelated.** But the obvious design —
"re-resolve the touched file's references, plus every file with an edge
into a definition the touched file used to hold or now holds, found by the
previous chart's reverse adjacency" — is **not sound**, and the hole is
worth stating exactly because it is the design T-14 sketched.

The resolver's answer for a file G is a pure function of G's own facts
(its references and imports, cached by content) and of **what the index
answered to every name G asked about** — including the names it asked
about and *did not find*. Reverse adjacency records only the hits. Four
edits to a file F change G's answer without touching any edge between
them:

- **miss → hit.** F adds a definition, or a `pub use` alias, that a
  reference in G was declining; G held no edge into F, so reverse
  adjacency never names G, and the full survey emits an edge the
  incremental one lacks.
- **an ambiguity flip.** F adds a second definition under a name H
  already defines; `DefIndex.ambiguous` flips and G's edge to H (an edge
  that never involved F) disappears in the full survey and survives in
  the incremental one. Removing the duplicate flips it back the other way.
- **alias retargeting.** F's `pub use a::x` becomes `pub use b::x`; G
  resolved `x` through F's alias to `a.x`, and the edge is G → `a.x`,
  which reverse adjacency files under `a`, not `F`.
- **shadowing order.** `DefIndex::lookup` tries `{module}.{name}` before
  the absolute `{name}`; F adding an item under G's module shadows the
  absolute one G was reaching.

(A fifth, once T-17 lands: a decline moving between `external` and
`internal` because the set of crate roots changed is a change to segment
bytes, not only to a count.)

The fix is the standard one — the same idea Salsa and rustc's red-green
queries use, and, in stack-graph terms, the same object as a file's
partial paths specialised to a flat index: **record what the resolution
read**. Every `lookup_absolute`, `ambiguous.contains`, `aliases.get` and
`is_root` the resolver performs on behalf of file G appends the absolute
name it asked about to G's *read set*, hits and misses alike, alias hops
included. When F changes, the names whose *answer* changed are computed
from F's old and new facts — every absolute name F used to own or now
owns (definitions, aliases), every name whose ambiguity flipped, every
alias whose target moved — and G is re-resolved **iff its read set meets
that delta**. G's re-resolution is then, by construction, the same
function of the same inputs the full survey computes, so its edges are
identical; every file not re-resolved had no input change; the union is
the full survey's edge set; and the segment builder treats its input as a
set, so the bytes are identical. That is C13 extended from "incremental
by content hash" to "incremental by touched file", and step 4 checks it
after every prefix of an edit sequence rather than arguing it.

What the read set cannot cover, the session refuses and re-surveys in
full, counted: a change to any file's `#[path]` declarations or `mod`
cfgs (they rewrite *other* files' module names and cfgs — a whole-tree
input); a change to the crate-root set (a manifest touched); a touched
path the held list does not know (a new, deleted, renamed or relocated
file — a discovery change); more than `BURST` touched files (the watch
rule: a burst is a cold start in costume); a file that does not parse
(the watch rule: refuse the event, publish nothing); and a session whose
held view is not the store's live one (below).

**The second hole is the resident process itself.** A session that
believes `touched` is the whole truth is wrong the first time a `git
checkout`, a formatter, or another agent changes a file nobody reported.
The session therefore treats `--touched` as a hint and verifies it: on
every event it reads `.git/HEAD` and the ref it names (no spawn) and runs
`git status --porcelain -z --untracked-files=no` (one spawn, ~5ms on this
repo); every path git reports modified is added to the touched set,
added/deleted/renamed paths force the discovery path, and a moved `HEAD`
forces a full re-hash of the tree. The differential in step 4 includes an
edit the session is *not* told about.

**Against the three candidates in ANALYSIS.md:**

- **(a) session-held index + reverse adjacency, per-file re-resolution,
  differential over prefixes** — the DBSP shape the watch plan
  established. Recommended, **with the read set replacing reverse
  adjacency** (reverse adjacency is a strict subset of it: a hit is a
  read). The resolver is a join of per-file references against the
  index, and the read set is exactly the join key each file consumed;
  the state held is the two integrals DBSP asks for — the index (of
  definitions) and the per-file resolved rows — and nothing else.
- **(b) stack-graph partial paths per file.** The right shape for a
  push/pop resolver over scope graphs, which is the TS frontend's plan.
  For the flat `DefIndex` resolver grid has, a reference's partial path
  *is* its probe list, and materialising it as a graph buys nothing the
  read set does not. Not adopted here; the read set is the same idea in
  sixty lines.
- **(c) SCC-shared closures.** A different problem: it makes closure
  walks (test selection, `grid cache`) cheaper by sharing reach sets
  within a strongly connected component. It does not touch how edges are
  *produced*, which is what a survey does. Recorded as unrelated; it
  stays where ANALYSIS.md left it.

### The daemon

`grid serve` exists: a unix socket, newline-framed JSON-RPC, one
`rpc::Chart` for its life, which already holds the survey cache. Two
things are missing for it to serve the hook: the CLI never talks to it
(every verb is in-process — "both call the same rpc functions the daemon
serves"), and it holds a cache, not a session. This plan makes `grid
check --quick --touched <path>` a client when `.grid/grid.sock` answers
and an in-process full survey when it does not — correct either way,
fast only in the first case, and honest about which one happened.

Lifecycle is deliberately manual: `grid serve &` (a `just serve` recipe),
documented beside the hook line. A hook that spawns a daemon is a policy
this plan does not set; `ponytail:` an auto-start (`grid serve --daemonize`
from the first `check --quick` that finds no socket) is one flag away
when the manual form measures annoying.

### Data flow

```
 hook: grid check --quick --touched src/x.rs
   │
   ├─ socket present ──► daemon: Session.resurvey(touched)
   │                        ├─ verify: HEAD sha, git status -z  ──► widen touched / force discovery / full
   │                        ├─ phase 1: extract touched (cache)
   │                        ├─ invalidate(prev, touched facts) ──► affected files | Reject(reason) ──► full
   │                        ├─ index: remove F_old, insert F_new (multiset; ambiguity maintained)
   │                        ├─ phase 3: re-resolve affected, record read sets
   │                        ├─ build (all defs, all edges — O(tree), linear, ~1ms) ──► publish
   │                        └─ drift at proven floor + compile record  ──► CheckResult (+ incremental report)
   │
   └─ no socket ──────► in-process: grid_survey::run (full) + warn "no daemon; full survey"
```

---

## Part II — Feature set

**The invalidation set (step 1).** A pure function over the previous
survey's per-file state and the touched files' new facts: the files to
re-resolve, or a typed rejection naming why the set cannot be established.
Its own tests, independent of any session.

**The session (step 2).** `grid_survey::Session`: discovery, cache, index,
per-file read sets and resolved rows held across calls; `resurvey(touched)
-> Survey` that publishes a real snapshot; `watch::Session` uses it
instead of `run_sources`, so the four watch algebra tests (C32) become
this plan's tests too.

**The client and the flag (step 3).** `grid check --quick --touched
<path>...` and the `grid.check.touched` wire parameter; the daemon holds
one session; the CLI talks to it when present and says so when not; the
hook line passes the edited path from the tool input.

**The differential (step 4).** After every prefix of a seeded edit
sequence — including an edit the session was not told about — the
incremental snapshot id equals the full survey's. C13 extended.

**Surface (step 5).** README hook line and daemon recipe, PORCELAIN's
incremental clause, ONTOLOGY `session`, ASSURANCE C47–C49, TODOS T-14 and
R-2, the numbers.

**Not built, recorded as decisions:** an incremental segment builder, an
incremental membership fold, an OS file watcher, daemon auto-start, the
SCC work, Windows — the cut line.

---

## Part III — Use cases

**1. The hook, with a daemon up.**
```
$ grid serve &
grid: serving on .grid/grid.sock
$ grid check --quick --touched crates/survey/src/index.rs
drift proven  grid_survey.resolve_rs.target_of  <-calls-  grid_survey.index.DefIndex.lookup  1233→4bc4  crates/survey/src/resolve_rs.rs:88
miss  11985  references declined (external 1673, method 8852, internal 1460)
```
Exit 3 in under 50ms: one file parsed, four files re-resolved (the read
sets that met `grid_survey.index.DefIndex.lookup`), one snapshot
published.

**2. The same, no daemon.**
```
$ grid check --quick --touched crates/survey/src/index.rs
warn  -  no daemon at .grid/grid.sock; --touched ignored, full survey (run `grid serve &`)
drift proven  …
```
Same answer, ~120ms, and the reason on the record.

**3. A rename the hook reported by its new path.**
```
$ git mv crates/survey/src/index.rs crates/survey/src/defindex.rs
$ grid check --quick --touched crates/survey/src/defindex.rs
warn  -  touched set rejected: discovery (path not held); full survey
```
The session did not guess which module moved; it re-discovered.

**4. An edit nobody reported.**
A formatter rewrote `crates/grid/src/drift.rs` between two hook calls.
`git status` names it; the session widens the touched set and the
snapshot is the one a full survey produces. Nothing printed unless the
widening was a rejection.

**5. `grid watch`, unchanged on the outside.**
The signed-delta stream is byte-identical to today's; its per-event cost
drops from "whole-tree phases 2/3" to the affected files'.

---

## Standing rules for every step (the repo's own, restated)

- **Additive wire evolution only.** New params and fields are `Option` +
  `skip_serializing_if`; `grid.survey.run` and `grid.check` gain a
  `touched` parameter and an optional `incremental` report; no method is
  renamed or repurposed. No segment epoch moves in this plan.
- **No new dependencies.** The client is `std::os::unix::net::UnixStream`
  and `serde_json`, both in the TCB; the read set is a `BTreeSet<String>`;
  `git status` is one more spawn of the tool discovery already spawns.
- **Panic budget zero, HAR posture.** Index removal is by owned key,
  never by position; the multiset counts saturate; a session that cannot
  establish its state refuses rather than guessing.
- **Budgets rejected, never truncated.** A touched set over `BURST` is
  the cold-start path, said on the record — never a partial re-resolve.
- **Every step lands with its conservatism test**, seen red before the
  fix is claimed, the mutation recorded in the commit message.
- **Evidence grades are load-bearing.** This plan produces no edge; it
  reproduces the survey's edges faster. Byte-identity is the whole claim.
- **Perf stated, never rounded.** Every commit re-measures its row on
  this repo and on yi; a budget that does not hold is stated.
- **Deviations recorded per commit**, one line each, folded into this
  file's Status header when the plan closes.

Sequencing (each step independently shippable, gates green between):

```
1. invalidation      grid_survey::incremental: read sets, name deltas, the pure function, rejections
2. session           grid_survey::Session; watch::Session on top of it                     (needs 1)
3. daemon + client   grid serve holds a Session; check --quick --touched; hook line         (needs 2)
4. differential      seeded edit sequences, every prefix, incl. an unreported edit          (needs 2; asserts 3)
5. surface           README, PORCELAIN, ONTOLOGY, ASSURANCE C47–C49, TODOS, numbers          (needs 3, 4)
```

---

## Part IV — Technical implementation plan

### 1. The invalidation set as a pure function

**Problem.** Nothing records what a file's resolution depended on. The
index is built once, consumed, and dropped; `Resolved` holds edges and
counts. To re-resolve *some* files soundly, the survey must know, per
file, which index answers it consumed — hits and misses — and, per edit,
which answers changed.

**Design.** A new module `crates/survey/src/incremental.rs`.

*Read sets.* `DefIndex` gains a probe sink: `lookup_absolute` and
`lookup` take `&mut ReadSet` (a `BTreeSet<String>`; the full-survey path
passes one too — the cost is a string insert per probe, measured in the
perf row) and record every absolute name they consult, including every
alias hop's intermediate name and every `ambiguous` check. `is_root`
records `root:<name>`. A glob import records `glob:<module prefix>` at
extraction time, so a future glob-following resolver inherits a sound
dependency rather than a silent one. `Resolved` gains `reads: ReadSet`.

*Owned names.* Per file, from its facts: the absolute names of its
definitions (with datums), the alias names its re-exports declare (with
their raw targets), the module it materialises, its `path_mods`,
`mod_cfgs`, and the crate root its path maps to.

*State.* `Prev { files: HashMap<path, FileState { content_hash, module,
owned: Owned, reads: ReadSet, resolved: Resolved }>, roots: BTreeSet
<String>, name_owners: HashMap<abs name, Vec<(path, datum)>> }`. The last
is what makes ambiguity a *derived* fact: a name is ambiguous iff its
owners hold more than one distinct datum, and removing a file's entries
can flip it back.

*The function.*

```rust
pub fn invalidate(prev: &Prev, touched: &[(path, FileFacts)]) -> Result<Plan, Reject>
```

1. Any touched path not in `prev.files` → `Reject::Discovery(path)`.
2. Any touched file whose `path_mods` or `mod_cfgs` differ from before,
   or whose path is a manifest (`Cargo.toml`, `pyproject.toml`) →
   `Reject::WholeTreeInput(what)`.
3. `touched.len() > BURST` → `Reject::Burst(n)`.
4. Compute `delta: BTreeSet<String>` — every name whose answer may have
   changed: the old and new owned names of every touched file (a datum
   that moved is a changed answer; a name added or removed is one); every
   name whose ambiguity flips when the touched files' owners are replaced
   (computed against `name_owners` without mutating it); every alias name
   whose raw target changed. If the crate-root set would change, add
   `root:*` — which meets every read set, i.e. a full re-resolve without
   a rediscovery.
5. `Plan { reresolve: touched ∪ { G : G.reads ∩ delta ≠ ∅ }, delta }`.

Pure: no I/O, no index mutation; the session applies the plan.

**Tests** (`crates/survey/tests/incremental.rs`, unit tests in the
module).
- Ground truth on synthetic `Prev`s (three files, hand-written reads and
  owners): a body edit in F re-resolves F and every reader of F's names;
  a rename inside F re-resolves readers of both the old and the new name;
  adding a definition F previously lacked re-resolves the file that
  *missed* on that name (the miss→hit case); adding a same-named
  definition flips ambiguity and re-resolves the readers of the *other*
  owner; retargeting an alias re-resolves its readers; touching a
  manifest rejects `WholeTreeInput`.
- Conservatism (seen red first): *a touched path the previous state does
  not hold is `Reject::Discovery`, never an empty plan* (mutation: treat
  it as a new file with no readers); *a name that goes from one owner to
  two is in the delta even though the touched file's own edges did not
  change* (mutation: compute the delta from datums only).
- Property (`proptest`, already a workspace dev-dependency; no package
  added): over random small trees (modules, definitions, imports, one
  edit), `reresolve(plan) ∪ kept` equals a full resolution — the read-set
  soundness law, checked on trees the seeded corpus would not think of.
  Also: the plan is monotone in `touched`; an empty delta yields
  `reresolve == touched`.

**Perf.** Recording reads on a full survey of this repo: one `BTreeSet`
insert per probe (≈ 3–5 per reference, ≈ 15k references → ≈ 60k inserts).
Budget: phase 3 +2ms on this repo (6.5 → ≤ 8.5ms), +6ms on yi. State it.

---

### 2. `grid_survey::Session`

**Problem.** `run_sources` is the only entry: discovery in, image out,
everything recomputed. `watch::Session` already holds discovery and the
cache across events and marks the O(tree) phases with a `ponytail:`; the
rest of the state lives nowhere.

**Design.** `pub struct Session` in `crates/survey/src/session.rs`,
holding `sources`, `roots`, `cache`, the `DefIndex` (now removable:
`remove_file(path)` deletes the file's symbols and aliases by owned key
and re-derives ambiguity from `name_owners`), and `Prev`.

- `Session::open(root) -> Result<(Session, Survey)>`: one full
  `run_sources`, recording read sets — the cold start.
- `Session::resurvey(&mut self, touched: &[String]) -> Result<Survey,
  SurveyError>`:
  1. `verify(touched)`: read `.git/HEAD` and the ref it names; if the sha
     moved since the last call → rediscover and re-hash everything (a
     full run through the cache, which still serves unchanged blobs).
     Run `git status --porcelain -z --untracked-files=no`; modified paths
     join `touched`; any `A`/`D`/`R` entry, or any modified path outside
     the held list → the discovery path.
  2. `files::reread` each touched path (content hash + source); a path
     `reread` returns false for → the discovery path.
  3. Extract touched files through the cache (`extract_all` on the
     touched slice).
  4. `incremental::invalidate(&self.prev, &touched_facts)`; on `Reject`,
     record the reason and fall through to a full `run_sources` over the
     held sources (still no rediscovery unless step 1 or 2 asked for it),
     which also rebuilds `Prev`.
  5. Apply the plan: `index.remove_file` / `insert_file` for the touched
     files (aliases re-inserted after definitions, as the full path does);
     re-resolve the plan's files with fresh read sets; replace their
     `FileState`s.
  6. Build: `grid_chart::build` over `index.symbols()` and the union of
     every file's resolved edges. O(tree), linear, 1.0ms / 2.7ms —
     `ponytail:` an incremental builder would have to patch CSR rows in
     place for less than the cost of one `memcpy`; not worth its bugs.
  7. Return `Survey` with three new fields: `incremental: Option
     <Incremental { touched, reresolved, fallback: Option<String> }>`,
     and the counts as today.
- A file that does not parse: the extract yields no facts (today's rule:
  a broken file has a chart); the *session* keeps watch's stricter rule
  at the caller (`watch::Session::unparseable` refuses the event before
  calling `resurvey`), and `grid check` adopts the same refusal (a hook
  firing mid-edit on a half-typed file must not retract every suspect the
  file holds).
- `watch::Session::resurvey` becomes one call into this session; its
  `sources`/`cache` fields move here. The `ponytail:` note about O(tree)
  phases is deleted.

Publish stays where it is (`rpc::Chart::survey_run` publishes the image
the survey returns) and is measured separately: 12ms here, of which the
membership fold is the suspect (the image's O(chain) rewrite carries its
own `ponytail:`). If the row below does not hold because of it, the fold
is made lazy — membership is derived and "safe to delete; the next survey
rebuilds it" (ONTOLOGY), so a publish may defer it and the first
membership *query* pays it. Decided by the measurement, recorded in the
commit.

**Wire.** `SurveyRunParams.touched: Option<Vec<String>>` (additive);
`SurveyRunResult.incremental: Option<IncrementalReport { touched: u64,
reresolved: u64, fallback: Option<String> }>`.

**Porcelain.** The `snapshot` record gains a trailing clause when the
survey was incremental: `snapshot @02f27c2 defs 1813 edges 3419 reread 1
… incremental 1 touched · 4 re-resolved` or `… full (touched set
rejected: discovery)`. The existing `reread` column keeps its meaning.

**Tests** (`crates/survey/tests/session.rs`; `crates/grid/src/watch.rs`'s
existing five).
- Ground truth: open a session on the Python fixture repo; edit
  `pkg/api.py`; `resurvey(["pkg/api.py"])` re-resolves `pkg/api.py` and
  `pkg/use.py` (whose read set holds `pkg.api.window`) and nothing else,
  and its image equals a cold `run_sources` byte for byte.
- Conservatism (seen red first, one mutation each): *a touched file whose
  module moved — `git mv` reported by the new path, or a `#[path]`
  attribute added — takes the full path with `fallback: Some("discovery
  …")`, never a partial re-resolve* (mutation: treat the unknown path as
  new); *a glob import added to the touched file, with a call to a name
  only the glob supplies, yields an image equal to the full survey's and
  a read set that holds the `glob:` prefix* (mutation: drop the prefix
  record — the image still matches today, because the resolver declines
  globs; the assertion on the read set is what keeps a future resolver
  honest); *a session whose held snapshot is not the store's live one —
  another process published in between, or the store is superseded by an
  epoch bump — refuses the incremental path and re-surveys in full*
  (mutation: skip the live-id check); *an edit the session was not told
  about is picked up from `git status`* (mutation: trust `touched`).
- The watch algebra tests (C32) run unchanged on top of the session; the
  cold-start-as-one-event property now also exercises the `Burst`
  rejection when `BURST` is lowered in the test.

**Perf.** `resurvey` of one touched file, in-process, this repo: extract
≈ 0.5ms, verify ≈ 5ms (`git status`), invalidate < 1ms, re-resolve the
affected files ≈ 1ms, build 1.0ms, parse-back 0.2ms — budget **< 10ms
before publish**; yi < 15ms. Publish measured and stated separately.

---

### 3. The daemon holds the session; `check --quick --touched`

**Problem.** The session is worth nothing to a hook unless a process
outlives the hook. `grid serve` outlives it and already owns the survey
cache; it does not own a session, and the CLI does not call it.

**Design.**
- `rpc::Chart` gains `session: Option<grid_survey::Session>`, opened on
  the first `survey_run` with `touched: Some(..)` (a cold start, ~90ms
  once) and consulted on every later one; a `survey_run` without
  `touched` still runs the full path but *through the session* when one
  exists (so the daemon's full and incremental surveys share the cache
  and the index).
- `grid.check` gains `CheckParams.touched: Option<Vec<String>>`, passed
  to the head survey inside `rpc::drift` (`DriftParams.touched`,
  additive) and reported back as `CheckResult.incremental:
  Option<IncrementalReport>`.
- **The client.** `crates/grid/src/client.rs`: if `.grid/grid.sock` (or
  `GRID_SOCKET`) exists and connects, `grid check` serialises its
  `CheckParams` as the JSON-RPC request the daemon already answers, reads
  one response line, and prints it through the same porcelain the
  in-process path uses (the `--json` bytes are identical by construction:
  they are the daemon's). Any connect or protocol failure falls back to
  the in-process path with `warn - daemon at <path> did not answer: <e>;
  full survey`. Only `check` gets a client in this plan; every other verb
  stays in-process (`ponytail:` a `--daemon` switch for the rest is the
  same forty lines once a second verb wants it).
- `--touched <path>` (repeatable; also `--touched -` to read paths from
  stdin, one per line, the shape `grid watch` reads) on `grid check`.
  Without a daemon it is accepted and ignored with the warning in use
  case 2 — never an error, the hook must not fail for lack of a daemon.
- **The hook line.** Claude Code's PostToolUse hook receives the tool
  call as JSON on stdin; for `Edit`/`Write`/`MultiEdit` the path is
  `.tool_input.file_path`. README's script becomes:

  ```sh
  #!/bin/sh
  # PostToolUse: exit 0 silently; on drift, feed the report back (exit 2)
  f=$(jq -r '.tool_input.file_path // empty' | sed "s#^$PWD/##")
  grid check --quick ${f:+--touched "$f"} >/dev/null 2>&1 && exit 0
  grid check --quick ${f:+--touched "$f"} 2>&1 | head -40 >&2
  exit 2
  ```

  `jq` is the user's, as the hook is. `ponytail:` two invocations on the
  failure path, as today; the second is the one whose output matters.
- `grid serve` gains nothing but the session; `--once` still works. A
  `just serve` recipe and one README paragraph on lifecycle.

**Wire (additive).** `CheckParams.touched`, `DriftParams.touched`,
`CheckResult.incremental`, `SurveyRunParams.touched`,
`SurveyRunResult.incremental`.

**Porcelain.** `check`'s existing `snapshot`-less output gains nothing on
the fast path; the two `warn` records above on the slow paths. Version
2.2.0 if the method-calls plan has not already taken it (they compose;
the later one bumps the minor).

**Tests** (`crates/grid/tests/conformance.rs`, `check.rs`).
- Conformance (C10's rule): `grid.check` with `touched` answered over the
  real socket by a daemon started `--once` in the test, byte-identical to
  the in-process answer for the same params minus the `incremental`
  report.
- Ground truth: a daemon up, one edit, `check --quick --touched` reports
  the drift suspect and `incremental.reresolved` equals the read-set
  count from step 2's ground-truth test.
- Conservatism (seen red first): *no socket → the same exit code and the
  same `drift` records as the in-process form, plus exactly one `warn`*
  (mutation: exit 1 on connect failure); *a daemon whose store was
  superseded answers `check` with the refusal the in-process form gives,
  never a stale chart* (mutation: skip the live check in the daemon
  path); *`--touched` naming a file that does not parse refuses the
  check with exit 1 and the reason* (the watch rule, inherited).

**Perf.** The row that matters: `check --quick --touched <f>` through the
daemon, this repo, one edited file — **< 50ms** end to end (CLI process
start ≈ 3ms, connect + one line each way ≈ 1ms, resurvey < 10ms, publish
≈ 12ms, drift < 5ms, compile-record read < 1ms). Against 120–140ms today.
yi: **< 50ms** against ~170ms + drift today. If publish makes the row
miss, step 2's lazy membership fold is the named lever, and the number is
stated either way.

---

### 4. The differential: C13 extended

**Problem.** Step 2's ground-truth test says one edit is right. State
corrupts *across* edits — a stale read set, an ambiguity that flipped
twice, an alias that moved and moved back. The watch plan learned this
and wrote the sequence-integration test; the survey needs the same test
at the level of bytes.

**Design.** `crates/survey/tests/incremental_sequence.rs`: one session,
one seeded sequence over the two-language fixture repo (Python and Rust
files side by side, so both resolvers are exercised), and after **every
prefix** the assertion `session.resurvey(touched).image == run_sources
(fresh sources, empty cache).image` — byte-identical, which C2 makes a
snapshot-id comparison. The sequence, in order, is the list of holes
from Part I plus the rejections:

1. body edit (datum moves, sig holds);
2. sig edit (callers' edges keep their datum endpoints — the src datum
   held — and the dst datum moved);
3. add a definition a sibling file was declining by name (miss → hit);
4. add a same-named definition in another module (ambiguity flip);
   then remove it (flip back);
5. add a `pub use` re-export and a caller through it; retarget it;
6. add an item under a module that shadows an absolute name a reader
   used;
7. add a glob import and a call only the glob supplies;
8. an edit to a file the session is **not** told about (the `git
   status` widening);
9. a `git mv` (discovery rejection, full path);
10. a manifest touch (whole-tree-input rejection);
11. `BURST + 1` files touched at once (burst rejection);
12. revert everything (the group inverse: the final id equals the first).

Each step also asserts the `incremental` report's `fallback` is exactly
what the design says for that step (`None` for 1–8, `discovery` for 9,
`whole-tree input` for 10, `burst` for 11).

**Tests.** The sequence above is the test. It is the only test in this
plan whose failure means "the design is wrong" rather than "a step is
incomplete", and it is the evidence for C47.

**Perf.** Test time only.

---

### 5. Surface

- **README.md.** The hook script above; the daemon paragraph (`grid
  serve &`, `just serve`, what happens without it); the numbers table
  row for `check --quick` rewritten with both forms (daemon / no daemon),
  this repo and yi.
- **PORCELAIN.md.** The `snapshot` record's incremental clause; the two
  `warn` texts; `--touched`.
- **ONTOLOGY.md.** New row `session`: "a resident survey — discovery,
  the parse cache, the index and every file's read set held across
  calls, so a touched file costs its own parse and the resolution of the
  files that read its names. The daemon holds one; `grid watch` holds
  one. A session verifies `touched` against git and refuses to guess
  when discovery moved, a whole-tree input changed, or its held snapshot
  is not the store's live one." `read set` as a second row, and
  `touched` as a third (the hint, never the truth).
- **ASSURANCE.md.** C47: *a session's incremental survey after every
  prefix of an edit sequence is byte-identical to a full survey of the
  same tree, including an edit the session was not told about; a touched
  path discovery does not hold, a whole-tree input, a burst, or a held
  snapshot that is not live each refuse the incremental path and re-
  survey in full, counted and named* → `incremental_sequence.rs
  session.rs`. The next free claim number: *the invalidation set is a pure
function of the
  previous per-file read sets and owned names and the touched files'
  new facts; a file is re-resolved iff a name it read changed its answer,
  hits and misses alike, and a name that gains or loses an owner, flips
  ambiguity, or retargets an alias is such a change* →
  `incremental.rs`. C49: *`grid check` through the daemon answers the
  same bytes as in-process for the same parameters; without a daemon the
  same records and exit code with one warning; a superseded store
  refuses through either path* → `conformance.rs check.rs`. Maintained-
  views table gains a row: "per-file resolution — bilinear (references ⋈
  index) — state: the index and each file's read set". Dependency line:
  "this plan: no new packages."
- **TODOS.md.** T-14 and R-2 closed with the numbers; the cut-line items
  below become T-numbers; the "Recorded ceilings" list drops "the watch
  session's O(tree) phases" and gains the two `ponytail:`s here.
- **docs/research/ANALYSIS.md.** One paragraph under DBSP: the survey
  itself is now a maintained bilinear view, and the read set is the
  partial-path idea specialised to a flat index (the stack-graphs section
  gets the cross-reference).

---

## Verification posture (har-verify ladder, applied)

- Rung 1 everywhere: every step's fixtures are ground-truth pairs and
  every conservatism assertion is a named test seen red first, with the
  mutation recorded in the commit message. Step 1's reds are the two
  wrong invalidation rules; step 2's are the four trusts (a path, a glob,
  a live id, `touched` itself); step 3's are the two silent fallbacks.
- Rung 3: the read-set soundness law under proptest over random trees
  (1); plan monotonicity (1); the watch algebra — sequence integration,
  group inverse, cold start as one event — inherited unchanged (2); the
  daemon and in-process answers agree (3); the final id of the sequence
  equals the first (4).
- Rung 5 (differential): **every prefix of the seeded sequence against a
  cold full survey, byte for byte** (4) — the theorem `Q^Δ = D∘Q∘I`
  checked at the level of segment bytes, which is C13's own statement;
  the daemon's bytes against in-process bytes over the real socket (3).
- Fuzz/miri/loom/kani: nothing here parses untrusted bytes (the client
  reads the daemon's JSON through `serde_json` into the wire structs, the
  same trust posture as every other wire consumer), owns `unsafe`, or
  adds concurrency beyond the daemon's existing serial loop.

## Perf ledger

Budgets, to be measured and stated in each commit (Apple M4 Max, release,
this repo at 1,813 defs / 3,419 edges / 114 files; yi at 4,312 defs / 384
files). Today's numbers measured 2026-09-01 in-process; the composite rows
are wall time of the CLI.

| surface | today | budget | why it should hold |
|---|---|---|---|
| phase 3 with read-set recording, full survey | 6.5ms / 20.5ms | ≤ 8.5ms / ≤ 26.5ms | one set insert per probe |
| `Session::resurvey`, one touched file, before publish | — (13ms of O(tree) phases + 0.5ms parse) | < 10ms / < 15ms | extract one file; re-resolve the read-set hits; a linear build |
| publish (segment + membership fold) | 12ms / 15ms | stated; lazy fold if it is the reason a composite row misses | derived data may be deferred |
| `grid check --quick --touched f`, daemon up | — (no such path) | **< 50ms** / **< 50ms** | resurvey + publish + drift < 5ms + one socket round trip |
| `grid check --quick`, no daemon | 120–140ms / ~430ms† | no regression: ≤ today | the in-process path is untouched |
| `grid watch`, one file event | ~46ms (agent-loop ledger; 13ms today's phases 2/3 + parse + drift) | < 20ms | the same session |
| `grid survey`, full, this repo | 90ms wall | within noise: ≤ 95ms | read sets are the only addition |

† yi's `check --quick` number includes a base rev survey the store did not
hold; the steady-state number is stated when measured.

## Cut line

Recorded so each is a decision, not an omission:

- **An incremental segment builder.** Build is 1.0ms / 2.7ms and linear;
  patching CSR rows in place is more code than the whole session and
  saves less than a millisecond. `ponytail:` in step 2.
- **An incremental membership fold.** Membership is derived and
  rebuildable; if the fold is why a row misses, it is deferred, not
  incrementalised. The packed-index idea (T-23) is unrelated.
- **An OS file watcher.** A dependency decision; `fswatch . | grid watch`
  and the hook's `--touched` already reach the session. Stays out.
- **Daemon auto-start.** One flag when the manual form measures
  annoying; a hook that spawns daemons is policy this plan does not set.
- **A client for every verb.** Forty lines per verb once a second verb
  wants it; `check` is the one on the hook.
- **SCC-shared closures.** A different problem (closure walks, not
  surveys); stays in ANALYSIS.md's chart/cache item.
- **Stack-graph partial paths.** The TS frontend's resolver; for the flat
  index the read set is the same object.
- **Windows** (T-16). The daemon is `std::os::unix::net`; the in-process
  fallback is what Windows gets, and README says so.
- **Cross-worktree sessions.** One session per root per daemon; `grid
  interfere` opens the other store read-only as today.
