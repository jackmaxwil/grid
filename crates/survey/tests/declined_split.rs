//! What the resolver declined, split by why it declined.
//!
//! One number for three populations was the honesty gap: a path into `std`
//! costs a claim about this repo nothing, a call through an untyped receiver
//! is a hole no static rule closes, and a name this tree could have held is a
//! hole in exactly what `grid drift` claims. This suite pins the split, and
//! pins that a method call still buys no edge.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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

/// Ground truth, Rust: one call into `std`, one method call on a receiver
/// the file cannot type (a `for` binding), one name only a glob import
/// could have supplied, one call to a sibling. One edge, and one of each
/// decline.
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
             pub fn caller(v: Vec<Local>) {\n\
             \x20   HashMap::<u8, u8>::new();\n\
             \x20   for x in v { x.frob(); }\n\
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
    // The identity `decline()` enforces by construction, checked once here
    // rather than by surveying the whole workspace.
    assert_eq!(
        s.declined_refs,
        s.declined_external + s.declined_method + s.declined_internal
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

/// Verb sweep B1: a call whose only spelling sits inside a macro argument
/// yields no edge — right, a macro's tokens are its own grammar — and was
/// counted nowhere, so `declined` read all zeros and the omission was
/// indistinguishable from a file with no references. Red before the fix
/// with `declined_macro == 0`. The tokens stay unread: what is counted is
/// the invocation, never the names inside it.
#[test]
fn a_macro_invocation_is_declined_into_its_own_population() {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"probe\"\n"),
        (
            "src/main.rs",
            "fn helper() -> i32 {\n    7\n}\nfn main() {\n    println!(\"{}\", helper());\n}\n",
        ),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.edges, 0, "nothing inside a macro is bound: {s:?}");
    assert_eq!(s.declined_macro, 1, "the invocation is counted: {s:?}");
    assert_eq!(
        split(&s),
        (0, 0, 0),
        "and it is none of the name populations"
    );
    assert_eq!(
        s.declined_refs,
        s.declined_external + s.declined_method + s.declined_internal + s.declined_macro,
        "the four populations are the total"
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

/// A method call on a receiver the file *inferred* — `Foo::new().m()` —
/// yields an edge graded **modeled**, never proven: the type is grid's
/// reading of syntax, not the language's binding rule. It is no longer
/// counted `method`, and the sub-count `refused` never exceeds `method`.
#[test]
fn an_inferred_receiver_yields_a_modeled_edge_never_a_proven_one() {
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
    let seg = grid_chart::Segment::parse(s.image.as_slice()).expect("parse");
    let mut grades: Vec<grid_wire::Grade> = (0..seg.def_count())
        .flat_map(|i| seg.out_edges(grid_chart::Ordinal(i)).map(|n| n.grade))
        .collect();
    grades.sort_by_key(|g| format!("{g:?}"));
    // `Foo::new()` is a plain path call and is proven; `.m()` is modeled.
    assert_eq!(
        grades,
        [grid_wire::Grade::Modeled, grid_wire::Grade::Proven]
    );
    assert_eq!(s.declined_method, 0, "the call resolved: {s:?}");
    assert!(s.declined_method_refused <= s.declined_method);
    assert_eq!(
        s.declined_refs,
        s.declined_external + s.declined_method + s.declined_internal
    );
}

/// The per-definition split, read back off the segment.
fn per_def(s: &Survey) -> Vec<(String, grid_chart::Declined)> {
    let seg = grid_chart::Segment::parse(s.image.as_slice()).expect("parse");
    (0..seg.def_count())
        .map(|i| {
            let ord = grid_chart::Ordinal(i);
            (
                seg.callsign(ord).unwrap_or("?").to_owned(),
                seg.declined(ord).expect("a fresh image counts every def"),
            )
        })
        .collect()
}

/// Property, T-17: the sum over every definition of each counter equals
/// the survey's repo-wide counter — module-level references are counted in
/// neither — and `refused ≤ method` holds per definition.
#[test]
fn per_definition_declines_sum_to_the_surveys_counters() {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        (
            "src/lib.rs",
            "use std::collections::HashMap;\n\
             use crate::hidden::*;\n\
             pub mod hidden;\n\
             pub struct Local;\n\
             pub trait Tr { fn frob(&self); }\n\
             impl Local {\n  pub fn frob(&self) {}\n  pub fn zap(&self) { self.frob(); glob_supplied(); }\n}\n\
             pub fn sibling(x: Local) { x.frob(); }\n\
             pub fn caller(v: Vec<Local>) {\n\
             \x20   HashMap::<u8, u8>::new();\n\
             \x20   for x in v { x.frob(); }\n\
             \x20   glob_supplied();\n\
             \x20   sibling(Local);\n\
             }\n\
             const AT_MODULE_LEVEL: u8 = { helper(); 1 };\n",
        ),
        ("src/hidden.rs", "// no definitions here\n"),
    ]);
    let s = survey(dir.path());
    let rows = per_def(&s);
    let sum = |f: fn(&grid_chart::Declined) -> u16| -> u64 {
        rows.iter().map(|(_, d)| u64::from(f(d))).sum()
    };
    assert_eq!(sum(|d| d.external), s.declined_external, "{rows:?}");
    assert_eq!(sum(|d| d.method), s.declined_method, "{rows:?}");
    assert_eq!(sum(|d| d.internal), s.declined_internal, "{rows:?}");
    assert_eq!(sum(|d| d.refused), s.declined_method_refused, "{rows:?}");
    assert_eq!(
        sum(|d| d.external) + sum(|d| d.method) + sum(|d| d.internal),
        s.declined_refs
    );
    for (name, d) in &rows {
        assert!(d.refused <= d.method, "{name}: {d:?}");
    }
    // Non-vacuity: the rows are not all zero, and one of them is the
    // refused trait candidate in `sibling`.
    assert!(s.declined_refs >= 4, "{s:?}");
    let sibling = rows
        .iter()
        .find(|(n, _)| n == "fixture.sibling")
        .map(|(_, d)| *d)
        .expect("sibling");
    assert_eq!((sibling.method, sibling.refused), (1, 1), "{sibling:?}");
    assert_eq!(s.declined_saturated, 0);
}

