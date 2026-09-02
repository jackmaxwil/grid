//! Receiver facts: what the file states about a method call's receiver,
//! and what a definition states about itself as a callee. One assertion per
//! origin, and the conservatism cases — the bindings this frontend must
//! refuse to type, because a hint invented here becomes an edge the
//! language never established.
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
use grid_datum::{Receiver, ReceiverForm, ReceiverOrigin, RefKind};
use grid_lang_rs::{definitions, references};

/// The receiver hint of every method call, in source order.
fn hints(src: &str) -> Vec<Option<Receiver>> {
    references(src)
        .expect("valid Rust")
        .refs
        .into_iter()
        .filter(|r| r.kind == RefKind::MethodCall)
        .map(|r| r.receiver)
        .collect()
}

/// `(type path, form, origin)` of the one call of `m` in `src` — the
/// `.m()` every case below is about; a chain's other calls are walked too.
fn hint(src: &str) -> Option<(String, ReceiverForm, ReceiverOrigin)> {
    let mut all: Vec<Option<Receiver>> = references(src)
        .expect("valid Rust")
        .refs
        .into_iter()
        .filter(|r| r.kind == RefKind::MethodCall && r.path.last().is_some_and(|m| m == "m"))
        .map(|r| r.receiver)
        .collect();
    assert_eq!(all.len(), 1, "one call of `m` expected in {src:?}");
    all.pop()
        .flatten()
        .map(|r| (r.ty.path.join("."), r.ty.form, r.origin))
}

const FOO: &str = "pub struct Foo { bar: Bar, n: u32 }\npub struct Bar;\n\
                   impl Foo {\n    pub fn new() -> Self { Foo { bar: Bar, n: 0 } }\n\
                   pub fn open() -> Result<Self, String> { Ok(Self::new()) }\n\
                   pub fn maybe() -> Option<Foo> { None }\n\
                   pub fn g(&self) -> &Bar { &self.bar }\n    pub fn m(&self) {}\n}\n\
                   impl Bar { pub fn m(&self) {} }\n\
                   pub fn make() -> Foo { Foo::new() }\n";

#[test]
fn a_parameter_annotation_is_a_written_receiver() {
    let src = format!("{FOO}fn f(x: &Foo) {{ x.m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Ref, ReceiverOrigin::Param))
    );
    let src = format!("{FOO}fn f(x: &mut Foo) {{ x.m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::RefMut, ReceiverOrigin::Param))
    );
    let src = format!("{FOO}fn f(x: Foo) {{ x.m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::Param))
    );
}

#[test]
fn self_in_an_annotation_is_the_impls_own_type() {
    let src = "struct Bar;\nimpl Bar {\n    fn m(&self) {}\n    fn f(x: &Self) { x.m(); }\n}";
    assert_eq!(
        hint(src),
        Some(("Bar".into(), ReceiverForm::Ref, ReceiverOrigin::Param))
    );
}

#[test]
fn a_let_annotation_is_written_and_an_initializer_is_inferred() {
    let src = format!("{FOO}fn f() {{ let x: Foo = make(); x.m(); }}");
    assert_eq!(
        hint(&src),
        Some((
            "Foo".into(),
            ReceiverForm::Value,
            ReceiverOrigin::LetAnnotated
        ))
    );
    let src = format!("{FOO}fn f() {{ let x = Foo::new(); x.m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::LocalCtor))
    );
    let src = format!("{FOO}fn f() {{ let x = make(); x.m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::LocalCtor))
    );
    let src = format!("{FOO}fn f() {{ let x = Foo {{ bar: Bar, n: 1 }}; x.m(); }}");
    assert_eq!(
        hint(&src),
        Some((
            "Foo".into(),
            ReceiverForm::Value,
            ReceiverOrigin::LocalLiteral
        ))
    );
}

#[test]
fn a_reference_to_a_typed_local_sets_the_form() {
    let src = format!("{FOO}fn f() {{ let x = Foo::new(); (&x).m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Ref, ReceiverOrigin::LocalCtor))
    );
}

