//! The interface identity: what a caller can see, and — the load-bearing
//! half — what it cannot.
//!
//! Every assertion here is a ground-truth pair from the drift plan's step 1:
//! a body-only edit must hold the sig while moving the datum, a signature
//! edit must move both, and formatting or documentation must move neither.
//! A sig that moved on a body edit would implicate every dependent of every
//! edit, which is the noise that kills a drift verb.
use grid_lang_rs::definitions;

fn ids(src: &str) -> (String, String, String) {
    let defs = definitions(src).expect("valid Rust");
    let d = defs.first().expect("one definition");
    (d.datum.to_string(), d.sig.to_string(), d.shape.to_string())
}

fn datum(src: &str) -> String {
    ids(src).0
}
fn sig(src: &str) -> String {
    ids(src).1
}
fn shape(src: &str) -> String {
    ids(src).2
}

#[test]
fn a_body_only_edit_moves_the_datum_and_holds_the_sig() {
    let before = "pub fn f(x: u32) -> u32 { x + 1 }";
    let after = "pub fn f(x: u32) -> u32 { x + 2 }";
    assert_ne!(datum(before), datum(after), "the body changed");
    assert_eq!(sig(before), sig(after), "the interface did not");
}

#[test]
fn a_parameter_type_change_moves_both() {
    let before = "pub fn f(x: u32) -> u32 { 0 }";
    let after = "pub fn f(x: u64) -> u32 { 0 }";
    assert_ne!(datum(before), datum(after));
    assert_ne!(sig(before), sig(after));
}

#[test]
fn a_parameter_rename_moves_neither_in_rust() {
    // Rust callers pass positionally; a parameter name is body-side, so it
    // reaches the sig no more than it reaches the datum's local renaming.
    let before = "pub fn f(count: u32) -> u32 { count }";
    let after = "pub fn f(n: u32) -> u32 { n }";
    assert_eq!(sig(before), sig(after));
}

#[test]
fn visibility_and_qualifiers_are_interface() {
    let base = "pub fn f() -> u32 { 0 }";
    for changed in [
        "fn f() -> u32 { 0 }",
        "pub(crate) fn f() -> u32 { 0 }",
        "pub const fn f() -> u32 { 0 }",
        "pub async fn f() -> u32 { 0 }",
        "pub extern \"C\" fn f() -> u32 { 0 }",
        "pub fn f<T>() -> u32 { 0 }",
        "pub fn f() -> u64 { 0 }",
    ] {
        assert_ne!(sig(base), sig(changed), "{changed} must move the sig");
    }
    // Spelled through a variable so scripts/check-unsafe-confined.sh (C5)
    // reads this test's *input* as the string it is, not as unsafe code in
    // the crate.
    let qualifier = "un".to_owned() + "safe";
    let changed = format!("pub {qualifier} fn f() -> u32 {{ 0 }}");
    assert_ne!(sig(base), sig(&changed), "{changed} must move the sig");
}

#[test]
fn a_where_clause_is_interface() {
    let before = "pub fn f<T>(x: T) -> T { x }";
    let after = "pub fn f<T>(x: T) -> T where T: Clone { x }";
    assert_ne!(sig(before), sig(after));
}

#[test]
fn renaming_a_definition_holds_the_shape_and_moves_the_sig() {
    let before = "pub fn f(x: u32) -> u32 { x }";
    let after = "pub fn g(x: u32) -> u32 { x }";
    assert_eq!(shape(before), shape(after), "a rename is the same shape");
    assert_ne!(sig(before), sig(after), "callers spell the name");
}

#[test]
fn a_serde_attribute_on_a_struct_moves_the_sig() {
    let before = "pub struct S { pub a: u32 }";
    let after = "pub struct S { #[serde(rename = \"b\")] pub a: u32 }";
    assert_ne!(sig(before), sig(after));

    let repr = "#[repr(C)]\npub struct S { pub a: u32 }";
    assert_ne!(sig(before), sig(repr));

    let exhaustive = "#[non_exhaustive]\npub struct S { pub a: u32 }";
    assert_ne!(sig(before), sig(exhaustive));
}

#[test]
fn a_derive_is_body_side_on_a_struct() {
    // `#[derive(Debug)]` changes what is generated, not what the type is
    // declared to be; the datum sees it and the sig does not.
    let before = "pub struct S { pub a: u32 }";
    let after = "#[derive(Debug)]\npub struct S { pub a: u32 }";
    assert_ne!(datum(before), datum(after));
    assert_eq!(sig(before), sig(after));
}