/// A definition's counter saturates at `u16::MAX` and the survey says so;
/// the repo-wide counter stays exact.
#[test]
fn a_per_definition_counter_saturates_and_is_reported() {
    let calls = "y.m();".repeat(70_000);
    let src = format!("pub fn f(x: Vec<u8>) {{ for y in x {{ {calls} }} }}\n");
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        ("src/lib.rs", &src),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.declined_method, 70_000, "the repo-wide count is exact");
    assert_eq!(s.declined_saturated, 1, "one definition saturated");
    let rows = per_def(&s);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1.method, u16::MAX, "{rows:?}");
}

/// Saturation is counted per definition, not per column: one definition
/// whose `method` and `external` both overflow is one saturated definition.
#[test]
fn a_definition_saturating_two_columns_is_counted_once() {
    let method = "y.m();".repeat(70_000);
    let external = "x.m();".repeat(70_000);
    let src = format!("pub fn f(x: Vec<u8>) {{ for y in x {{ {method} }} {external} }}\n");
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        ("src/lib.rs", &src),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.declined_method, 70_000, "{s:?}");
    assert_eq!(s.declined_external, 70_000, "{s:?}");
    let rows = per_def(&s);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].1.method, rows[0].1.external), (u16::MAX, u16::MAX));
    assert_eq!(s.declined_saturated, 1, "one definition saturated");
}

/// A receiver typed by a primitive is typed outside the tree: `external`,
/// never `method` — the tree cannot hold `str::len`.
#[test]
fn a_primitive_receiver_counts_external() {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        (
            "src/lib.rs",
            "pub fn f(s: &str, n: u32, b: bool, c: char, x: f64) {\n\
             \x20   s.len(); n.count_ones(); b.then_some(1); c.len_utf8(); x.floor();\n}\n",
        ),
    ]);
    let s = survey(dir.path());
    assert_eq!(
        (s.declined_external, s.declined_method, s.declined_internal),
        (5, 0, 0)
    );
}

/// A prelude name is not a hole in this tree: `Some(..)`, `Ok(..)`,
/// `drop(..)` and a derived or implemented prelude trait were never the
/// tree's to hold, and counting them `internal` inflated the one number
/// `grid drift` is honest by.
#[test]
fn rust_prelude_names_are_external_not_internal() {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        (
            "src/lib.rs",
            "#[derive(Debug, Clone)]\n\
             pub struct Local;\n\
             impl Default for Local {\n    fn default() -> Self {\n        Local\n    }\n}\n\
             pub fn f() -> Option<u32> {\n    Some(1)\n}\n\
             pub fn g() -> Result<u32, u32> {\n    Ok(1)\n}\n\
             pub fn h() {\n    drop(f());\n}\n",
        ),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.edges, 1, "`h` calls `f`; nothing else binds");
    assert_eq!(
        s.declined_internal,
        0,
        "nothing here is a name the tree could have held: {:?}",
        split(&s)
    );
    assert_eq!(
        s.declined_external,
        s.declined_refs,
        "Some, Ok, drop, Debug, Clone, Default are all prelude: {:?}",
        split(&s)
    );
    // The derives are attribute paths the extractor filters as builtin,
    // so the calls and the `impl Default` base are what is counted.
    assert!(s.declined_refs >= 4, "{:?}", split(&s));
}

/// The same for Python's builtins: `len`, `print`, `isinstance` and an
/// `Exception` base are every scope's without an import.
#[test]
fn python_builtins_are_external_not_internal() {
    let dir = repo(&[
        ("pyproject.toml", "[project]\nname = \"fixture\"\n"),
        ("pkg/__init__.py", ""),
        (
            "pkg/mod.py",
            "class E(Exception):\n    pass\n\n\n\
             def f(xs):\n    print(len(xs))\n    return isinstance(xs, list)\n",
        ),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.edges, 0);
    assert_eq!(
        s.declined_internal,
        0,
        "builtins are not names this tree could have held: {:?}",
        split(&s)
    );
    assert_eq!(s.declined_external, s.declined_refs, "{:?}", split(&s));
    assert!(s.declined_refs >= 4, "{:?}", split(&s));
}