#[test]
fn a_field_of_self_is_typed_by_the_same_files_struct() {
    let src = format!("{FOO}impl Foo {{ fn f(&self) {{ self.bar.m(); }} }}");
    assert_eq!(
        hint(&src),
        Some(("Bar".into(), ReceiverForm::Value, ReceiverOrigin::SelfField))
    );
    let src = format!("{FOO}fn f(x: &Foo) {{ x.bar.m(); }}");
    assert_eq!(
        hint(&src),
        Some((
            "Bar".into(),
            ReceiverForm::Value,
            ReceiverOrigin::TypedField
        ))
    );
}

#[test]
fn a_field_of_a_struct_elsewhere_carries_the_base_and_the_name() {
    // The struct is another file's: the resolver looks the field up.
    let src = "impl Foo { fn f(&self) { self.bar.m(); } }";
    let r = hints(src).pop().flatten().expect("a hint");
    assert_eq!(r.ty.path, ["Foo"]);
    assert_eq!(r.origin, ReceiverOrigin::TypedField);
    assert_eq!(r.field.as_deref(), Some("bar"));
}

#[test]
fn a_question_mark_or_unwrap_on_a_written_wrapper_reaches_the_inner_type() {
    let src = format!("{FOO}fn f() -> Result<(), String> {{ Foo::open()?.m(); Ok(()) }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::Unwrapped))
    );
    let src = format!("{FOO}fn f() {{ let x = Foo::maybe().unwrap(); x.m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::Unwrapped))
    );
    let src = format!("{FOO}fn f() {{ Foo::open().expect(\"open\").m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::Unwrapped))
    );
}

#[test]
fn a_chain_through_a_same_file_written_return_is_typed() {
    let src = format!("{FOO}fn f(x: &Foo) {{ x.g().m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Bar".into(), ReceiverForm::Ref, ReceiverOrigin::ChainReturn))
    );
    let src = format!("{FOO}fn f() {{ Foo::new().m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::LocalCtor))
    );
}

/// Conservatism: an annotated closure parameter carries no hint. The
/// closure's call site fixes the type, not the annotation this frontend
/// reads, so a hint here is deliberately refused in v1.
#[test]
fn a_closure_parameter_carries_no_hint() {
    let src = format!("{FOO}fn f() {{ let c = |x: &Foo| x.m(); }}");
    assert_eq!(hint(&src), None);
    // And it shadows an outer typed binding of the same name.
    let src = format!("{FOO}fn f(x: &Foo) {{ let c = |x| x.m(); }}");
    assert_eq!(hint(&src), None);
}

/// Conservatism: a shadowed local takes the innermost binding. The mutation
/// that turns this red is dropping the block-scope push — the inner `let x
/// = 3` then leaks and the outer typed `x` is read as typed again below.
#[test]
fn a_shadowed_local_takes_the_innermost_binding() {
    let src = format!("{FOO}fn f() {{ let x = Foo::new(); {{ let x = 3; x.m(); }} }}");
    assert_eq!(hint(&src), None);
    // After the block the outer binding is back.
    let src = format!("{FOO}fn f() {{ let x = Foo::new(); {{ let x = 3; }} x.m(); }}");
    assert_eq!(
        hint(&src),
        Some(("Foo".into(), ReceiverForm::Value, ReceiverOrigin::LocalCtor))
    );
    // A `let` rebinding in the same block also shadows, in source order.
    let src = format!("{FOO}fn f() {{ let x = Foo::new(); let x = 3; x.m(); }}");
    assert_eq!(hint(&src), None);
}

/// Conservatism: a wrapper is recorded as the wrapper, never looked through.
#[test]
fn a_wrapper_records_the_wrapper_with_args_and_never_the_inner_type() {
    let src = format!("{FOO}fn f(x: Option<Foo>) {{ x.m(); }}");
    let r = hints(&src).pop().flatten().expect("a hint");
    assert_eq!(r.ty.path, ["Option"]);
    assert!(r.ty.args);
    assert_eq!(
        r.ty.inner.as_ref().map(|i| i.path.clone()),
        Some(vec!["Foo".into()])
    );
}

