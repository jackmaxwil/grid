# grid

Single responsibility: CLI + daemon (single binary: survey, query, gate,
serve).

```
grid                     what changed, and the three commands on stderr
grid survey [rev]        survey the tree and publish a snapshot
grid diff [base] [head]  what actually changed, in the chart
grid resolve <name>      the definitions a name refers to
grid version             engine, wire and porcelain versions
grid gate <track> [rev]  check a track against the delta at head
grid track new|show|lint the planning loop
grid serve [socket]      unix socket, newline-framed JSON-RPC
```

Agent verbs (stage 1–2, socket-first — the daemon serves `grid.scope`,
`grid.todo` and `grid.edit.apply`):

```
grid scope <pattern>... [--depth N]  context pack around an intended edit
grid todo <track> [head]             the gate verdict as a worklist
grid edit apply <edits.json|->       CAS writes; conflicts are exit 4
grid uses <name>                     who reaches X, proven edges only
grid explain <word|@prefix>          what any token in this output means
```

`edit apply` addresses *definitions* (`name` or `name@prefix`, CAS on
`base_datum`); an absent base is a creation whose target is a repo-relative
path that must not exist — also grid's own config bootstrap, per D6. One
boundary to know: an import line has no datum, so adding an import is not an
`edit apply` edit at this wave.

Records on stdout (`docs/PORCELAIN.md`), hints on stderr (`GRID_HINTS=0`
silences), `--json` for the wire result verbatim. Exit codes are uniform
across verbs: `0` pass · `1` internal · `2` bad input · `3` divergence ·
`4` stale coordinate.

The CLI and the daemon are not two implementations: both call `rpc`, so
`grid gate --json` and a `grid.gate.check` request cannot drift.

Repo state lives under `.grid`: `tracks/` (planner-authored promises),
`invariants.json` (standing rules, checked at every gate and lint), and
`chart/` (the segment store — content-addressed, append-only).

`gate` and `diff` survey what they need, so CI runs one command. Surveying is
idempotent by construction: an unchanged tree is the same snapshot id, so it
cannot be run "too often".
