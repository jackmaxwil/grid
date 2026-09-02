# Grid porcelain grammar

version: 2.12.0

Stdout of every grid verb is typed records: one record per line, fields
separated by runs of spaces, stable column order per record type. stderr
carries hints and banners (`GRID_HINTS=0` disables); machines read stdout
only. `--json` emits the wire result verbatim and is not covered by this
document.

## Records

| tag | columns |
|---|---|
| `snapshot` | id (`@rev`; `@rev+<prefix>` when the tree was a dirty worktree — labelled with the rev it was checked out from, and never taken for the tree at that rev), stats (`defs N  edges N  reread N  <duration>`) |
| `miss` (survey) | count, `references declined (external N, method N of which N refused, internal N)` — the one record `grid survey` closes with when the resolver declined anything; the `of which N refused` clause is the typed receivers the method-call rule declined with a reason, and is absent when the snapshot predates the count |
| `def` | glyph, callsign, hash transition (`old→new` or new hash), `sig` when the interface identity also moved (absent when it held, and absent when a pre-epoch-3 snapshot means nobody can say); inside `grid gate` output, an unaccounted definition's trailing column reads `unaccounted: interface` when a def promise's `interface` claim is what holds it |
| `ref` | callsign, datum prefix, `file:line` when the snapshot carries locations (`grid resolve` — a definition that exists, not one that changed) |
| `orphan` | callsign, datum prefix, `file:line`, and under `--traced` a trailing `trace N` — a definition with no *proven* caller (`grid orphans`); not the same as dead, and `trace 0` is still a maybe |
| `edit` | `@from -> @to`, rev — one patch edge in the datum's history, newest first (`grid log`) |
| `root` | import name, directory — the map callsigns are rooted in (`grid roots`) |
| `hot` | callsign, `cx N`, `edits M`, `file:line` — complexity × churn, descending (`grid hotspots`); a ranking, never a gate |
| `edge` | glyph, verdict role, or evidence grade (query results), `src -relation-> dst`, evidence or context |
| `test` | role, test id, `expected=actual` |
| `inv` | status, invariant id |
| `path` | callsign, hop count, weakest evidence |
| `hop` | relation, target, evidence |
| `hit` | callsign, verdict, recorded time |
| `fork` | caller, green callee (`-` if the trace had no hop there), red callee |
| `drift` | evidence grade, dependent callsign, `<-relation-`, changed callsign, sig transition (`old4→new4`, or `sig?` when either side carries no sig), reason (`-` when the edge speaks for itself; `uncompiled:<cfg>` from `grid check --compile`), dependent `file:line` (`-` when unknown) — `grid drift`, `grid check --compile` |
| `covenant` | pattern A, pattern B, `via` word — one declared covenant (`grid covenant`) |
| `delta` | `+` or `-`, then a `drift` record's columns — one signed change to the suspect set (`grid watch`) |
| `miss` | callsign, reason |
| `pass` / `hold` | plan id, snapshot id (first record of gate output) |
| `warn` | callsign, message |
| `conflict` | callsign, hash transition |
| `done` / `todo` | promise fraction / promise record |

A `done` record from `grid todo` carries the overall fraction, then the
per-kind split (`edges 2/3 · defs 1/1 · tests 0/1`). A `done` record from
`edit apply` is not emitted: applied edits are `def` records, conflicts are
`conflict` records.

`grid suspects` emits ranked `hit` records: datum handle, Ochiai score, then
the `via` evidence text. `grid bisect` — answerable but unlisted, since it
always refuses until retained history exists — emits one `hit` record for the
culprit when known, followed by a `done` record carrying measured `steps` and
`cache_hits`. Neither verb turns missing retained history into an empty
success: insufficient history is bad input (exit `2`) with the explanation on
stderr and no stdout records. Seeded test fixtures exercise this grammar but
are not production-history claims.

