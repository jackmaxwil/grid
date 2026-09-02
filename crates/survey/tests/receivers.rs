//! Method calls on typed receivers, end to end, with **rustc as the
//! oracle**: the fixture package's `main` prints, per call site, the
//! callsign the call actually reached, and every edge the survey emits is
//! checked against it. This is the one suite that can say "the rule is
//! wrong" rather than "the rule changed" — a by-value trait method beside an
//! inherent `&self` one prints `Foo@Tr.by_val`, and the rule must have
//! declined.
//!
//! The conservatism half is the four refusal rules, each removable one at a
//! time as a named mutation, plus the receivers no rule may type.
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
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use grid_chart::{Ordinal, Segment};
use grid_survey::{run, Cache, Survey};
use grid_wire::Grade;

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
    run(root, &mut Cache::new()).expect("survey")
}

/// Every edge as `(src callsign, dst callsign, grade)`, read back off the
/// built segment.
fn edges(s: &Survey) -> Vec<(String, String, Grade)> {
    let seg = Segment::parse(s.image.as_slice()).expect("parse");
    let mut out = Vec::new();
    for i in 0..seg.def_count() {
        let src = Ordinal(i);
        for n in seg.out_edges(src) {
            out.push((
                seg.callsign(src).unwrap_or("?").to_owned(),
                seg.callsign(n.other).unwrap_or("?").to_owned(),
                n.grade,
            ));
        }
    }
    out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    out
}

/// The survey of a one-file Rust package with `src` as its `lib.rs`.
fn lib(src: &str) -> Survey {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        ("src/lib.rs", src),
    ]);
    survey(dir.path())
}

/// The method edges of `lib(src)`: every edge whose target is under `Foo`
/// or `Bar` and is not an associated function.
fn method_edges(s: &Survey) -> Vec<(String, String, Grade)> {
    edges(s)
        .into_iter()
        .filter(|(_, dst, _)| matches!(last_segment(dst), "m" | "mm" | "v"))
        .collect()
}

fn last_segment(callsign: &str) -> &str {
    callsign.rsplit('.').next().unwrap_or(callsign)
}

const FIXTURE: &str = include_str!("fixtures/receivers.rs.txt");
const OTHER: &str = include_str!("fixtures/receivers_other.rs.txt");

/// Every method callsign the fixture defines — the value domain of the
/// transcript, and what separates a method edge from a path-call edge.
fn fixture_methods() -> BTreeSet<String> {
    FIXTURE
        .lines()
        .chain(OTHER.lines())
        .filter_map(|l| l.trim().strip_prefix('"')?.strip_suffix('"'))
        .filter(|c| c.contains('.'))
        .map(str::to_owned)
        .collect()
}

