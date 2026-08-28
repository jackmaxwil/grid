# Verb completions: materialize, remember, observe

Status: planned, not started. Scope: the eight P0–P2 items from the
2026-08-27 verb-coverage review, ordered here by dependency and cost, not
priority label. Authority: ONTOLOGY.md for vocabulary, ASSURANCE.md for
dependency discipline, LANG_RS.md for the Rust frontend's ceilings.

Standing rules for every item:

- **Additive schema evolution only.** New wire fields are `Option` +
  `skip_serializing_if`; new enum variants extend, never repurpose. The
  segment format is the one versioned binary: epoch bumps are declared in
  this plan, and `Segment::parse` keeps reading every epoch ever shipped.
- **No new dependencies.** Every parser this plan needs (Cargo.toml package
  names, lcov records) is a line-oriented string scan; std only. Each
  deliberate under-parse is a named ceiling, `ponytail:`-marked in code.
- **Perf budget holds.** `grid survey` on this repo stays under 50ms
  release; each phase that grows gets re-measured in its PR and the number
  goes in the commit message.
- **Panic budget zero, HAR posture.** New index/offset math is
  `get`-guarded or saturating; new ids are newtypes; no `unwrap` outside
  tests. Wire-facing enums keep an escape hatch only where the wire
  already has one.
- **Every step lands with its conservatism test** — the assertion that the
  feature *declines* what it cannot prove, in the shape of
  `lang-rs/tests/refs.rs`.

Sequencing (each step independently shippable, gates green between):

```
1. crate roots      (small, unlocks cross-crate everything)
2. rename detection (small, pure join)
3. materialization  (segment epoch 2 — the load-bearing middle)
4. orphans + uses --relation (small, reads what 3 wrote)
5. lcov observed tier        (medium, needs 3)
6. field edges               (small, frontend-only)
7. legs, grid log, bisect    (large, needs nothing above but pays last)
```

---

## 1. Cargo.toml-aware crate roots

**Problem.** `module_of_rs` roots a module at its directory name; packages
named `grid-*` in dirs named `chart`, `lang-py` mean every cross-crate path
(`grid_chart::build`) resolves to nothing. Grid's own chart is intra-crate
only.

**Design.** A `CrateRoots` map built once per survey from the same file
list the survey already walks:

```rust
/// Directory prefix -> the crate name `use` spells (hyphens underscored).
pub struct CrateRoots(Vec<(String, String)>);   // sorted, longest-first
impl CrateRoots {
    pub fn from_manifests(manifests: &[(String, String)]) -> Self { .. }
    fn root_of<'a>(&'a self, path: &str) -> Option<&'a str> { .. }
}
```

- `files::sources` / `sources_at` already list tracked paths; every
  `Cargo.toml` (worktree: `fs::read`; rev: `git show`, batched into the
  one listing pass) is scanned for the first `name = "…"` line after a
  `[package]` line — a string scan, not a TOML parser. Ceiling, marked in
  code: a `name` key split across lines or spelled via workspace
  inheritance falls back to the directory name, costing an unresolved
  edge, never a false one.
- `module_of` gains the map: `module_of(lang, path, roots)`. Callers
  (survey ×2, edit ×3) all have a survey in hand. The Python arm ignores
  it.
- Rust rule becomes: nearest ancestor manifest's crate name is the root
  segment; everything after that manifest's `src/` is the dotted tail;
  no manifest on the path keeps today's dir-name rule.

**Determinism.** The map is a pure function of tracked file contents, so
worktree and rev surveys stay reproducible; `sources_at` reads manifests
out of the object store like any other blob.

**Tests.** `survey/tests/rust_edges.rs` grows: a two-crate repo where
`crates/app-core/` declares `name = "app-core"` and `crates/cli/` calls
`app_core::helper()` — the cross-crate edge is the assertion. Corpus
acceptance: `grid uses grid_chart.build` on this repo answers with the
survey/rpc callers; the LANG_RS.md "intra-crate only" amendment is deleted
in the same change.

**Ceilings.** `#[path]`, `[lib] name`, multi-target manifests: directory
fallback. Add when a real repo pays the omission.

---

## 2. Rename detection in the delta