#[test]
fn pattern_bindings_for_loops_and_type_parameters_carry_no_hint() {
    let src = format!("{FOO}fn f(v: Vec<Foo>) {{ for x in v {{ x.m(); }} }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f(o: Option<Foo>) {{ if let Some(x) = o {{ x.m(); }} }}");
    assert_eq!(hint(&src), None);
    let src =
        format!("{FOO}fn f(o: Option<Foo>) {{ match o {{ Some(x) => x.m(), None => {{}} }} }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f<T>(x: &T) {{ x.m(); }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f(x: &impl Sized) {{ x.m(); }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f(x: &dyn Sized) {{ x.m(); }}");
    assert_eq!(hint(&src), None);
    // A trait's own type parameter shadows the file's same-named struct
    // inside a default body: `Foo` here is the parameter, and rustc binds
    // `x.m()` to the bound's method, never to `Foo::m`.
    let src = format!("{FOO}pub trait Visit<Foo: Sized> {{ fn f(&self, x: &Foo) {{ x.m(); }} }}");
    assert_eq!(hint(&src), None);
    let src = format!(
        "{FOO}pub trait Visit<Foo: Sized> {{ fn f(&self, x: Foo) {{ let y: &Foo = &x; y.m(); }} }}"
    );
    assert_eq!(hint(&src), None);
}

/// Conservatism: a reference to a reference is not the written type's
/// first lookup step — rustc probes `&&Foo` before `&Foo`, and a trait
/// implemented for `&Foo` matches there — so it records no hint at all.
#[test]
fn a_reference_to_a_reference_carries_no_hint() {
    let src = format!("{FOO}fn f(x: &&Foo) {{ x.m(); }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f(x: &mut &Foo) {{ x.m(); }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f() {{ let x: &&Foo = &&Foo::new(); x.m(); }}");
    assert_eq!(hint(&src), None);
    // `&` around an already-referenced binding is the same double reference.
    let src = format!("{FOO}fn f(x: &Foo) {{ (&x).m(); }}");
    assert_eq!(hint(&src), None);
    // `&` around a field the index will type: the field's own form is not
    // known here, so the form cannot be composed and nothing is recorded.
    let src = "impl Foo { fn f(&self) { (&self.bar).m(); } }";
    assert_eq!(hint(src), None);
}

/// Conservatism: a base written as a multi-segment path is never this
/// file's same-named type. `other::Foo`'s field, return and constructor are
/// not in this file's tables; the field goes to the index by its written
/// path, the rest record nothing.
#[test]
fn a_multi_segment_base_is_never_this_files_same_named_type() {
    let src = format!("{FOO}fn f(x: &other::Foo) {{ x.bar.m(); }}");
    let r = hints(&src).pop().flatten().expect("a hint");
    assert_eq!(r.ty.path, ["other", "Foo"]);
    assert_eq!(r.origin, ReceiverOrigin::TypedField);
    assert_eq!(r.field.as_deref(), Some("bar"));
    let src = format!("{FOO}fn f(x: &other::Foo) {{ x.g().m(); }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f() {{ let x = other::Foo::new(); x.m(); }}");
    assert_eq!(hint(&src), None);
    let src = format!("{FOO}fn f() {{ other::make().m(); }}");
    assert_eq!(hint(&src), None);
}

#[test]
fn a_chain_through_an_unwritten_return_carries_no_hint() {
    let src = format!("{FOO}fn h(x: &Foo) -> impl Sized {{ 1 }}\nfn f(x: &Foo) {{ h(x).m(); }}");
    assert_eq!(hint(&src), None);
    // `m` has no written return, so the outer call's receiver is untyped;
    // the inner call's receiver `x` is typed, as ever.
    let src = format!("{FOO}fn f(x: &Foo) {{ x.m().m(); }}");
    let all = hints(&src);
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(Option::is_none), "{all:?}");
    assert!(all.iter().any(Option::is_some), "{all:?}");
}

fn method_facts(src: &str, callsign: &str) -> grid_datum::MethodFacts {
    definitions(src)
        .expect("valid Rust")
        .into_iter()
        .find(|d| d.callsign == callsign)
        .unwrap_or_else(|| panic!("no {callsign}"))
        .method
        .unwrap_or_else(|| panic!("{callsign} has no method facts"))
}

