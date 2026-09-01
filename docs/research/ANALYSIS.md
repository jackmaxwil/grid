# Deep analysis: the four research references

2026-09-01. Four parallel deep-reads against grid's architecture and the
drift plan (`docs/plans/2026-09-01-drift.md`). Verdicts here supersede
the one-line takes in `README.md` where they differ — notably the DAG
reachability paper, which a close read disqualifies.

The cross-cutting result first: three of the four papers independently
validate grid's core bets from different directions. Stack graphs prove
at GitHub scale that *file-local facts + query-time composition* is the
right decomposition (grid's phase-1/2/3 survey). HyperAST proves that
*identity precomputed once, equality as id-comparison* is what makes
history-scale analysis cheap (grid's datum). DBSP proves that
*incremental ≡ full* is not just a test but an algebra — and that grid's
"cold survey equals one giant delta" is a theorem, not a convention.
None of the four requires a new dependency to exploit.

---

## 1. Stack Graphs (Creager & van Antwerpen, EVCS 2023, arXiv:2211.01224)

**Mechanics.** Four node types per file subgraph: root, scope, push
symbol (references), pop symbol (definitions). Symbols include operators
`.` and `()` — member access and calls ride the same stack as names. A
name binding is a *complete path*: reference → definition with an empty
symbol stack; push prepends, pop guards and consumes. Edges never cross
files; the only cross-file hop is a *virtual edge* between root nodes,
created at query time, never stored. Type-dependent lookup (`B().x`)
works by keeping pending lookups paused on the stack — the expressive
power of van Antwerpen scope graphs without their eager cross-file
edges, which is what destroys file-incrementality there. *Partial
paths* (pre/postcondition symbol stacks, variable-tailed) shift
path-finding work to index time while staying file-local — the
persisted artifact per file version is its partial-path list, keyed by
git blob id. Runs in production on every Python push to GitHub since
Nov 2021; the paper has latency goals (<100ms query) but no benchmark
tables.

**Fit.** Grid's survey is already the same decomposition; the paper
adds what a flat `DefIndex` cannot express:

- Chained/type-dependent lookups (`foo().bar`, chains through
  re-exports) — today unresolvable refusals even where binding rules do
  fix them.
- Inherited-member lookup routed through the superclass *reference*,
  resolved lazily — no globally materialized MRO.
- Re-export chains and barrel files as pure push/pop plumbing — the
  killer capability for the planned swc TS frontend.
  `tree-sitter-stack-graphs-typescript` is an existing, debugged
  encoding of the TS rules worth reading before writing `resolve_ts`,
  even though grid parses with swc.
- Partial paths are the theoretically sound cacheable middle object
  between grid's "unresolved reference" fact and a resolved edge —
  file-local, so cacheable under the existing blake3 file key.

**Tension — the one that matters.** The paper's precision stance is the
opposite of grid's: footnote 7 deliberately permits ambiguous and
missing bindings (recall-first, for navigation UX). Grid adopting the
machinery must impose its own acceptance policy: exactly one complete
path via purely syntactic rules → proven; multiple complete paths →
refusal (counted); a path using a type-dependent hop → modeled at best.
Also constrain virtual edges to grid's resolved package roots (the
paper allows any-root-to-any-root, a deliberate overapproximation).
Determinism: no conflict — result sets are deterministic; only
discovery order is incidental, and grid already canonicalizes by
sorting. Note the partial-path completeness proof is deferred to a
future paper; grid's incremental-vs-full differential test covers that
gap.

**Steal:** partial-path-shaped reference facts in phase 1 (survey,
lang-rs/lang-py fact schema); push/pop resolver for the TS frontend;
the file-local-edges invariant stated as the formal reason "edges are
never cached" (ASSURANCE.md); the grade policy above. **Skip:**
tree-sitter and the DSL (grid's real parsers are a feature; 3 languages,
not 500); query-time-only resolution (grid materializes edges at survey
time — keep that); multi-result UX.

---

## 2. HyperDiff (Le Dilavrec et al., ESEC/FSE 2023)

**Mechanics.** HyperAST: one DAG for an entire git history, subtrees
deduplicated globally across all commits down to AST-node granularity —
structural equality precomputed at construction, so GumTree's
isomorphism test becomes O(1) id-comparison. Per-subtree metadata
(height, size, structural hash, label hash) rides on the node. Diffing
uses a lazily decompressed struct-of-arrays overlay; subtrees shared
between the two versions are matched at the top by reference equality
and their interiors are never decompressed at all.

**Eval.** 19 Java projects up to 1.6M LoC / 33k commits. 99.3% of
18,092 diffs bit-identical to GumTree; memory ×4.5 better per AST node
(0.278 bytes/node amortized in the DAG); total diff time ×1.2–×12.7,
top-down phase alone ×3.2–×233; including parsing, ×14.5 median over
GumTree-Spoon, with baselines OOMing on the largest repos. Gains grow
with codebase size and shrink with diff size — cost decorrelated from
tree size, correlated with change size.

**Fit.** The load-bearing idea is not the AST DAG — it is: *identity
stored once globally, snapshots as cheap reference sets into it*, so
cross-snapshot questions become set algebra on stable ids instead of
patch replay. Grid already has the identity half (datum ≈ label hash,
shape ≈ structural hash — grid's narrower name-only erasure is the
better rename key; nothing to change). What grid lacks is the access
structure: `log`/`bisect` and drift step 6 currently walk
patch-by-patch, which is the per-pair recomputation GumTree did.

**Steal** (all additive to the segment store, no redesign):

1. Global dedup def table keyed by datum + per-snapshot sorted
   datum-id reference lists — history queries become merge-joins.
2. Per-shape-key runs `(datum, first_snapshot, last_snapshot)` —
   `grid log` becomes a scan, `bisect` a binary search.
3. "Unchanged = id-equal" as drift's fast path — never load a def or
   test whose datum matches across endpoints (the single trick behind
   their ×226).
4. Cheap metadata (size, height) beside the datum to rank shape-join
   candidates in rename resolution.
5. Conditionally: blob-OID-keyed file cache for bulk historical
   ingest, if bisect ever surveys many commits (HyperAST's economics
   at file granularity, free — git blob ids are already content
   hashes).

**Skip:** sub-definition subtree DAG (storage redesign for
intra-changed-file sharing grid doesn't need at 38ms/survey); the lazy
decompression machinery; GumTree/Chawathe edit scripts (grid's diff
unit is the definition). Their one un-lazified stage eats 26.6% of
runtime — the lesson for drift is to stream output over the merge-join,
never materialize per-snapshot def sets.

---

## 3. DBSP (Budiu et al., VLDB 2023 best paper)

**Mechanics.** Streams over an abelian group; four operators — lift,
delay `z⁻¹`, differentiation `D(s) = s − z⁻¹(s)`, integration
`I = Σ` — with `I∘D = D∘I = id`. Relations live in Z-sets (rows with
integer weights; negative = retraction). Incrementalization is
definitional — `Q^Δ = D∘Q∘I` — made cheap by rewrite rules, above all
the chain rule `(Q1∘Q2)^Δ = Q1^Δ∘Q2^Δ`: incrementalize a composite
query sub-query by sub-query, mechanically. Linear operators (filter,
project, map, group-by, count/sum) are *their own* incremental form:
zero state, delta in, delta out. Bilinear operators (joins) need the
integrals of both inputs as state:
`Δ(a⋈b) = Δa⋈Δb + a⋈Δb + Δa⋈b`. Recursion (transitive closure)
incrementalizes via nested streams — correct under arbitrary deletions,
but the expensive case. All theorems machine-checked in Lean. Cold
start is "feed the whole database as one big change," correct by the
same algebra.

**Fit.** Drift-suspects as a query is **bilinear, not linear**: one
equi-join (`EDGE ⋈ CHANGED`) plus an antijoin. DBSP's cost model then
dictates exactly what a live watch mode must hold as state: the edge
table (grid's segment already is `I(ΔEDGE)`) and the changed-sig set —
and nothing else. Per file event: Δsigs from re-hash, reverse-edge
probe, three-term bilinear formula, emit signed suspect deltas.
Closures are the recursive case — DBSP confirms incremental
maintenance is possible but is the nested-stream expensive path; grid's
budgeted invalidate-and-rewalk stays the right call.

**Steal:**

1. **Signed deltas on the wire** — watch-mode drift events as
   `{def, weight ±1}` (suspect appeared/retracted), never "current
   list". The biggest single steal; makes the daemon contract
   composable and testable by summation.
2. Linear/bilinear/recursive labeling of every maintained view in
   ASSURANCE.md — each watch-mode feature declares its state up front.
3. Four tests (~200 lines, no deps): sequence-integration
   (`integrate(Δouts) == full` after every prefix), revert/retraction
   (edit-then-undo nets to zero — the group inverse law),
   cold-start-as-one-delta (collapses watch mode and incremental≡full
   into one property), plus the existing differential test (which is
   the theorem `Q^Δ = D∘Q∘I` checked pointwise).
4. Minimal watch mode in `grid serve`: the bilinear formula
   hand-derived against the resident segment. No circuits, no operator
   framework — two queries do not justify one.

**Skip:** the DBSP framework itself; nested-stream incremental
closures; Z-set generality beyond weights {−1, +1}; any general
circuit API.

---

## 4. DAG decomposition reachability (Kritikakis & Tollis, arXiv:2212.03945)

**Mechanics.** Linear-time chain-decomposition heuristics (best: node-
order with concatenation via reversed DFS whose dead ends are deleted
from the graph, bounding total cost); a Simon-'88-style index storing
per vertex one entry per chain (lowest reachable position), built in
O(k_c·|E_red|) by processing vertices in reverse topo order and paying
the k_c-wide merge only on non-transitive edges. Queries are genuine
O(1) *pair-membership* tests. Evaluated only on synthetic graphs ≤160k
nodes against full transitive closure; no query latencies, no real
graphs, no dynamic version.

**Verdict: disqualified for grid — a close read reverses the README's
one-liner.** Four independent reasons:

1. **Wrong regime.** k_c ≈ graph width ≈ |V|/avg_degree. Code graphs
   are sparse (degree ~2), so a 10⁶-def monorepo has width ~10⁵ →
   index space k_c·|V| ≈ 10¹¹ entries. The paper's sweet spot is dense
   graphs; code graphs are the opposite.
2. **Wrong primitive.** The index accelerates "does A reach B?"; grid's
   queries are set-producing (`uses` = full reach set, closures = full
   set). Enumerating a reach set from the index is still O(answer) —
   no faster than BFS over CSR.
3. **No dynamic support.** A non-transitive edge change forces a full
   rebuild, and sparse graphs have almost no transitive edges — so
   nearly every edit invalidates the index.
4. **No filtered queries.** The index encodes one fixed edge set; grid
   filters every walk by grade floor × relation subset. One index per
   filter combination, each invalidated per edit.

**Salvage — three small pieces, none actually requiring the paper:**

1. **SCC condensation with shared per-SCC closures** (the real win):
   defs in one SCC have identical reach sets; compute the closure once
   per SCC representative, key members' verdicts to it. Deterministic
   Tarjan, ~150 lines, pays at any scale where recursion and mutual
   calls exist. Per-filter-view, matching grid's existing closure
   kinds.
2. Topo-ordered adjacency in CSR — structural determinism plus cheap
   already-covered pruning in walks.
3. H3 chain-count (~50 lines, linear) as *instrumentation only*: run
   once per chart build; the measured width settles empirically — with
   data, not argument — whether any reachability index could ever pay.

---

## Consequences for the drift plan

Amendments applied to the plan on 2026-09-01 (steps 6 and 9):

- **Step 6 (green-but-stale) and `log`/`bisect`**: build on the
  HyperDiff steal — snapshot-membership segments (global def table +
  per-snapshot sorted datum-id lists + shape-key runs). Turns the
  cross-snapshot four-way join into streaming merge-joins. This is
  additive segment work and arguably belongs *before* step 6 lands.
- **New step (watch mode, after 1–7)**: DBSP-shaped suspect
  maintenance in `grid serve` — signed-delta wire contract, the two
  declared state tables, the four algebra tests. This is the original
  brainstorm's "intelligent file watcher," now with its theory and its
  minimal correct form.
- **TS frontend plan (separate)**: stack-graph push/pop resolution
  with grid's acceptance policy (unique path → proven; ambiguity →
  counted refusal; type-dependent hop → modeled). Read
  tree-sitter-stack-graphs-typescript's rules before writing
  `resolve_ts`.
- **Chart/cache (independent)**: SCC-shared closures.
- **Research README**: DAG paper verdict corrected (done in this
  file; README updated alongside).