`grid suspects <test> --closure [<snapshot|rev>]` is the same verb's second
evidence mode, for the failure that has no history at all — the test passed
at the last snapshot and fails now. With no snapshot after it, the flag uses
the last one at which that test recorded a `pass` — the verdict log's entry,
through the trace it names, to the snapshot that trace was mapped against —
and one stderr line says which snapshot answered. No such record is bad input
(exit `2`) with the two ways out on stderr, never a ranking against head,
which would be empty and read as "nothing in the closure moved". Its rows are the same `hit` records with
the word `closure` where the Ochiai score sits, and `via <k> hops, datum
moved` as the evidence: the definitions the test's closure at head can reach
whose datum that snapshot did not hold — a body that moved, or one that
arrived since — nearest first, datum identity as the tie-break. Heuristic — a
candidate is a definition that changed and is reachable, not a proven cause —
so the exit is `0` on any answer, including an empty one. When two retained
traces of the test straddle the snapshot, one `fork` record follows the
ranking, from the same `grid trace diff` machinery.
The output closes with one `miss` record saying how much of the closure the
chart could not follow; today that is `miss ?  references declined uncounted
(no survey ran in this call)`, because the declined split is a property of a
survey run and this verb answers from the published snapshot. A closure that
runs past its fuel is refused whole (exit `1`) and a `since` the retained
chain never held is bad input (exit `2`) — never an empty ranking, which
would read as "nothing in the closure moved".

Verdict roles on `edge` records: `match`, `miss`, `surplus`, `tolerated`.

`grid survey` closes with one `miss` record when the resolver declined any
reference — `miss 8413  references declined (external 1272, method 6091 of
which 261 refused, internal 1050)` — the existing tag and its existing
"what could not be checked" meaning. The three populations add up to the
total by construction; `refused` is a sub-count of `method`, the calls
whose receiver *was* typed to a charted type and the method-call rule
declined with a reason (a trait candidate, a generic impl, no inherent
method, a wrapper), and the clause is omitted when the snapshot predates
the count. They mean opposite things: `external` costs a claim about this
repo nothing, `method` is a hole no static rule closes — the receiver could
not be typed, or was typed and refused — and `internal` is a hole in exactly
what `grid drift` claims. A survey that declined nothing prints no such
record.

`grid drift` closes its output with `miss` records for what it could not
check: `miss <n>  unresolved references unchecked (external N, method N,
internal N)` when the head survey declined names, and `miss ?  unresolved
references uncounted (head was not surveyed)` when head was a published
snapshot nobody re-surveyed, so the count is unknown rather than zero. The
split is omitted, and the message is the bare `unresolved references
unchecked`, only when a report predates the split.

A drift report with no suspects and nothing unchecked prints nothing and
exits `0`; suspects exit `3`. An inbound
walk that runs past its fuel is refused whole — exit `1` with the reason on
stderr — never served as a shorter list. A declared covenant whose pattern
matches nothing at drift time closes the output as a `warn` record: a rule
that quietly stopped being checked is worse than no rule.

`grid drift --tests` adds one `miss` record per test whose recorded verdict
no longer applies — `miss <test>  closure-changed` when the closure key moved,
`miss <test>  closure-rejected` when the closure could not be established at
all (a rejection is a miss, never a hit) — plus `miss <n>  unattributed
entries` for recorded verdicts no retained trace claims. Verdict-staleness is
identity math over the closure, so it exits `3` like any other divergence. A
test the head chart cannot resolve is a `warn`, not a gate: the test being
gone is not a divergence in the code.

It also emits `warn <test>  weak-for <def>` for a test that covered a
definition, kept passing while that definition's body changed, and was never
itself touched — it proved nothing about the change. Heuristic, so a ranking
and never a gate: the exit code does not move for it. Coverage carried by a
decayed trace produces no such record at all and is counted as
`miss <n>  weak-checks skipped (stale coverage)`, because a heuristic built
on a demoted heuristic is noise.

`grid drift --compat` adds the Deserialize compatibility check over the same
changed set: for every `struct` or `enum` deriving `Deserialize` whose
interface moved, the two field lists are reconstructed from source at both
snapshots and compared under the six rules old bytes care about — `required
field added`, `field renamed without alias`, `type changed`, `variant
removed`, `field removed under deny_unknown_fields`, `representation
changed`. A finding is `warn <type>  compat: <rule>` and heuristic: old bytes
are not *known* to exist, so it ranks and never moves the exit code. A type
matching a standing `persisted` invariant is `inv violated persisted:<type>
<rule>` instead, at `modeled`, exit `3` — the declaration is the human fact
that old bytes exist, and it is what a static check cannot supply. A changed
type whose derive the source does not spell out — behind a `cfg_attr`,
produced by a macro, or carrying a serde attribute grid cannot parse — is
skipped and closed over as `miss <n>  compat unchecked (derive not visible)`,
never as a clean bill of health. Without `--compat` nothing of this is
emitted and the report carries no `compat` field at all.

