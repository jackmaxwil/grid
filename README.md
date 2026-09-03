# grid

What your code promises vs what it does — for the agent editing it.

Grid surveys a repository into a chart: every definition with a
content-derived identity (a *datum*, blake3 of its normalized AST; a *sig*
for what callers can see), every relationship it can prove, the evidence
grade of each claim, and the count of everything it could not bind. Diffs,
drift, test selection, gates, attribution and context packs run over that
chart in identity space, not text space, and every answer states its
horizon and its refusals.

Languages: Rust and Python. TS/JS next. One static binary; no build
integration and no config beyond two optional declaration files. Discovery
is `git ls-files` minus your ignore rules. The CLI answers in-process and
needs no daemon; a daemon exists for programs, and two clients speak to it
over the same contract the CLI prints.

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

Pre-commit runs `grid check`. CI runs `just traces-ci` (`grid test --trace
--all`) — that is what refreshes the traces local selection reads — plus
`grid check --compile -- --all-targets` and `grid gate <plan>` when there is
a plan.

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
definitions with `interface: true|false`, promised edges) — or `grid plan
new --from-diff <base>` drafts one from a change already made, for pruning;
`grid plan lint`; implement; `grid drift --plan f` lists the callers nobody promised;
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

## Speaking to it from a program

`grid serve` is the daemon: a unix socket, newline-framed JSON-RPC 2.0,
one line in and one line out, 23 methods (`docs/wire-v1.openrpc.json`).
Two clients ship with it, and neither is hand-written where it can be
generated — both type layers come out of `scripts/gen-wire.py` reading the
same contract, so they cannot disagree about a shape or a method name.

```rust
// Rust: crates/client — grid-wire, serde, std. Nothing else.
use grid_client::Client;
use grid_wire::methods::ResolveParams;
use grid_wire::{Callsign, Locator};

let mut grid = Client::connect(".grid/grid.sock")?;
let defs = grid.call(&ResolveParams {
    locator: Locator::Callsign { callsign: Callsign("grid_chart.Segment.parse".into()) },
    snapshot: None,
})?;
```

```python
# Python: clients/python — json and socket. Nothing else, and no lockfile.
from grid_client import Client
with Client.connect(".grid/grid.sock") as grid:
    defs = grid.resolve(locator={"callsign": "grid_chart.Segment.parse"})
```

What both clients guarantee, and what they refuse to paper over:

- **One connection, one call in flight, matched by id.** A response whose
  id is not the outstanding one is a fault, never handed back as the
  answer to the call that was waiting. Pipelining is what the transport
  allows and neither client exposes: against a serial daemon on a local
  socket it buys round trips and costs a correlation path nothing
  exercises.
- **Adopt, never spawn.** `connect` requires a daemon already running.
  `grid serve` unlinks its socket path before binding, so a client that
  started one would silently steal a live daemon's socket; lifecycle is
  the caller's (`docs/TODOS.md` T-60).
- **The handshake is first.** `connect` asks `grid.version` before
  anything else and refuses a different wire major. Majors are compared
  as integers off the front of the string — there is no semver type,
  because a dependency for that is a dependency.
- **Faults are typed and they poison.** Truncated line, foreign id, a
  body carrying both `result` and `error`, a bad `jsonrpc`, non-UTF-8:
  each is its own variant, and after any of them the connection is closed
  to further calls rather than left desynced. A fresh connection is the
  recovery, and it is equivalent — the daemon holds no per-connection
  state.
- **An unknown error code stays decodable.** The eight wire codes decode
  to a named variant; a ninth, or any JSON-RPC code, arrives with the
  integer intact and no name. Nothing turns a code into an enum by a
  match that could panic.
- **Fuel exhaustion is a result, not an error.** A walk that ran out of
  fuel returns `aground` with its frontier and the radius it finished.
  Resuming is a new walk from that frontier; neither client hides that
  behind a helper.
- **No cache, no reconnect, no retry.** Three methods write. A silent
  re-issue is a second write, so both clients make the caller decide.

The Python package declares `dependencies = []` and has no lockfile, and
`check_manifests.py` fails the gate on the first line that changes either.

## What grid will not pretend

