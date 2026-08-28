//! Rust edges end to end: extraction, indexing, and Rust-rules resolution
//! through the public `run` — the contract a chart consumer sees.
//!
//! The conservatism half matters most: a survey that invented an edge for a
//! glob-supplied or macro-hidden name would poison the proven tier, so this
//! suite pins counts, not just presence.
use std::path::Path;

use grid_survey::{run, Cache};

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

fn survey(root: &Path) -> grid_survey::Survey {
    run(root, &mut Cache::new()).expect("survey")
}

#[test]
fn same_module_and_use_bound_calls_resolve() {
    let dir = repo(&[
        (
            "crates/app/src/lib.rs",
            "mod util;\nuse crate::util::helper;\npub fn go() -> u32 { helper() }\n",
        ),
        ("crates/app/src/util.rs", "pub fn helper() -> u32 { 1 }\n"),
    ]);
    let s = survey(dir.path());
    // go -> helper is the one resolvable relationship in the tree.
    assert_eq!(s.defs, 2);
    assert_eq!(s.edges, 1);
}

#[test]
fn self_calls_and_trait_impls_resolve() {
    let dir = repo(&[(
        "crates/app/src/lib.rs",
        "pub trait Frob { fn frob(&self); }\n\
         pub struct Foo;\n\
         impl Foo {\n  pub fn a(&self) -> u32 { self.b() }\n  fn b(&self) -> u32 { 2 }\n}\n\
         impl Frob for Foo { fn frob(&self) { self.a(); } }\n",
    )]);
    let s = survey(dir.path());
    // Foo implements Frob; Foo.a -> Foo.b; Foo@Frob.frob -> Foo.a.
    assert_eq!(s.edges, 3);
}

#[test]
fn glob_names_macro_bodies_and_typed_receivers_resolve_to_nothing() {
    let dir = repo(&[
        (
            "crates/app/src/lib.rs",
            "use crate::other::*;\n\
             pub fn f() -> u32 { mystery() }\n\
             pub fn g() { println!(\"{}\", f()); }\n\
             pub fn h(x: crate::other::Thing) { x.frob(); }\n",
        ),
        (
            "crates/app/src/other.rs",
            "pub struct Thing;\nimpl Thing { pub fn frob(&self) {} }\npub fn mystery() -> u32 { 1 }\n",
        ),
    ]);
    let s = survey(dir.path());
    // Every reference in lib.rs needs the glob, a macro body, or a typed
    // receiver — all declined. Zero edges is the assertion that matters.
    assert_eq!(s.edges, 0);
}

#[test]
fn cross_crate_calls_resolve_through_manifest_names_and_reexports() {
    // The package is named with a hyphen and a prefix its directory lacks,
    // and the callee is only reachable through a `pub use` — the two ways
    // cross-crate resolution failed before CrateRoots and aliases.
    let dir = repo(&[
        (
            "crates/core/Cargo.toml",
            "[package]\nname = \"app-core\"\nversion = \"0.1.0\"\n",
        ),
        (
            "crates/core/src/lib.rs",
            "mod util;\npub use util::helper;\n",
        ),
        ("crates/core/src/util.rs", "pub fn helper() -> u32 { 1 }\n"),
        (
            "crates/cli/Cargo.toml",
            "[package]\nname = \"app-cli\"\nversion = \"0.1.0\"\n",
        ),
        (
            "crates/cli/src/main.rs",
            "fn main() { let _ = app_core::helper(); }\n",
        ),
    ]);
    let s = survey(dir.path());
    // main -> app_core.util.helper, through the alias.
    assert_eq!(s.edges, 1);
}

#[test]
fn a_reexport_never_shadows_a_real_definition() {
    let dir = repo(&[(
        "crates/app/src/lib.rs",
        "mod inner;\npub use inner::helper;\npub fn helper() -> u32 { 2 }\npub fn f() -> u32 { helper() }\n",
    ),
    ("crates/app/src/inner.rs", "pub fn helper() -> u32 { 1 }\n")]);
    let s = survey(dir.path());
    // f -> the real lib.rs helper only; the alias must not add a second
    // edge or divert the first.
    assert_eq!(s.edges, 1);
}

#[test]
fn python_and_rust_survey_side_by_side() {
    let dir = repo(&[
        (
            "py/pkg/mod.py",
            "def caller():\n    return callee()\n\ndef callee():\n    return 1\n",
        ),
        (
            "crates/app/src/lib.rs",
            "pub fn a() -> u32 { b() }\npub fn b() -> u32 { 1 }\n",
        ),
    ]);
    let s = survey(dir.path());
    assert_eq!(s.defs, 4);
    assert_eq!(s.edges, 2);
    assert_eq!(s.langs.len(), 2);
}

#[test]
fn a_tracked_but_ignored_file_is_not_charted() {
    let dir = repo(&[
        ("crates/app/src/lib.rs", "pub fn a() -> u32 { 1 }\n"),
        ("vendor/dep.rs", "pub fn v() -> u32 { 1 }\n"),
    ]);
    // Committed, then ignored: the vendored-code case. The repo's ignore
    // rules say not-source, and tracking must not override that.
    std::fs::write(dir.path().join(".gitignore"), "vendor/\n").expect("write");
    let s = survey(dir.path());
    assert_eq!(s.defs, 1);
}

#[test]
fn a_lib_rename_is_the_name_a_use_reaches_for() {
    let dir = repo(&[
        (
            "crates/app/Cargo.toml",
            "[package]\nname = \"my-app\"\n\n[lib]\nname = \"renamed\"\n",
        ),
        ("crates/app/src/lib.rs", "pub fn helper() -> u32 { 1 }\n"),
        (
            "crates/cli/src/main.rs",
            "fn main() { let _ = renamed::helper(); }\n",
        ),
    ]);
    let s = survey(dir.path());
    // main -> renamed.helper: `[lib] name` wins over `[package] name`.
    assert_eq!(s.edges, 1);
}