`grid test` uses existing tags only. `hit <test>  <pass|fail>  <recorded>`
per replayed verdict; `miss <test>  <reason>` per selected test, the reason
one of `closure-changed` (the walk from the changed set reached it, or the
test itself changed), `no-trace` (no live trace watched it, so the static
closure alone does not license a skip), `unresolved` (the chart cannot name
it), `aground` (the walk ran past its fuel, so every test is selected), or
`not-recorded` (nothing on record to replay); `test <ok|fail>  <test>
pass=<pass|fail>` per outcome the runner reported; `warn <test>
nondeterministic  same closure key, same env: <v1> <t1>, <v2> now` when a
key already held the opposite verdict; `warn <def>  uncovered` per changed
definition no selected test reaches; and a closing `done  tests R/N ran ·
H replayed · F failed`. The first column of `hit` and `miss` is the test
id as the runner names it, not a datum: a test is addressed by name in this
verb, and the datum rides in `--json`. Exit `3` when a run test failed or
a replayed verdict is `fail` — the closure is unchanged, so the failure
stands, and `F` in the `done` line counts both (a selected test the runner
never reported on counts as failed and is named in a `warn`), `1` when the runner could not be spawned, `0` otherwise —
`uncovered` and `nondeterministic` are heuristic and never move it. Runner
output goes to stderr; the horizon line on stderr says how many tests were
selected out of how many, and what the plan and the survey cost.

`grid check --compile [-- <cargo check args>]` runs `cargo check` with the
given arguments from the worktree root, the compiler's output on stderr. A
failed compile passes cargo's exit code through and prints nothing on
stdout — the diagnostics are the answer, and no record is written. A green
compile writes the compile record (`.grid/compile.json`) and then, for the
same changed set `grid drift` would name (the worktree against `HEAD~1`,
sig moved or unknown), prints one `drift` record per *proven* dependent
sitting under a `cfg` the record decidably excluded — reason
`uncompiled:<cfg>`, the dependent's `file:line` last, exit `3` — and one
`warn <dependent>  cfg unknown: <text>` per dependent whose `cfg` cannot be
decided (`cfg_attr`, a custom key, a target fact the record lacks, or
`pre-epoch-4 snapshot` for a chart written before the cfg column), which
never moves the exit. Compiled under `-- --tests`, `-- --features x` or
`-- --all-features`, the same callers print nothing. With `--plan` the
composite prints in its usual order — drift, the compile records, then
the verdict; `--json` emits the
`CompileReport` alone without a plan and the `CheckResult` with one. A tree
with no `Cargo.toml` has no compile to wrap: one hint, exit `0`.

