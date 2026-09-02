//! Reference extraction conservatism, the same contract lang-py pins:
//! every assertion is either "this name was found, attributed to this
//! definition" or "this construct produced no reference at all" — the second
//! kind matters more, because a reference invented here becomes a false edge
//! in the chart.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr
)]
use grid_datum::RefKind;
use grid_lang_rs::references;

fn refs(src: &str) -> Vec<(String, RefKind, String)> {
    references(src)
        .expect("valid Rust")
        .refs
        .into_iter()
        .map(|r| (r.from, r.kind, r.path.join(".")))
        .collect()
}

fn imports(src: &str) -> Vec<(String, String, String)> {
    references(src)
        .expect("valid Rust")
        .imports
        .into_iter()
        .map(|i| (i.alias, i.target, i.scope))
        .collect()
}

#[test]
fn a_call_is_attributed_to_its_definition() {
    let r = refs("fn a() { b(); }\nfn b() {}");
    assert_eq!(r, vec![("a".into(), RefKind::Call, "b".into())]);
}

#[test]
fn module_level_code_belongs_to_no_definition() {
    let r = refs("const X: u32 = helper();\nconst fn helper() -> u32 { 1 }");
    assert_eq!(r, vec![(String::new(), RefKind::Call, "helper".into())]);
}

#[test]
fn use_declarations_become_imports() {
    let i = imports("use a::b::{C, d as e};\nuse crate::x::Y;\nfn f() { use std::mem; }");
    assert_eq!(
        i,
        vec![
            ("C".into(), "a.b.C".into(), String::new()),
            ("e".into(), "a.b.d".into(), String::new()),
            ("Y".into(), "crate.x.Y".into(), String::new()),
            ("mem".into(), "std.mem".into(), "f".into()),
        ]
    );
}

#[test]
fn a_glob_import_binds_no_name() {
    assert!(imports("use a::b::*;").is_empty());
}

#[test]
fn pub_use_is_a_reexport_and_plain_use_is_not() {
    let refs = references("mod util;\npub use util::helper;\nuse util::other;").expect("valid");
    let rx: Vec<(String, String)> = refs
        .reexports
        .iter()
        .map(|i| (i.alias.clone(), i.target.clone()))
        .collect();
    assert_eq!(rx, vec![("helper".into(), "util.helper".into())]);
    // Both bindings are still ordinary imports for this file's own refs.
    assert_eq!(refs.imports.len(), 2);
}

#[test]
fn a_self_leaf_binds_the_module_itself() {
    let i = imports("use crate::chart_provider::{self, ChartDelta};\nuse a::b::{self as c};");
    assert_eq!(
        i,
        vec![
            (
                "chart_provider".into(),
                "crate.chart_provider".into(),
                String::new()
            ),
            (
                "ChartDelta".into(),
                "crate.chart_provider.ChartDelta".into(),
                String::new()
            ),
            ("c".into(), "a.b".into(), String::new()),
        ]
    );
}

#[test]
fn self_calls_are_kept_apart_and_stripped() {
    let src = "struct Foo;\nimpl Foo {\n  fn a(&self) { self.b(); Self::c(); }\n  fn b(&self) {}\n  fn c() {}\n}";
    let r = refs(src);
    assert!(r.contains(&("Foo.a".into(), RefKind::SelfCall, "b".into())));
    assert!(r.contains(&("Foo.a".into(), RefKind::SelfCall, "c".into())));
}

#[test]
fn a_method_call_is_recorded_as_one_and_never_as_a_call() {
    // Binding it needs types; types are modeled, and this frontend emits no
    // guesses. It is recorded all the same, under its own kind, so the
    // resolver can decline it *and count it* — an invisible call and a
    // declined one are the same hole, and only one of them can be reported.
    let r = refs("fn f(x: Foo) { x.frob(); }");
    assert_eq!(r, [("f".into(), RefKind::MethodCall, "x.frob".into())]);
}

#[test]
fn a_method_call_on_an_expression_keeps_the_method_and_no_receiver() {
    // `Foo::new()` is not a name, so there is no receiver path to record —
    // but the call still happened, and a reference with no receiver still
    // says one method call was declined here.
    let r = refs("fn f() { Foo::new().m(); }");
    assert!(
        r.contains(&("f".into(), RefKind::MethodCall, "m".into())),
        "{r:?}"
    );
}

/// Conservatism, verb sweep B1: a macro's tokens are not a place grid can
/// bind names, so `g()` and `h()` are never references — and each
/// invocation is recorded under its own kind, so the resolver can count
/// what it did not read. Red before the fix with `r` empty: the omission
/// was invisible, and `check --quick` passed a signature change whose only
/// caller sat inside a `println!`.
#[test]
fn a_macro_invocation_is_recorded_and_nothing_inside_it_is() {
    let r = refs("fn f() { println!(\"{}\", g()); vec![h()]; }");
    assert_eq!(
        r,
        vec![
            ("f".into(), RefKind::Macro, "println".into()),
            ("f".into(), RefKind::Macro, "vec".into()),
        ]
    );
}

