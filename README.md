# grid

What your code promises vs what it does.

Grid surveys a repository into a chart: every definition with a
content-derived identity (a *datum* — blake3 of its normalized AST), every
relationship it can *prove* (calls, constructs, implements, writes_field),
and the evidence grade of each claim. Queries, diffs, gates, and fault
localization all run over that chart, in identity space, not text space.

Languages: Rust and Python today. TS/JS next (`swc`-based frontend planned).
One binary, no daemon required, no build integration, no config. Discovery
is `git ls-files` minus your ignore rules — vendored code behind a
`.gitignore` line is never charted, committed or not. Survey of this repo
(~1700 defs): ~80–120ms release (81ms warm per the most recent measurement,
up to ~120ms cold) — not the ~38ms this file used to say; the number grew
as the declined-reference split, method-call refs, and per-def cfg facts
landed on top of the walk.

## Why a developer wants it

- `grid` — what actually changed, in definitions and edges, not lines.
  Renames print as `def > new (was old)`, because the identity held.
- `grid uses X` — who reaches X, cross-crate, through re-exports, with
  proof. `--relation implements` answers "who implements this trait".
- `grid resolve X` — where X is, `file:line`, pasteable. Works backwards
  too: `grid resolve src/lib.rs:145` names the enclosing definition.
- `grid orphans` — definitions with no proven caller. Deletion candidates,
  labeled honestly: no proven caller is not the same as dead.
- `grid hotspots` — complexity × edit-churn, descending. What to refactor
  first. A ranking, never a gate; clippy owns lints.
- `grid log X` — a definition's history in identity space. Renames do not
  break the chain.
- `grid gate <plan>` — CI checks the diff against a promised delta. Exit 3
  when unpromised work appeared or promised work is missing.
- `grid suspects <test>` — likely culprits, ranked from recorded test
  outcomes over the retained chain.

Every claim carries its evidence grade (`proven > observed > modeled >
heuristic`). Grid never guesses: a name the language's own binding rules
do not establish resolves to nothing, and the refusal is counted and visible.

## Why an agent wants it

An agent's loop is locate → edit → verify. Grid serves each step with an
exact, machine-checkable answer instead of a grep guess, and the surface
collapses to three verbs plus two hooks:

- **Locate** — `grid scope <pattern> --bodies N --sigs M` is the context
  pack before an edit: the definition's body, its proven neighbourhood out
  to M hops as signatures, and a `horizon` line stating exactly how far
  each tier is complete to. A budget that does not fit drops a whole hop,
  never truncates a body — so the horizon can be believed.
- **Edit** — `grid edit apply` is compare-and-swap by identity. The edit
  names the datum it read; if the definition moved since, the edit is a
  typed conflict, never a splice against wrong bytes.
- **Verify** — `grid test` runs what the change can reach and replays the
  rest from the verdict cache; `grid check [base] [head] [--plan f]
  [--quick]` is the one call and one exit code for everything else grid
  can find about the change — drift, uncompiled callers, plan adherence,
  stale and weak coverage, clone-of, deserialize compatibility — gated
  findings first, a ranked tail that never moves the exit.