- **An edge exists only where the language's own binding rules establish
  it.** Glob imports, macro bodies, and method calls on receivers whose
  type is not written down are omissions, never guesses — and every
  omission is counted: `miss 17924 references declined (external 5033,
  method 8958 of which 64 refused, internal 743, macro 3190)` closes a
  survey of this repo — a prelude name or a Python builtin left unbound
  is `external`, never a hole. A method call on a receiver whose type the
  file states resolves by rustc's own lookup order: proven where the
  written form matches the inherent method's at the first step, modeled
  where the type was inferred or matched later, refused with a reason
  where a trait could outrank it. Seventy percent of Rust method calls
  have no syntactic type; `cargo check` and traces cover them, not the
  resolver.
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
  and selection starts — `grid test` says so itself, once, while the
  trace store is empty, and `just traces` runs the install check for you.
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

`--json` is also the equivalence the clients are tested against: a typed
client call returns what the CLI prints for the same question, asserted
over the real socket rather than in-process (`crates/grid/tests/client.rs`,
claim C53).

## Coverage closes the loop, with no tracer

Grid instruments nothing. The runner produces a coverage report; grid maps
executed lines onto definitions; one report is one test, one `observed`
edge per definition it reached, with the snapshot it was mapped against.

```bash
# Rust, per test name $t from `cargo nextest list`; the report is removed
# first, or a test whose run wrote none inherits the previous test's reach
rm -f cov.lcov
cargo llvm-cov nextest --lcov --output-path cov.lcov -E "test(=$t)"
grid trace ingest --lcov cov.lcov --test "$t"

# Python, one run then one report per test node id $t
pytest --cov --cov-context=test
coverage json --contexts="$t" -o cov.json
grid trace ingest --coverage-json cov.json --test "$t"
```

`.grid/traces` is per machine. `grid trace export <dir>` copies the retained
traces into a directory CI can hand up as an artifact, and `grid trace
import <dir>` pours one back into a local store — content-addressed,
idempotent both ways, and an import that would put different content under a
trace id the store already holds refuses the whole transport and names the
id. An imported trace decays like any other: it is graded against the chart
it is asked about, not the one it was taken on.

`grid test --trace` runs the recipe for the tests it selected; `--all`
for the inventory — one `cargo llvm-cov nextest` per test, because one
profile cannot be attributed back to the tests that shared a process.
`scripts/templates/` holds a nextest wrapper script and a
`.config/nextest.toml` that fix the attribution half: one `.profraw` per
test, named by the test id, from one nextest run. Copy them in; grid
installs nothing, notices the files and says so, and turning them into
traces without a second export is `docs/TODOS.md` T-45. The wrapper stands
down under a runner grid drives (`GRID_TEST` is set), so an installed
template and `grid test --trace` never trace the same process twice — a
report every count of which is zero is refused as a lost profile, never
retained as a test that reached nothing. `just traces` is that run with the tool check in front of
it — it prints the exact install commands and stops rather than half-tracing
the inventory; `just traces-ci` is the same run without the check, for a
workflow that installed them itself. Traces decay honestly: an observation whose endpoint no
longer exists in the chart asked about is demoted to heuristic with reason
`stale-trace`, and the retained file is never rewritten.

### What the loop cost here, measured

This repository, 2026-09-02, Apple M4 Max, release binary, 546 tests under
`cargo nextest`, on a trace store created empty for the run. `just traces`
— one `cargo llvm-cov nextest` per test — took **29m 13s** and retained
**508 traces**; 38 reports came back with every `DA:` count zero and were
refused as lost profiles, their verdicts kept without a trace. All 38 are
tests that drive the real binary or daemon (`grid::conformance`, two
`grid-wire::conformance`, one `grid-lang-py::properties` case); why their
profile is empty is not diagnosed. Before that refusal existed, 36 of them
were retained as traces of nothing and replayed on every later run. That is
a CI-shaped cost, which is where the recipe belongs; the two lines in
`docs/TODOS.md` T-45 are what would make it one nextest run.

What the traces then buy, all with the build already warm:

| run | selected | wall |
|---|---|---|
| `grid test --all` (the whole inventory) | 546 | 29.8s |
| `grid test`, nothing changed | 62 | 3.2s |
| `grid test`, one body edit in a leaf module (`interfere::compute`) | 201 | 22.4s |
| `grid test`, one body edit in a crate everything depends on (`grid-chart`) | 250 | 41.1s |