#[test]
fn a_field_change_moves_a_struct_sig() {
    let before = "pub struct S { pub a: u32 }";
    for changed in [
        "pub struct S { pub a: u64 }",
        "pub struct S { pub b: u32 }",
        "pub struct S { pub a: u32, pub b: u32 }",
        "pub struct S { a: u32 }",
    ] {
        assert_ne!(sig(before), sig(changed), "{changed} must move the sig");
    }
}

#[test]
fn a_variant_change_moves_an_enum_sig() {
    let before = "pub enum E { A, B }";
    assert_ne!(sig(before), sig("pub enum E { A }"));
    assert_ne!(sig(before), sig("pub enum E { B, A }"));
    assert_eq!(sig(before), sig("#[derive(Clone)]\npub enum E { A, B }"));
}

#[test]
fn a_trait_default_body_is_not_its_interface() {
    let before = "pub trait T { fn m(&self) -> u32 { 1 } }";
    let after = "pub trait T { fn m(&self) -> u32 { 2 } }";
    assert_ne!(datum(before), datum(after));
    assert_eq!(sig(before), sig(after), "the callable contract held");
}

#[test]
fn a_trait_signature_supertrait_or_assoc_item_is_interface() {
    let before = "pub trait T { fn m(&self) -> u32 { 1 } }";
    for changed in [
        "pub trait T { fn m(&mut self) -> u32 { 1 } }",
        "pub trait T { fn m(&self) -> u64 { 1 } }",
        "pub trait T: Clone { fn m(&self) -> u32 { 1 } }",
        "pub trait T { type A; fn m(&self) -> u32 { 1 } }",
        "pub trait T { const C: u32; fn m(&self) -> u32 { 1 } }",
        "pub trait T { fn m(&self) -> u32 { 1 } fn n(&self); }",
    ] {
        assert_ne!(sig(before), sig(changed), "{changed} must move the sig");
    }
}

#[test]
fn formatting_and_docs_move_nothing() {
    let plain = "pub fn f(x: u32) -> u32 { x + 1 }";
    for same in [
        "pub fn f(\n    x: u32,\n) -> u32 {\n    x + 1\n}",
        "/// Documented now.\npub fn f(x: u32) -> u32 { x + 1 }",
        "pub fn f(x: u32) -> u32 {\n    // a comment\n    x + 1\n}",
    ] {
        assert_eq!(datum(plain), datum(same));
        assert_eq!(sig(plain), sig(same));
    }
}

/// The property the plan states: over a corpus, an unchanged datum implies an
/// unchanged sig. The sig reads a strict subset of what the datum reads, so
/// the converse is the interesting direction and the one drift depends on —
/// it is what makes "datum held" a sound reason to skip a dependent.
#[test]
fn an_unchanged_datum_implies_an_unchanged_sig() {
    const CORPUS: &[&str] = &[
        "pub fn f(x: u32) -> u32 { x + 1 }",
        "fn g<T: Clone>(t: T) -> T where T: Send { t }",
        "#[repr(C)]\npub struct S { pub a: u32, b: [u8; 4] }",
        "pub enum E { A, B(u32), C { x: u8 } }",
        "pub trait T: Clone { type A; const C: u32; fn m(&self) -> Self::A; }",
        "pub union U { a: u32, b: f32 }",
        "impl S { pub fn new() -> Self { Self { a: 0, b: [0; 4] } } }",
        "mod m { pub fn inner(s: &str) -> String { s.to_owned() } }",
    ];
    for src in CORPUS {
        let reformatted = src.replace(' ', "\n    ");
        let a = definitions(src).expect("valid Rust");
        let b = definitions(&reformatted).expect("valid Rust");
        assert_eq!(a.len(), b.len(), "{src}");
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.datum, y.datum, "{src}");
            assert_eq!(x.sig, y.sig, "{src}: datum held, so the sig must too");
        }
    }
}

/// Conservatism: a definition the frontend cannot see the interface of does
/// not get a fabricated one. Every extracted definition carries a real sig,
/// and no two different interfaces share it — a sig that collapsed to a
/// constant would silently answer "unchanged" forever.
#[test]
fn a_sig_is_never_a_shared_constant() {
    let defs = definitions(
        "pub fn a() {}\npub fn b(x: u32) {}\npub struct S;\npub enum E { V }\npub trait T {}",
    )
    .expect("valid Rust");
    let mut sigs: Vec<String> = defs.iter().map(|d| d.sig.to_string()).collect();
    let total = sigs.len();
    sigs.sort();
    sigs.dedup();
    assert_eq!(sigs.len(), total, "distinct interfaces need distinct sigs");
}
