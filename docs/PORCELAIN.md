# Grid porcelain grammar

version: 1.4.0

Stdout of every grid verb is typed records: one record per line, fields
separated by runs of spaces, stable column order per record type. stderr
carries hints and banners (`GRID_HINTS=0` disables); machines read stdout
only. `--json` emits the wire result verbatim and is not covered by this
document.

## Records

| tag | columns |
|---|---|
| `snapshot` | id, stats (`defs N  edges N  reread N  <duration>`) |
| `def` | glyph, callsign, hash transition (`old→new` or new hash), `sig` when the interface identity also moved (absent when it held, and absent when a pre-epoch-3 snapshot means nobody can say) |
| `ref` | callsign, datum prefix, `file:line` when the snapshot carries locations (`grid resolve` — a definition that exists, not one that changed) |
| `orphan` | callsign, datum prefix, `file:line` — a definition with no *proven* caller (`grid orphans`); not the same as dead |
| `leg` | `@from -> @to`, rev — one patch edge in the datum's history, newest first (`grid log`) |
| `root` | import name, directory — the map callsigns are rooted in (`grid roots`) |
| `hot` | callsign, `cx N`, `legs M`, `file:line` — complexity × churn, descending (`grid hotspots`); a ranking, never a gate |
| `edge` | glyph, verdict role, or evidence grade (query results), `src -relation-> dst`, evidence or context |
| `test` | role, test id, `expected=actual` |
| `inv` | status, invariant id |
| `path` | callsign, hop count, weakest evidence |
| `hop` | relation, target, evidence |
| `hit` | callsign, verdict, recorded time |
| `fork` | caller, green callee (`-` if the trace had no hop there), red callee |
| `drift` | evidence grade, dependent callsign, `<-relation-`, changed callsign, sig transition (`old4→new4`, or `sig?` when either side carries no sig), reason (`-` when the edge speaks for itself), dependent `file:line` (`-` when unknown) — `grid drift` |
| `miss` | callsign, reason |
| `pass` / `hold` | track id, snapshot id (first record of gate output) |
| `warn` | callsign, message |
| `conflict` | callsign, hash transition |
| `done` / `todo` | promise fraction / promise record |

A `done` record from `grid todo` carries the overall fraction, then the
per-kind split (`edges 2/3 · defs 1/1 · tests 0/1`). A `done` record from
`edit apply` is not emitted: applied edits are `def` records, conflicts are
`conflict` records.

`grid suspects` emits ranked `hit` records: datum handle, Ochiai score, then
the `via` evidence text. `grid bisect` emits one `hit` record for the culprit
when known, followed by a `done` record carrying measured `steps` and
`cache_hits`. Neither verb turns missing retained history into an empty
success: insufficient history is bad input (exit `2`) with the explanation on
stderr and no stdout records. Seeded test fixtures exercise this grammar but
are not production-history claims.

Verdict roles on `edge` records: `match`, `miss`, `surplus`, `tolerated`.

`grid drift` closes its output with `miss` records for what it could not
check: `miss <n>  unresolved references unchecked` when the head survey
declined names, and `miss ?  unresolved references uncounted (head was not
surveyed)` when head was a published snapshot nobody re-surveyed, so the
count is unknown rather than zero. A drift report with no suspects and
nothing unchecked prints nothing and exits `0`; suspects exit `3`. An inbound
walk that runs past its fuel is refused whole — exit `1` with the reason on
stderr — never served as a shorter list.

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
stale-coordinate conflict (`edit apply`). An empty result *set* (zero
orphans, zero edges) exits `0` with no records; a name that resolves to
*nothing* is bad input (`2`), and the stderr hint lists the closest charted
callsigns — silence was read as "does not exist", which is the one
conclusion grid never licenses.

## Versioning

Semver. Adding a record type or a trailing column is minor; changing column
order or meaning is major. The daemon reports this version via `grid version`
and the wire contract's `x-porcelain` field.