The unchanged case is the one an agent meets most often, and it is 9×. The
62 it still runs are exactly the tests grid refuses to skip: 25
`unresolved:module-charted` (every `proptest!`-generated test — the survey
charts items and a macro produces none) and 37 `no-trace` (the refused
reports above; the 38th is a `proptest!` case already in the first group).
The 37 are the slow ones — each spawns the daemon — which is why 3.2s and
not the 0.8s first claimed: that number was measured with 36 of them
replayed on empty traces. A single edit still selects a third of the suite,
because this repository's conformance tests spawn the real binary and a
trace of one of those covers most of it — honest coverage, not a selection
bug. The deep edit's wall time is mostly `cargo` rebuilding what depends on
the crate; selection saves the running, never the building. On a suite of
slow, independent tests the ratio is the selection ratio; here it is not.

## Numbers (2026-09-03, Apple M4 Max, release, this repo: 2223 defs / 4503 edges, store warm)

| verb | measured |
|---|---|
| `grid survey` (worktree) | 120ms (the dirty-diff hash rides the survey's own git thread) |
| `grid survey <rev>` | 130ms here; 240ms on a 4312-def repo (was 5.4s: one git process, not one per file) |
| `grid check --quick` | ~117ms, most of it the worktree survey a hook must re-run |
| `grid check` | ~125ms with the base already retained |
| `grid.test.plan` | 4–5ms warm; the run is the runner's |
| `grid scope --bodies`, 3 hops | ~20ms |
| `grid suspects --closure` | ~4ms by snapshot id |
| `grid interfere`, both stores warm | ~83ms |
| `grid.version` over the socket, raw | 2.371ms (mean of 200, warm connection) |
| `grid.version` through the Python client | 2.377ms (mean of 200) — the client costs 6µs; the framing is the price |

The declined split after the method-calls plan (`docs/plans/2026-09-01-
method-calls.md`), same tree, previous binary against this one:

| | this repo before | after | yi (5,176 defs) before | after |
|---|---:|---:|---:|---:|
| proven / modeled edges | 3,648 / 0 | 3,769 / 71 | 6,876 / 0 | 7,159 / 519 |
| `external` | 2,534 | 4,303 | 10,062 | 14,670 |
| `method` (of which refused) | 9,549 | 7,599 (64) | 23,871 | 18,146 (338) |
| `internal` | 765 | 672 | 2,946 | 2,942 |

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

The Python client is not on an index and does not need to be — it is a
directory with no dependencies:

```bash
PYTHONPATH=clients/python python3 -c 'from grid_client import Client'
```

Per-repo state lives in `.grid/`: the content-addressed segment store, the
membership image, retained traces, the verdict log, the compile record.
All of it is rebuilt by the next survey — gitignore it, except the two
declarations and the plans a change is gated against:

```gitignore
.grid/*
!.grid/invariants.json
!.grid/covenants.json
!.grid/plans/
.grid/plans/*
!.grid/plans/*.json
```

`.grid/*`, not `.grid/`: git does not descend into an excluded directory, so
a negation under `.grid/` is never reached and the declaration would be
ignored anyway.

Gates before any commit: `just check` — `fmt-check`, `clippy --workspace
--all-targets` (all + pedantic, denied), the guardrails, and the suite. The
three are independent lanes, and CI runs each through `just lane`, which
carries the invariant that the gate regenerated nothing into a split run.

`just guardrails` on its own runs the checks under `scripts/guardrails/` —
manifest and crate-boundary law, source-shape ceilings, blob size, commit
style, `cargo deny check`, `scripts/check-assurance-links.sh`,
`scripts/check-unsafe-confined.sh`, and the proof that every generated file
is still byte-identical to what `scripts/gen-wire.py` writes from the
contract — `crates/wire/src` and the Python client's type layer alike, so a
contract edit cannot ship having regenerated only one language.
`codespell`, `cargo-deny`, `cargo-machete`, `cargo-nextest` and `mypy` are
used when they are on `PATH` and announced when they are not; none of them
is a dependency, and none of them absent makes the gate red.

`just hooks` points git at `.githooks/`: the subject rule on the message being
written, `just precommit` (the build-free subset) on commit, the whole gate on
push. Nothing installs them for you.

Deeper rungs, by the claim they evidence: `just miri` (C1), `just verify`
(kani, C3), `just soak` (C11), `just fuzz <target>`. `just har` runs the
assurance lane. `docs/TOOLING.md` says where each mechanism came from.

## Architecture

Thirteen crates, one direction, no cycles.

| crate | owns |
|---|---|
| `grid-wire` | every serialized shape; generated from the OpenRPC contract; serde only |
| `grid-datum` | identity vocabulary: datum, sig, shape, definition, references — parser-free |
| `grid-lang-py` / `grid-lang-rs` | frontends: parse → normalize → extract (rustpython / syn) |
| `grid-survey` | tree → definitions + proven edges; roots, re-exports, per-language resolvers, the declined split |
| `grid-chart` | the segment (a validated, mmap-able CSR image; epoch 5), the membership image, the diff |
| `grid-chart-io` | the store: atomic publish, crash-safe by kill-9 soak, content-addressed, superseded-epoch recovery |
| `grid-cache` | verdicts keyed by identity closure; budgets rejected, never truncated |
| `grid-trace` | coverage and runtime traces → observed edges; decay grading |
| `grid-walk` | budgeted multi-hop walks with a completeness certificate |
| `grid-gate` | plan vs diff: the gate verdict, `interface` promises |
| `grid-client` | the socket client: one connection, typed calls over `Method`, faults typed and poisoning |
| `grid` | CLI + daemon; the only composition root |

`grid-client` depends on `grid-wire` and nothing else in the workspace, so
the schema wall stays the only surface a client shares with the engine.
The Python client sits outside `crates/` at `clients/python/`, which keeps
the naming law (`crates/x` is `grid-x`) true by construction: `crates/`
holds Rust and only Rust.

Load-bearing invariants (`docs/ASSURANCE.md`, claims C1–C54):

- **Determinism.** Same tree → byte-identical segments → same snapshot
  id, any machine, any order. Incremental ≡ full, tested against the
  answer.
- **Identity.** The datum ignores formatting, comments and local names
  and keeps the definition's name; the sig holds what a caller can
  observe; the shape erases the name and is the rename-join key.
  Normalization changes are declared epoch bumps, never silent re-hashes.
- **Honesty.** An edge exists only where the language's binding rules
  establish it; everything declined is counted and reported.
- **Formats.** Segment epoch 5 is current; every earlier epoch parses
  forever, pinned by a committed fixture, answering columns it lacks with
  *unknown*. Wire epoch 1 (`docs/wire-v1.openrpc.json`), additive-only
  within an epoch. Porcelain is semver.
- **Generation.** Every serialized shape, in every language grid ships, is
  written by one script from one contract, and the gate refuses a tree
  where any of them is stale.

## Documents

- `docs/ONTOLOGY.md` — the vocabulary; `grid explain` is generated from it
- `docs/PORCELAIN.md` — the output contract
- `docs/ASSURANCE.md` — claims and evidence; every dependency justified
- `docs/LANG_RS.md` — the Rust frontend's design and its paid-for lessons
- `docs/TODOS.md` — the journeys, their pressure test, and what is next
- `docs/plans/` — each plan as written, then its Status as implemented
  with deviations and measured numbers
- `docs/research/ANALYSIS.md` — the four papers grid steals from

## Forge

This repository lives on the estate's Forgejo: `git.example.invalid/apex/grid`
(clone and push over SSH, `ssh://git@forge.example.invalid:2222/apex/grid.git`).
Pull requests open and merge there, and `.forgejo/workflows/` is the gate that
has to be green: `main` takes merges only through a pull request whose checks
passed. GitHub (`github.com/jackmaxwil/grid`) is a mirror of the forge and
carries no CI; nothing pushes to it from a keyboard. Sessions drive pull
requests with `tea`, the Gitea CLI, as described in yi's `docs/FORGE.md` under
"Working the forge from a session".

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
  changes are declared epoch bumps with a forever-parsed fixture. Never
  hand-edit a generated file — edit `docs/wire-v1.openrpc.json` and run
  `just wire`, which writes both languages.
- Deviations from a plan are allowed and recorded, one line each with the
  reason, in the commit that made them.
- Commit messages: conventional subject (`type(scope): sentence`), body
  says why and states the measured numbers, gates green first.