/// Ground truth against the compiler: survey the fixture, run it, and hold
/// every emitted edge to the callsign rustc's own lookup reached.
#[test]
fn every_emitted_method_edge_agrees_with_rustc_and_every_refusal_is_annotated() {
    let dir = repo(&[
        (
            "Cargo.toml",
            "[package]\nname = \"receivers\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        ),
        ("src/main.rs", FIXTURE),
        ("src/other.rs", OTHER),
    ]);
    let s = survey(dir.path());
    let methods = fixture_methods();
    assert!(methods.len() >= 12, "{methods:?}");
    assert!(methods.contains("other.Qux.m"), "{methods:?}");

    // rustc's binding, observed: `<function>\t<callsign reached>`.
    let out = std::process::Command::new(env!("CARGO"))
        .args(["run", "-q", "--manifest-path"])
        .arg(dir.path().join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", dir.path().join("target"))
        .output()
        .expect("cargo run");
    assert!(
        out.status.success(),
        "fixture must compile and run:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let transcript: BTreeMap<String, String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(f, c)| (f.to_owned(), c.to_owned()))
        .collect();
    assert_eq!(transcript.len(), 27, "{transcript:?}");

    // The survey's method edges, per call site.
    let mut found: BTreeMap<String, Vec<(String, Grade)>> = BTreeMap::new();
    for (src, dst, grade) in edges(&s) {
        let (Some(name), Some(target)) = (
            src.strip_prefix("receivers."),
            dst.strip_prefix("receivers."),
        ) else {
            continue;
        };
        if methods.contains(target) && transcript.contains_key(name) {
            found
                .entry(name.to_owned())
                .or_default()
                .push((target.to_owned(), grade));
        }
    }

    // Every call site judged, then one verdict: a disagreement anywhere is
    // reported beside every other one.
    let mut wrong = Vec::new();
    for (name, reached) in &transcript {
        let got = found.get(name).cloned().unwrap_or_default();
        let want = if name.contains("c_p_") {
            vec![(reached.clone(), Grade::Proven)]
        } else if name.contains("c_m_") {
            vec![(reached.clone(), Grade::Modeled)]
        } else {
            assert!(
                last_segment(name).starts_with("r_"),
                "{name}: unexpected fixture name"
            );
            vec![]
        };
        if got != want {
            wrong.push(format!(
                "{name}: rustc reached {reached}; want {want:?}, got {got:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // Every emitted edge above is one rustc reached — and the by-value
    // trait method the fixture plants is what makes that non-vacuous.
    assert_eq!(transcript["r_by_val_trait"], "Foo@Tr.by_val");
    assert_eq!(transcript["c_p_trait_shadowed"], "Foo.t");
    // The three receivers that look written and first-step but are not: a
    // trait's type parameter, a double reference, a foreign same-named
    // struct. rustc reaches something else in each; the rule must decline.
    assert_eq!(transcript["Visit.r_trait_generic"], "Foo@Tr.t");
    assert_eq!(transcript["r_double_ref"], "Foo@Dr.d");
    assert_eq!(transcript["r_foreign_field"], "other.Qux.m");
    // Every refusal is counted `method`, and counted refused where the
    // receiver was typed: instantiated, where-clause, no inherent, trait
    // candidate ×2 — five; the alias, the ambiguous type, the closure and
    // the `for` binding are `method` without a reason.
    assert_eq!(s.declined_method_refused, 5, "{s:?}");
    assert!(s.declined_method >= 9, "{s:?}");
}

/// Conservatism (mutation: remove the trait-candidate check): a method
/// both an inherent impl and a trait in the tree supply, matched only at a
/// later step, yields no edge and is counted `method` — and refused.
#[test]
fn a_trait_candidate_at_a_later_step_yields_no_edge() {
    let s = lib("pub struct Foo;\n\
                 pub trait Tr { fn m(self); }\n\
                 impl Foo { pub fn m(&self) {} }\n\
                 impl Tr for Foo { fn m(self) {} }\n\
                 pub fn f(x: Foo) { x.m(); }\n");
    assert_eq!(method_edges(&s), vec![], "{:?}", edges(&s));
    assert_eq!((s.declined_method, s.declined_method_refused), (1, 1));
    // A trait declared in the tree but never implemented for Foo is still
    // a candidate: scope is not decided, and a maybe is a refusal.
    let s = lib("pub struct Foo;\n\
                 pub trait Tr { fn m(&self); }\n\
                 impl Foo { pub fn m(&self) {} }\n\
                 pub fn f(x: Foo) { x.m(); }\n");
    assert_eq!(method_edges(&s), vec![]);
    // A prelude method name is a candidate too.
    let s = lib("pub struct Foo;\n\
                 impl Foo { pub fn clone(&self) {} }\n\
                 pub fn f(x: Foo) { x.clone(); }\n");
    assert_eq!(edges(&s), vec![]);
    assert_eq!((s.declined_method, s.declined_method_refused), (1, 1));
}

/// Conservatism (mutation: ignore `plain_impl`): an inherent impl with a
/// where clause, a bound, or a concrete instantiation yields no edge.
#[test]
fn a_bounded_or_instantiated_impl_yields_no_edge() {
    let s = lib("pub struct Foo;\n\
                 impl Foo where u8: Copy { pub fn m(&self) {} }\n\
                 pub fn f(x: &Foo) { x.m(); }\n");
    assert_eq!(method_edges(&s), vec![]);
    assert_eq!((s.declined_method, s.declined_method_refused), (1, 1));
    let s = lib("pub struct Foo<T>(T);\n\
                 impl<T: Clone> Foo<T> { pub fn m(&self) {} }\n\
                 pub fn f(x: &Foo<u8>) { x.m(); }\n");
    assert_eq!(method_edges(&s), vec![]);
    let s = lib("pub struct Foo<T>(T);\n\
                 impl Foo<u8> { pub fn m(&self) {} }\n\
                 pub fn f(x: &Foo<u8>) { x.m(); }\n");
    assert_eq!(method_edges(&s), vec![]);
}

/// Conservatism: `x: &Foo` calling a `&mut self` method cannot go through
/// the inherent method, so a trait supplied it — no edge.
#[test]
fn a_shared_reference_never_reaches_a_mutable_method() {
    let s = lib("pub struct Foo;\n\
                 impl Foo { pub fn mm(&mut self) {} }\n\
                 pub fn f(x: &Foo) { x.mm(); }\n");
    assert_eq!(method_edges(&s), vec![]);
    assert_eq!((s.declined_method, s.declined_method_refused), (1, 1));
}

/// Conservatism: a wrapper is never looked through. `Option<Foo>` yields
/// no edge; `Vec<Foo>` counts `external` — unless the tree implements one
/// of its own traits for `Vec`, when the call stays `method`.
#[test]
fn a_wrapper_yields_no_edge_and_a_std_wrapper_counts_external() {
    let s = lib("pub struct Foo;\n\
                 impl Foo { pub fn m(&self) {} }\n\
                 pub fn f(x: Option<Foo>) { x.m(); }\n\
                 pub fn g(v: &Vec<Foo>) { v.len(); }\n");
    assert_eq!(method_edges(&s), vec![]);
    assert_eq!(
        (
            s.declined_external,
            s.declined_method,
            s.declined_method_refused
        ),
        (2, 0, 0),
        "{s:?}"
    );
    let s = lib("pub struct Foo;\n\
                 pub trait Tally { fn len(&self) -> usize; }\n\
                 impl Tally for Vec<Foo> { fn len(&self) -> usize { 0 } }\n\
                 pub fn g(v: &Vec<Foo>) { v.len(); }\n");
    // `Vec` is not charted, so not even the implements edge has a source;
    // the call is a hole in the tree's own claim and stays `method`.
    assert_eq!(edges(&s), vec![]);
    assert_eq!((s.declined_external, s.declined_method), (0, 1), "{s:?}");
}

/// Conservatism: a type that resolves to two datums yields no edge, and so
/// does a type alias, which is not a definition.
#[test]
fn an_ambiguous_type_or_an_alias_yields_no_edge() {
    let s = lib("#[cfg(unix)]\npub struct Foo { a: u8 }\n\
                 #[cfg(not(unix))]\npub struct Foo { b: u8 }\n\
                 impl Foo { pub fn m(&self) {} }\n\
                 pub fn f(x: &Foo) { x.m(); }\n");
    assert_eq!(method_edges(&s), vec![]);
    assert_eq!(
        (s.declined_method, s.declined_method_refused),
        (1, 0),
        "{s:?}"
    );
    let s = lib("pub struct Foo;\npub type Alias = Foo;\n\
                 impl Foo { pub fn m(&self) {} }\n\
                 pub fn f(x: &Alias) { x.m(); }\n");
    assert_eq!(method_edges(&s), vec![]);
    assert_eq!(
        (s.declined_method, s.declined_method_refused),
        (1, 0),
        "{s:?}"
    );
}

/// A field of a struct in another file is typed off the index.
#[test]
fn a_field_declared_in_another_file_is_typed_off_the_index() {
    let dir = repo(&[
        ("Cargo.toml", "[package]\nname = \"fixture\"\n"),
        ("src/lib.rs", "pub mod bar;\npub mod foo;\n"),
        (
            "src/bar.rs",
            "pub struct Bar;\nimpl Bar { pub fn m(&self) {} }\n",
        ),
        (
            "src/foo.rs",
            "use crate::bar::Bar;\npub struct Foo { pub bar: Bar }\n\
             impl Foo { pub fn f(&self) { self.bar.m(); } }\n\
             pub fn g(x: &Foo) { x.bar.m(); }\n",
        ),
    ]);
    let s = survey(dir.path());
    let m: Vec<_> = edges(&s)
        .into_iter()
        .filter(|(_, dst, _)| dst == "fixture.bar.Bar.m")
        .collect();
    assert_eq!(
        m,
        vec![
            (
                "fixture.foo.Foo.f".into(),
                "fixture.bar.Bar.m".into(),
                Grade::Modeled
            ),
            (
                "fixture.foo.g".into(),
                "fixture.bar.Bar.m".into(),
                Grade::Modeled
            ),
        ]
    );
}

/// Python: an annotated parameter's method resolves modeled, to the
/// annotated class's own method even when a subclass overrides it;
/// unannotated resolves nothing.
#[test]
fn python_annotated_parameters_resolve_modeled_and_nothing_else_does() {
    let dir = repo(&[
        ("pyproject.toml", "[project]\nname = \"fixture\"\n"),
        ("pkg/__init__.py", ""),
        (
            "pkg/mod.py",
            "class Foo:\n    def m(self):\n        return 1\n\n\n\
             class Sub(Foo):\n    def m(self):\n        return 2\n\n\n\
             def f(x: Foo):\n    return x.m()\n\n\n\
             def g(x):\n    return x.m()\n",
        ),
    ]);
    let s = survey(dir.path());
    let calls: Vec<_> = edges(&s)
        .into_iter()
        .filter(|(src, _, _)| matches!(last_segment(src), "f" | "g"))
        .collect();
    assert_eq!(
        calls,
        vec![("pkg.mod.f".into(), "pkg.mod.Foo.m".into(), Grade::Modeled)]
    );
    assert_eq!(s.declined_method, 1, "g's call is declined: {s:?}");
}
