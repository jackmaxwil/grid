# Method-call resolution for typed receivers (T-06, with T-17 and T-18)

Status: **implemented** (steps 4, 1, 2, 3 — in the plan's own sequencing
order), 2026-09-02, on `claude/grid-methods`. Every step landed as its
own commit with the gates green — `cargo fmt --all --check`, `cargo clippy
--workspace --all-targets` under pedantic deny, `cargo test --workspace`,
`cargo deny check`, `scripts/check-assurance-links.sh`,
`scripts/check-unsafe-confined.sh` — its conservatism test seen red first
with the mutation named in the commit body, its perf row re-measured and
stated, and its deviations listed. No new dependencies; `Cargo.lock`
gained nothing. ASSURANCE.md gained C44–C46 and the epoch sentence moved
to "epochs 1–4 as of epoch 5". Step 5's surface landed inside the steps
that changed each word, and this header.

Deviations from this document, one line each; the reasoning is in the
commit that made each one.

- **Step 4 (T-18, faa5629).** None. The differential harness
  `crates/survey/tests/edge_dump.rs` (ignored; chart as text) landed here
  because step 2 needs it.
- **Step 1 (30f41e0).** `Receiver` carries one `TypeRef` (path, args,
  form, inner) rather than the plan's flattened `ty`/`args`/`form` — one
  type for one idea, shared with `MethodFacts.ret` and `field_types`.
  `FileRefs` also gained `trait_methods`, the names every trait declares,
  because a required trait method is not a `Definition` and the resolver's
  candidate set needs it.
- **Step 2 (3819612).** `DriftUnchecked.unresolved_method_refused` is not
  on the wire and `grid uses`' refused hint is not printed: both live in
  files a sibling branch was editing this session; the refused share rides
  `Snapshot.declined.refused` and `grid survey`'s `miss` line. Refusal
  reasons are one `refused` counter, not itemized per reason. Porcelain is
  2.11.0, not 2.2.0 — the plan predates the agent-loop bumps.
- **Step 3 (6b71bcb, 6fc7855).** The counts ride `build_declined`'s map
  keyed by datum, not `DefFacts.declined`: no `DefFacts {` literal moves,
  and two definitions interning to one datum sum rather than drop.
  `DefRef.declined`, `declined_within`, and the four consumers (`suspects
  --closure`, `check`, `scope --bodies`, `grid uses`) are not wired — every
  one sits in the sibling's files; the column, its accessor, and the sum
  identity are in, the consumer-level conservatism test is pinned one
  layer down at `Segment::declined` (T-46). Saturation is stated on
  `Survey.declined_saturated`, not as a porcelain `warn`.

### The ledger, re-measured

Same tree (6fc7855), the pre-plan binary against the final one, Apple M4
Max, release. `grid survey` timings were taken interleaved with the
previous binary under a sibling build's load (load average 5–13) and are
pairwise within noise; idle, both binaries measured 90ms / 170ms.

| | grid before | grid after | yi before | yi after |
|---|---:|---:|---:|---:|
| defs | 1,943 | 1,943 | 5,176 | 5,176 |
| edges | 3,648 | 3,840 | 6,876 | 7,678 |
| proven | 3,648 | 3,769 | 6,876 | 7,159 |
| modeled | 0 | 71 | 0 | 519 |
| declined | 12,848 | 12,574 | 36,879 | 35,758 |
| external | 2,534 | 4,099 | 10,062 | 13,742 |
| method | 9,549 | 7,803 | 23,871 | 19,074 |
| of which refused | — | 64 | — | 338 |
| internal | 765 | 672 | 2,946 | 2,942 |
| segment bytes (epoch 4 → 5) | 438,170 (1,932 defs) | 456,436 | 1,150,811 | 1,205,953 |
| `grid survey`, warm, interleaved | 100–140ms | 110–140ms | 180–320ms | 190–320ms |

Against Part I's targets: `external` rose by 1,565 / 3,680 (predicted
~1,800 / ~5,400); edges emitted 121 proven + 71 modeled on this repo (of
which ~93 proven are T-18's inline-module references, so step 2's own are
~28 proven), 279 proven + 519 modeled on yi (predicted 68 / 221 and 578 /
1,004). R-1's walker counted call sites; this counts distinct edges after
the builder's dedup, follows no cross-file chain, and refuses where the
walker counted a `self.field` place as first-step. `method` fell 18% on
this repo and 20% on yi; the half is the residual only a type checker
moves (R-5).

Corrected after the review of PR #22 (same tree, same method): three
receivers that looked written and first-step were not — a trait's own
type parameter, a double reference, a field of a same-named type written
as `other::Foo` — and each now records no hint. On this repo the edge set
is unchanged and three references moved population (`external` 4,100 →
4,099, `method` 7,802 → 7,803, refused 66 → 64: `rpc::cache_record` and
`rpc::cache_check` typed `grid_cache::Chart::new(..)` by this file's own
`Chart`, `serde_shape::field_of` typed `syn::Field.ty` by this file's own
`Field`); yi is unchanged to the reference.

Original plan follows, unchanged.

---

Status (as planned): **planned**, 2026-09-01. Written after the agent-loop plan landed
(`docs/plans/2026-09-01-agent-loop.md`, whose shape this follows) and after
R-1 was measured — the measurement is in Part I and it changes the goal.
Every step is one commit with the gates green (`cargo fmt --all --check`,
`cargo clippy --workspace --all-targets` under pedantic deny, `cargo test
--workspace`, `cargo deny check`, `scripts/check-assurance-links.sh`,
`scripts/check-unsafe-confined.sh`), its conservatism test seen red first
with the mutation named in the commit message, its perf row re-measured
and stated, and its deviations from this document listed. No new
dependencies: everything here is `syn` visitors the tree already has and
lookups over `DefIndex`.

The subjects are TODOS.md items **T-06** (typed-receiver resolution),
**T-17** (per-definition declined counts, segment epoch 5), **T-18** (`use
super::x` inside inline modules) and research question **R-1** (how much
of the `method` count a syntactic rule recovers — answered below, before
anything is designed on it).

---

## Part I — Architecture overview

### The number this plan was asked to halve

The agent-loop plan's step 0b made Rust method calls visible: on this repo
8,852 of 11,985 declined references are `x.m()` (`method`), on yi 23,772
of 36,689. TODOS.md's A1 pressure test called it "the single largest gap
between grid's promise and its evidence" and asked for the `method` count
to be cut by half by resolving receivers whose type is syntactically
written.

