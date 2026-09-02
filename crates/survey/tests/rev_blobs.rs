//! Reading a rev's blobs through one `git cat-file --batch` child.
//!
//! The oracle is `git show rev:path`, one spawn per file — what this replaced.
//! The batch protocol is positional, so the failure it can have that the old
//! per-file spawns could not is *desync*: an answer read onto the wrong path.
//! Every test here is aimed at that.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::path::Path;
use std::process::Command;

use grid_survey::files::{lang_of, sources_at, SourceFile};

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "grid")
        .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
        .env("GIT_COMMITTER_NAME", "grid")
        .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?}: {:?}", out.stderr);
    out.stdout
}

/// A repo whose HEAD holds the awkward cases: a non-UTF-8 blob, a symlink, a
/// file only the rev has, and a manifest that has to be read at the rev too.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture-crate\"\n",
    )
    .expect("write");
    std::fs::write(root.join("src/lib.rs"), "pub fn a() {}\n").expect("write");
    // Not UTF-8: an adapter cannot parse it, and the read must not stop the
    // stream for the files behind it.
    std::fs::write(root.join("src/latin1.rs"), b"// caf\xe9\npub fn b() {}\n").expect("write");
    std::fs::write(root.join("src/zed.rs"), "pub fn z() {}\n").expect("write");
    std::fs::write(root.join("src/mod.py"), "def p():\n    pass\n").expect("write");
    // A symlink is a blob holding its target's path.
    std::os::unix::fs::symlink("lib.rs", root.join("src/link.rs")).expect("symlink");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "one"]);
    // Present only at the rev: HEAD keeps it, the worktree does not.
    std::fs::write(root.join("src/only_at_rev.rs"), "pub fn gone() {}\n").expect("write");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "two"]);
    std::fs::remove_file(root.join("src/only_at_rev.rs")).expect("rm");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "three"]);
    dir
}

fn tracked_at(root: &Path, rev: &str) -> Vec<String> {
    String::from_utf8_lossy(&git(root, &["ls-tree", "-r", "--name-only", "-z", rev]))
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Differential (rung 5): every blob the batch read equals what `git show`
/// serves for the same `rev:path`, over the awkward fixture.
#[test]
fn batch_blobs_match_git_show() {
    let dir = fixture();
    let root = dir.path();
    let (sources, _) = sources_at(root, "HEAD~1").expect("survey the rev");
    assert!(
        sources.iter().any(|f| f.path == "src/only_at_rev.rs"),
        "the rev's own file must be read from the rev, not the worktree"
    );
    for file in &sources {
        let shown = git(root, &["show", &format!("HEAD~1:{}", file.path)]);
        assert_eq!(
            shown,
            file.source.as_bytes(),
            "batch and `git show` disagree on {}",
            file.path
        );
    }
    // And nothing readable was dropped: every tracked path `git show` serves
    // as UTF-8 source is present.
    for path in tracked_at(root, "HEAD~1") {
        let is_source = lang_of(&path).is_some();
        let readable = String::from_utf8(git(root, &["show", &format!("HEAD~1:{path}")])).is_ok();
        if is_source && readable {
            assert!(
                sources.iter().any(|f| f.path == path),
                "{path} is readable source at the rev but the survey lost it"
            );
        }
    }
}

/// Ground truth: the manifest read through the same child still binds the
/// crate root, so callsigns keep the package's name.
#[test]
fn manifest_read_at_the_rev_still_names_the_crate() {
    let dir = fixture();
    let mut cache = grid_survey::Cache::new();
    let survey = grid_survey::run_at(dir.path(), "HEAD~1", &mut cache).expect("survey");
    assert!(survey.defs >= 4, "definitions surveyed: {}", survey.defs);
    let segment = grid_chart::Segment::parse(survey.image.as_slice()).expect("parse segment");
    let names: Vec<String> = (0..segment.def_count())
        .filter_map(|i| segment.callsign(grid_chart::Ordinal(i)).map(str::to_owned))
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("fixture_crate.")),
        "the rev's Cargo.toml did not bind the crate root: {names:?}"
    );
}

/// A non-UTF-8 blob is skipped, and the file *behind it in the stream* still
/// gets its own bytes — the desync a per-file spawn could not have.
#[test]
fn an_unreadable_blob_does_not_shift_the_files_behind_it() {
    let dir = fixture();
    let (sources, _) = sources_at(dir.path(), "HEAD~1").expect("survey the rev");
    let by_path = |p: &str| sources.iter().find(|f: &&SourceFile| f.path == p);
    assert!(by_path("src/latin1.rs").is_none(), "latin1 is not source");
    assert_eq!(
        by_path("src/zed.rs").map(|f| f.source.as_str()),
        Some("pub fn z() {}\n"),
        "the file after the unreadable one lost its own bytes"
    );
}

/// Conservatism: a path the listing names that the rev cannot serve is
/// skipped — never fabricated as empty, and never allowed to slide the next
/// answer onto the wrong path. A gitlink to a commit this repo does not have
/// is the cheapest way to make git answer `missing`.
#[test]
fn a_path_the_rev_cannot_serve_is_skipped_not_fabricated() {
    let dir = fixture();
    let root = dir.path();
    // Named `.rs` so the survey actually asks for it; a gitlink is requested
    // exactly like any other entry and answers with a commit that is absent.
    git(
        root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,0000000000000000000000000000000000000001,src/ghost.rs",
        ],
    );
    git(root, &["commit", "-qm", "gitlink"]);
    let (sources, _) = sources_at(root, "HEAD").expect("survey the rev");
    assert!(
        !sources.iter().any(|f| f.path == "src/ghost.rs"),
        "an unservable path was fabricated as a file"
    );
    assert_eq!(
        sources
            .iter()
            .find(|f| f.path == "src/lib.rs")
            .map(|f| f.source.as_str()),
        Some("pub fn a() {}\n"),
        "the unservable path shifted a later answer onto the wrong file"
    );
    assert_eq!(
        sources
            .iter()
            .find(|f| f.path == "src/zed.rs")
            .map(|f| f.source.as_str()),
        Some("pub fn z() {}\n"),
        "the unservable path shifted a later answer onto the wrong file"
    );
}