`grid check [<base>] [<head>] [--plan <file>] [--quick] [--why]` is the
composite — the one verb an agent runs after an edit — and prints its
sections in gate-first order, each exactly as its own verb prints it: the
drift section (`drift` suspects, covenant `warn`s, the closing `miss`
counts), the compile record's `drift … uncompiled:<cfg>` records and
`warn … cfg unknown`, the gate verdict under `--plan` (`pass`/`hold` and
its records; with `--why`, a `path` per fulfilling promise with `hop`
records beneath it and a `miss <def>  unexecuted` per changed definition
the named trace never reached), stale verdicts as `miss <test>
closure-changed|closure-rejected`, compat as `inv violated
persisted:<type>  <rule>` or `warn <type>  compat: <rule>`; then the tail
that ranks — `warn <test>  weak-for <def>`, `warn <def>  uncovered`, `warn
<new>  clone-of <existing>` — the check's own `warn -  no compile record
since @<prefix>` when `.grid/compile.json` is absent or predates the last
survey, and finally, when no gate fired, one `done  refactor  interfaces
held · <n> covering tests replayed pass · <m> changed defs uncovered` (or
`<k> interfaces moved`, or `no retained traces` in place of the counts).
Exit `3` when any gate fired — drift suspects, uncompiled callers, a `hold`,
a stale verdict, a covenant fire, a declared compat finding — `1` when a
section refused (aground, an unreadable store), `2` on bad input; a
refusal prints nothing on stdout. The default base is the plan's, else
`HEAD~1`; head defaults to `HEAD`, the worktree. `--quick` prints the drift
section at the proven floor and the compile section only.

`--quick` has one property the full check does not: **it says an unchanged
count once**. The closing `miss <n>  unresolved references …` line and the
check's own `warn -  no compile record since @<prefix>` are printed only when
their text differs from what the last `--quick` in this store printed —
remembered as a hash of those lines in `.grid/check-quick.last`, engine state,
safe to delete. A hook fires per edit and those two lines are otherwise
identical every turn. A count that *moved* is always printed, which is the
only direction that matters, and a state file grid cannot read or write
suppresses nothing. The full `grid check` always prints both, and `--json` is
the record form and is never filtered. Suppressing a line never moves the exit
code.

`grid invariant persisted <pattern> [--note <text>]` declares a standing
rule and `grid invariant --list` prints the repo's set; both emit `inv
standing <id>` records, the same record `grid todo` prints for a plan's own
rules, with the id `persisted:<pattern>`. Declaring the same pattern twice is
one rule. The pattern is resolved against the live chart and one that matches
nothing is bad input (exit `2`) with the closest charted callsigns on stderr
and nothing written — a rule that can never fire reads as protection and is
not. `persisted` is the only kind this verb writes; `forbid-edge` and
`require-edge` are authored in `.grid/invariants.json` by hand and survive a
`grid invariant` call byte for byte, which is also how any rule is removed.
`--note` rides as the rule's `description`.

`grid interfere <other-root> [--base <rev>]` compares two worktrees of one
repository against their common base — the merge base of the two heads unless
`--base` names one. A definition both sides moved is `warn <callsign>
both-touched`: git's merge will find that one too, it carries no edge and so
no evidence grade, and it does not gate. The findings only grid can see are
`drift` records with reason `interfere:<other-root>` — one side's drift
suspects that land in the other side's edits — carrying the grade of the edge
that found them, and they exit `3`. The other root's store is opened
read-only and never written, and grid never surveys it: a store that cannot
be opened, is superseded, or does not hold the base is exit `2` with the
command that fixes it on stderr (a dirty worktree survey labelled with the
base rev is not the base, and does not count), **never** an empty answer, because "I could
not read that chart" and "those two worktrees do not interfere" are opposite
claims. `theirs` is therefore as fresh as that worktree's last `grid survey`
and no fresher.

`grid watch` emits `delta` records rather than a current list: `+` a suspect
appeared, `-` it was retracted. A consumer holds a running sum, and the sum
after any prefix of events is what a fresh `grid drift` would say — so
nobody has to diff lists to see what changed. Watch mode has no verdict and
always exits `0`; `grid drift` is the verb that holds. A file event the
frontend cannot parse emits no deltas at all and says so on stderr, because
a file grid cannot read is not a file with nothing in it.

## Text-mode output

Two verbs answer in prose rather than records, because their answer *is*
prose: `grid explain <word>` and `grid scope --bodies`/`--sigs`. Their stdout
is a document — `## <callsign>  <file:line>  (body, hop k)` and `## sig
<callsign>  <file:line>  (hop k)` headers, each followed by the text, closed
by one `# horizon: bodies complete through <N|none>, sigs through <M|none>,
≈<T> tokens (bytes/4)` line that names the depth each tier is complete to.
The record grammar above does not apply to those lines and no consumer should
parse them as records: `--json` is the record form, and for `scope` it is the
`ScopePack`'s additive `bodies`, `sigs` and `horizon`. The budget is spent in
whole hop layers, farthest first, so the horizon is never a claim about a
layer that was cut in half; a body layer that will not fit is demoted to its
sigs before it is dropped, and a pack that packed nothing still prints its
horizon line. Without `--bodies` and `--sigs`, `grid scope` prints the same
`ref`/`edge`/`inv` records it always has.

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
`3` divergence verdict (gate hold, unaddressed anchors) · `4`
stale-coordinate conflict (`edit apply`). An empty result *set* (zero
orphans, zero edges) exits `0` with no records; a name that resolves to
*nothing* is bad input (`2`), and the stderr hint lists the closest charted
callsigns — silence was read as "does not exist", which is the one
conclusion grid never licenses.

A refusal a survey would fix says so, in one hint line that names the
command **and the tree to run it in** — `hint: run \`grid survey\` in <root>`,
or `hint: run \`grid survey <rev>\` in <root>`. It is one line because it is
one action; it names the tree because the tree is usually not the one the
reader is standing in (`grid interfere` refuses about *the other* worktree),
and a superseded store — which opens empty, and so looks exactly like a store
nobody surveyed — says which format epoch it superseded before the same
command. A refusal a survey does *not* fix (a rev git cannot name, a snapshot
id that names content the store never held) carries no such line: a hint that
would not work is worse than none.

## Versioning

Semver. Adding a record type, a trailing column, a text-mode output, or a
new verb's meaning for an existing tag is minor; widening a record's
message is minor. Changing column order or meaning is major — and so is
renaming a tag, which is what carried 1.6.0 to 2.0.0
(`docs/plans/2026-09-01-naming.md`). One line per bump lives in the commit
that made it (`git log --grep PORCELAIN`). The daemon reports this
version via `grid version` and the wire contract's `x-porcelain` field.
