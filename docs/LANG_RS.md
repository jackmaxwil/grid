# grid-lang-rs — the Rust frontend

Status: implemented (`crates/lang-rs`, plus the Rust resolver arm in
`grid-survey`). The scheme below was argued on paper before it became a
cache key; "What implementation taught" at the bottom records where the
paper was wrong or silent.

## Goal

A `grid-lang-rs` crate that surveys Rust source into the `grid-datum`
vocabulary — definitions, references, imports — plus a Rust arm in
`grid-survey`'s resolver, holding the same discipline the Python frontend
holds: an edge is recorded only when the language's own binding rules fix
its target, and everything else resolves to nothing. First corpus: grid
itself. A Rust frontend that cannot chart its own repository is not done.

## Non-goals (v1)

- Macro expansion. A `#[derive]`-generated impl or a call inside a macro
  body is invisible: a missing definition or edge is an omission, and
  omissions are permitted; invented edges are not.
- Trait-method receiver resolution (`x.frobnicate()` where the method comes
  from a trait in scope). Needs types; types are `modeled`, and this wave
  emits no modeled edges — same rule that keeps `obj.method()` out of the
  Python chart.
- `#[path]` attributes and non-conventional module layouts.
- TS/JS. That is a third frontend, not this one — `swc_ecma_parser` (pure
  Rust, the parser under Next.js/Vite toolchains, near-complete test262
  coverage) is the standing candidate.

## Parser: `syn`, not tree-sitter

