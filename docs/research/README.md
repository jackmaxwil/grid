# Research references

Papers pulled during the 2026-09-01 drift ideation (see
`docs/plans/2026-09-01-drift.md`). Each is here because it touches a
problem grid actually has, not for general interest.

| file | paper | why grid cares |
| --- | --- | --- |
| `stack-graphs-2211.01224.pdf` | Creager, *Stack Graphs: Name Resolution at Scale* (EVCS 2023, arXiv:2211.01224) | File-incremental name resolution powering GitHub Precise Code Navigation. Same decomposition as grid's survey (per-file facts, global resolve, never cache cross-file conclusions). Roadmap material for the TS frontend: declarative per-language binding rules. |
| `hyperdiff-fse2023-hal-04189855.pdf` | Le Dilavrec, Khelladi, Blouin, Jézéquel, *HyperDiff: Computing Source Code Diffs at Scale* (ESEC/FSE 2023) | HyperAST: content-addressed DAG sharing unchanged subtrees across a whole commit history — the datum idea below def granularity, history-wide. Relevant if `grid log`/history joins (drift plan step 6, co-change) ever need one walk instead of per-patch walks. |
| `dbsp-ivm-vldb2023.pdf` | Budiu et al., *DBSP: Automatic Incremental View Maintenance for Rich Query Languages* (VLDB 2023, best paper) | The theory of the live drift watcher: query-as-circuit, mechanically incrementalized, with a correctness algebra. Grid's "incremental ≡ full" invariant is IVM correctness stated as a test. Adopt the discipline for `grid serve` watch mode, not the framework. |
| `dag-reachability-2212.03945.pdf` | Kritikakis & Tollis, *Fast and Practical DAG Decomposition with Reachability Applications* (arXiv:2212.03945) | Chain-decomposition reachability index. **Deep read disqualified it** (see ANALYSIS.md): index space is width × defs and code graphs are sparse/wide; it answers pair-membership while grid needs reach sets; no dynamic rebuild; no filtered queries. Salvage: SCC condensation with shared per-SCC closures. |

Also considered and passed over: Duan–Mao–Mao–Shu–Yin directed SSSP
(arXiv:2504.17033) — no fit (grid's walks are unweighted BFS), but its
partial-execution contract inspired the certified-aground step of the
drift plan.

`ANALYSIS.md` holds the full deep-read of all four papers: mechanics,
evaluation numbers, fit against grid's architecture, steal/skip
verdicts, and the resulting recommended amendments to the drift plan.
