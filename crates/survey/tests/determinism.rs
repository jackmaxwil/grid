//! Survey determinism and incrementality (claim C2).
//!
//! The oracle this suite exists for is **incremental ≡ full**: a tree surveyed
//! from cold and the same tree surveyed through a warm cache must produce
//! byte-identical segments. It is the strongest statement available about
//! incrementality, because it does not test the cache against a description of
//! what the cache should do — it tests it against the answer, and any stale
//! fact that survives an edit changes the bytes.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::path::Path;

use grid_survey::{run, Cache};

/// A git repo containing `files`, since discovery is `git ls-files`.
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
    write(dir.path(), files);
    git(&["add", "-A"]);
    git(&["commit", "-qm", "surveyed"]);
    dir
}

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, source) in files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(full, source).expect("write");
    }
}

/// Re-add so `git ls-files` sees changes to the tree.
fn readd(root: &Path) {
    let ok = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(root)
        .output()
        .expect("run git")
        .status
        .success();
    assert!(ok, "git add failed");
}

fn cold(root: &Path) -> grid_survey::Survey {
    run(root, &mut Cache::new()).expect("survey")
}

const CALLER: &str = "from pkg.util import helper\n\n\ndef caller():\n    helper()\n";
const UTIL: &str = "def helper():\n    return 1\n";

#[test]
fn a_survey_finds_definitions_and_proves_the_edge_between_them() {
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    let s = cold(dir.path());
    assert_eq!(s.defs, 2, "caller and helper");
    assert_eq!(s.edges, 1, "caller -calls-> helper");
    assert_eq!(s.langs, [grid_wire::Lang::Py]);
}

#[test]
fn the_same_tree_surveys_to_the_same_bytes() {
    // Determinism is what makes the snapshot id a content id (D5); without it
    // every survey would mint a new snapshot for an unchanged tree.
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    assert_eq!(cold(dir.path()).image, cold(dir.path()).image);
}

#[test]
fn incremental_equals_full_after_editing_one_file() {
    // THE incrementality oracle. A warm cache that kept one stale fact about
    // the edited file would produce different bytes here.
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    let mut cache = Cache::new();
    run(dir.path(), &mut cache).expect("cold survey");

    let edited = "def helper():\n    return 2\n";
    write(dir.path(), &[("pkg/util.py", edited)]);
    readd(dir.path());

    let warm = run(dir.path(), &mut cache).expect("warm survey");
    let full = cold(dir.path());
    assert_eq!(warm.image, full.image, "incremental must equal full");
    assert_eq!(warm.reread, 1, "the unedited file came from the cache");
}

/// The cache is bounded by the tree, not by the session.
///
/// A `Cache` lives as long as whatever holds it. The CLI surveys once and
/// exits, but the daemon's chart and a `grid watch` session hold one for
/// hours — and it is keyed by content hash, so before this every save of every
/// file added an entry that nothing ever removed. What a survey must leave
/// behind is what that survey read.
#[test]
fn a_survey_leaves_only_its_own_files_in_the_cache() {
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    let mut cache = Cache::new();
    run(dir.path(), &mut cache).expect("cold survey");
    assert_eq!(cache.len(), 2, "one entry per source file");

    // Three more saves of the same file, the way a session accumulates them.
    for body in ["    return 2\n", "    return 3\n", "    return 4\n"] {
        let source = format!("def helper():\n{body}");
        write(dir.path(), &[("pkg/util.py", &source)]);
        readd(dir.path());
        run(dir.path(), &mut cache).expect("warm survey");
    }
    assert_eq!(
        cache.len(),
        2,
        "the cache holds the tree, not every version of it the session saw"
    );
}