Two hooks turn that into standing configuration (documented, not shipped
as code — Claude Code hooks are the user's own):

- PostToolUse → `grid check --quick` (drift plus the last compile record)
- pre-commit → `grid check`
- CI → `grid test --trace --all`, which is what refreshes the traces
  local selection reads

A compiler owns types: on a Rust-only repo, `cargo check` already catches
a proven caller of a moved signature, and Rust test time is mostly compile
time. What grid adds there is narrower than "game-changing" and is true —
test selection at the binary level (skipping links, where Rust's test time
goes), callers a compile skipped (`#[cfg(test)]`, feature gates),
attribution without rerunning, semantic interference between worktrees,
deserialize compatibility across snapshots, and plan adherence. Where
there is no compiler (Python, later TS), the graph carries the weight a
type checker would — per-definition test selection, unused code,
attribution, and the observed-edge fill all matter more.

Cheap enough to call every turn: idempotent by construction (unchanged
tree → same snapshot id, no write). `grid check --quick` on this repo
measures ~141ms and the full form ~204ms — both dominated by the worktree
survey a PostToolUse hook must re-run to see the edit it fires on, stated
honestly rather than rounded down to the original design ceiling.

## Build

Rust 1.85+ (edition 2024 workspace). No C, no protoc, no submodules.

```bash
cargo build --release
```

Gates before any commit: `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets` (pedantic, deny), `cargo test --workspace`,
`cargo deny check`. `just check` bundles them; `just fuzz`, `just miri`,
`just soak` for the deeper rungs.

## Install

```bash
cargo install --path crates/grid
```

Or copy `target/release/grid` onto PATH. Single static binary. Per-repo
state lives in `.grid/`: a content-addressed segment store, the patch log,
retained traces, the verdict cache. All caches — safe to delete, rebuilt by
the next survey — so gitignore the directory, with two exceptions:

```gitignore
.grid/
!.grid/invariants.json
!.grid/covenants.json
```

Those two are *declarations*, not caches. A standing invariant and a
covenant are things a human decided about this repo; deleting them loses
information no survey can rebuild, and they should travel with the code that
they are about.

## Use

```bash
grid survey                 # chart the worktree; idempotent
grid                        # what changed vs the promise (the everyday verb)
grid uses TagContract       # proven inbound edges, cross-language
grid resolve validate       # unique-suffix lookup -> callsign, datum, file:line
grid check --quick          # drift plus the last compile record, one exit code
grid test                   # run what the change can reach; replay the rest
grid explain grade          # any word or id in grid output, explained
```

A dozen more verbs behind those; `grid help` lists every one, `grid
explain <anything>` defines any word in the output, and `grid roots`
prints the import-name -> directory map callsigns are rooted in.
Porcelain records on stdout are stable and column-aligned
(docs/PORCELAIN.md); hints and banners go to stderr; `--json` emits the
wire result verbatim. Exit codes are uniform: 0 pass, 1 internal, 2 bad
input, 3 divergence, 4 stale coordinate. A name that resolves to nothing
is exit 2 with the closest callsigns on stderr — silence is never an
answer.

## How agents should use it

Shell out; parse `--json` or the porcelain columns. The socket
(`grid serve`) speaks the same wire contract for long-lived sessions.

The intended loop:

```bash
grid survey                                  # once per turn; ~80-120ms release
grid plan lint plan.json                     # promise the delta up front
grid scope Reaper.sweep --bodies 1 --sigs 2  # context pack before editing, with its horizon
grid edit apply edits.json                   # CAS by datum; conflicts are typed
grid cache lookup test $ENV $(grid ...)      # what actually needs rerunning
grid gate plan.json                          # did the change match the promise
grid check --plan plan.json                  # or the one composite call above, gate-first
grid test                                    # run what the change can reach; replay the rest
```

Rules of the road: trust the refusals — an empty answer means "cannot
prove", not "does not exist"; use `name@datumprefix` handles to pin
identity across turns; on `conflict`, re-read the `now` coordinate and
retry the whole request.

### Coverage closes the loop, with no tracer

Grid instruments nothing. The runner produces a coverage report, grid maps
its executed lines onto definitions, and the result is one `observed` edge
per definition the test reached — which is what `grid suspects` ranks over,
and what selection and attribution read. One report is one test, so the
recipes below produce one report per test.

Rust, per test (`cargo llvm-cov` is your toolchain, not grid's):

```bash
# per test name $t, from `cargo nextest list`
cargo llvm-cov nextest --lcov --output-path cov.lcov -E "test(=$t)"
grid trace ingest --lcov cov.lcov --test "$t"
```

One export per test is not an oversight: a single profile cannot be
attributed back to tests that shared a process, and nextest's process per
test is what makes the loop correct at all.

Python, one run and then one report per test — the contexts are already in
coverage.py's data file, so nothing reruns:

```bash
pytest --cov --cov-context=test
# per test node id $t
coverage json --contexts="$t" -o cov.json
grid trace ingest --coverage-json cov.json --test "$t"
```

Either way the trace records the snapshot it was mapped against, and every
executed line grid could not place — a file the chart does not hold, a line
inside no definition — is counted on stderr rather than dropped.

### `grid test`: run what the change can reach, replay the rest

```bash
grid test                    # since the last run (else HEAD), against the worktree
grid test --trace            # and refresh one trace per test that ran
grid test --all              # CI: run the inventory, record everything
grid test -- --profile ci    # runner args, folded into the environment hash
```

The inventory is the runner's — `cargo nextest list` (or `cargo test --
--list` without nextest) and `pytest --collect-only` — never grid's guess.
Selection is one inbound walk from every definition whose datum moved,
over every grade plus the retained traces' observed edges; a test the walk
reaches runs, a test no live trace ever watched runs regardless, a test the
chart cannot name runs, and a walk that runs out of fuel runs everything and
says why. What is left is replayed from the verdict cache under this
environment. Rust selections roll up to `(package, binary)` and nextest is
invoked once, so unselected binaries are neither compiled nor linked.

```
hit     grid-cache::closure keys_are_deterministic  pass  2026-09-01T10:02:11Z
miss    grid::bin/grid drift::tests::a_walk_over_budget_refuses  closure-changed
miss    grid::conformance test_plan_is_served_over_the_socket    no-trace
test    ok         grid::bin/grid drift::tests::a_walk_over_budget_refuses  pass=pass
warn    grid.drift.fold_rows  uncovered
done    tests 2/33 ran · 31 replayed · 0 failed
```

Exit 3 when a run test failed, 1 when the runner could not be spawned, 0
otherwise. `uncovered` (a changed definition no selected test reaches) and
`nondeterministic` (same closure key, same environment, opposite verdict —
the claim is "not the charted code", never "flake") are reported and never
move the exit code. `--trace` needs `cargo llvm-cov` or `pytest-cov`; when
they are absent it says so and runs untraced.

### `grid check`: everything the change did that nobody checked

```bash
grid check                          # since the last recorded base, against the worktree
grid check --quick                  # drift plus the last compile record only — no trace, verdict or source read
grid check --plan plan.json         # plus the gate verdict against the promised delta
grid check --plan plan.json --why   # plus the fulfilling paths and open anchors
grid check --compile                # a mode of the same verb: wrap cargo check, record what it did not compile
```

One call, one exit code, for every finding grid can make about a change.
Any of these gates the exit to 3: drift suspects; the compile record's
uncompiled callers (when `.grid/compile.json` names the snapshot this
check surveyed); a failed gate verdict under `--plan`; stale verdicts; a
fired covenant; a compat finding on a type marked `persisted`. A ranked
tail never moves the exit: weak-for, uncovered, clone-of, undeclared
compat. When nothing gates:

```
done  refactor  interfaces held · 12 covering tests replayed pass · 0 changed defs uncovered
```

`--quick` is the proven floor plus the last compile record — no trace
store, verdict log, or source read — built for a PostToolUse hook that
fires on every edit. On this repo `grid check --quick` measures ~141ms and
the full form ~204ms, both dominated by the worktree survey the check must
re-run to see the edit; the original < 50ms / < 200ms design ceilings do
not hold at that cost, and the numbers are stated rather than rounded down
to them.

## Architecture

Ten crates, one direction, no cycles. `docs/ONTOLOGY.md` is the
vocabulary, `docs/ASSURANCE.md` the dependency ledger and claims,
`docs/PORCELAIN.md` the output contract, `docs/LANG_RS.md` the Rust
frontend's design and its paid-for lessons.

| crate | owns |
|---|---|
| `grid-wire` | every serialized shape; serde only |
| `grid-datum` | identity vocabulary: Datum, shape, Definition, refs — parser-free |
| `grid-lang-py` / `grid-lang-rs` | frontends: parse → normalize → extract (rustpython / syn) |
| `grid-survey` | tree → defs + proven edges; crate roots, re-export aliases, per-language resolvers |
| `grid-chart` | the segment: a validated, mmap-able CSR graph image; delta with rename detection |
| `grid-chart-io` | the store: atomic publish, crash-safe by kill-9 soak, content-addressed |
| `grid-cache` | verdicts keyed by identity closure |
| `grid-trace` | runtime traces → observed edges |
| `grid-walk` | budgeted multi-hop query walks |
| `grid-gate` | plan vs diff: the gate verdict |
| `grid` | CLI + daemon; the only composition root |

Load-bearing invariants:

- Determinism: same tree → byte-identical segments → same snapshot id,
  any machine, any order. Incremental ≡ full, tested against the answer.
- Identity: the datum ignores formatting and comments, keeps the name (the
  chart's primary key must not collapse same-body defs); the *shape*
  erases the name and is the rename-join key. Normalization changes are
  declared epoch bumps, never silent re-hashes.
- Honesty: an edge exists only where the language's binding rules establish
  it.
  Glob imports, macro bodies, typed receivers: omissions, never guesses.
  Everything declined is counted and reported.
- Formats: segment epoch 2 is current; epoch-1 images parse forever,
  pinned by committed fixture. Wire evolution is additive-only.

## Contributing

- Read `docs/ONTOLOGY.md` first; a word means one thing everywhere.
- Every test defends one externally observable contract, and conservatism
  tests (the feature *declines* what it cannot prove) ship with the
  feature. A regression test is seen red before the repair is claimed.
- No new dependencies without an ASSURANCE.md ledger entry; parsers for
  config-shaped inputs are line scans with named ceilings, not new crates.
- `#![forbid(unsafe_code)]` everywhere except the one audited mmap block;
  clippy pedantic at deny; no unwrap outside tests.
- Schema changes: wire additive-only; segment format changes are epoch
  bumps with a forever-parsed fixture for every old epoch.
- Perf budget: `grid survey` on this repo stays under 50ms release, and a
  PR that moves it says so with numbers.
- Commit messages: imperative subject, body says why, gates green first.
