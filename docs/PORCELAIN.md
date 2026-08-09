# Grid porcelain grammar

version: 1.2.0

Stdout of every grid verb is typed records: one record per line, fields
separated by runs of spaces, stable column order per record type. stderr
carries hints and banners (`GRID_HINTS=0` disables); machines read stdout
only. `--json` emits the wire result verbatim and is not covered by this
document.

## Records

| tag | columns |
|---|---|
| `snapshot` | id, stats (`defs N  edges N  reread N  <duration>`) |
| `def` | glyph, callsign, hash transition (`old→new` or new hash) |
| `ref` | callsign, datum prefix (`grid resolve` — a definition that exists, not one that changed) |
| `edge` | glyph or verdict role, `src -relation-> dst`, evidence or context |
| `test` | role, test id, `expected=actual` |
| `inv` | status, invariant id |
| `path` | callsign, hop count, weakest evidence |
| `hop` | relation, target, evidence |
| `hit` | callsign, verdict, recorded time |
| `fork` | caller, green callee (`-` if the trace had no hop there), red callee |
| `miss` | callsign, reason |
| `pass` / `hold` | track id, snapshot id (first record of gate output) |
| `warn` | callsign, message |
| `conflict` | callsign, hash transition |
| `done` / `todo` | promise fraction / promise record |

Verdict roles on `edge` records: `match`, `miss`, `surplus`, `tolerated`.

## Glyphs

`+` added · `-` removed · `~` modified · `>` moved

## Evidence column

`proven` · `observed` · `modeled` · `heuristic` (strongest first)

## Display policy

Callsigns are the default identity — stable-length, human-unique. `@prefix`
datum handles appear only where identity beyond the name matters: hash
columns, ambiguity errors, cross-time references. `hit` and `miss` records are
such a case: a cache verdict is addressed by content across time, so their
first column is the datum handle, not a callsign that may since have moved. Input accepts any unique
name suffix or `name@prefix` (git short-SHA auto-lengthening).

## Exit codes (uniform across verbs)

`0` success/pass · `1` internal error · `2` bad input or ambiguous name ·
`3` divergence verdict (gate hold, unaddressed fix anchors) · `4`
stale-coordinate conflict (`edit apply`). Empty result sets exit `0` with no
records.

## Versioning

Semver. Adding a record type or a trailing column is minor; changing column
order or meaning is major. The daemon reports this version via `grid version`
and the wire contract's `x-porcelain` field.
