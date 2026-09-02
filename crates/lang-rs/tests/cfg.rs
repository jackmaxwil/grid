//! The cfg fact: every `#[cfg]` that conditions a definition, as written,
//! conjoined over the item and its enclosing modules.
use grid_lang_rs::{definitions, file_facts};

fn cfg_of(source: &str, callsign: &str) -> String {
    definitions(source)
        .expect("parses")
        .into_iter()
        .find(|d| d.callsign == callsign)
        .unwrap_or_else(|| panic!("no definition `{callsign}`"))
        .cfg
}

#[test]
fn an_unconditioned_definition_carries_no_cfg() {
    assert_eq!(cfg_of("pub fn f() {}", "f"), "");
}

#[test]
fn an_items_own_cfg_is_recorded_as_written_minus_whitespace() {
    let src = "#[cfg(test)] fn t() {}\n#[cfg(feature = \"tui\")] fn ui() {}\n\
               #[cfg(all(unix, not(feature = \"a\")))] fn u() {}\n";
    assert_eq!(cfg_of(src, "t"), "test");
    assert_eq!(cfg_of(src, "ui"), "feature=\"tui\"");
    assert_eq!(cfg_of(src, "u"), "all(unix,not(feature=\"a\"))");
}

#[test]
fn enclosing_modules_conjoin_outermost_first() {
    let src = "#[cfg(test)] mod tests {\n    #[cfg(feature = \"x\")] mod inner {\n        fn t() {}\n    }\n    fn plain() {}\n}\n";
    assert_eq!(cfg_of(src, "tests.inner.t"), "all(test,feature=\"x\")");
    assert_eq!(cfg_of(src, "tests.plain"), "test");
}

#[test]
fn impl_trait_and_nested_items_inherit() {
    let src = "#[cfg(test)] mod m {\n    struct S;\n    impl S { fn a(&self) {} }\n    \
               trait T { fn d(&self) {} }\n    fn outer() { fn inner() {} }\n}\n";
    for callsign in ["m.S", "m.S.a", "m.T", "m.T.d", "m.outer", "m.outer.inner"] {
        assert_eq!(cfg_of(src, callsign), "test", "{callsign}");
    }
}

#[test]
fn a_cfg_attr_is_its_own_predicate_text() {
    let src = "#[cfg_attr(kani, allow(dead_code))] fn f() {}\n";
    assert_eq!(cfg_of(src, "f"), "cfg_attr(kani,allow(dead_code))");
}

#[test]
fn two_cfgs_on_one_item_conjoin() {
    let src = "#[cfg(test)]\n#[cfg(unix)]\nfn f() {}\n";
    assert_eq!(cfg_of(src, "f"), "all(test,unix)");
}

/// An out-of-line module's file cannot see the `#[cfg]` on its `mod x;`, so
/// the declaration is reported for the survey to join back on.
#[test]
fn out_of_line_module_declarations_report_their_cfg() {
    let src = "#[cfg(kani)] mod proofs;\nmod plain;\nmod outer { #[cfg(test)] mod tests; }\n";
    let (_, refs) = file_facts(src).expect("parses");
    assert_eq!(
        refs.mod_cfgs,
        vec![
            ("proofs".to_owned(), "kani".to_owned()),
            ("outer.tests".to_owned(), "test".to_owned()),
        ]
    );
}