**Problem.** A rename prints as `- old` + `+ new`. The ontology defines a
rename as callsign rebound, datum held — the delta should say so.

**Design.** In `rpc::delta` (where `DefChange` rows are built), after
added/removed sets are known: join removed × added on datum. Every pair
with equal datum and unequal callsign collapses into one row:

- Wire, additive: `DefChangeAction::Moved` (`"moved"`), plus
  `moved_from: Option<Callsign>` on `DefChange` — the same shape edges
  already use.
- A datum present on both sides more than once (two identical bodies
  renamed at once) is ambiguous: leave those as -/+ rather than guess a
  pairing. Refusal over invention, same as `DefIndex`.
- Porcelain: glyph `>` with the old name in the context column, mirroring
  the edge `moved_from` rendering:
  `def     >  survey.extract_all  (was survey.collect_facts)  @6e05`.
- xte/gate: `TrackDefPromiseAction::Move` (`"move"`, additive) so a rename
  can be promised; an unpromised Moved row is one unpromised change, not
  an unpromised add *and* an unpromised remove.

**Tests.** Delta fixture: rename a def between two surveyed states, assert
one `moved` row and zero added/removed rows; the ambiguous-pair case
asserts the -/+ fallback. Gate test: a promised `move` passes, an
unpromised one holds the gate with a single xte line.

---

## 3. Materialization — file:line in every answer (segment epoch 2)

**Problem.** `resolve`/`uses`/`scope` name definitions but cannot take
anyone to them: `materialization: None` everywhere, `Locator::FileLine`
is a `false` stub, segments store no spans. Frontends produce byte spans
and the survey drops them at the chart door. The agent loop is
locate → read → edit; grid does only locate. Item 5 also needs this.

**Design.**

*Segment format*, `FORMAT_EPOCH: 2`, three new sections appended to the
section table:

| section | contents |
|---|---|
| `PathOffsets` | `path_count + 1` u32 byte starts into `PathBytes` |
| `PathBytes` | UTF-8 repo-relative paths, concatenated, interned |
| `DefLocs` | per def: `(path: u32, line: u32, byte_start: u32, byte_end: u32)` |
| `DefMetrics` | per def: `(complexity: u16, reserved: u16)` |

`DefMetrics` rides this epoch bump because format bumps are rare and a
per-def column is free now versus a third epoch later. Complexity is
cyclomatic (branch-count: `if`, `match` arms beyond the first, loops,
`&&`/`||`, `?`), computed by each frontend at parse time. It is a pure
function of the normalized AST — the same invariance the datum has — so
it is datum metadata, cached by identity forever. `reserved` is zero and
ignored on read, so the next metric is a semantic change, not a format
one.

- Line is computed at survey time from the source the survey already
  holds (`memchr`-free manual newline count over the span head — std).
  Both line and byte span are stored: porcelain wants lines, item 5's
  lcov mapping wants lines, `grid edit` wants bytes.
- `Segment::parse` accepts epoch 1 (locations answer `None`) and epoch 2;
  `build` emits epoch 2 only. A fixture segment at epoch 1 is committed
  and parsed forever. Content addressing means every snapshot id moves on
  the first epoch-2 survey — expected, documented in the commit.
- Accessor: `segment.location(ord) -> Option<DefLoc>`; newtype
  `DefLoc { path: &str, line: u32, span: Range<u32> }`.

*Plumbing*: `grid_chart::Def` gains `loc: Option<(String, u32, u32, u32)>`
(survey fills it from `Definition.span` + the file it came from);
`chart_provider::resolve` fills `DefRefMaterialization { file, line }`
(the wire type has been waiting); `Locator::FileLine` becomes a real arm:
containment test over `DefLocs`, innermost (shortest span) def wins.
Porcelain `ref` rows grow a trailing `file:line` column.

**Tests.** Contract per consumer: `grid resolve survey.extract --json`
carries `{file: "crates/survey/src/lib.rs", line: N}` and N is asserted
against the real repo (corpus test, tolerant only to the file's own
edits); `grid resolve crates/survey/src/lib.rs:140` (FileLine porcelain)
answers the enclosing def; an epoch-1 fixture still parses with `None`
locations; determinism suite re-runs unchanged (locations are content).

**Ceilings.** No column tracking (line is enough for editors and lcov);
macro-generated defs have no span and answer `None` honestly.