#[test]
fn incremental_equals_full_with_an_inline_module_anchor() {
    // The re-anchored `use super::x` is a per-file fact and rides the cache;
    // the edge it buys is recomputed. Editing the *callee's* file must not
    // leave the test module's edge behind.
    const FOO: &str =
        "pub fn x() -> u32 { 1 }\nmod tests {\n    use super::x;\n    fn t() -> u32 { x() }\n}\n";
    let dir = repo(&[
        ("crates/app/src/lib.rs", "pub mod foo;\n"),
        ("crates/app/src/foo.rs", FOO),
    ]);
    let mut cache = Cache::new();
    let before = run(dir.path(), &mut cache).expect("cold survey");
    assert_eq!(before.edges, 1, "tests.t -> x through `use super::x`");

    write(
        dir.path(),
        &[(
            "crates/app/src/lib.rs",
            "pub mod foo;\npub fn y() -> u32 { 2 }\n",
        )],
    );
    readd(dir.path());
    let warm = run(dir.path(), &mut cache).expect("warm survey");
    assert_eq!(
        warm.image,
        cold(dir.path()).image,
        "incremental must equal full"
    );
    assert_eq!(warm.reread, 1, "foo.rs came from the cache");
}

#[test]
fn incremental_equals_full_when_a_callee_is_removed() {
    // The hard direction: an edge's *target* disappears. The caller's own
    // bytes did not change, so a cache holding edges would keep an edge to a
    // definition that no longer exists.
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    let mut cache = Cache::new();
    let before = run(dir.path(), &mut cache).expect("cold survey");
    assert_eq!(before.edges, 1);

    write(
        dir.path(),
        &[("pkg/util.py", "def other():\n    return 1\n")],
    );
    readd(dir.path());

    let warm = run(dir.path(), &mut cache).expect("warm survey");
    assert_eq!(warm.edges, 0, "the call no longer resolves");
    assert_eq!(warm.image, cold(dir.path()).image);
}

#[test]
fn incremental_equals_full_when_a_file_is_added_and_when_one_is_removed() {
    let dir = repo(&[("pkg/caller.py", CALLER)]);
    let mut cache = Cache::new();
    let alone = run(dir.path(), &mut cache).expect("cold survey");
    assert_eq!(alone.edges, 0, "nothing to call yet");

    write(dir.path(), &[("pkg/util.py", UTIL)]);
    readd(dir.path());
    let added = run(dir.path(), &mut cache).expect("warm survey");
    assert_eq!(added.edges, 1);
    assert_eq!(added.image, cold(dir.path()).image);

    std::fs::remove_file(dir.path().join("pkg/util.py")).expect("rm");
    readd(dir.path());
    let removed = run(dir.path(), &mut cache).expect("warm survey");
    assert_eq!(removed.image, cold(dir.path()).image);
}

#[test]
fn a_second_survey_of_an_unchanged_tree_parses_nothing() {
    // "You cannot survey too often" is a claim about work, not just output.
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    let mut cache = Cache::new();
    let first = run(dir.path(), &mut cache).expect("cold");
    assert_eq!(first.reread, 0);
    let second = run(dir.path(), &mut cache).expect("warm");
    assert_eq!(second.reread, 2, "every file came from the cache");
    assert_eq!(first.image, second.image);
}

type DefRow = (String, String, Vec<(u32, u8)>);

#[test]
fn a_reformatted_file_reuses_nothing_but_keeps_every_identity() {
    // The datum ignores formatting (C6). Since epoch 2 the segment also
    // carries *locations*, which are physical and legitimately move under
    // reformatting — so the contract is semantic: every datum, callsign and
    // edge is unchanged, while the image as a whole may differ.
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    let mut cache = Cache::new();
    let before = run(dir.path(), &mut cache).expect("cold");

    write(
        dir.path(),
        &[(
            "pkg/util.py",
            "def helper():\n\n    # a comment\n    return 1\n",
        )],
    );
    readd(dir.path());
    let after = run(dir.path(), &mut cache).expect("warm");
    let semantics = |image: &[u8]| -> Vec<DefRow> {
        let seg = grid_chart::Segment::parse(image.to_vec()).expect("parse");
        (0..seg.def_count())
            .map(|i| {
                let ord = grid_chart::Ordinal(i);
                let edges = seg
                    .out_edges(ord)
                    .map(|n| (n.other.0, grid_chart::relation_tag(n.relation)))
                    .collect();
                (
                    seg.datum(ord).map(|d| d.to_string()).unwrap_or_default(),
                    seg.callsign(ord).unwrap_or_default().to_owned(),
                    edges,
                )
            })
            .collect()
    };
    assert_eq!(
        semantics(&before.image),
        semantics(&after.image),
        "formatting is not identity, so every datum, name and edge holds"
    );
}

