//! The datum invariants v1 actually holds — and the one it does not.
//!
//! The contract pinned: a consumer keys caches and gates on the datum, so
//! formatting and comments must never move it (a doc edit re-verifying the
//! world is the failure), while a local rename moving it is the documented
//! v1 ceiling — asserted here so silently *gaining* the invariance later
//! shows up as a test to update, not an unnoticed epoch change.
use grid_lang_rs::definitions;

fn datum_of(src: &str) -> String {
    let defs = definitions(src).expect("valid Rust");
    defs.first().expect("one definition").datum.to_string()
}

#[test]
fn formatting_never_moves_the_datum() {
    let a = datum_of("fn f(x: u32) -> u32 { x + 1 }");
    let b = datum_of("fn f(\n    x: u32,\n) -> u32 {\n    x + 1\n}");
    assert_eq!(a, b);
}

#[test]
fn comments_and_doc_comments_never_move_the_datum() {
    let plain = datum_of("fn f() -> u32 { 1 }");
    let commented = datum_of("fn f() -> u32 {\n    // a comment\n    1\n}");
    let documented = datum_of("/// Documented now.\nfn f() -> u32 { 1 }");
    let inner = datum_of("fn f() -> u32 { #![doc = \"inner\"] 1 }");
    assert_eq!(plain, commented);
    // The syn trap docs/LANG_RS.md names: doc comments are AST attributes,
    // and without erasure this assertion is exactly what fails.
    assert_eq!(plain, documented);
    assert_eq!(plain, inner);
}

#[test]
fn a_one_tuple_comma_is_grammar_not_formatting() {
    // `(1,)` is a tuple and `(1)` is a parenthesized 1 — the one trailing
    // comma canonicalization must never strip.
    assert_ne!(
        datum_of("fn f() -> (u32,) { (1,) }"),
        datum_of("fn f() -> u32 { (1) }")
    );
}

#[test]
fn a_body_change_moves_the_datum() {
    assert_ne!(
        datum_of("fn f() -> u32 { 1 }"),
        datum_of("fn f() -> u32 { 2 }")
    );
}

#[test]
fn local_rename_moves_the_datum_v1_ceiling() {
    // NOT the ontology's full invariant — the documented v1 ceiling. If an
    // alpha renamer lands, this flips to assert_eq and the epoch bumps.
    assert_ne!(
        datum_of("fn f() -> u32 { let a = 1; a }"),
        datum_of("fn f() -> u32 { let b = 1; b }")
    );
}

#[test]
fn nesting_and_position_never_reach_the_hash() {
    let top = datum_of("fn inner() -> u32 { 1 }");
    let nested = definitions("fn outer() { fn inner() -> u32 { 1 } }").expect("valid Rust");
    let nested_inner = nested
        .iter()
        .find(|d| d.callsign == "outer.inner")
        .expect("nested definition extracted");
    assert_eq!(top, nested_inner.datum.to_string());
}

#[test]
fn impl_methods_and_trait_impls_get_distinct_callsigns() {
    let src = "struct Foo;\n\
               trait Frob { fn go(&self); }\n\
               impl Foo { fn go(&self) {} }\n\
               impl Frob for Foo { fn go(&self) {} }";
    let defs = definitions(src).expect("valid Rust");
    let signs: Vec<&str> = defs.iter().map(|d| d.callsign.as_str()).collect();
    assert!(signs.contains(&"Foo.go"));
    assert!(signs.contains(&"Foo@Frob.go"));
    assert!(signs.contains(&"Foo"));
    assert!(signs.contains(&"Frob"));
}
