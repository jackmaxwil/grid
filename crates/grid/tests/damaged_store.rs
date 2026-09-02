//! A damaged chart — bytes on disk that contradict the manifest — is a
//! refusal that names its recovery, and `grid survey` is that recovery.
//!
//! Before this, one flipped byte in a segment wedged the tree for good: every
//! verb refused with `segment … does not match manifest`, `grid survey`
//! refused with the same line, and nothing said that `rm -rf .grid/chart`
//! was the way out. Exit codes disagreed too — `1` from survey, `2` from
//! everything else — for one condition.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::path::Path;
use std::process::{Command, Output};

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let git = |args: &[&str]| {
        let ok = Command::new("git")
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
    std::fs::write(
        dir.path().join("a.py"),
        "def a():\n    pass\n\ndef b():\n    a()\n",
    )
    .expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "one"]);
    dir
}

fn grid(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run grid")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("stderr is utf-8")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout is utf-8")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// Flip the last byte of the one segment the store holds. Grid never does
/// this; a bit-flip, an operator, or a past release's race did.
fn damage(root: &Path) {
    let seg = std::fs::read_dir(root.join(".grid/chart"))
        .expect("read store")
        .flatten()
        .find(|e| e.file_name().to_string_lossy().ends_with(".seg"))
        .expect("a segment")
        .path();
    let mut bytes = std::fs::read(&seg).expect("read segment");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&seg, bytes).expect("write segment");
}

#[test]
fn a_damaged_chart_is_refused_with_its_recovery_and_survey_is_the_recovery() {
    let repo = repo();
    let root = repo.path();
    let first = grid(root, &["survey"]);
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    damage(root);

    // Every read verb: one exit code, and the refusal names the command and
    // the tree — the same shape as every other refusal a survey fixes. (The
    // tree is spelled the way every verb spells its own: as `.`.)
    let hint = "hint: run `grid survey` in .";
    for verb in [
        &["orphans"][..],
        &["drift"],
        &["resolve", "a"],
        &["uses", "a"],
    ] {
        let out = grid(root, verb);
        assert_eq!(
            code(&out),
            2,
            "{verb:?}: a damaged chart is bad input, uniformly:\n{}",
            stderr(&out)
        );
        assert_eq!(
            stdout(&out),
            "",
            "{verb:?} wrote records over a damaged chart"
        );
        assert!(
            stderr(&out).contains("damaged") && stderr(&out).contains(hint),
            "{verb:?}: the refusal names the damage and the recovery:\n{}",
            stderr(&out)
        );
    }

    // The recovery works, and says what it dropped.
    let rebuilt = grid(root, &["survey"]);
    assert_eq!(
        code(&rebuilt),
        0,
        "survey is the recovery, so it cannot refuse the thing it recovers from:\n{}",
        stderr(&rebuilt)
    );
    assert!(
        stderr(&rebuilt).contains("dropped a damaged chart")
            && stderr(&rebuilt).contains("does not match manifest"),
        "the cold survey says why it was cold:\n{}",
        stderr(&rebuilt)
    );
    assert!(
        stdout(&rebuilt).starts_with("snapshot"),
        "{}",
        stdout(&rebuilt)
    );

    // And the tree answers again.
    let after = grid(root, &["orphans"]);
    assert_eq!(code(&after), 0, "{}", stderr(&after));
    let again = grid(root, &["survey"]);
    assert!(
        !stderr(&again).contains("dropped"),
        "a healthy store drops nothing:\n{}",
        stderr(&again)
    );
}
