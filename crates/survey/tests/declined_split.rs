//! What the resolver declined, split by why it declined.
//!
//! One number for three populations was the honesty gap: a path into `std`
//! costs a claim about this repo nothing, a call through an untyped receiver
//! is a hole no static rule closes, and a name this tree could have held is a
//! hole in exactly what `grid drift` claims. This suite pins the split, and
//! pins that a method call still buys no edge.
use std::path::Path;

use grid_survey::{run, Cache, Survey};

fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "grid")
            .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
            .env("GIT_COMMITTER_NAME", "grid")
            .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
            .output()
            .expect("run git")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    };
    git(&["init", "-q", "-b", "main"]);
    for (path, text) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(full, text).expect("write");
    }
    git(&["add", "-A"]);
    git(&["commit", "-qm", "surveyed"]);
    dir
}

fn survey(root: &Path) -> Survey {
    let mut cache = Cache::new();
    run(root, &mut cache).expect("survey")
}

fn split(s: &Survey) -> (u64, u64, u64) {
    (s.declined_external, s.declined_method, s.declined_internal)
}

/// Ground truth, Rust: one call into `std`, one method call on a typed local,
/// one name only a glob import could have supplied, one call to a sibling.
/// One edge, and one of each decline.
#[test]
fn rust_declines_split_into_external_method_and_internal() {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        (
            "src/lib.rs",
            "use std::collections::HashMap;\n\
             use crate::hidden::*;\n\
             pub mod hidden;\n\
             pub struct Local;\n\
             impl Local {\n  pub fn frob(&self) {}\n}\n\
             pub fn sibling() {}\n\
             pub fn caller(x: &Local) {\n\
             \x20   HashMap::<u8, u8>::new();\n\
             \x20   x.frob();\n\
             \x20   glob_supplied();\n\
             \x20   sibling();\n\
             }\n",
        ),
        // Declared `pub mod hidden;` but never charted: the glob's supply is
        // a name this tree could have held, which is the internal case.
        ("src/hidden.rs", "// no definitions here\n"),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.edges, 1, "only the sibling call is proven");
    assert_eq!(
        split(&s),
        (1, 1, 1),
        "declined split for {} refs",
        s.declined_refs
    );
}

/// Ground truth, Python: one call into the standard library, one method call
/// on a local value, one name only a glob import could have supplied, one
/// call to a sibling definition.
#[test]
fn python_declines_split_into_external_method_and_internal() {
    let dir = repo(&[
        ("pyproject.toml", "[project]\nname = \"fixture\"\n"),
        (
            "pkg/mod.py",
            "import os\n\
             from pkg.hidden import *\n\
             \n\
             def sibling():\n    pass\n\
             \n\
             def caller(x):\n\
             \x20   os.getcwd()\n\
             \x20   x.frob()\n\
             \x20   glob_supplied()\n\
             \x20   sibling()\n",
        ),
        ("pkg/hidden.py", "# no definitions here\n"),
        ("pkg/__init__.py", ""),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.edges, 1, "only the sibling call is proven");
    assert_eq!(
        split(&s),
        (1, 1, 1),
        "declined split for {} refs",
        s.declined_refs
    );
}

/// A qualified call through an imported module is *not* a method call: the
/// import binds it, so it is a proven edge and is never declined. The split
/// must not be bought by losing edges.
#[test]
fn an_imported_modules_function_still_resolves_in_python() {
    let dir = repo(&[
        ("pyproject.toml", "[project]\nname = \"fixture\"\n"),
        ("pkg/__init__.py", ""),
        ("pkg/util.py", "def helper():\n    pass\n"),
        (
            "pkg/mod.py",
            "from pkg import util\n\ndef caller():\n    util.helper()\n",
        ),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.edges, 1, "`util.helper()` is bound by the import");
    assert_eq!(
        s.declined_method, 0,
        "a bound attribute call is not a method call"
    );
}

/// Conservatism, exactly as the plan states it: a method call on a receiver
/// whose type is syntactically obvious still yields **no edge**, and is
/// counted as `method`. Types are `modeled`; this survey emits none.
#[test]
fn a_syntactically_obvious_receiver_still_yields_no_edge() {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        (
            "src/lib.rs",
            "pub struct Foo;\n\
             impl Foo {\n\
             \x20   pub fn new() -> Self {\n        Self\n    }\n\
             \x20   pub fn m(&self) {}\n\
             }\n\
             pub fn caller() {\n    Foo::new().m();\n}\n",
        ),
    ]);
    let s = survey(dir.path());
    // `Foo::new()` is a plain path call and is proven; `.m()` is not.
    assert_eq!(
        s.edges, 1,
        "only `Foo::new()` is proven — `.m()` would need the receiver's type"
    );
    assert_eq!(
        s.declined_method, 1,
        "the method call must be counted, not invisible"
    );
}

/// Property (rung 3): the total is the sum, over every corpus this repo has.
#[test]
fn the_total_is_always_the_sum_of_the_three() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let s = survey(root);
    assert_eq!(
        s.declined_refs,
        s.declined_external + s.declined_method + s.declined_internal,
        "declined split does not add up on {}",
        root.display()
    );
    assert!(
        s.declined_method > 0,
        "a Rust tree this size has method calls to count"
    );
}
