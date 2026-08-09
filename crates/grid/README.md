# grid

Single responsibility: CLI + daemon (single binary: survey, query, gate,
serve).

```
grid                     the three commands, on stderr
grid version             engine, wire and porcelain versions
grid gate <track> [rev]  check a track against the delta at head
grid track new|show|lint the planning loop
grid serve [socket]      unix socket, newline-framed JSON-RPC
```

Records on stdout (`docs/PORCELAIN.md`), hints on stderr (`GRID_HINTS=0`
silences), `--json` for the wire result verbatim. Exit codes are uniform
across verbs: `0` pass · `1` internal · `2` bad input · `3` divergence ·
`4` stale coordinate.

The CLI and the daemon are not two implementations: both call `rpc`, so
`grid gate --json` and a `grid.gate.check` request cannot drift.

Repo state lives under `.grid`: `tracks/` (planner-authored promises) and
`invariants.json` (standing rules, checked at every gate and lint).
