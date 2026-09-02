---
name: grid
description: >
  Query this repository's own code chart with the `grid` binary: where is X
  defined, who calls X, what surrounds this definition, which definitions
  have no proven caller, what to refactor first. Use before grep whenever
  the question is about definitions or relationships rather than text —
  resolve names to file:line, list proven callers cross-crate, get a
  context pack around an edit, rank refactor candidates by complexity and
  churn.
---

# grid

`grid` charts every definition in the repo with a content-derived identity
and every relationship it can prove. Answers are exact and machine-checkable
where grep answers are guesses. Grid charts itself, which is both the best
corpus it owns and a bootstrap hazard: a grid bug can block a grid fix, so
`grid`'s answers are evidence, never the last word.

Applicability check, first use in a session: run `grid version`. If the
binary is absent, build it — `cargo build -p grid` — or fall back to grep.
Grid needs a git worktree and charts Rust and Python.

Run `grid survey` once at the start of a task (idempotent, fast), then
query. Every verb takes `--json` for the wire result; porcelain columns on
stdout are stable and documented in `docs/PORCELAIN.md`. Exit codes: 0 pass,
1 internal, 2 bad input, 3 divergence, 4 stale. State lives in `.grid/` —
all caches, safe to delete, rebuilt by the next survey — except
`.grid/invariants.json` and `.grid/covenants.json`, which are declarations a
human made and no survey can rebuild.

## When to reach for it

- "Where is X defined?" — `grid resolve X` returns callsign, identity hash,
  and `file:line`. Works backwards: `grid resolve crates/chart/src/lib.rs:145`
  names the enclosing definition, and passing just the file lists everything
  it contains. A miss is exit 2 with the closest callsigns on stderr — take
  the suggestion instead of retrying blind.
- "What are names rooted at?" — `grid roots` prints the import-name →
  directory map (`grid_chart` → `crates/chart`; Python roots at the package).
- "Who calls X?" — `grid uses X` lists proven inbound edges, cross-crate,
  through re-exports. `--relation implements` answers "who implements this
  trait".
- "What surrounds this edit?" — `grid scope X --depth 2` is the context
  pack: the definition with its location, proven callers and callees, and
  any standing rules that reach it. Read this before reading whole files,
  then pull the named spans in one call rather than one read per span.
- "What did my change actually move?" — bare `grid` is the everyday verb:
  what changed against what the track promised. `grid check` is the
  composite — drift, stale and weak tests, compatibility findings, and the
  gate verdict, each section the standalone verb's own answer.
- "What else might this have broken?" — `grid drift` names the dependents
  whose own datum did not move; `grid suspects --closure` ranks them.
- "What should be refactored?" — `grid hotspots --under crates/` ranks by
  complexity × edit churn. A ranking, never a verdict.
- "Is this dead?" — `grid orphans --under <path>` lists definitions with no
  proven caller. That is weaker than dead: public API, tests, and
  macro-reached code all match.
- Any word or id in grid output — `grid explain <word>` defines it, from
  `docs/ONTOLOGY.md` and the wire contract. `grid help` lists every verb.

## Rules of the road

- **An empty answer means "cannot prove", not "does not exist".** Grid
  declines glob imports, macro bodies, and typed receivers rather than
  guess, and every decline is counted in the report. Never conclude code is
  unused or a name is absent from a refusal alone; fall back to grep to
  close the gap.
- Names are dotted callsigns rooted at the crate's import name:
  `grid_chart.build`, not `crates/chart/src/build.rs`. `Foo.m` is an
  inherent method, `Foo@T.m` a trait impl.
- Pin identity across turns with a handle, `name@hexprefix`, from any grid
  output. The pin survives renames and fails loudly (exit 4) when stale.
- A `Grade::Modeled` hop is a static table's answer, not the language's. It
  is deterministic and it never becomes proven — do not report it as one.
- Gitignored files are never charted, tracked or not. If a whole directory
  is unexpectedly missing from answers, check the ignore rules first.
- Do not use `grid edit` to modify this repository — the ordinary edit tools
  are the write path here, and `edit.apply` is the code under test.
