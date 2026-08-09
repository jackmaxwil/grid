# grid-survey

Single responsibility: extraction: repo -> defs + edges (language adapters: py
first, ts via Oxc, rs).

A survey turns a working tree into a chart segment. Discovery is `git
ls-files`, so the repo's ignore rules are the only policy; identity comes from
`grid-datum`, so a reformatted file is the same chart.

Two properties are load-bearing:

- **Determinism.** The same tree surveys to byte-identical segments on any
  machine, in any order. The snapshot id is the hash of the result (D5), so an
  identical re-survey is a no-op rather than a duplicate.
- **Incremental ≡ full.** The cache is keyed on file content hash and holds
  only *per-file* facts — definitions and unresolved references. Everything
  cross-file is recomputed, so an edge whose target changed cannot survive.
  That is why the cache stores references rather than edges: it is the
  difference between an optimization and a second, diverging implementation.

Edges are `proven` only. A name binds to a datum when the language's own rules
say it does and exactly one definition answers; an ambiguous or unresolvable
name yields no edge at all. `obj.method()` where `obj` is a parameter needs
types, which are `modeled`, so this wave records nothing for it.