---

## 4. `grid orphans` + `grid uses --relation`

**`orphans`** — the deletion tool. One scan of the live epoch: every def
whose in-edge row is empty across all relations. Porcelain:

```
orphan  survey.files.module_of_rs  @1df9  crates/survey/src/files.rs:109
```

Header line states the evidence honestly: *no proven caller* — pub API,
tests, and macro-reached code all look like this. Never the word "dead".
`--lang rs` filter; exit 0 with zero rows is an answer. ~40 lines in
main.rs + one chart_provider walk; lands after item 3 so rows carry
file:line (the whole point: pasteable into an editor).

**`uses <name> --relation <r>`** — filter the existing inbound-edge walk
by `Relation`'s wire names (`implements` answers "who implements this
trait"). Parse via the wire enum's serde names; unknown relation is a
BAD_INPUT hint listing the vocabulary. Flag, not verb.

**Tests.** Orphans: corpus assertion that a freshly added never-called fn
appears and `survey.run` (called by rpc) does not; the header wording is
pinned (it is the safety feature). Uses-filter: the `impl Frob for Foo`
fixture answers `Foo` for `--relation implements` and nothing for
`--relation constructs`.

---

## 5. Observed tier for Rust: lcov ingestion, no tracer

**Problem.** `wake`/`suspects`/`review --wake`/`bisect` need runtime
evidence; only Python traces exist. Writing a Rust tracer is a project.
Coverage already exists: `cargo llvm-cov --lcov` per test run.

**Design.** New ingest path: `grid wake ingest --lcov <file> --test <name>`.

- lcov is line-oriented: `SF:<path>` opens a file record, `DA:<line>,<n>`
  marks an executed line. The parser is a fold over lines; std only.
  Unknown record types are skipped by design (lcov has many; we read two).
- Mapping: executed line → `Locator::FileLine` containment over the live
  epoch's `DefLocs` (item 3) → datum. Executed lines that land in no def
  (module level, macro output) are dropped — omission, counted and
  reported in the ingest summary.
- Wire fit, no schema change: `WakeObservation { caller:
  Symbol("test:<name>"), callee: Datum(d) }` — "this test reached this
  datum", observed grade. One wake per test invocation; per-test
  granularity comes from running suites test-by-test (nextest or
  `--test-threads` batching is the caller's business, not grid's).
- Suspects/Ochiai then works on Rust unchanged: it consumes touch sets.

**Tests.** A committed lcov fixture (captured from a real
`cargo llvm-cov` run over a two-fn crate, per the external-ground-truth
doctrine) ingests to exactly the expected datum set; a line outside every
def is asserted *absent* and counted in the summary. End-to-end: run a
seeded failing test's lcov + a passing one's, assert `grid suspects`
ranks the touched-only-by-failing datum first.

**Ceilings.** Branch/region coverage ignored (line grade is enough for
touch sets); inlined-away code under-reports (lcov's ceiling, inherited
and named in the ingest summary).

---

## 6. Field edges — the provable half

**Problem.** `reads_field`/`writes_field` sit in the wire `Relation`
vocabulary, never emitted.

**Design.** Frontend-only, no schema change. lang-rs emits, for a struct
literal `Foo { x: 1, y }`: one reference of a new internal kind
`RefKind::FieldWrite` with the type's path; the resolver maps it to an
edge `src -writes_field-> Foo` (dst is the *type's* datum — the wire edge
carries no field name, and inventing a per-field def would be a schema
change this plan does not spend). Functional-update syntax
(`Foo { x, ..old }`) emits the same single edge. Field *reads* need
receiver types: declined entirely, same doctrine as method calls.
lang-py: attribute assignment `self.x = …` at `__init__` scope is the
analogous provable half; deferred until a consumer asks (ponytail: the
Rust half has a consumer today — `grid uses Foo --relation writes_field`
—, the Python half does not).

**Tests.** Conservatism pair: `Foo { x: 1 }` yields the writes_field
edge; `foo.x = 1` (typed receiver) yields nothing. Corpus: `grid uses
survey.FileFacts --relation writes_field` answers `survey.extract`.

---

## 7. Legs, `grid log`, and a bisect that opens

