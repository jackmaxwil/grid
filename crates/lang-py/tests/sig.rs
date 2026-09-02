//! The interface identity for Python, where the call convention is wider
//! than Rust's: a parameter *name* is interface (callers pass it by keyword),
//! and so is a default *value* — changing one silently changes every caller
//! that omits the argument, which is precisely the hidden drift the sig
//! exists to name.
use grid_lang_py::definitions;

fn ids(src: &str) -> (String, String, String) {
    let defs = definitions(src).expect("valid Python");
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
    let before = "def f(x):\n    return x + 1\n";
    let after = "def f(x):\n    return x + 2\n";
    assert_ne!(datum(before), datum(after), "the body changed");
    assert_eq!(sig(before), sig(after), "the interface did not");
}

#[test]
fn a_parameter_rename_moves_the_sig() {
    // The Rust rule inverted: `f(count=1)` is a real call, so the name is
    // part of what a caller depends on.
    let before = "def f(count):\n    return count\n";
    let after = "def f(n):\n    return n\n";
    assert_ne!(sig(before), sig(after));
}

#[test]
fn a_changed_default_value_moves_the_sig() {
    let before = "def f(x=1):\n    return x\n";
    let after = "def f(x=2):\n    return x\n";
    assert_ne!(sig(before), sig(after), "callers that omit x now get 2");
    assert_ne!(
        sig(before),
        sig("def f(x):\n    return x\n"),
        "presence too"
    );
}

#[test]
fn annotations_and_markers_are_interface() {
    let base = "def f(a, b=1):\n    return a\n";
    for changed in [
        "def f(a: int, b=1):\n    return a\n",
        "def f(a, *, b=1):\n    return a\n",
        "def f(a, /, b=1):\n    return a\n",
        "def f(a, b=1, *args):\n    return a\n",
        "def f(a, b=1, **kw):\n    return a\n",
        "def f(a, b=1) -> int:\n    return a\n",
        "def f(b=1, a=None):\n    return a\n",
        "async def f(a, b=1):\n    return a\n",
    ] {
        assert_ne!(sig(base), sig(changed), "{changed} must move the sig");
    }
}

#[test]
fn a_decorator_is_interface() {
    let before = "def f():\n    return 1\n";
    let after = "@cache\ndef f():\n    return 1\n";
    assert_ne!(sig(before), sig(after));
    assert_ne!(
        sig(after),
        sig("@cache(maxsize=1)\ndef f():\n    return 1\n")
    );
}

#[test]
fn renaming_a_definition_holds_the_shape_and_moves_the_sig() {
    let before = "def f(x):\n    return x\n";
    let after = "def g(x):\n    return x\n";
    assert_eq!(shape(before), shape(after));
    assert_ne!(sig(before), sig(after));
}

#[test]
fn a_class_base_annotation_or_init_is_interface() {
    let base = "class C:\n    a: int\n    def __init__(self, x):\n        self.x = x\n";
    for changed in [
        "class C(Base):\n    a: int\n    def __init__(self, x):\n        self.x = x\n",
        "class C:\n    a: str\n    def __init__(self, x):\n        self.x = x\n",
        "class C:\n    a: int = 0\n    def __init__(self, x):\n        self.x = x\n",
        "class C:\n    a: int\n    def __init__(self, y):\n        self.y = y\n",
        "class C(metaclass=M):\n    a: int\n    def __init__(self, x):\n        self.x = x\n",
    ] {
        assert_ne!(sig(base), sig(changed), "{changed} must move the sig");
    }
}

#[test]
fn a_method_body_is_not_the_class_interface() {
    // A non-`__init__` method has its own definition and its own sig; folding
    // it into the class's would make every method edit look like a class
    // interface change.
    let before = "class C:\n    def m(self):\n        return 1\n";
    let after = "class C:\n    def m(self):\n        return 2\n";
    assert_ne!(datum(before), datum(after));
    assert_eq!(sig(before), sig(after));
}

#[test]
fn formatting_and_comments_move_nothing() {
    let plain = "def f(x, y=1):\n    return x + y\n";
    for same in [
        "def f(\n    x,\n    y=1,\n):\n    return x + y\n",
        "def f(x, y=1):\n    # a comment\n    return x + y\n",
        "def f(x, y=1):\n    \"\"\"A docstring is a body statement.\"\"\"\n    return x + y\n",
    ] {
        assert_eq!(sig(plain), sig(same), "sig must ignore layout");
    }
}

/// The plan's property: over a corpus, an unchanged datum implies an
/// unchanged sig. Local renaming is the sharp case — the datum
/// alpha-renames locals away, and the sig must not then move on one.
#[test]
fn an_unchanged_datum_implies_an_unchanged_sig() {
    const PAIRS: &[(&str, &str)] = &[
        (
            "def f(x):\n    tmp = x + 1\n    return tmp\n",
            "def f(x):\n    other = x + 1\n    return other\n",
        ),
        (
            "class C:\n    a: int\n    def m(self):\n        v = 1\n        return v\n",
            "class C:\n    a: int\n    def m(self):\n        w = 1\n        return w\n",
        ),
    ];
    for (a, b) in PAIRS {
        assert_eq!(datum(a), datum(b), "locals are not identity");
        assert_eq!(sig(a), sig(b), "datum held, so the sig must too");
    }
}

/// Conservatism: distinct interfaces get distinct sigs. A sig that collapsed
/// to a constant would answer "unchanged" forever, which is the one answer a
/// drift verb must never fabricate.
#[test]
fn a_sig_is_never_a_shared_constant() {
    let defs = definitions("def a():\n    pass\ndef b(x):\n    pass\nclass C:\n    pass\n")
        .expect("valid Python");
    let mut sigs: Vec<String> = defs.iter().map(|d| d.sig.to_string()).collect();
    let total = sigs.len();
    sigs.sort();
    sigs.dedup();
    assert_eq!(sigs.len(), total, "distinct interfaces need distinct sigs");
}

/// `sig_text` renders the header the sig identity encodes, as source: the
/// parameter names, annotations and defaults a caller depends on, and nothing
/// of the body. A method arrives indented off its class and must still parse.
#[test]
fn sig_text_is_the_header_and_never_the_body() {
    let text = grid_lang_py::sig_text("def f(a: int, b: str = 'x') -> bool:\n    return True\n")
        .expect("a def");
    assert_eq!(text, "def f(a: int, b: str = 'x') -> bool:");

    let method = grid_lang_py::sig_text("    def fill(self, buf):\n        return 0\n")
        .expect("a method, indented off its class");
    assert_eq!(method, "def fill(self, buf):");

    let cls = grid_lang_py::sig_text("class P(Base):\n    x: int = 1\n").expect("a class");
    assert_eq!(cls, "class P(Base):");

    let wrapped =
        grid_lang_py::sig_text("def g(\n    a: int,\n    b: int,\n) -> int:\n    return a\n")
            .expect("a multi-line header");
    assert!(wrapped.starts_with("def g("), "{wrapped}");
    assert!(wrapped.ends_with(':'), "{wrapped}");
    assert!(!wrapped.contains("return"), "{wrapped}");

    assert_eq!(grid_lang_py::sig_text("x = 1\n"), None, "not a definition");
}