R-1 was measured before this plan was designed, as TODOS.md demanded, and
the answer is that **no syntactic rule halves that count**. The count is
not mostly a hole; it is mostly calls into `std` and receivers whose type
no syntax in the file states. The plan's goal is therefore restated at
the end of this section as three numbers — edges recovered, calls
reclassified as harmless, and the residual that stays a hole — and the
half is recorded as unreachable by this route, with the route that would
reach it named in the cut line.

### R-1, measured

A throwaway `syn` walker (not committed; ~400 lines in a `lang-rs`
integration test, run with `--ignored` against a root) classified every
`x.m(...)` on a non-`self` receiver inside a definition, on this repo
(113 Rust files) and on yi (343 Rust files, 51 Python files), 2026-09-01.
The walker follows types the way step 1 will: parameter annotations, the
impl's own type for `Self`, `let` annotations, struct-literal and
`Type::f(..)` initializers with the associated function's written return
type, free-function return types, inherent-method return types along a
chain, struct field types for `self.field` and `typed.field`, and `?` /
`unwrap` / `expect` on `Option<T>` / `Result<T, _>`. It counts 8,809 calls
on this repo and 25,366 on yi against the survey's 8,852 / 23,772 — the
survey attributes by definition and the walker by function body, and the
two disagree by trait-default bodies and nested items; the shares are what
matter.

**Receiver forms.** What the walker could and could not type:

| receiver form | grid | yi |
|---|---:|---:|
| chain through a method call whose receiver is untyped (`a.b().c()`) | 2,364 | 6,154 |
| identifier bound by a closure parameter, pattern, `for`, `match`, or a static | 960 | 3,007 |
| chain through a call with no typable return (`f(..).m()`) | 647 | 1,007 |
| field of an untyped base (`x.f.m()`, `x` untyped) | 578 | 1,216 |
| local `let x = <chain>` | 556 | 1,428 |
| local `let x = f(..)`, `f`'s return not typable | 368 | 1,499 |
| local bound by `match`/`if`/other expression | 336 | 1,053 |
| literal, index, `&`/paren, `?` on an untyped value, macro, tuple, multi-segment path | 484 | 1,997 |
| parameter with an untypable annotation (tuple, fn pointer, array) | 154 | 331 |
| **untyped, subtotal** | **6,447 (73%)** | **17,692 (70%)** |
| typed, outside the tree: a `std`/external type, or `Vec`/`Option`/`Result`/`Arc`/`Mutex`/`Box` of anything | 1,813 (21%) | 5,415 (21%) |
| typed, charted, inherent `m` matched at the first lookup step | 68 | 578 |
| typed, charted, inherent `m` at a later step, no trait candidate | 221 | 1,004 |
| typed, charted, refused: no inherent `m` (a trait method — `clone`, `to_string`, an in-tree trait) | 149 | 414 |
| typed, charted, refused: the inherent impl is generic-with-bounds or concretely instantiated | 77 | 43 |
| typed, charted, refused: a trait in the tree, or an impl `Type@Trait.m`, also supplies `m` | 7 | 162 |
| typed, `impl Trait` / `dyn Trait` / a type parameter | 27 | 58 |
| **total** | **8,809** | **25,366** |

**Where the typed, charted receivers came from** (the origins step 1 has
to carry; grid / yi):

| origin | proven | modeled | refused |
|---|---:|---:|---:|
| parameter annotated `x: &Foo` | 42 / 322 | — | 89 / 91 |
| parameter annotated `x: &mut Foo` | 3 / 124 | 1 / 45 | 5 / 24 |
| parameter annotated `x: Foo` | 3 / 14 | 3 / 14 | 4 / 23 |
| local `let x: Foo = …` (annotated) | — | — / 3 | — / 6 |
| `self.field` with a charted field type | 1 / 18 | 9 / 232 | 13 / 107 |
| local `let x = Type::f(..)` with a written return | 3 / 14 | 80 / 289 | 20 / 84 |
| local `let x = Type { .. }` | — | 10 / 34 | 8 / 16 |
| local `let x = Type::f(..)?` / `.unwrap()` / a typed field read | — / 9 | 106 / 199 | 42 / 99 |
| local `let x = free_fn(..)` with a written return | — / 2 | 2 / 157 | 13 / 126 |
| local `let x = y.g()` with `g`'s written return | — / 2 | — / 15 | — / 2 |
| chain or literal receiver: `Type::f(..).m()`, `x.g().m()`, `x.g()?.m()`, `Foo { .. }.m()` | 16 / 73 | 10 / 16 | 39 / 41 |
| **total** | **68 / 578** | **221 / 1,004** | **233 / 619** |

**Python (yi, 51 files, 1,255 attribute calls on non-`self` receivers).**
Parameters annotated with a class the tree charts: **0**. Annotated with
an uncharted type (`str`, `Path`, a stdlib class): 25. Unannotated
parameters: 67. Names bound elsewhere (locals, globals, imports): 892.
Attribute chains and call chains: 150. Other expressions: 121. The T-06
Python rule recovers nothing on the one Python corpus at hand.

### What the measurement says

1. **Seventy percent of Rust method calls have no syntactic type in the
   file.** Closure parameters, iterator chains, `match` bindings, and
   locals bound by calls whose return types live in other crates. Only a
   type checker reaches them. On Rust that checker exists and is already
   in the loop (`cargo check`, `grid check --compile`); grid's own answer
   for those calls is the observed edge from a trace, not a resolver.
2. **Twenty-one percent are calls on `std` and external types** —
   `v.push(..)`, `s.len()`, `map.get(..)`, `Arc::clone(&x).m()`. They are
   in the `method` count today because step 0b declined every method call
   unconditionally. They are not a hole in any claim grid makes about this
   repository: the callee is not in the tree. They belong in `external`,
   with one guard — a tree that implements one of its own traits for a
   `std` type (`impl Tally for Vec<Foo>`) has charted `Vec@Tally.m`, and a
   call of that `m` stays `method`.
3. **Six to nine percent have a charted receiver type.** Of those, a
   little over half are resolvable under the rules below — 289 edges on
   this repo, 1,582 on yi — and the rest are refused for a reason grid can
   state per call: a trait supplies the method, the impl is generic, or
   there is no inherent method of that name at all (which means a trait
   provides it, in the tree or in `std`).

### The goal, restated in numbers

After this plan, on this repo: `method` falls from 8,852 to roughly
**6,700** — 6,447 unknown receivers plus ~260 charted receivers refused
with a stated reason — while ~1,800 move to `external` and **289 edges**
are emitted (68 proven, 221 modeled). On yi: 23,772 to roughly **18,400**,
~5,400 to `external`, **1,582 edges** (578 proven, 1,004 modeled). Python
on yi: 0 edges, said plainly. "Halved" is the residual, not the counter,
and only a type checker moves it; the cut line names that route.

