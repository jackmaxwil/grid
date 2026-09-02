# grid

What your code promises vs what it does — for the agent editing it.

Grid surveys a repository into a chart: every definition with a
content-derived identity (a *datum*, blake3 of its normalized AST; a *sig*
for what callers can see), every relationship it can prove, the evidence
grade of each claim, and the count of everything it could not bind. Diffs,
drift, test selection, gates, attribution and context packs run over that
chart in identity space, not text space, and every answer states its
horizon and its refusals.

Languages: Rust and Python. TS/JS next. One static binary; no daemon
required, no build integration, no config beyond two optional declaration
files. Discovery is `git ls-files` minus your ignore rules.

## Three verbs and one hook

```
grid scope <target> --bodies 1 --sigs 3 --budget 3000   # context, with the depth it is complete to
grid check --quick                                      # after every edit: drift + uncompiled callers, exit 3 stops you
grid test                                               # run what the change can reach, replay the rest
grid check                                              # before commit: everything the change did that nobody checked
```

The PostToolUse hook that makes `check --quick` a reflex (Claude Code
hooks are your configuration, not grid's; this is the whole script):

```bash
#!/bin/sh
# silent on pass; on a finding, feed it back and block
grid check --quick >/dev/null 2>&1 && exit 0
grid check --quick 2>&1 | head -40 >&2
exit 2
```

Pre-commit runs `grid check`. CI runs `grid test --trace --all` — that is
what refreshes the traces local selection reads — plus `grid check
--compile -- --all-targets` and `grid gate <plan>` when there is a plan.

## Journeys

### Agent

**Edit loop, every turn.** `scope` for context with a stated horizon;
edit; the hook runs `check --quick`; `test` runs only what the change
reaches and names the changed definitions no test reaches and the tests
whose verdict flipped under an unchanged closure; `check` before commit
prints a refactor certificate or a gate. The agent's claims — "only
refactored", "tests cover it", "no API change" — become checked
predicates.

**Plan, implement, gate.** Plan mode writes a plan file (promised
definitions with `interface: true|false`, promised edges); `grid plan
lint`; implement; `grid drift --plan f` lists the callers nobody promised;
`grid gate f` holds on an unpromised interface move or a promise not made
good. Exit 3 means the plan was not made good.

**A test went red.** `grid suspects <test> --closure <last-green>` —
the definitions inside the test's closure whose bodies moved since,
ranked by hops, with the declined share stated. Milliseconds, no rerun,
no history required.

**Parallel agents.** `grid interfere <other-worktree>` before a merge — a
moved signature here against a new caller there, with no file in common.
Git's merge cannot see it.

### Human

**Review the agent's PR.** `grid check <base> <head> --plan f --why
--json` — interface moves, unpromised work, uncompiled callers, stale
verdicts, weak tests, uncovered changes, clone-of, deserialize
compatibility, covenant fires. Read that first, the diff second.

**"Can I change this?"** `grid uses X`, `grid scope X --sigs 2`, then
stub the new signature and `grid drift`. Decide with numbers and a
horizon.

**The compiler-blind cases.** Declare once and gate forever: `grid
covenant A B --via protocol` for a coupling across a process or language
boundary; a `persisted` invariant in `.grid/invariants.json` for a type
whose old bytes exist on disk. Both are the only *modeled* evidence that
gates, because a human declared it.

**Orient in an unfamiliar repo.** `grid scope <entry> --bodies 1 --sigs
2`, `grid explain <any word in the output>`, `grid scope --since
<snapshot>` on return.

Everything else — `diff`, `uses`, `resolve`, `orphans`, `hotspots`, `log`,
`watch`, `trace diff`, `edit apply`, `serve` — is behind those; `grid
help` lists every verb, `grid explain` defines every word.

## What grid will not pretend

- **An edge exists only where the language's own binding rules establish
  it.** Glob imports, macro bodies, and method calls on receivers whose
  type is not written down are omissions, never guesses — and every
  omission is counted: `miss 11985 references declined (external 1673,
  method 8852, internal 1460)` closes a survey of this repo. Method calls
  dominate on Rust; a moved signature reached only through `x.m()`
  produces no suspect today (the compiler catches it on Rust; on Python
  nothing does). Closing that is the top item in `docs/TODOS.md`.
- **Evidence grades are load-bearing.** `proven > observed > modeled >
  heuristic`. Proven and observed gate (exit 3); modeled gates only under
  a declaration; heuristic ranks and never moves the exit.
- **Budgets are rejected, never truncated.** A walk that runs out of fuel
  refuses the whole answer or reports exactly how far it is complete
  (`aground`, `horizon`); it never serves a shorter list as if it were
  the list.
- **Test selection skips a test only with evidence.** A test with no
  retained trace always runs; a walk over budget runs everything and says
  why. Without traces `grid test` is your runner plus bookkeeping — run
  `grid test --trace --all` once (needs `cargo-llvm-cov` or `pytest-cov`)
  and selection starts.
- **Nondeterministic, never "flake".** Same closure key, same
  environment, opposite verdict: the claim is "not the charted code". A
  race is code.

## Output contract

Porcelain records on stdout, one per line, stable columns
(`docs/PORCELAIN.md`); hints and banners on stderr (`GRID_HINTS=0`
silences them); `--json` on every verb emits the wire result verbatim
(`docs/wire-v1.openrpc.json`). Exit codes are uniform: `0` pass, `1`
internal, `2` bad input, `3` divergence, `4` stale coordinate. A name
that resolves to nothing is exit 2 with the closest callsigns on stderr —
silence is never an answer. `scope --bodies` and `explain` are the two
text-mode verbs; their `--json` is the record form.

## Coverage closes the loop, with no tracer

Grid instruments nothing. The runner produces a coverage report; grid maps
executed lines onto definitions; one report is one test, one `observed`
edge per definition it reached, with the snapshot it was mapped against.

```bash
# Rust, per test name $t from `cargo nextest list`
cargo llvm-cov nextest --lcov --output-path cov.lcov -E "test(=$t)"
grid trace ingest --lcov cov.lcov --test "$t"

# Python, one run then one report per test node id $t
pytest --cov --cov-context=test
coverage json --contexts="$t" -o cov.json
grid trace ingest --coverage-json cov.json --test "$t"
```

`grid test --trace` runs the recipe for the tests it selected; `--all`
for the inventory. Traces decay honestly: an observation whose endpoint no
longer exists in the chart asked about is demoted to heuristic with reason
`stale-trace`, and the retained file is never rewritten.

## Numbers (2026-09-01, Apple M4 Max, release, this repo: 1813 defs / 3419 edges)

| verb | measured |
|---|---|
| `grid survey` (worktree) | 80–145ms |
| `grid survey <rev>` | 130ms here; 240ms on a 4312-def repo (was 5.4s: one git process, not one per file) |
| `grid check --quick` | ~141ms, ~94ms of it the worktree survey a hook must re-run |
| `grid check` | ~204ms |
| `grid.test.plan` | 4–5ms warm; the run is the runner's |
| `grid scope --bodies`, 3 hops | ~20ms |
| `grid suspects --closure` | ~4ms by snapshot id |
| `grid interfere`, both stores warm | ~83ms |

Every commit that moves a number states it; a budget that does not hold is
stated, never rounded (`docs/plans/*.md`, each plan's Status ledger).

## Build and install

Stable Rust (1.85+), edition 2021 workspace. No C, no protoc, no
submodules, no runtime dependencies beyond what `docs/ASSURANCE.md`
justifies line by line.

```bash
cargo build --release
cargo install --path crates/grid      # or copy target/release/grid onto PATH
```

Per-repo state lives in `.grid/`: the content-addressed segment store, the
membership image, retained traces, the verdict log, the compile record.
All of it is rebuilt by the next survey — gitignore it, except the two
declarations:

```gitignore
.grid/
!.grid/invariants.json
!.grid/covenants.json
```

Gates before any commit: `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets` (pedantic, deny), `cargo test --workspace`,
`cargo deny check`, `scripts/check-assurance-links.sh`,
`scripts/check-unsafe-confined.sh`. `just check` bundles them; `just
fuzz`, `just miri`, `just soak`, `just verify` for the deeper rungs.

## Architecture

Twelve crates, one direction, no cycles.

| crate | owns |
|---|---|
| `grid-wire` | every serialized shape; generated from the OpenRPC contract; serde only |
| `grid-datum` | identity vocabulary: datum, sig, shape, definition, references — parser-free |
| `grid-lang-py` / `grid-lang-rs` | frontends: parse → normalize → extract (rustpython / syn) |
| `grid-survey` | tree → definitions + proven edges; roots, re-exports, per-language resolvers, the declined split |
| `grid-chart` | the segment (a validated, mmap-able CSR image; epoch 4), the membership image, the diff |
| `grid-chart-io` | the store: atomic publish, crash-safe by kill-9 soak, content-addressed, superseded-epoch recovery |
| `grid-cache` | verdicts keyed by identity closure; budgets rejected, never truncated |
| `grid-trace` | coverage and runtime traces → observed edges; decay grading |
| `grid-walk` | budgeted multi-hop walks with a completeness certificate |
| `grid-gate` | plan vs diff: the gate verdict, `interface` promises |
| `grid` | CLI + daemon; the only composition root |

Load-bearing invariants (`docs/ASSURANCE.md`, claims C1–C43):

- **Determinism.** Same tree → byte-identical segments → same snapshot
  id, any machine, any order. Incremental ≡ full, tested against the
  answer.
- **Identity.** The datum ignores formatting, comments and local names
  and keeps the definition's name; the sig holds what a caller can
  observe; the shape erases the name and is the rename-join key.
  Normalization changes are declared epoch bumps, never silent re-hashes.
- **Honesty.** An edge exists only where the language's binding rules
  establish it; everything declined is counted and reported.
- **Formats.** Segment epoch 4 is current; every earlier epoch parses
  forever, pinned by a committed fixture, answering columns it lacks with
  *unknown*. Wire epoch 1 (`docs/wire-v1.openrpc.json`), additive-only
  within an epoch. Porcelain is semver.

## Documents

- `docs/ONTOLOGY.md` — the vocabulary; `grid explain` is generated from it
- `docs/PORCELAIN.md` — the output contract
- `docs/ASSURANCE.md` — claims and evidence; every dependency justified
- `docs/LANG_RS.md` — the Rust frontend's design and its paid-for lessons
- `docs/TODOS.md` — the journeys, their pressure test, and what is next
- `docs/plans/` — each plan as written, then its Status as implemented
  with deviations and measured numbers
- `docs/research/ANALYSIS.md` — the four papers grid steals from

## Contributing

- Read `docs/ONTOLOGY.md` first; a word means one thing everywhere, and
  retired words are never emitted again (`grid explain` still answers
  them).
- Every feature ships with its conservatism test — the assertion that it
  *declines* what it cannot prove — seen red before the fix is claimed;
  the commit message records the mutation that made it red.
- No new dependencies without an ASSURANCE.md entry; runners (cargo,
  nextest, pytest, coverage.py, git) stay outside grid and are spawned.
- `#![forbid(unsafe_code)]` everywhere except the one audited mmap block;
  clippy pedantic at deny; no `unwrap` outside tests; index math guarded.
- Schema changes: wire additive-only inside an epoch; segment format
  changes are declared epoch bumps with a forever-parsed fixture.
- Deviations from a plan are allowed and recorded, one line each with the
  reason, in the commit that made them.
- Commit messages: imperative subject, body says why and states the
  measured numbers, gates green first.
