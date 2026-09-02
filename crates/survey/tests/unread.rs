//! A file the survey could not read is charted as nothing and *named* —
//! never as a file with nothing in it. Verb sweep B2/B6: an unparseable
//! file and a non-UTF-8 file were both dropped silently, `defs` shrank,
//! exit 0, and `grid drift` then read the absence as a deletion.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::path::Path;

use grid_survey::{run, Cache, Survey, Unread};

fn repo(files: &[(&str, &[u8])]) -> tempfile::TempDir {
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
    for (path, bytes) in files {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(full, bytes).expect("write");
    }
    git(&["add", "-A"]);
    git(&["commit", "-qm", "surveyed"]);
    dir
}

fn survey(root: &Path, cache: &mut Cache) -> Survey {
    run(root, cache).expect("survey")
}

/// Red before the fix with `unread` empty and `defs` 1: both files were
/// simply absent. An empty file is not unread — it parses to nothing.
#[test]
fn an_unparseable_and_a_non_utf8_file_are_named_not_dropped() {
    let dir = repo(&[
        ("Cargo.toml", b"[package]\nname = \"probe\"\n"),
        (
            "src/lib.rs",
            b"pub mod good;\npub mod broken;\npub mod bad;\npub mod empty;\n",
        ),
        ("src/good.rs", b"pub fn good() {}\n"),
        ("src/broken.rs", b"pub fn broken( { this is not rust ]]]\n"),
        ("src/bad.rs", b"pub fn utf() -> &str { \"caf\xe9\" }\n"),
        ("src/empty.rs", b""),
    ]);
    let mut cache = Cache::new();
    let s = survey(dir.path(), &mut cache);
    assert_eq!(s.defs, 1, "the readable definition is charted: {s:?}");
    assert_eq!(
        s.unread,
        vec![
            ("src/bad.rs".to_owned(), Unread::Unreadable),
            ("src/broken.rs".to_owned(), Unread::Unparseable),
        ],
        "every file that contributed nothing is named, sorted by path"
    );
    // Incremental equals full: the second survey answers from the cache and
    // says the same thing, byte for byte and file for file.
    let again = survey(dir.path(), &mut cache);
    assert_eq!(again.image, s.image);
    assert_eq!(again.unread, s.unread);
    assert!(
        again.reread >= 2,
        "the parsed files came from the cache: {}",
        again.reread
    );
}
