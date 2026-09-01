# Grid ontology

The normative vocabulary. The domain is navigation; every noun is a newtype,
and every porcelain token means exactly one thing. This file is one of the
two sources `grid explain` is generated from (the other is the wire
contract's schema descriptions), so a word that changes meaning changes here
or does not change at all.

Format contract: the tables below are parsed by `crates/grid/build.rs` —
rows are `| \`token\` | one answer, at most three lines when printed |`.

## Nouns

| token | meaning |
|---|---|
| `datum` | a definition's identity: blake3 of its normalized AST. Reformat it, rename its locals, rewrite its comments — the datum holds; change what it *is* and the datum moves. The reference every coordinate is measured from. |
| `fix` | an exact position in code: a datum plus an optional interior AST path. |
| `callsign` | the mutable dotted name bound to a datum (`pkg.mod.Class.method`). A rename rebinds the callsign; the datum does not change. |
| `handle` | `callsign@datum-prefix` — a name with its identity pinned, e.g. `Reaper.sweep@e03f`. Typed anywhere a name is accepted; the prefix auto-lengthens like a git short SHA. |
| `leg` | one edit edge in the patch DAG: a definition's predecessor and successor identities. |
| `log` | the whole patch DAG: every leg the repo has sailed. |
| `membership` | derived data beside the chart: every datum the retained chain ever held, stored once, plus one sorted reference list per snapshot. Makes "changed between two snapshots" a merge-diff and "unchanged" an id comparison. Safe to delete; the next survey rebuilds it. |
| `chart` | the semantic graph: every definition and proven edge, drawn in one place. |
| `survey` | an indexing pass producing chart segments. Idempotent by construction: an unchanged tree is the same snapshot, so it cannot be run too often. |
| `snapshot` | one published survey, identified by the hash of its own manifest (`snap:b3:...`); displayed as `@rev`. Content, not a counter. |
| `wake` | a runtime trace from a test run: the path actually traveled. Ingested observations become `observed` edges. |
| `leadline` | a budgeted multi-hop query walk over the chart. Running out of fuel is not an error: the walk returns `aground` with its frontier and can be resumed. |
| `track` | the promised delta: what the spec said would change, before it changed. Checked against the MadeGood by the gate. |
| `made-good` | the actual delta (`MadeGood`): what the diff between two snapshots really did, datum-space with callsign annotations. |
| `xte` | cross-track error: the gate verdict, Track vs MadeGood. Holds when promised work is missing or unpromised work appeared. |
| `sig` | a definition's *interface* identity: blake3 of everything a caller can observe without reading the body. A body-only edit moves the datum and holds the sig; a signature edit moves both. Each frontend owns its canonical form. |
| `drift` | dependents of a changed interface that did not themselves change. What the compiler cannot enumerate across languages, re-exports and processes — and what nobody has even promised to touch. |
| `covenant` | a declared, symmetric coupling between two callsign patterns that no language establishes — a serde boundary, a JSON contract. Written with `grid bind`, stored in `.grid/covenants.json`, and the one modeled evidence that gates, because a human declared it. |
| `zoc` | evidence grade on an edge, strongest first: proven > observed > modeled > heuristic (after CATZOC — real charts grade every sounding's confidence). |

## Evidence grades

| token | meaning |
|---|---|
| `proven` | compiler-grade static evidence: the language's own binding rules establish this edge. |
| `observed` | happened in a wake at least once. |
| `modeled` | established by a bridge (registry, schema, route unification), not by the language. |
| `heuristic` | name or shape match. Never blocks a gate by itself. |
| `stale-trace` | why an observed edge was demoted to `heuristic`: an endpoint of the trace is not in the chart being asked about, so the observation describes code that no longer exists there. The retained wake is never rewritten — decay is a property of the question. |

## Verdicts and records

| token | meaning |
|---|---|
| `pass` | the gate verdict when the code made every promise good: track and MadeGood agree. Exit 0. |
| `hold` | the gate verdict when code diverges from its promise. Fix the code or amend the track, then re-run the gate. Exit 3. |
| `def` | record: a definition. First column is a glyph — `+` added, `-` removed, `~` modified, `>` moved. |
| `ref` | record: a definition that exists (a resolve or scope answer), as opposed to one that changed. Callsign, then datum prefix. |
| `edge` | record: one edge. Change glyph or verdict role first, then `src -relation-> dst`, then evidence. |
| `test` | record: a test comparison, `expected=actual`. |
| `inv` | record: a standing invariant — `standing` when declared, `violated` when the gate found it broken. |
| `hit` | record: a cache hit — datum handle, verdict, when it was recorded. |
| `weak-for` | a test that covered a definition, kept passing while that definition's body changed, and was never itself touched — so it proved nothing about the change. Heuristic: reported as a `warn`, ranked, never a gate. |
| `verdict-stale` | a recorded test verdict that no longer applies: the test's closure key, recomputed now, is not one the cache holds. Identity math, not a guess, so it gates. Reported as `miss <test> closure-changed`. |
| `miss` | record: a cache miss — the datum must be run, not looked up. Not a failure; exits 0. |
| `warn` | record: something reported rather than refused — callsign, then the message. |
| `bind` | record: one declared covenant — pattern A, pattern B, and the boundary word (`grid bind --list`). |
| `suspect` | one drift finding: a dependent, the changed definition it depends on, and the edge between them. Reported as a `drift` record. |
| `conflict` | record: a compare-and-swap failure in `edit apply` — the datum moved since you read it. Re-resolve the name and retry with the fresh `now` coordinate. Exit 4. |
| `done` | record: a fraction — promises made good, edits applied, verdicts recorded. |
| `todo` | record: a promise not yet made good — one item of remaining work. |
| `match` | verdict role: a promised edge the MadeGood made good. |
| `missing` | verdict role: a promised edge never built. Holds the gate. |
| `surplus` | verdict role: an edge built but never promised. Holds the gate. |
| `tolerated` | verdict role: a surplus change the track's tolerate rules excuse. Reported, never a hold cause. |

## Exit codes

`0` pass · `1` internal error · `2` bad input or ambiguous name ·
`3` divergence (gate hold) · `4` stale coordinate (`edit apply` conflict).
Uniform across verbs; see `docs/PORCELAIN.md`.