The other two subjects need no measurement to justify. T-18 is a live
**C14 violation** verified today on a six-definition fixture: `self::x()`
inside `mod tests` resolves to the *file's* `x`, not the module's, because
the resolver anchors `self`/`super` against the file's module path and
never against the inline module's — one wrong proven edge, and `use
super::y;` in the same module declined as `internal`. T-17 is the per-
definition form of the count every consumer already prints repo-wide.

### The grade rule (the one design decision)

rustc's method lookup is fixed by the reference: for each candidate
receiver type in the autoderef chain (`R`, `*R`, …), first by value, then
`&U`, then `&mut U`, and **at every candidate type, inherent methods
before trait methods**. Two consequences carry every grade below.

- If the written receiver type `R` and an inherent method's receiver type
  coincide at the very first step — `x: &Foo` calling `fn m(&self)`,
  `x: &mut Foo` calling `fn m(&mut self)`, `x: Foo` calling `fn m(self)` —
  the inherent method wins over **every** trait: in the tree, in `std`,
  in any crate, imported or not. No trait knowledge is needed. That is
  the **proven** tier, and it is exactly what the language's own rule
  establishes (C14).
- If the inherent method matches at a later step — `x: Foo` calling
  `fn m(&self)` needs the autoref step; `self.field` is a by-value place
  — a trait method matching at an earlier step outranks it: a by-value
  `fn m(self)` from `impl T for Foo`, or from a blanket impl in a crate
  grid never reads (`Iterator::count` on a `Foo: Iterator`). Grid can
  check the tree's traits, the tree's `Foo@*.m` impls and a table of
  prelude method names; it cannot check external blanket impls. That is
  the **modeled** tier: correct unless an invisible trait intervenes, and
  never presented as more.
- Everything else is **declined and counted**, with the reason kept: any
  trait candidate grid *can* see, an inherent impl with bounds or
  concrete generic arguments (`impl Foo<u8>` does not serve `Foo<u16>`,
  and the callsign erases the difference), no inherent method of that
  name, an ambiguous type or method, a wrapper type, `impl`/`dyn`/a type
  parameter, a closure or pattern binding, a chain through an unwritten
  return.

The origin of the type matters once more on top: a type **written** by
the programmer (a parameter annotation, a `let` annotation, a struct
field's declared type, the impl's own type) can be proven; a type
**inferred** by grid (a local bound by `Foo::new()`, a struct literal, a
chain through a written return type, `?`/`unwrap` on a written
`Result<Foo, _>`) is modeled at best, because the inference is grid's
reading of syntax, not the language's binding rule.

### Modeled edges and the gate

Modeled edges exist on the wire already (`Grade::Modeled`; bridges and
covenants emit them) and every walk filters by floor. Drift's default
floor is `observed` and `check --quick`'s is `proven`, so **a modeled
method edge never gates by default** and never enters the hook. It ranks:
it appears in `grid scope`, `grid uses --floor modeled`, the test-selection
walk (which runs over every grade, by design — a skipped test is the
expensive mistake) and `suspects --closure`.

Two promotions were considered. A standing declaration `{"resolve":
"modeled-receivers"}` in `.grid/invariants.json`, on the model of
`persisted`; or the existing per-call `--floor modeled` on `drift` and
`check`. **Recommendation: the flag, and no new declaration.** The flag is
already on the wire and the CLI, it is the caller's own declaration per
call — which is what "modeled gates only under a declaration" asks for —
and the measured population (221 edges on this repo, 1,004 on yi, all of
them Rust, where `cargo check` already covers a moved signature) does not
earn a persistent rule file entry. The standing declaration is the cut
line, to land with the first Python repository whose annotated receivers
actually resolve — which yi is not.

### Data flow (what changes, in the survey's own phases)

```
phase 1  extract (per file, cached)      lang-rs: per-fn binding table → Reference.receiver hint
                                          lang-rs: Definition.method (receiver form, plain impl, written return)
                                          lang-rs: FileRefs.field_types
                                          lang-py: Reference.receiver for annotated params
phase 2  index                           DefIndex: field-type table; trait-method name set; Type@Trait.m set
phase 3  resolve                         resolve_rs: MethodCall arm → proven | modeled | declined(reason) | external
         segment build                   epoch 5: DefDeclined per def (T-17)
