//! Reference extraction: what a definition reaches for, and nothing more.
//!
//! The contract this suite pins is *conservatism*. Every assertion is either
//! "this name was found, attributed to this definition" or "this construct
//! produced no reference at all" — the second kind matters more, because a
//! reference invented here becomes a false edge in the chart, and a false
//! proven edge is worse than a missing one.
use grid_datum::RefKind;
use grid_lang_py::references;

/// `(from, kind, dotted path)` — the shape assertions are written in.
fn refs(src: &str) -> Vec<(String, RefKind, String)> {
    references(src)
        .expect("parse")
        .refs
        .into_iter()
        .map(|r| (r.from, r.kind, r.path.join(".")))
        .collect()
}

fn imports(src: &str) -> Vec<(String, String, String)> {
    references(src)
        .expect("parse")
        .imports
        .into_iter()
        .map(|i| (i.alias, i.target, i.scope))
        .collect()
}

#[test]
fn a_call_is_attributed_to_the_definition_containing_it() {
    let src = "def caller():\n    callee()\n";
    assert_eq!(
        refs(src),
        [("caller".to_string(), RefKind::Call, "callee".to_string())]
    );
}

#[test]
fn nested_definitions_carry_their_dotted_callsign() {
    let src = "class C:\n    def m(self):\n        helper()\n";
    assert_eq!(
        refs(src),
        [("C.m".to_string(), RefKind::Call, "helper".to_string())]
    );
}

#[test]
fn module_level_code_is_attributed_to_the_empty_callsign() {
    // Import-time side effects belong to no definition. Attributing them to
    // one would invent an edge that the code does not have.
    let src = "configure()\n";
    assert_eq!(
        refs(src),
        [(String::new(), RefKind::Call, "configure".to_string())]
    );
}

#[test]
fn a_base_class_is_a_base_reference_and_a_decorator_is_a_decorator_reference() {
    let src = "@register\nclass C(Base):\n    pass\n";
    let got = refs(src);
    assert!(got.contains(&("C".to_string(), RefKind::Decorator, "register".to_string())));
    assert!(got.contains(&("C".to_string(), RefKind::Base, "Base".to_string())));
}

#[test]
fn a_decorator_is_attributed_to_what_it_decorates_not_to_the_enclosing_scope() {
    // `@d` runs because `f` exists, so the edge is f→d. Attributing it to the
    // module would lose the only definition that explains it.
    let src = "def f():\n    pass\n\n@d\ndef g():\n    pass\n";
    assert!(refs(src).contains(&("g".to_string(), RefKind::Decorator, "d".to_string())));
}

#[test]
fn a_called_decorator_names_the_decorator_not_the_call_result() {
    // `@d(x)` references `d`; `x` is an ordinary expression in scope.
    let src = "@d(x)\ndef f():\n    pass\n";
    let got = refs(src);
    assert!(got.contains(&("f".to_string(), RefKind::Decorator, "d".to_string())));
    assert_eq!(
        got.iter()
            .filter(|(_, k, _)| *k == RefKind::Decorator)
            .count(),
        1,
        "the decorator is `d`, not `d(x)` and not `x`: {got:?}"
    );
}

#[test]
fn a_dotted_call_keeps_its_whole_path() {
    let src = "def f():\n    a.b.c()\n";
    assert_eq!(
        refs(src),
        [("f".to_string(), RefKind::Call, "a.b.c".to_string())]
    );
}

#[test]
fn self_calls_are_distinguished_from_plain_calls_and_lose_the_receiver() {
    // `self.m()` resolves against the enclosing class, which a plain call
    // never does — so the two cannot share a kind.
    let src = "class C:\n    def a(self):\n        self.b()\n";
    assert_eq!(
        refs(src),
        [("C.a".to_string(), RefKind::SelfCall, "b".to_string())]
    );
}

#[test]
fn self_outside_a_method_is_an_ordinary_name() {
    // A module-level variable called `self` is not a receiver. Treating it as
    // one would bind a name to a class that has nothing to do with it.
    let src = "self.b()\n";
    assert_eq!(
        refs("self.b()\n"),
        [(String::new(), RefKind::Call, "self.b".to_string())],
        "module-level `self` is a plain name: {src}"
    );
}

#[test]
fn a_call_on_a_subscript_or_literal_produces_no_reference() {
    // `handlers[k]()` names nothing statically resolvable. The proven tier
    // records nothing rather than guessing.
    // `handlers[k]()` has no statically resolvable callee, so no Call is
    // recorded for it — but `handlers` and `k` are still ordinary names and
    // the walk must not stop.
    let got = refs("def f():\n    handlers[k]()\n");
    assert!(
        !got.iter().any(|(_, k, _)| *k == RefKind::Call),
        "a subscript call names nothing: {got:?}"
    );
    // Same for a call on a literal: the receiver is a value, not a name.
    let got = refs("def f():\n    (1).bit_length()\n");
    assert!(
        !got.iter().any(|(_, k, _)| *k == RefKind::Call),
        "a method on a literal names nothing: {got:?}"
    );
}

