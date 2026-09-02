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
(~1000 defs): ~38ms.

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

An agent's loop is locate → read → edit → verify. Grid serves each step
with exact, machine-checkable answers instead of grep guesses:

- Locate: `grid resolve`, `grid uses`, `grid scope` return callsigns,
  datums, and `file:line` — `--json` on every verb for the wire result.
- Read less: `grid scope X` is the context pack around an edit — the
  definition, its proven callers and callees, the rules that reach it.
- Edit safely: `grid edit apply` is compare-and-swap by identity. The edit
  names the datum it read; if the definition moved since, the edit is a
  typed conflict, never a splice against wrong bytes.
- Verify: `grid gate` checks the change against the promise; `grid cache`
  keys test verdicts by identity closure, so unchanged code needs no rerun.
- Cheap enough to call every turn: ~38ms per survey, idempotent by
  construction (unchanged tree → same snapshot id, no write).

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
grid survey                                  # once per turn; ~38ms
grid plan lint plan.json                     # promise the delta up front
grid scope Reaper.sweep --depth 2            # context pack before editing
grid edit apply edits.json                   # CAS by datum; conflicts are typed
grid cache lookup test $ENV $(grid ...)      # what actually needs rerunning
grid gate plan.json                          # did the change match the promise
```

Rules of the road: trust the refusals — an empty answer means "cannot
prove", not "does not exist"; use `name@datumprefix` handles to pin
identity across turns; on `conflict`, re-read the `now` coordinate and
retry the whole request. Coverage closes the loop with no tracer:
`grid trace ingest --lcov cov.lcov --test <name>`, then `grid suspects`.

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

## Forge

This repository lives on the estate's Forgejo: `git.example.invalid/apex/grid`
(clone and push over SSH, `ssh://git@forge.example.invalid:2222/apex/grid.git`).
Pull requests open and merge there, and `.forgejo/workflows/` is the gate that
has to be green: `main` takes merges only through a pull request whose checks
passed. GitHub (`github.com/jackmaxwil/grid`) is a mirror of the forge and
carries no CI; nothing pushes to it from a keyboard. Sessions drive pull
requests with `tea`, the Gitea CLI, as described in yi's `docs/FORGE.md` under
"Working the forge from a session".