```

Nothing above the survey changes shape. Drift, walks, scope and test
selection read edges with a grade, as they do today.

---

## Part II — Feature set

**Receiver facts (step 1).** Extraction records, per method call, what
the file says about the receiver's type and how it said it; per inherent
method, its receiver form and whether its impl is plain; per struct, its
fields' declared types. In memory only — the per-file cache holds
`FileFacts`, never a segment — so no epoch moves for it.

**The resolver arm (step 2).** A `MethodCall` with a hint resolves the
type path by the same rules a `Call` path resolves, then the method under
it, then applies the grade rule. A method call on a receiver typed
outside the tree is `external`. Every refusal keeps its reason. Python
resolves annotated parameters the same way, modeled, and is measured at
zero on yi.

**Per-definition declined counts (step 3, T-17).** Segment epoch 5 adds
one section: four saturating `u16` per definition — external, method,
internal, and *refused* (a method call whose receiver was typed and the
rule declined). `suspects --closure`, `check` and `scope` can then say how
much of *this* neighbourhood the chart could not follow. The epoch-4
fixture, which does not exist yet, is committed first.

**Inline-module anchors (step 4, T-18).** `self::` and `super::` inside an
inline `mod`, in `use` declarations and in paths, anchor against the
inline module, which fixes one wrong proven edge and one declined
reference per fixture that today spells `super::*` to dodge it.

**Surface (step 5).** The `miss` line's split gains the refused count;
ONTOLOGY, ASSURANCE (C44–C46), LANG_RS.md's non-goals, README's numbers,
TODOS.md's items.

**Not built, recorded as decisions:** see the cut line.

---

## Part III — Use cases

**1. A moved inherent signature, reached only through a typed parameter.**
```
$ grid check --quick
drift proven  grid_survey.index.DefIndex.lookup  <-calls-  grid_survey.resolve_rs.target_of  1233→4bc4  crates/survey/src/resolve_rs.rs:88
```
Exit 3. Today this call is `index.lookup(..)` on `index: &DefIndex` and is
invisible to drift; `cargo check` would have found it sixty seconds later.

**2. The same call, but the receiver was built in the body.**
```
$ grid drift --floor modeled
drift modeled  grid.rpc.survey_run  <-calls-  grid_chart_io.Store.publish  8ae0→5d1c  crates/grid/src/rpc.rs:176
$ grid drift
miss  6712  references declined (external 3480, method 6712 of which 261 refused: trait candidate 7, generic impl 77, no inherent method 149, …; internal 1460)
```
`let mut store = Store::open(..)?;` — the type is grid's inference, so
the edge is modeled: shown under `--floor modeled`, absent from the
default gate, and counted in the honest line either way.

**3. A method both an inherent impl and a trait supply.**
```
$ grid uses grid_walk.Walk.next --floor modeled
grid: no callers at or above modeled — 14 method calls named `next` on a typed `Walk` were refused (trait candidate: Iterator)
```
Nothing is guessed; the refusal is itemized because T-17 holds it per
definition.

**4. Attribution with an honest horizon.**
```
$ grid suspects tests::parse_roundtrip --closure @a1b2
hit  crate.parse.header@e03f   closure  via 1 hop, datum moved
miss 41  references declined inside the closure (external 22, method 13 of which 4 refused, internal 2)
```
Per-definition counts summed over the closure's definitions, not the
repo's 11,985.

**5. `use super::x` in a test module.**
```
$ grid uses crate.foo.x
ref  crate.foo.tests.t   src/foo.rs:40   proven   calls
```
Today: declined `internal`, and `self::x()` in the same module would have
pointed at the wrong `x`.

---

## Standing rules for every step (the repo's own, restated)

- **Additive wire evolution only.** New fields are `Option` +
  `skip_serializing_if`; new records are new structs; enum variants extend,
  never repurpose. The one segment epoch bump (4 → 5) is declared in step 3,
  batches every per-definition fact this plan needs, and the epoch-4
  fixture is committed *before* `FORMAT_EPOCH` flips. `Segment::parse`
  keeps reading every epoch ever shipped.
- **No new dependencies.** `syn` visitors, `DefIndex` lookups, a static
  table of prelude method names.
- **Panic budget zero, HAR posture.** Index math `get`-guarded or
  saturating; no `unwrap` outside tests; the per-definition counters
  saturate at `u16::MAX` and say so.
- **Budgets rejected, never truncated.** No walk is added here; the one
  bounded search (the alias chain, depth 8) keeps refusing past its cap.
- **Every step lands with its conservatism test**, seen red before the
  fix is claimed, the mutation recorded in the commit message.
- **Evidence grades are load-bearing.** Proven gates. Modeled ranks and
  gates only under `--floor modeled`. Nothing in this plan emits heuristic.
- **Perf stated, never rounded.** The number to beat is in the ledger;
  every commit re-measures its row on this repo and on yi.
- **Deviations recorded per commit**, one line each, in the commit body,
  and folded into this file's Status header when the plan is closed.

Sequencing (each step independently shippable, gates green between):

```
0. R-1 measured                  done; the tables above; the walker is not committed
4. inline-module anchors (T-18)  first — it is a wrong proven edge today, and small
1. receiver facts                lang-rs binding table + hints; lang-py annotated params
2. resolver arm                  grade rule, refusal reasons, external reclassification      (needs 1)
3. per-def declined (T-17)       epoch-4 fixture pinned, then epoch 5 with DefDeclined        (needs 2 for `refused`)
5. surface                       porcelain, ONTOLOGY, ASSURANCE C44–C46, LANG_RS, README, TODOS
```

---

## Part IV — Technical implementation plan

### 4. Inline-module anchors (T-18) — first, because it is wrong today

**Problem.** `Scan` in `crates/lang-rs/src/refs.rs` records `use` targets
and reference paths as written; `resolve_rs::target_of` anchors a `self::`
or `super::` head with `anchor_str(module, …)` where `module` is the
**file's** module path. Inside an inline `mod tests` in `crate.foo`,
`self::x` therefore names `crate.foo.x` and `super::x` climbs to `crate.x`
(or to nothing). Verified 2026-09-01 on a six-definition fixture: `mod
tests { fn x(); fn t() { self::x() } }` beside a top-level `fn x` yields
one proven edge `tests.t -calls-> crate.x` — the wrong target — and `use
super::y; y()` and `super::x()` are declined `internal`. The first is a C14
violation; the second is the gap the agent-loop plan's step 3 recorded.

**Design.** Fix it where the nesting is known — the extractor. `Scan`
gains `mods: Vec<String>`, pushed in `visit_item_mod` for inline modules.
One function `anchor_in_mod(path) -> path` rewrites a path whose head is
`self` or `super` relative to the inline nesting:

- `self::rest` → `self::{mods…}::rest`;
- `super^k::rest` with `k ≤ mods.len()` → `self::{mods[..len−k]}::rest`;
- `super^k::rest` with `k > mods.len()` → `super^(k−len)::rest` (climbs
  the file's own module, which the resolver already handles);
- `crate::rest` is untouched.

Applied in `flatten_use` (to `Import.target` and therefore `reexports`)
and in `push` for every reference path. The resolver does not change; the
`Import.scope`/`Reference.from` callsign already carries the mod prefix,
so bindings stay scoped as they are. `use super::*` remains a glob and
binds nothing (the name then resolves by the existing module-relative
climb, which is why it "worked").

`mod_cfgs` and `path_mods` already record `join(scope, name)` and are not
affected.

**Wire.** None.

**Porcelain.** None.

**Tests** (`crates/lang-rs/tests/refs.rs`, `crates/survey/tests/rust_edges.rs`).
- Conservatism (seen red first): *`self::x()` inside an inline module that
  shadows a top-level `x` yields the edge to the module's `x` and never the
  file's.* Red today with the wrong edge — the mutation is the fixture
  itself.
- Ground truth: `mod tests { use super::x; fn t() { x() } }` → one edge to
  `crate.foo.x`; `super::x()` written inline → the same edge; nested `mod
  a { mod b { use super::super::x; } }` → the file's `x`; `super::super`
  past the file's module climbs the file's module path as before.
- Property: on every file without an inline module, imports and
  references are byte-identical to today's (the corpus determinism test,
  `crates/survey/tests/determinism.rs`, already pins snapshot ids for the
  fixture repos; add one with an inline module).
- The fixtures in `crates/grid/tests/compile.rs` and `drift.rs` that spell
  `use super::*` or `crate::x` to reach a parent item are re-spelled `use
  super::x` where that was the natural form, so they pin the fix.

**Perf.** Zero — one `Vec<String>` push per inline module, one path
rewrite per anchored reference.

---

### 1. Receiver-type facts in extraction

**Problem.** `visit_expr_method_call` records the receiver's dotted path
and the method name and nothing about types, and no other fact in
`FileFacts` says what an inherent method's receiver form is or what a
struct's fields are declared as. The resolver cannot apply the grade rule
without those three facts.

**Design.** Three additive fields, all in-memory (`FileFacts` is cached by
content hash in a `HashMap`, never serialized; no epoch moves).

`grid_datum::Reference` gains `receiver: Option<Receiver>`:

```rust
pub struct Receiver {
    /// The receiver's type as a path, as written or as inferred, `Self`
    /// replaced by the impl's own type callsign. Wrappers are not
    /// unwrapped: `Option<Foo>` is recorded as `["Option"]` with `args`.
    pub ty: Vec<String>,
    /// Whether the outermost type carries generic arguments (`Foo<u8>`,
    /// `Vec<Foo>`). Recorded so the resolver can refuse, not so it can
    /// look inside.
    pub args: bool,
    pub form: ReceiverForm,      // Ref | RefMut | Value
    pub origin: ReceiverOrigin,  // Param | LetAnnotated | SelfField | TypedField
                                 // | LocalCtor | LocalLiteral | ChainReturn | Unwrapped
}
```

`Param`, `LetAnnotated`, `SelfField` and `TypedField` are **written**
origins; the other four are **inferred**. `Value` is the form of a place
expression (`self.field`, a by-value parameter, a local) and of a
temporary.

`grid_datum::Definition` gains `method: Option<MethodFacts>`:

```rust
pub struct MethodFacts {
    pub recv: ReceiverForm,        // the method's own `self` form
    /// True for `impl Foo` and `impl<T> Foo<T>` with no bounds and no where
    /// clause — the impls whose methods apply to every `Foo`. False for
    /// `impl Foo<u8>`, `impl<T: Bound> Foo<T>`, `impl<T> Foo<T> where …`.
    pub plain_impl: bool,
    /// The written return type as a path, `Self` substituted, wrapper kept
    /// (`Result<Self, E>` is `["Result"]` + `inner: ["Foo"]`), for chains.
    pub ret: Option<TypeRef>,
}
```

`grid_datum::FileRefs` gains `field_types: Vec<(String, String, TypeRef)>`
— `(struct callsign, field, declared type)` — and `assoc_returns` is not
needed: an associated function is a `Definition` and its return rides on
`MethodFacts` with `recv` absent (`method: Some(MethodFacts { recv: None,
.. })`; make `recv` an `Option`).

The `lang-rs` extractor grows a per-function binding table in `Scan`: a
stack of `HashMap<String, Binding>` pushed per function body and per
block, so shadowing follows source order and block scope. Params bind
from their annotation; `let` binds from its annotation, else from
`type_of(init)`; `type_of` is the walker used for R-1: a single-segment
path looks up the table (`self` binds to the impl's type with the
method's own receiver form); a field access on a typed base looks up
`field_types` collected in the *same file* (a cross-file field type is
resolved in phase 3 from the index, so the hint carries the base type and
the field name: `origin: TypedField`, `ty: base type`, plus `field:
Option<String>`); a method call on a typed base follows the same-file
inherent method's written return; `Type::f(..)` follows the same-file
associated function's return; `?`, `.unwrap()`, `.expect(..)` on a
`Result<T, _>` / `Option<T>` yield `T` with origin `Unwrapped`; `&e` and
`&mut e` set the form. Everything else — closure parameters, `for`
bindings, `match`/`if let` patterns, literals, indexes, macros, tuple
fields, chains through an unwritten return — records no hint, and the
call stays what it is today.

Cross-file returns are deliberately *not* followed at extraction: a
`let x = other_crate::make()` has a hint only if `make`'s return is in the
same file. `ponytail:` phase-3 chain following through the index (the
method's return type is on `MethodFacts` there) is the upgrade; R-1 puts
the same-file half at 135 chain receivers on this repo and 281 on yi, and
the cross-file half is not measured.

`lang-py`: `visit_stmt_function_def` reads each `arg.annotation` that is a
`Name` or an `Attribute` chain and binds the parameter; `visit_expr_call`
attaches `receiver: Some(Receiver { ty, form: Value, origin: Param })` to
the `MethodCall` it already records when the head is such a parameter.
`self` stays `SelfCall`. Nothing else — no locals, no returns: measured
at zero on yi, and the symmetric Rust machinery is not worth porting for
zero.

**Wire.** None — `Reference`, `Definition` and `FileRefs` are the survey's
in-memory vocabulary.

**Porcelain.** None.

**Tests** (`crates/lang-rs/tests/refs.rs`; a new `receivers.rs` beside it;
`crates/lang-py/tests/refs.rs`).
- Ground truth, one assertion per origin: `fn f(x: &Foo) { x.m() }` →
  hint `(["Foo"], Ref, Param)`; `&mut Foo`; `Foo`; `x: &Self` inside `impl
  Bar` → `["Bar"]`; `let x: Foo = …`; `let x = Foo::new(); x.m()` →
  `LocalCtor`; `let x = Foo { .. }` → `LocalLiteral`; `self.field.m()` →
  `SelfField` with the field's declared type when the struct is in the
  file, else `TypedField` with `field: Some("field")`; `Foo::open(..)?.m()`
  → `Unwrapped`; `x.g().m()` with `fn g(&self) -> &Bar` in the file →
  `ChainReturn (["Bar"], Ref)`.
- Conservatism (seen red first): *a receiver bound by a closure parameter
  carries no hint* (`|x: &Foo| x.m()` — annotated closure params are
  deliberately excluded in v1: the closure's call site fixes the type,
  not the annotation grid reads); *a shadowed local takes the innermost
  binding* (`let x = Foo::new(); { let x = 3; x.m(); }` → no hint;
  mutation: drop the block push); *`Option<Foo>` records `args: true` and
  `ty: ["Option"]`, never `["Foo"]`*.
- Per method: `impl Foo { fn m(&self) }` → `recv: Ref, plain_impl: true`;
  `impl Foo<u8>` → `plain_impl: false`; `impl<T: Clone> Foo<T>` → false;
  `impl<'a> Foo<'a>` → true; `fn new() -> Self` → `recv: None, ret:
  ["Foo"]`; `fn open() -> Result<Self, E>` → wrapper `Result`, inner
  `["Foo"]`.
- Python: `def f(x: Foo): x.m()` → hint; `def f(x): x.m()` → none; `def
  f(x: "Foo")` (a string annotation) → none, stated.
- Identity: the datum, sig and shape of every definition are unchanged by
  this step (`crates/lang-rs/tests/identity.rs` pins them; the hints ride
  beside the definition, not in it).

**Perf.** Extraction on this repo is ~40ms cold; the binding table adds
one map per function body and a lookup per method call. Budget: within
noise of 40ms cold / 90ms `grid survey` warm on this repo, 80ms / 170ms on
yi (Part I of the incremental-survey plan has the phase breakdown). State
the number.

---

### 2. The resolver arm

**Problem.** `resolve_rs::resolve` declines every `MethodCall`
unconditionally. With step 1's facts it can bind the ones the language
binds, grade the ones it can only model, reclassify the ones that were
never the tree's, and keep a reason for every refusal.

**Design.** `DefIndex` gains three small tables built in phase 2 from the
step-1 facts: `field_types: HashMap<absolute struct, HashMap<field,
TypeRef>>`; `trait_methods: HashSet<method name>` (every method declared
in every `trait` in the tree — the tree-wide over-approximation of "in
scope", chosen because deciding scope needs trait name resolution and a
maybe is a refusal); and the set of `Type@Trait.m` callsigns, which the
index already holds by name and which is queried as "does any symbol
`{type}@*.{m}` exist" via a per-type method-name map filled at insert.
`Symbol` gains `method: Option<MethodFacts>` copied from the definition.

The arm, in order, for a `MethodCall` `r` from `src`:

1. **No hint** → `Decline::Method` (unknown receiver), as today.
2. **Hint with `origin: TypedField`** → resolve the base type (step 3
   below), read `field_types[base][field]`; absent → `Method`.
3. **Resolve the type path** exactly as a `Call` path resolves: `Self` was
   substituted at extraction; otherwise bindings, anchors, module-relative
   climb, `lookup_absolute` — the existing `target_of` on a synthetic
   `Reference { kind: Call, path: hint.ty }`. The result must be a
   `Symbol` with `is_class` and not ambiguous. If it resolves to nothing:
   `decline_of` decides `External` vs `Internal` for the *type* path, and
   the call is counted `External` when the type is external **and** no
   `{last segment}@*.{m}` symbol exists in the tree (the guard for an
   in-tree trait implemented on a foreign type); otherwise `Method`.
4. **`hint.args`** (`Foo<u8>`, `Vec<Foo>`) → `Method`, reason `wrapper or
   instantiated`. The callsign erased the argument and the rule cannot
   see whether the impl serves it. (`Vec<Foo>` already went to `External`
   in step 3 unless the guard held.)
5. **Look up the inherent method** `{type absolute}.{m}` → a `Symbol`
   with `method: Some(facts)` and `facts.recv: Some(recv)`. Absent (or an
   associated function, or ambiguous) → `Method`, reason `no inherent
   method`.
6. **Refusals that precede the grade:** `!facts.plain_impl` → `Method`,
   reason `generic impl`; `trait_methods.contains(m)` or
   `{type}@*.{m}` exists or `PRELUDE_METHODS.contains(m)` → `Method`,
   reason `trait candidate` — *except* when the first-step condition in
   7 holds, where the inherent method wins by rustc's own order and the
   candidate is irrelevant. (R-1's "no-candidate" split was computed
   without this exception; the exception can only add proven edges.)
7. **Grade.** `first_step = hint.form == recv`, with one impossibility
   refused first: `hint.form == Ref && recv == RefMut` cannot compile
   through the inherent method, so if the program compiles a trait
   supplied `m` → `Method`, reason `trait candidate`. Then:
   - written origin ∧ first step → **Proven**;
   - written origin ∧ not first step ∧ no candidate → **Modeled**;
   - inferred origin ∧ (first step ∨ no candidate) → **Modeled**;
   - otherwise → `Method`, reason `trait candidate`.
8. **Emit** `ResolvedEdge { src, dst: method datum, relation: Calls,
   grade }`; `src == dst` is skipped as everywhere.

`PRELUDE_METHODS` is a `const` slice of the method names the 2021 prelude's
traits provide (`clone`, `to_string`, `into`, `try_into`, `from`, `eq`,
`ne`, `cmp`, `partial_cmp`, `lt`/`le`/`gt`/`ge`, `max`/`min`/`clamp`,
`default`, `to_owned`, `as_ref`/`as_mut`, `drop`, `next`, `fmt`, `hash`,
`borrow`/`borrow_mut`, `deref`/`deref_mut`, `into_iter`, `extend`,
`from_iter`, `size_hint`) — about thirty names; a name on the list makes
a later-step match a refusal, never a proven one. `ponytail:` the list is
the prelude, not `std`; an inherent `fn map` beside a `Foo: Iterator` at
a later step is caught by the `Foo@Iterator.next` guard (the impl is in
the tree), and an inherent `fn map` beside a blanket external impl is
the modeled tier's stated residual.

`Resolved` gains `declined_method_refused: u32` (a sub-count of
`declined_method`, not a fourth population — the sum identity C34 stays
`external + method + internal`) and `refusals: Vec<(String /*from*/,
Refusal)>` for T-17 (step 3) to file per definition; `Survey` gains
`declined_method_refused: u64`.

The Python arm (`resolve.rs`): a `MethodCall` whose hint is `Param` with
a type path resolving (bindings, module-relative) to a `Class` symbol,
and `{class}.{m}` present and unambiguous → **Modeled**, `Calls` (or
`Constructs` when the target is a class, by `relation_of`). Python's
binding rules do not fix an attribute call from an annotation — a subclass
may override — so it is never proven. Everything else stays as it is.

**Wire (additive).** `DriftUnchecked.unresolved_method_refused:
Option<u64>` and `Snapshot.declined.refused: Option<u64>` (both
`skip_serializing_if`). Edge grades already carry `modeled`.

**Porcelain.** The existing `miss` record's parenthetical grows: `miss 6712
references declined (external 3480, method 6712 of which 261 refused,
internal 1460)`. No new record. `grid uses <method>` with no callers at
the floor and refused calls on record says so in its hint (needs step 3
for the per-definition count; until then the hint is repo-wide).

**Tests.**
- **Ground truth with the compiler as the oracle** (`crates/survey/tests/
  receivers.rs`): a fixture cargo package `crates/survey/tests/fixtures/
  receivers/` whose `main` exercises every receiver form and every
  refusal case, where each inherent and trait method body prints its own
  callsign (`println!("Foo.m")`, `println!("Foo@T.m")`) and each call
  site prints its line. The test surveys the fixture and, in the same
  test, runs `cargo run -q` on it (cargo is what is running the test) and
  reads the transcript — rustc's own binding decisions made observable.
  Assertions: every emitted edge (proven or modeled) names the method the
  transcript shows that line reaching; every proven edge's line has
  exactly one transcript target; the set of call lines with no edge is
  exactly the fixture's annotated refusal set (`// refused: trait
  candidate` comments parsed by the test). The fixture is the place this
  plan's rule would be *wrong* if the rule is wrong: a by-value `impl T
  for Foo { fn m(self) }` beside `impl Foo { fn m(&self) }` with a
  by-value receiver prints `Foo@T.m`, and the rule must have declined.
- Conservatism (seen red first, one commit each or one commit naming
  each mutation): *a method that exists on both an inherent impl and a
  trait in scope yields no edge and is counted `method`* (mutation:
  remove the trait-candidate check — the transcript test also catches it);
  *an inherent impl with a where-clause yields no edge* (mutation: ignore
  `plain_impl`); *`x: &Foo` calling a `&mut self` method yields no edge*;
  *`Option<Foo>` yields no edge and `Vec<Foo>` counts `external`*; *a
  type that resolves to two datums (the ambiguity set) yields no edge*;
  *a receiver typed by a same-file `type Alias = Foo;` yields no edge*
  (type aliases are not definitions; the path resolves to nothing).
- Property: `declined == external + method + internal` and
  `refused ≤ method` on every file of the corpus (`declined_split.rs`);
  incremental ≡ full over the fixture (the corpus determinism harness
  gains it); every proven method edge has a first-step receiver match
  (checked by re-deriving the facts in the test).
- Differential against the previous binary's chart on this repo and yi:
  the set of *proven* edges is a strict superset (68 / 578 added, none
  removed), the `external` count rises by the typed-outside count, the
  `method` count falls by exactly `external moved + edges emitted`. The
  numbers go in the commit message.
- Python: `def f(x: Foo): x.m()` with `class Foo: def m` → one modeled
  edge; the same with a subclass overriding `m` → still the annotated
  class's `m` (the annotation is what was written), graded modeled;
  unannotated → none.

**Perf.** Phase 3 on this repo is ~6.5ms and on yi ~20.5ms (measured
2026-09-01, in-process, release); the arm adds up to three index lookups
per hinted call (≈2,400 on this repo, ≈7,700 on yi). Budget: phase 3
within +3ms on this repo, +8ms on yi; `grid survey` within noise of 90ms /
170ms wall. State the numbers.

---

### 3. Per-definition declined counts (T-17), segment epoch 5

**Problem.** Every consumer prints one repo-wide split. `suspects
--closure` has to say how much of *the closure* the chart could not
follow; `check` wants the same for the changed set; `grid uses X` wants
to say how many refused method calls named `X`'s method. The count is a
fact about the definition the reference sits in, which is a segment
fact.

**Design.**
- **First, the missing epoch-4 fixture.** `crates/chart/tests/fixtures/`
  holds `epoch1.seg`, `epoch2.seg`, `epoch3.seg`; ASSURANCE.md's sentence
  says "epochs 1, 2 and 3 as of epoch 4" and there is no `epoch4.seg`.
  Commit one — the fixture repo surveyed with the current binary — and
  the test `an_epoch4_image_parses_and_answers_its_own_columns` (cfg
  present, declined absent) *before* `FORMAT_EPOCH` flips. Its own commit.
- **Epoch 5.** `Section::DefDeclined = 19`: per definition `(external:
  u16, method: u16, internal: u16, refused: u16)`, saturating, 8 bytes per
  def. Epoch-4 and older images answer `None`. The builder takes the
  counts beside `DefFacts` (`DefFacts.declined: Option<Declined>`); the
  survey fills them from `Resolved` by attributing each decline to
  `r.from` (module-level references are not counted, as today — they have
  no definition).
- **Batched into the same bump:** nothing else. The agent-loop plan's
  cut line listed "a doc hash" as a candidate to ride a bump; it has no
  consumer and stays out. Step 1's receiver facts are per-reference and
  in-memory; the method facts are derivable from source and are not
  stored either. The bump is one section.
- **Consumers.** `suspects --closure`'s closing `miss` sums the closure's
  definitions (replacing the "per-file counts summed over the closure's
  files" the agent-loop plan settled for); `check`'s and `drift`'s `miss`
  keep the repo-wide sum (the changed set's own sum is added as a second
  clause: `… of which 12 inside the changed definitions`); `grid uses X`
  reports refused calls of `X`'s method name summed over the tree's
  definitions as a hint; `scope --bodies`' horizon line gains `· N
  references declined inside the pack`.
- The superseded-store recovery from the drift plan handles the flip;
  `.grid/chart` is a cache and is dropped once.

**Wire (additive).** `DefRef.declined: Option<DeclinedSplit { external,
method, internal, refused }>` where the walk/scope/suspects items already
carry a `DefRef`; `SuspectsResult`/`ScopePack` gain `declined_within:
Option<DeclinedSplit>`.

**Porcelain.** `miss` lines as above; `scope`'s horizon clause. PORCELAIN.md
gains the clause.

**Tests.**
- Epoch: `epoch4.seg` parses and answers `declined` `None`; an epoch-5
  image round-trips the four counters; `adversarial.rs` gains the section
  (a length not a multiple of 8, an overlap) — the validate-then-accept
  path, C1.
- Property: the sum over every definition of each counter equals the
  survey's repo-wide counter (module-level declines excluded, asserted
  separately as the difference); `refused ≤ method` per definition;
  saturation at `u16::MAX` is a stated `warn`, never a wrap.
- Conservatism (seen red first): *a pre-epoch-5 snapshot makes `suspects
  --closure` print `miss ? references declined inside the closure
  (pre-epoch-5 snapshot)` rather than `0`* — unknown is never zero
  (mutation: default the missing column to zeros).
- Differential: `suspects --closure`'s per-closure sum on a fixture equals
  an independent sum computed in the test from `grid scope` at full depth
  joined with each definition's `DefRef.declined`.

**Perf.** Segment size +8 B/def: this repo's 407,520-byte image grows by
~14.5 KB (1,813 defs); yi's 1,150,811 by ~34 KB. Survey within noise.
State the bytes.

---

### 5. Surface

- **Porcelain.** The `miss` parenthetical's `refused` clause; the
  `scope` horizon clause; `grid uses`' refused hint. PORCELAIN.md updated;
  version 2.2.0 (2.1.0 was the agent-loop plan's).
- **ONTOLOGY.md.** `method` rewritten: "a declined reference whose callee
  sits behind a receiver grid could not type, or typed and refused — a
  trait candidate, a generic impl, no inherent method; the refused share
  is counted. A call whose receiver's type is written and outside the
  tree is `external`." `modeled` gains the second source: "… or a method
  call whose receiver type is written but matched only at a later lookup
  step, or inferred from a constructor, a literal, or a written return
  type — correct unless a trait grid cannot see supplies the method."
  New rows: `receiver` (the hint), `refused` (the reason-bearing decline).
  `proven` gains "… including an inherent method a written receiver type
  reaches at the first lookup step, where rustc's own order lets no trait
  outrank it."
- **ASSURANCE.md.** C44: *a method-call edge is emitted only from a
  receiver whose type the file states or grid infers from a constructor,
  literal, or written return; it is proven only where the written
  receiver form matches the inherent method's at the first lookup step
  of a plain impl, modeled otherwise, and declined with a stated reason
  wherever any trait grid can see supplies the method, the impl is
  bounded or instantiated, the type is a wrapper, or the receiver is a
  closure or pattern binding; rustc's own binding, observed by running
  the fixture, agrees with every emitted edge* → `receivers.rs refs.rs
  declined_split.rs`. C45: *every definition's declined split sums to the
  survey's, a pre-epoch-5 snapshot answers unknown, never zero* →
  `epochs.rs membership.rs attribution.rs`. C46: *`self::` and `super::`
  inside an inline module anchor against that module in `use`
  declarations and paths; a shadowed name resolves to the inline
  module's item* → `refs.rs rust_edges.rs`. The dependency line: "this
  plan: no new packages." The epoch sentence: "epochs 1–4 as of epoch 5,
  which added the per-definition declined split."
- **LANG_RS.md.** Non-goals: "Trait-method receiver resolution" stays,
  reworded to what it is now: inherent methods on written or
  syntactically inferred receiver types resolve (proven / modeled by the
  first-step rule); trait methods, wrapper receivers, closure and pattern
  bindings, and chains through unwritten returns do not. "What
  implementation taught" gains the R-1 paragraph and the inline-module
  anchor bug.
- **README.md.** The numbers table gains the declined split after this
  plan (this repo and yi), and the honest sentence: "Seventy percent of
  Rust method calls have no syntactic type; `cargo check` and traces
  cover them, not the resolver."
- **TODOS.md.** T-06, T-17, T-18, R-1 marked with their measured outcome;
  the cut-line items below become the next T-numbers.

---

## Verification posture (har-verify ladder, applied)

- Rung 1 everywhere: every step's fixtures are ground-truth pairs and every
  conservatism assertion is a named test seen red first, with the
  mutation recorded in the commit message. Step 4's red is the wrong edge
  itself; step 2's reds are the four refusal rules removed one at a time.
- Rung 3: the declined sum identity extended with `refused ≤ method` (2);
  per-definition sums equal the repo-wide counters (3); every proven
  method edge re-derives to a first-step match (2); identity, sig and
  shape unchanged by the hints (1); files without inline modules extract
  byte-identically (4).
- Rung 5 (differential): **rustc as the oracle** — the fixture's runtime
  transcript against the emitted edges (2), which is the one test in this
  plan that can say "the rule is wrong" rather than "the rule changed";
  the previous binary's chart against the new one on this repo and yi
  (proven strictly grows, `external` rises by the typed-outside count,
  `method` falls by exactly the sum) (2); `suspects --closure`'s sum
  against an independent join (3).
- Fuzz/miri/loom/kani: the epoch-5 section is parsed by `grid-chart`'s
  validate-then-accept path and joins the adversarial corpus (C1);
  nothing else here parses untrusted bytes, owns `unsafe`, or adds
  concurrency.

## Perf ledger

Budgets, to be measured and stated in each commit (Apple M4 Max, release,
this repo at 1,813 defs / 3,419 edges / 114 files; yi at 4,312 defs / 384
files, unless a row says otherwise). Today's numbers were measured
2026-09-01 with per-phase timers in-process, not committed.

| surface | today | budget | why it should hold |
|---|---|---|---|
| `grid survey`, this repo, warm store | 90ms wall (discovery 20–26, extract 38–43 cold, index 6.3, resolve 6.5, build 1.0, publish 12) | within noise: ≤ 95ms | a binding table per function; ≤ 3 lookups per hinted call |
| `grid survey`, yi, warm store | 170ms wall (discovery 28, extract 80, index 13, resolve 20.5, build 2.7, publish 15) | ≤ 180ms | same |
| phase 1 extract, this repo, cold | 38–43ms | ≤ 46ms | one map per fn body |
| phase 3 resolve, this repo / yi | 6.5ms / 20.5ms | ≤ 9.5ms / ≤ 28ms | ~2,400 / ~7,700 hinted calls |
| segment size, epoch 5 | 407,520 B / 1,150,811 B | +8 B/def: ≈ +14.5 KB / +34 KB | one section |
| `grid check --quick` | 120–140ms | unchanged (this plan does not touch it) | the incremental-survey plan owns this row |

## Cut line

Recorded so each is a decision, not an omission:

- **A type checker.** The seventy percent needs one. rust-analyzer as a
  sidecar (an LSP `textDocument/definition` per call site, or its
  `ra_ap_*` crates in-process) is the only route to halving the counter;
  it is a dependency decision the size of `syn` again, and on Rust the
  compiler is already in the loop. Not this plan; a research item with
  its own gate (R-5: does an rust-analyzer sidecar answer this repo's
  8,800 call sites under a second, cold?).
- **Trait-method resolution by impl search.** Picking among `Foo@A.m`,
  `Foo@B.m` and a blanket impl needs the trait in scope and the receiver
  type's bounds; declined with a reason instead.
- **Autoderef through `Box`/`Rc`/`Arc`/`Option`.** The wrapper's own
  methods and trait impls sit earlier in the lookup chain; a rule that
  looks through them is the modeled tier with a bigger residual. Counted
  as `external` (the wrapper is `std`'s), which is honest.
- **Closure, `for`, `match` and `if let` bindings.** Need the iterated or
  matched value's type — the same seventy percent.
- **Cross-file chain following.** `ponytail:` in step 1; measured same-
  file only.
- **A standing `resolve: modeled-receivers` declaration.** `--floor
  modeled` is the per-call declaration and it exists; the standing form
  lands with a Python repository whose annotated receivers resolve.
- **Python locals bound by a constructor.** Symmetric and cheap; measured
  at zero on yi; lands with the repo that measures otherwise.
- **A `std` signature table** (to say `v.iter().map(..)` is `Iterator::map`
  by name). Would turn part of the seventy percent into `external`, not
  into edges, and is a table grid would have to maintain against a
  moving standard library.
- **Macro-generated callers** (T-19). Unchanged: invisible.