**Problem.** `bisect` fails closed: grid retains snapshots, not an
ordered history. The ontology already names the missing nouns: `leg` (one
edit edge, was→now) and `log` (the patch DAG). `grid log X` — a datum's
history, blame in identity space — falls out of the same store.

**Design.**

- *Storage*: `.grid/log` — append-only JSONL of wire `Leg` rows
  (`from`/`to` datums, `git_rev`, `at`), fsynced on append with the same
  write-then-rename discipline as the manifest. JSONL because legs are
  wire types already and the log is read start-to-end; a CSR log is
  premature until a real log is slow.
- *Writers*, two: `grid edit apply` records one leg per `Modified` detail
  (it already computes was→now); `survey_run` records legs derived from
  the delta against the previous live snapshot — same-datum joins from
  item 2 give `from`, modified rows give both ends. Edits made outside
  grid are thus still captured at the next survey, coarser (one leg per
  survey interval, not per edit).
- *`grid log <name>`*: resolve the name, walk the log backwards
  collecting legs whose `to` (then transitively `from`) chains to it:
  `leg  @6e05 -> @3243  @1177722  2026-08-27` rows, newest first.
- *Bisect*: `bisect.rs`'s pure search is already written and seeded-
  history-tested. Feed it the leg chain for the suspect datum plus cache
  verdicts (`grid cache record test …` outcomes keyed by datum, which
  exist). The wire method stops failing closed when the chain for the
  queried datum has ≥ 2 entries with at least one recorded verdict at
  each end. Insufficient history keeps today's exact error — fail closed
  stays the contract, the condition just becomes satisfiable.

**Tests.** Log append survives kill-9 (extend the crash_safety harness to
the log file — same temp+rename invariant); `grid log` over a seeded
three-edit chain prints three legs in order; bisect over that chain with
a verdict flip localizes the culprit leg; a datum with no history keeps
the fail-closed error verbatim.

**Ceilings.** No cross-rename identity chase in `log` v1 (a rename is a
new callsign, same datum — the chain follows datums, so it survives
renames for free; what it does not follow is a *body* edit chain merged
across branches — single-writer linear history assumed, named in the
verb's help).

---

## 8. `grid hotspots` — complexity × churn

**Problem.** Complexity alone is noise (a flat match table scores high
and is fine); complexity times edit-frequency is the empirically backed
"refactor this first" signal. Grid is uniquely placed: its churn is in
identity space — legs chain datums, so the count survives renames.

**Design.** One read-only walk: for every live def, `score =
complexity × (1 + legs touching its datum lineage)`. Porcelain, sorted
descending, file:line from item 3:

```
hot     survey.run_sources  cx 14  legs 6  crates/survey/src/lib.rs:243
```

Until item 7 lands, `legs` degrades to "changed in the current worktree
diff" (0/1) and the header says so. No thresholds, no gate, no exit-code
opinion: clippy owns lints; grid reports, and the header names the
evidence grade (heuristic). `--limit N` defaults to 20.

**Tests.** Complexity function pinned per frontend with hand-counted
fixtures (a straight-line fn is 1; each branch kind adds exactly one —
the fixture comments show the count); datum invariance asserted
(reformat + rename locals, same complexity); hotspot ordering over a
seeded two-def repo where the flat-but-big def loses to the
branchy-and-edited one.

**Ceilings.** Cyclomatic, not cognitive complexity (no nesting weight):
simpler, language-uniform, and the churn factor does the prioritizing
work. Revisit only if hotspot rankings mislead in practice.

## Acceptance for the whole plan

- `grid uses grid_chart.build` answers cross-crate callers on this repo.
- `grid resolve survey.extract` prints `crates/survey/src/lib.rs:<line>`.
- A rename in the working tree shows one `>` row in bare `grid`.
- `grid orphans` on this repo returns a reviewed, plausible list.
- `cargo llvm-cov --lcov | grid wake ingest --lcov -` then
  `grid suspects <failing-test>` ranks datums on Rust.
- `grid log survey.extract` shows the perf-pass edit chain.
- `grid hotspots` ranks a branchy, frequently edited def above a large
  flat one, and says which evidence tier the ranking used.
- `grid survey` still under 50ms; `just check`, deny, crash soak green at
  every step.