#[test]
fn arguments_of_a_call_are_themselves_walked() {
    // `outer(inner())` is two calls, and the inner one is a real edge.
    let got = refs("def f():\n    outer(inner())\n");
    assert!(got.contains(&("f".to_string(), RefKind::Call, "outer".to_string())));
    assert!(got.contains(&("f".to_string(), RefKind::Call, "inner".to_string())));
}

#[test]
fn calls_inside_comprehensions_and_f_strings_are_found() {
    // These are the forms a hand-written matcher forgets; the generated
    // visitor is used precisely so that it cannot.
    let got = refs("def f():\n    return [g(x) for x in h()]\n");
    assert!(got.iter().any(|(_, _, p)| p == "g"), "{got:?}");
    assert!(got.iter().any(|(_, _, p)| p == "h"), "{got:?}");
    let got = refs("def f():\n    return f\"{fmt(x)}\"\n");
    assert!(got.iter().any(|(_, _, p)| p == "fmt"), "{got:?}");
}

#[test]
fn default_arguments_evaluate_in_the_enclosing_scope() {
    // `def f(x=make())` runs `make()` at definition time, in the module, not
    // inside `f`. Attributing it to `f` would claim `f` calls `make`.
    let got = refs("def f(x=make()):\n    pass\n");
    assert!(
        got.contains(&(String::new(), RefKind::Call, "make".to_string())),
        "{got:?}"
    );
}

#[test]
fn plain_imports_bind_their_first_segment_unless_aliased() {
    assert_eq!(
        imports("import a.b.c\n"),
        [("a".to_string(), "a.b.c".to_string(), String::new())]
    );
    assert_eq!(
        imports("import a.b.c as abc\n"),
        [("abc".to_string(), "a.b.c".to_string(), String::new())]
    );
}

#[test]
fn from_imports_record_the_full_target_path() {
    assert_eq!(
        imports("from pkg.mod import sym\n"),
        [("sym".to_string(), "pkg.mod.sym".to_string(), String::new())]
    );
    assert_eq!(
        imports("from pkg.mod import sym as s\n"),
        [("s".to_string(), "pkg.mod.sym".to_string(), String::new())]
    );
}

#[test]
fn relative_imports_keep_their_dots_for_the_resolver_to_anchor() {
    assert_eq!(
        imports("from . import sib\n"),
        [("sib".to_string(), ".sib".to_string(), String::new())]
    );
    assert_eq!(
        imports("from ..pkg import sym\n"),
        [("sym".to_string(), "..pkg.sym".to_string(), String::new())]
    );
}

#[test]
fn a_star_import_binds_nothing_followable() {
    // `from x import *` binds names we cannot enumerate without importing.
    // Recording a binding for `*` would be a name that resolves to nothing.
    assert_eq!(imports("from x import *\n"), []);
}

#[test]
fn a_function_local_import_records_its_scope() {
    // Scope is what keeps a local import from rewriting names elsewhere.
    assert_eq!(
        imports("def f():\n    import os\n"),
        [("os".to_string(), "os".to_string(), "f".to_string())]
    );
}

#[test]
fn extraction_never_panics_and_agrees_with_itself() {
    // The same source twice is the same references: extraction is a function,
    // and every downstream determinism claim rests on that.
    let src = "import a\n\n@d\nclass C(B):\n    def m(self):\n        self.n(a.b())\n";
    assert_eq!(references(src).unwrap(), references(src).unwrap());
}

#[test]
fn invalid_python_is_an_error_not_a_panic() {
    assert!(references("def (:\n").is_err());
}

#[test]
fn a_self_attribute_write_is_a_field_write_on_the_enclosing_class() {
    let r = refs("class Foo:\n    def m(self):\n        self.x = 1\n        self.y += 1\n        self.z: int = 2\n");
    let writes: Vec<_> = r
        .iter()
        .filter(|(_, kind, _)| *kind == RefKind::FieldWrite)
        .collect();
    // Three write forms, one target each: the class, lexically known.
    assert_eq!(writes.len(), 3);
    assert!(writes
        .iter()
        .all(|(from, _, path)| from == "Foo.m" && path == "Foo"));
}

#[test]
fn an_attribute_write_on_a_typed_receiver_produces_nothing() {
    // `foo.x = 1` needs foo's type; types are modeled, and this frontend
    // emits no guesses. A module-level `self` is not a receiver either.
    let r = refs("def f(foo):\n    foo.x = 1\nself = object()\nself.x = 1\n");
    assert!(r.iter().all(|(_, kind, _)| *kind != RefKind::FieldWrite));
}