#[test]
fn an_inherent_method_records_its_receiver_form_and_its_impls_plainness() {
    let m = method_facts("struct Foo;\nimpl Foo { fn m(&self) {} }", "Foo.m");
    assert_eq!(m.recv, Some(ReceiverForm::Ref));
    assert!(m.plain_impl);
    let m = method_facts("struct Foo;\nimpl Foo { fn m(&mut self) {} }", "Foo.m");
    assert_eq!(m.recv, Some(ReceiverForm::RefMut));
    let m = method_facts("struct Foo;\nimpl Foo { fn m(self) {} }", "Foo.m");
    assert_eq!(m.recv, Some(ReceiverForm::Value));
    // An explicitly typed receiver is not one the first-step rule reaches.
    let m = method_facts(
        "struct Foo;\nimpl Foo { fn m(self: Box<Self>) {} }",
        "Foo.m",
    );
    assert_eq!(m.recv, None);
}

#[test]
fn plainness_is_no_bounds_no_where_clause_no_instantiation() {
    let plain = |src: &str| method_facts(src, "Foo.m").plain_impl;
    assert!(plain(
        "struct Foo<T>(T);\nimpl<T> Foo<T> { fn m(&self) {} }"
    ));
    assert!(plain(
        "struct Foo<'a>(&'a u8);\nimpl<'a> Foo<'a> { fn m(&self) {} }"
    ));
    assert!(!plain("struct Foo<T>(T);\nimpl Foo<u8> { fn m(&self) {} }"));
    assert!(!plain(
        "struct Foo<T>(T);\nimpl<T: Clone> Foo<T> { fn m(&self) {} }"
    ));
    assert!(!plain(
        "struct Foo<T>(T);\nimpl<T> Foo<T> where T: Clone { fn m(&self) {} }"
    ));
}

#[test]
fn an_associated_function_records_its_written_return_with_self_substituted() {
    let src = "struct Foo;\nimpl Foo {\n    fn new() -> Self { Foo }\n    fn open() -> Result<Self, u8> { Ok(Foo) }\n    fn none() {}\n}";
    let m = method_facts(src, "Foo.new");
    assert_eq!(m.recv, None);
    assert_eq!(
        m.ret.as_ref().map(|t| t.path.clone()),
        Some(vec!["Foo".into()])
    );
    let m = method_facts(src, "Foo.open");
    let ret = m.ret.expect("written");
    assert_eq!(ret.path, ["Result"]);
    assert!(ret.args);
    assert_eq!(ret.inner.map(|i| i.path), Some(vec!["Foo".into()]));
    assert_eq!(method_facts(src, "Foo.none").ret, None);
}

#[test]
fn a_trait_impl_method_and_a_type_have_no_method_facts() {
    let defs =
        definitions("trait T { fn m(&self); }\nstruct Foo;\nimpl T for Foo { fn m(&self) {} }")
            .expect("valid Rust");
    for d in defs {
        assert!(
            d.method.is_none(),
            "{} should carry no method facts",
            d.callsign
        );
    }
}

#[test]
fn a_struct_declares_its_field_types() {
    let refs =
        references("struct Foo { bar: Bar, n: u32, t: (u8, u8) }\nmod m { struct Baz { x: Foo } }")
            .expect("valid Rust");
    let fields: Vec<(String, String, String)> = refs
        .field_types
        .iter()
        .map(|(s, f, t)| (s.clone(), f.clone(), t.path.join(".")))
        .collect();
    assert_eq!(
        fields,
        vec![
            ("Foo".into(), "bar".into(), "Bar".into()),
            ("Foo".into(), "n".into(), "u32".into()),
            ("m.Baz".into(), "x".into(), "Foo".into()),
        ]
    );
}

/// Identity: the hints ride beside the definition, never inside it.
#[test]
fn hints_never_move_any_identity() {
    let src = "struct Foo;\nimpl Foo { fn m(&self) {} }\nfn f(x: &Foo) { x.m(); }";
    let defs = definitions(src).expect("valid Rust");
    let f = defs.iter().find(|d| d.callsign == "f").expect("f");
    // The same definition, extracted from a file where the receiver has no
    // hint at all (no `Foo` in scope to type it by): identical identities.
    let bare = definitions("fn f(x: &Foo) { x.m(); }").expect("valid Rust");
    let g = bare.first().expect("f");
    assert_eq!((&f.datum, &f.sig, &f.shape), (&g.datum, &g.sig, &g.shape));
}