#[test]
fn a_file_moving_between_packages_is_a_new_module_path() {
    // Content-addressed caching means the move costs no parsing, but the
    // module path is applied after the cache, so names still change.
    let dir = repo(&[("pkg/caller.py", CALLER), ("pkg/util.py", UTIL)]);
    let mut cache = Cache::new();
    run(dir.path(), &mut cache).expect("cold");

    std::fs::rename(
        dir.path().join("pkg/util.py"),
        dir.path().join("pkg/moved.py"),
    )
    .expect("rename");
    readd(dir.path());
    let moved = run(dir.path(), &mut cache).expect("warm");
    assert_eq!(moved.reread, 2, "identical bytes, so nothing was reparsed");
    assert_eq!(moved.edges, 0, "the import names a module that is gone");
    assert_eq!(moved.image, cold(dir.path()).image);
}

#[test]
fn an_unparseable_file_does_not_fail_the_survey() {
    // A tool that refuses to run on a tree containing one broken file is a
    // tool that cannot be used to fix that file.
    let dir = repo(&[("pkg/util.py", UTIL), ("pkg/broken.py", "def (:\n")]);
    let s = cold(dir.path());
    assert_eq!(s.defs, 1, "the good file was still surveyed");
}

#[test]
fn untracked_files_are_not_surveyed() {
    // Discovery is git's, so the repo's ignore rules are the only policy.
    let dir = repo(&[("pkg/util.py", UTIL)]);
    write(
        dir.path(),
        &[("pkg/scratch.py", "def scratch():\n    pass\n")],
    );
    assert_eq!(cold(dir.path()).defs, 1);
}

#[test]
fn the_same_name_in_two_files_is_two_names_not_an_ambiguity() {
    // Module paths disambiguate: `a.dup` and `b.dup` are different names, so
    // both resolve and neither is ambiguous. This is the common case, and it
    // must not be confused with the real collision below.
    let dir = repo(&[
        ("a.py", "def dup():\n    pass\n"),
        ("b.py", "def dup():\n    pass\n"),
        ("c.py", "import a\n\n\ndef caller():\n    a.dup()\n"),
    ]);
    let s = cold(dir.path());
    assert_eq!(s.ambiguous, 0);
    assert_eq!(
        s.edges, 1,
        "a.dup is unambiguous; b.dup is a different name"
    );
}

#[test]
fn a_name_two_files_genuinely_collide_on_resolves_to_neither() {
    // `pkg.py` and `pkg/__init__.py` both claim the module path `pkg`, so
    // `pkg.dup` names two different definitions. Ambiguity is refused, never
    // guessed: an edge to the wrong definition is worse than no edge, and
    // `proven` is a promise, so both become unreachable by name.
    let dir = repo(&[
        ("pkg.py", "def dup():\n    return 1\n"),
        ("pkg/__init__.py", "def dup():\n    return 2\n"),
        ("c.py", "import pkg\n\n\ndef caller():\n    pkg.dup()\n"),
    ]);
    let s = cold(dir.path());
    assert_eq!(s.ambiguous, 1, "one name, two definitions");
    assert_eq!(s.edges, 0, "the collided name resolves to neither");
}

#[test]
fn a_call_that_needs_a_runtime_type_is_not_proven() {
    // `obj.method()` where `obj` is a parameter needs types. This wave ships
    // the proven tier, so it records nothing rather than guessing.
    let dir = repo(&[
        ("m.py", "class Thing:\n    def go(self):\n        pass\n"),
        ("u.py", "def use(obj):\n    obj.go()\n"),
    ]);
    assert_eq!(cold(dir.path()).edges, 0);
}