#[test]
fn a_trait_impl_is_an_implements_edge_from_the_type() {
    let r = refs("struct Foo;\ntrait T {}\nimpl T for Foo {}");
    assert_eq!(r, vec![("Foo".into(), RefKind::Base, "T".into())]);
}

#[test]
fn a_negative_impl_is_not_an_edge() {
    let r = refs("struct Foo;\nauto trait T {}\nimpl !T for Foo {}");
    assert!(r.iter().all(|(_, kind, _)| *kind != RefKind::Base));
}

#[test]
fn derives_are_decorators_and_std_derives_are_furniture() {
    let r = refs("#[derive(Clone, Serialize)]\nstruct Foo;");
    assert_eq!(
        r,
        vec![("Foo".into(), RefKind::Decorator, "Serialize".into())]
    );
}

#[test]
fn builtin_attributes_produce_nothing() {
    let r = refs("#[test]\n#[inline]\n#[must_use]\nfn f() -> u32 { 1 }");
    assert!(r.is_empty());
}

#[test]
fn struct_literals_reference_the_type() {
    // Construction and field-writing, both to the type — nothing else.
    let r = refs("struct Foo { x: u32 }\nfn f() { let _ = Foo { x: 1 }; }");
    assert_eq!(
        r,
        vec![
            ("f".into(), RefKind::Call, "Foo".into()),
            ("f".into(), RefKind::FieldWrite, "Foo".into()),
        ]
    );
}

#[test]
fn nested_functions_scope_their_references() {
    let r = refs("fn outer() { fn inner() { helper(); } inner(); }\nfn helper() {}");
    assert!(r.contains(&("outer.inner".into(), RefKind::Call, "helper".into())));
    assert!(r.contains(&("outer".into(), RefKind::Call, "inner".into())));
}

#[test]
fn qualified_self_paths_produce_nothing() {
    // `<Foo as T>::m()` names through a type, which is resolution this
    // frontend does not claim; recording nothing is the honest answer.
    let r = refs("fn f() { <Foo as T>::m(); }");
    assert!(r.is_empty());
}

/// Conservatism, T-18: `self::x()` inside an inline module names the
/// *module's* `x`, never the file's. Red before the fix with the path
/// recorded as written (`self.x`), which the resolver anchored against the
/// file's module — one wrong proven edge, a C14 violation.
#[test]
fn self_inside_an_inline_module_anchors_against_the_module() {
    let r = refs("fn x() {}\nmod tests {\n    fn x() {}\n    fn t() { self::x(); }\n}");
    assert_eq!(
        r,
        vec![("tests.t".into(), RefKind::Call, "self.tests.x".into())]
    );
}

#[test]
fn super_inside_an_inline_module_is_the_files_module() {
    let r = refs("fn x() {}\nmod tests {\n    fn t() { super::x(); }\n}");
    assert_eq!(r, vec![("tests.t".into(), RefKind::Call, "self.x".into())]);
    let i = imports("fn x() {}\nmod tests {\n    use super::x;\n}");
    assert_eq!(i, vec![("x".into(), "self.x".into(), "tests".into())]);
}

#[test]
fn nested_inline_modules_climb_one_module_per_super() {
    let i = imports(
        "mod a {\n    mod b {\n        use super::super::x;\n        use super::y;\n    }\n}",
    );
    assert_eq!(
        i,
        vec![
            ("x".into(), "self.x".into(), "a.b".into()),
            ("y".into(), "self.a.y".into(), "a.b".into()),
        ]
    );
}

#[test]
fn a_super_chain_past_the_inline_nesting_climbs_the_files_module() {
    // Two `super`s inside one inline module: one leaves the inline module,
    // the other leaves the file's module — which the resolver already
    // anchors, so the remainder is left for it.
    let i = imports("mod tests {\n    use super::super::x;\n}");
    assert_eq!(i, vec![("x".into(), "super.x".into(), "tests".into())]);
}

#[test]
fn anchors_outside_any_inline_module_are_recorded_as_written() {
    // The file-without-inline-modules case extracts exactly as before.
    let r = refs("fn t() { self::x(); super::y(); crate::z(); }");
    assert_eq!(
        r,
        vec![
            ("t".into(), RefKind::Call, "self.x".into()),
            ("t".into(), RefKind::Call, "super.y".into()),
            ("t".into(), RefKind::Call, "crate.z".into()),
        ]
    );
    let i = imports("mod tests {\n    use crate::x;\n}");
    assert_eq!(i, vec![("x".into(), "crate.x".into(), "tests".into())]);
}

#[test]
fn a_struct_literal_also_writes_the_types_fields() {
    let r = refs("struct Foo { x: u32 }\nfn f() { let _ = Foo { x: 1 }; }");
    assert!(r.contains(&("f".into(), RefKind::FieldWrite, "Foo".into())));
    // The provable half only: a typed-receiver field write emits nothing.
    let r = refs("fn g(mut foo: Foo) { foo.x = 1; }");
    assert!(r.is_empty());
}
