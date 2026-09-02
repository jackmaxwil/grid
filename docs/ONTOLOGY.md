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
| `position` | an exact position in code: a datum plus an optional interior AST path. (after *fix*, a navigator's fix: the place you are, established from evidence rather than guessed) |
| `callsign` | the mutable dotted name bound to a datum (`pkg.mod.Class.method`). A rename rebinds the callsign; the datum does not change. |
| `handle` | `callsign@datum-prefix` — a name with its identity pinned, e.g. `Reaper.sweep@e03f`. Typed anywhere a name is accepted; the prefix auto-lengthens like a git short SHA. |
| `edit` | one edit edge in the patch DAG: a definition's predecessor and successor identities. (after *leg*, one run between two waypoints of a passage) |
| `history` | the whole patch DAG: every edit the repo has sailed. (after the *log* as a noun, a ship's log book; the verb `grid log` keeps its git-familiar name) |
| `membership` | derived data beside the chart: every datum the retained chain ever held, stored once, plus one sorted reference list per snapshot. Makes "changed between two snapshots" a merge-diff and "unchanged" an id comparison. Safe to delete; the next survey rebuilds it. |
| `chart` | the semantic graph: every definition and proven edge, drawn in one place. |
| `survey` | an indexing pass producing chart segments. Idempotent by construction: an unchanged tree is the same snapshot, so it cannot be run too often. |
| `snapshot` | one published survey, identified by the hash of its own manifest (`snap:b3:...`); displayed as `@rev`, or `@rev+<prefix>` when surveyed from a dirty worktree — the rev is where that tree was checked out from, not what it is, so a rev lookup never answers with it. Content, not a counter. |
| `damaged` | a chart whose bytes contradict its own manifest: a segment that no longer hashes to the name the manifest holds, a manifest that does not parse, a named segment that is gone. Every read verb refuses it as bad input (exit 2) with the tree and the command that fixes it; `grid survey` is that command — it drops the whole chart, surveys cold, and says what it dropped. Never served, never read around. |
| `trace` | a runtime trace from a test run: the path actually traveled. Ingested observations become `observed` edges. One trace is one test, and a coverage report is the usual source: `grid trace ingest --lcov <f> --test <name>` reads `cargo llvm-cov`'s report, `--coverage-json <f>` reads coverage.py's `coverage json` report, and both map executed lines onto definitions against the live snapshot, which the trace records. The per-runner recipes that produce one report per test are in the README. (after *wake*, the disturbed water a hull leaves behind it) |
| `attribution` | which test a recorded verdict belongs to. A verdict from a traced run carries its trace id and the retained trace names the test; a verdict from an untraced `grid test` carries `test:<callsign>` instead — a namespace, not a trace file, and no trace is implied by it. A verdict carrying neither is `unattributed`: counted, never guessed at. |
| `walk` | a budgeted multi-hop query walk over the chart. Running out of fuel is not an error: the walk returns `aground` with its frontier, the radius it *finished* (`complete_within`), and can be resumed. (after *leadline*, the weighted line paid out by hand until it touches bottom) |
| `aground` | a walk that ran out of fuel with work left. It reports where it stopped (the frontier) *and* how far it is complete: every definition within `complete_within` hops, under that query's filter, is in the answer. A walk whose first layer did not finish certifies nothing and says so. |
| `plan` | the promised delta: what the spec said would change, before it changed. Checked against the diff by the gate. (after *track*, the course laid on the chart before anyone sails it) |
| `diff` | the actual delta: what the change between two snapshots really did, datum-space with callsign annotations. (after *made-good*, the course actually made good over the ground) |
| `verdict` | the gate's answer, plan vs diff. Holds when promised work is missing or unpromised work appeared. (after *xte*, cross-track error: how far off the laid course you actually are) |
| `sig` | a definition's *interface* identity: blake3 of everything a caller can observe without reading the body. A body-only edit moves the datum and holds the sig; a signature edit moves both. Each frontend owns its canonical form. |
| `interface` | a def promise's claim about whether callers can see the change: `true` means only a sig move (or unknown — conservative) makes it good, `false` means only a held sig does. A sig move under a `false` promise, or under no promise permitting one, is `unaccounted` with reason `interface`, which holds the gate. Absent is silent on the question, today's behavior. Rejected on an `add`/`remove` promise by `grid plan lint` — neither has a prior sig to hold or move against. |
| `cfg` | a definition's compile condition: the conjunction of every `#[cfg(...)]` predicate on the item, on each enclosing module (inline, or the `mod x;` that declared its file), and on its cargo target directory (`tests/`, `benches/` mean `test`), as written minus whitespace, `all(...)`-joined when nested; a `cfg_attr(...)` whose attributes can carry a `cfg(...)` is recorded whole (one that expands to a derive, an allow or a doc conditions nothing and is not a predicate). Stored per definition from segment epoch 4; an older snapshot answers *unknown*, never "unconditioned". |
| `compile-record` | what one `cargo check` was asked for — features after defaults, `--all-features`, `--tests`, the target and its cfg set as rustc reports it — written to `.grid/compile.json` by `grid check --compile`. What a check evaluates each dependent's `cfg` against. |
| `interference` | semantic overlap between two worktrees of one repository, against their common base: what each side's edits do to the other's, which git's textual merge cannot see. Reported by `grid interfere` as `drift` records with reason `interfere:<other-root>` — one side's drift suspects that land in the other side's edits. The other worktree's store is read, never written, and never surveyed; one grid cannot read is a refusal, never an answer of "no interference". |
| `both-touched` | a definition whose datum moved on both sides of an interference comparison. Listed for completeness as `warn <callsign>  both-touched` — git's merge finds this one too — and it does not gate: a `DefRef` carries no edge, so there is no evidence grade for it to gate on. |
| `drift` | dependents of a changed interface that did not themselves change. What the compiler cannot enumerate across languages, re-exports and processes — and what nobody has even promised to touch. |
| `persisted` | a standing invariant declaring that bytes on disk were written by an older version of a type: `{"persisted": "app.**Config"}` in `.grid/invariants.json`, written by `grid invariant persisted <pattern>` and listed by `grid invariant --list`. The human fact no static check can establish, and the only thing that turns a Deserialize compatibility finding from a warning into a gate — modeled evidence, exit 3, on the same footing as a covenant. |
| `compat` | a `warn` reason: a changed type deriving `Deserialize` whose new shape cannot read what its old shape wrote. Six decidable rules — a required field added, a field renamed without `alias`, a field's type changed, a variant removed, a field removed under `deny_unknown_fields`, `rename_all`/`tag`/`untagged` changed. Heuristic unless the type is declared `persisted`; a derive the source does not spell out is counted `compat unchecked`, never cleared. |
| `invariant` | a standing rule checked at every gate, whatever the plan: `persisted` (old bytes of a type exist), `forbid-edge`, `require-edge`. Stored in `.grid/invariants.json`, folded into every plan's own, and reported as an `inv` record — `standing` when declared, `violated` when the gate found it broken. `grid invariant persisted <pattern>` writes the one kind a single pattern fully describes; the two edge kinds carry a relation and a grade floor and stay hand-authored, as removing any of them is editing the file. |
| `covenant` | a declared, symmetric coupling between two callsign patterns that no language establishes — a serde boundary, a JSON contract. Written with `grid covenant`, stored in `.grid/covenants.json`, and the one modeled evidence that gates, because a human declared it. As a record: pattern A, pattern B, and the boundary word (`grid covenant --list`) — a record formerly tagged *bind*. |
| `pack` | the context bundle `grid scope` returns for a planned edit: the definitions a pattern named, the chart around them, the rules that reach them, and — when asked — their text, in tiers. Two named tier presets: `--orient` (`--bodies 0 --sigs 2`), landing in an unfamiliar tree, and `--edit` (`--bodies 1 --sigs 3`), the first step of the edit loop; an explicit depth beside either wins. |
| `horizon` | how far out a tiered pack is *complete*: bodies through one hop count, signatures through a further one, plus the token estimate. Budget is spent in whole hop layers, farthest first, and a layer that does not fit is dropped whole (a body layer is demoted to its sigs first), so the depth a horizon names is a depth every definition inside it is present at. The same promise `aground`'s `complete_within` makes about a walk. |
| `grade` | evidence grade on an edge, strongest first: proven > observed > modeled > heuristic (after *zoc*, CATZOC: real charts grade every sounding's confidence). |

## Evidence grades

| token | meaning |
|---|---|
| `proven` | compiler-grade static evidence: the language's own binding rules establish this edge — including an inherent method a written receiver type reaches at the first lookup step, where rustc's own order lets no trait outrank it. |
| `observed` | happened in a trace at least once. |
| `modeled` | established by a bridge (registry, schema, route unification), not by the language; or a method call whose receiver type is written but matched only at a later lookup step, or inferred from a constructor, a literal, or a written return type — correct unless a trait grid cannot see supplies the method. Ranks; gates only under a declaration or `--floor modeled`. |
| `heuristic` | name or shape match. Never blocks a gate by itself. |
| `stale-trace` | why an observed edge was demoted to `heuristic`: an endpoint of the trace is not in the chart being asked about, so the observation describes code that no longer exists there. The retained trace is never rewritten — decay is a property of the question. |
| `declined` | a reference the resolver would not bind to a definition, so no edge exists for it. Reported as a count, never as an absence of coupling, and split three ways because the three mean opposite things for any claim about this repo. Stored per definition from segment epoch 5 (four saturating `u16`: external, method, internal, refused), so a consumer can sum what the chart could not follow inside *this* neighbourhood; an older snapshot answers *unknown*, never zero. |
| `external` | a declined reference whose name was never this tree's to hold: `std`/`core`/`alloc` or another crate in Rust, a top-level module the tree does not root in Python. Harmless to intra-repo soundness. |
| `method` | a declined reference whose callee sits behind a receiver grid could not type — a closure parameter, a pattern binding, a chain through an unwritten return — or typed and refused: a trait candidate, a generic impl, no inherent method; the refused share is counted. A call whose receiver's type is written and outside the tree is `external`. On a Rust tree this is the largest population, and seventy percent of it has no syntactic type at all. |
| `refused` | a `method` decline with a reason: the receiver was typed to a charted type and the method-call rule declined — a trait grid can see supplies the method (in the tree, an impl on the type, or the prelude), the inherent impl is bounded or instantiated, no inherent method of that name exists, or the type carries generic arguments. A sub-count of `method`, never a fourth population. |
| `internal` | a declined reference to a name this tree could have held and the resolver could not bind: supplied by a glob import, an ambiguous callsign, an unresolved re-export. The population that is an actual hole in what `grid drift` claims. |
| `receiver` | what a file states about a method call's receiver type, recorded beside the reference as a hint: the type as a path, its form (`&`, `&mut`, by value) and its origin — *written* (a parameter or `let` annotation, a declared field type) or *inferred* (a constructor, a struct literal, a chain through a written return, `?`/`unwrap` on a written `Option`/`Result`). A closure parameter, a pattern binding, a `for`, a chain through an unwritten return carry none, and the call stays declined. |

## Verdicts and records

| token | meaning |
|---|---|
| `pass` | the gate verdict when the code made every promise good: plan and diff agree. Exit 0. |
| `hold` | the gate verdict when code diverges from its promise. Correct the code or amend the plan, then re-run the gate. Exit 3. |
| `def` | record: a definition. First column is a glyph — `+` added, `-` removed, `~` modified, `>` moved. |
| `ref` | record: a definition that exists (a resolve or scope answer), as opposed to one that changed. Callsign, then datum prefix. |
| `edge` | record: one edge. Change glyph or verdict role first, then `src -relation-> dst`, then evidence. |
| `test` | record: a test comparison, `expected=actual`. |
| `inv` | record: a standing invariant — `standing` when declared, `violated` when the gate found it broken. |
| `hit` | record: a cache hit — datum handle, verdict, when it was recorded. |
| `weak-for` | a test that covered a definition, kept passing while that definition's body changed, and was never itself touched — so it proved nothing about the change. Heuristic: reported as a `warn`, ranked, never a gate. |
| `verdict-stale` | a recorded test verdict that no longer applies: the test's closure key, recomputed now, is not one the cache holds. Identity math, not a guess, so it gates. Reported as `miss <test> closure-changed`. |
| `selection` | the inbound walk `grid test` runs from every definition whose datum moved, over every grade plus the retained traces' observed edges, that decides — per test — `selected` or `replayed`. Deterministic for a given pair of snapshots; adding a trace never adds a selection. |
| `selected` | a test `grid test` must run: the inbound walk from the changed set reached its definition (`closure-changed`), no live trace watched it (`no-trace`), the chart cannot name it (`unresolved`), the walk ran aground (`aground`), or nothing is on record to replay (`not-recorded`). Every reason is a refusal to skip. |
| `replay` | reusing a recorded verdict from the cache instead of rerunning a test — what a `replayed` test gets. Licensed only by a live trace over an unmoved closure; `--no-replay` (or `--all`) runs the inventory instead. |
| `replayed` | a test `grid test` did not run: its closure key at head, in this environment, is one the cache holds, and a live trace watched its reach. Printed as a `hit` with the recorded verdict and time. |
| `no-trace` | why a test is selected regardless of the walk: no retained trace still describes code base holds. The chart is known incomplete, so the static closure alone never licenses a skip. |
| `nondeterministic` | the same test, the same closure key, the same environment, the opposite verdict. The closure proves the charted code did not change, so the claim is exactly "not the charted code" — a race is code, and grid does not say *flake*. A `warn`, detected at record time from the last entry. |
| `uncovered` | a changed definition the selection walk connected to no test. Heuristic — "no charted path" is not "no path" — so a `warn` that ranks and never moves the exit code. |
| `clone-of` | a definition added since base whose *shape* — the name-erased canonical hash — is byte-identical to an existing definition's. Exact match only; a near-duplicate is not claimed. Heuristic: `warn <new>  clone-of <existing>` from `grid check`, ranked, never a gate. |
| `refactor` | `grid check`'s certificate: one `done  refactor  interfaces held (or N interfaces moved) · <n> covering tests replayed pass · <m> changed defs uncovered` record when no gate fired, over the check's own data — drift's changed set, the retained traces' reach, the verdict log. Three facts and nothing more: it never says *behavior-preserving*, because grid holds no evidence for that sentence. With no retained trace the two counts are unknown and it says `no retained traces`. |
| `check` | the composite verb an agent runs after an edit: every section is the corresponding verb's own result (drift, the compile record, the gate verdict under `--plan`, `drift --tests`'s stale and weak lists, `drift --compat`'s findings, `grid test`'s uncovered walk, clone-of). Gates exit 3; the tail ranks; a section that refuses — aground, an unreadable store, an unknown snapshot — refuses the whole check with its reason (exit 1 or 2), never a report with a section missing. `--format md` renders the same result as one markdown document for a pull-request comment, and `--json` is always the record form. `--quick` is drift at the proven floor plus the compile record, for a hook, and it says an unchanged count once — the declined-reference line and the no-compile-record warning are printed only when their text moved since the last `--quick` in this store. |
| `env_hash` | the environment a verdict is keyed under: blake3 over the toolchain version, the lockfile bytes, the runner args, the target triple, and whether the run was traced. Stated in `grid test --json` so a verdict can be re-keyed later. |
| `miss` | record: a cache miss — the datum must be run, not looked up. Not a failure; exits 0. |
| `warn` | record: something reported rather than refused — callsign, then the message. |
| `delta` | record: one signed change to the drift suspect set — `+` appeared, `-` retracted (`grid watch`). Consumers hold a running sum; the sum after any prefix of events equals a fresh `grid drift`. |
| `suspect` | one drift finding: a dependent, the changed definition it depends on, and the edge between them. Reported as a `drift` record. |
| `closure` | `grid suspects`'s second evidence mode, and the word its `hit` records carry where an Ochiai score would sit: the definitions the test's transitive closure at head can reach whose datum a named snapshot did not hold — a body that moved, or one that arrived since — nearest hop first. Needs no run history, which is the case that has none — the test passed at the last snapshot and fails now; with no snapshot named, the flag uses the last one the verdict log says that test passed at, and refuses when nothing on record says. Heuristic: reachable and changed is not a cause, so it ranks and never moves the exit code. |
| `uncompiled` | a `drift` reason: `uncompiled:<cfg>` — a proven dependent of a changed interface whose `cfg` the recorded compile decidably excluded, so the compiler never saw it call the old interface. Proven, so it gates (exit 3). A `cfg` the evaluator cannot decide (`cfg_attr`, a custom key such as `kani`, a target fact the record lacks, a pre-epoch-4 snapshot) is `warn <dependent>  cfg unknown: <text>` instead — an unproven exclusion never holds. |
| `conflict` | record: a compare-and-swap failure in `edit apply` — the datum moved since you read it. Re-resolve the name and retry with the fresh `now` coordinate. Exit 4. |
| `done` | record: a fraction — promises made good, edits applied, verdicts recorded. |
| `todo` | record: a promise not yet made good — one item of remaining work. |
| `match` | verdict role: a promised edge the diff made good. |
| `missing` | verdict role: a promised edge never built. Holds the gate. |
| `surplus` | verdict role: an edge built but never promised. Holds the gate. |
| `tolerated` | verdict role: a surplus change the plan's tolerate rules excuse. Reported, never a hold cause. |

## Former names

Every word this vocabulary retired, and what it is called now. `grid
explain` reads this table: an old word still answers, with the new word's
definition and the sentence "formerly `<old>`". Nothing emits the left
column any more.

| old | new |
|---|---|
| `bind` | `covenant` |
| `fix` | `position` |
| `leadline` | `walk` |
| `leg` | `edit` |
| `log` | `history` |
| `made-good` | `diff` |
| `track` | `plan` |
| `wake` | `trace` |
| `xte` | `verdict` |
| `zoc` | `grade` |

## Exit codes

`0` pass · `1` internal error · `2` bad input or ambiguous name ·
`3` divergence (gate hold) · `4` stale coordinate (`edit apply` conflict).
Uniform across verbs; see `docs/PORCELAIN.md`.