tree-sitter was the assumed answer; it is the wrong one here. It is a C
library (breaks ASSURANCE's "no C in the build"), its grammar crates are
per-language C blobs, and it yields a lossy CST with no notion of Rust's
name grammar. `syn` (with `full` + `proc-macro2` `span-locations` for byte
spans) is pure Rust, parses the entire language, is MIT/Apache, and its
`visit` module is the same shape as the rustpython `Visitor` the Python
frontend already uses. One new TCB line, zero C. tree-sitter re-enters the
conversation only for languages with no mature pure-Rust parser — and its
upstream is a warning sign anyway: GitHub archived stack-graphs (its
tree-sitter-based precise-navigation layer) on 2025-09-09.

Two consequences of syn, both load-bearing:

- `proc_macro2::Span::byte_range` is accurate *outside* a proc-macro
  context, which is exactly where a survey runs — the accuracy caveat in its
  docs applies only inside macro expansion, which we never do.
- syn drops plain comments but keeps doc comments **as `#[doc = "..."]`
  attributes in the AST**. Left alone they would enter the canonical bytes
  and a doc edit would move the datum — breaking the invariant the ontology
  states ("rewrite its comments — the datum holds"). The normalizer
  therefore erases doc attributes before hashing, the same way the Python
  normalizer never sees `#` comments. Plain-comment metadata (lang-py's
  `Comment` list) is omitted at v1: syn cannot see them, no consumer exists
  yet, and a lexer-level comment scan is additive later.

## Callsign scheme (the load-bearing decision)

The ontology fixes a callsign as a *dotted* name, and `DefIndex` files every
symbol under `{module}.{callsign}`. Rust callsigns therefore use `.` as the
segment separator — Rust identifiers cannot contain `.`, so the mapping from
`::` paths is injective. The frontend's spellings:

| item | callsign (within its module) |
|---|---|
| `fn free()` | `free` |
| `struct Foo` / `enum E` / `union U` / `trait T` | `Foo` |
| inherent `impl Foo { fn m() }` | `Foo.m` |
| trait impl `impl T for Foo { fn m() }` | `Foo@T.m` |
| nested `fn` / items in `fn` bodies | `outer.inner`, as Python nests |

`@` appears in no Rust identifier and no Python callsign, so `Foo@T.m`
cannot collide with anything either frontend emits. `T` in `Foo@T` is the
trait path as written at the impl, dots for `::`, generics erased. Generic
parameters and lifetimes are erased from callsigns everywhere: the callsign
is an address, the datum is the identity. Two impls that erase to the same
callsign with different datums (e.g. `impl Frob<u8> for Foo` and
`impl Frob<u16> for Foo`) collide into `DefIndex`'s existing ambiguity set
and become unresolvable by name — refusal, not a wrong edge, and the
ambiguity count already surfaces it. `#[cfg]`-gated duplicate impls (the
unix/windows pair defining the same method twice) land in the same refusal:
two datums under one name, unresolvable, counted.

Prior art says this is the right level of care: rust-analyzer's SCIP
indexer spells impls `impl#[SelfType]` / `impl#[SelfType][TraitType]` — the
same self-type + trait-type pair as `Foo@T` — and got there only after
shipping ambiguous inherent-impl symbols (rust-analyzer #18772), while
items nested inside functions *still* index without their enclosing
function's name (#18771, open). The scheme above carries nesting and the
trait discriminant from day one.

`DefKind` maps without new variants: `fn` → `Function`; `struct`, `enum`,
`union`, `trait` → `Class` (a base-position reference to one is
`implements`, construction is `constructs` — the existing relation rules
hold as-is).

## Module paths

`module_of` is Python-shaped and becomes frontend-dispatched. The Rust rule,
path-derived and deliberately dumb: strip `src/`, strip `.rs`, `mod.rs` and
`lib.rs`/`main.rs` fold into their directory, the crate's directory name is
the root segment, `/` becomes `.`. So `crates/chart/src/lib.rs` roots
`chart`, and `crates/survey/src/resolve.rs` is `survey.resolve`. Ceiling:
this is the file tree's opinion, not cargo's — a crate whose package name
differs from its directory (hyphens become underscores in the real crate
name), a `#[path]` module, and cargo's extra target roots (`src/bin/*.rs`,
`tests/`, `examples/`, `benches/` are each their own crate root, not
submodules) all file under approximate names. Documented, not defended; the
fix (reading the nearest `Cargo.toml` and its target table) is additive
later, and a wrong module name costs an unresolved edge, never a false one.

## References and imports

The existing vocabulary fits Rust better than expected; no core changes:

- `use pkg::mod::Item as I` → `Import { alias: "I", target: "pkg.mod.Item",
  scope }` — the same shape as a Python from-import. `crate`, `super`, and
  `self` stay as literal head segments in the target, and the Rust resolver
  arm anchors them against the referencing module (Python's leading-dot
  convention maps badly to `crate::`, so Rust spells its own anchors).
- A path call `free()`, `mod::f()`, `Foo::new()` → `RefKind::Call` with the
  dotted path as written.
- `impl Trait for Foo` → `RefKind::Base` from callsign `Foo` to the trait —
  the `implements` edge. Trait bounds in generics are *not* Base; a bound is
  a constraint, not an implementation.
- Attribute macros (`#[derive(Serialize)]`, `#[test]`) → `RefKind::Decorator`
  where the attribute names a resolvable path; built-in attributes produce
  nothing.
- `self.m()` / `Self::m()` inside an impl block → `RefKind::SelfCall` with
  the receiver stripped; the resolver rebuilds the target from the impl's
  type callsign, exactly as the Python resolver rebuilds from the enclosing
  class.

## Resolution and evidence

Resolution stays in `grid-survey` (an edge is a fact about two files) as a
per-language arm next to the Python one; the `Bindings` scope-containment
machinery is shared, the lookup order is not (Rust paths resolve through
`use` and absolute paths; there is no LEGB).

Grades stay a property of the evidence, and v1 emits only `Proven` — by
declining everything Rust's static binding rules do not fix:

- A name reached through a glob import (`use x::*` in scope) is never
  resolved: the glob makes shadowing undecidable without full name
  resolution, and a maybe-edge is a false edge.
- Method calls on non-`self` receivers: dropped (see non-goals).
- Anything inside a macro invocation: unseen, hence dropped.

What remains — explicit `use`-bound and absolute-path references to items
the index holds exactly one datum for — is what `rustc`'s own binding rules
would conclude. That subset is small and honest; growing it (types, macro
expansion, a `modeled` tier) is a later wave with its own evidence story.

## Acceptance

- `grid survey` over this repository produces a chart where hand-checked
  edges exist (`survey.lib` → `lang-py` extraction calls,
  `grid.edit` → `lang-py.definitions`) and no hand-checked non-edge does.
- Determinism corpus test, same harness as `lang-py`.
- A conservatism suite in the shape of `lang-py/tests/refs.rs`: every
  assertion either "this edge, from here" or "this construct produced
  nothing" — glob imports, macro bodies, and method receivers all in the
  second class.

## What implementation taught

- **Trailing commas are tokens, and rustfmt owns them.** The identity test
  caught it before the paper did: `f(x: u32,)` and `f(x: u32)` are different
  token streams, and rustfmt freely adds the comma — a pure reformat would
  have moved the datum. Canonicalization strips trailing commas, except in a
  paren group in value or pattern position with fewer than two commas, where
  `(x,)` really is a one-tuple; the "call shape" is decided by the preceding
  token (a non-keyword ident or a turbofish `>`). The one-tuple case is
  pinned by a test that must never pass by stripping.
- **The datum is not yet rename-invariant.** v1 canonical bytes are token
  streams after doc erasure, so a local rename moves the datum — a false
  "changed", never a false "same". The ceiling is asserted in
  `tests/identity.rs` so accidentally gaining the invariance (an epoch
  change) fails a test instead of passing silently. The upgrade is a
  scope-aware alpha renamer and a deliberate `grid-lang-rs/v2` epoch bump.
- **The module-path ceiling bites this very repo.** Self-hosting works —
  `grid survey` charts grid at ~920 defs / ~969 proven edges, and `grid`
  diffs its own working tree in datum space — but every proven Rust edge is
  intra-crate: the packages are named `grid-*` while their directories are
  `lang-py`, `chart`, …, so a cross-crate path like `grid_chart::build`
  never matches the dir-derived module root `chart`. The acceptance bar
  ("`grid.edit` -> lang-py extraction") therefore stands amended to
  intra-crate edges until the Cargo.toml-aware module root lands — the
  additive fix this page already named.
