//! The refusal class: everywhere grid holds the information and used to
//! answer with something weaker than what it knows.
//!
//! One rule, stated once in `docs/PORCELAIN.md` and asserted here: an empty
//! answer is an answer about a chart, so a verb that has no chart, a filter
//! that matches nothing, and a budget that cannot bind all refuse (exit `2`)
//! rather than exiting `0` on the empty set. `--json` never converts one of
//! those refusals into an empty success.
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

use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "grid")
        .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
        .env("GIT_COMMITTER_NAME", "grid")
        .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
        .output()
        .expect("run git")
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Two commits, two definitions, one proven call between them.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let write = |path: &str, body: &str| {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(full, body).expect("write");
    };
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write("pkg/__init__.py", "");
    write("pkg/util.py", "def helper():\n    return 1\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "one"]);
    write(
        "pkg/main.py",
        "from pkg.util import helper\n\n\ndef sweep():\n    helper()\n",
    );
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "two"]);
    dir
}

fn grid(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run grid")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// A store nobody surveyed says nothing about any name. Answering "nothing
/// is called that" for code that is plainly there is the one conclusion
/// PORCELAIN says grid never licenses, and an empty list at exit 0 is the
/// same claim written quietly.
#[test]
fn a_read_verb_with_no_chart_refuses_and_names_the_survey() {
    let dir = repo();
    for args in [
        vec!["resolve", "pkg.util.helper"],
        vec!["uses", "pkg.util.helper"],
        vec!["log", "pkg.util.helper"],
        vec!["orphans"],
        vec!["hotspots"],
        vec!["scope", "pkg.util.helper"],
    ] {
        let out = grid(dir.path(), &args);
        assert_eq!(code(&out), 2, "{args:?} on an unsurveyed store:\n{out:?}");
        assert!(
            stdout(&out).is_empty(),
            "{args:?} printed records without a chart:\n{}",
            stdout(&out)
        );
        let hint = stderr(&out);
        assert!(
            hint.contains("no snapshot published") && hint.contains("grid survey"),
            "{args:?} must name the survey that fixes it:\n{hint}"
        );
        assert!(
            !hint.contains("nothing is called"),
            "{args:?} claimed the definition does not exist:\n{hint}"
        );
    }
}

/// Same refusal in the machine-readable form. `--json` is an output form,
/// never a second contract about what grid knows.
#[test]
fn json_does_not_turn_a_refusal_into_an_empty_success() {
    let dir = repo();
    let no_chart = grid(dir.path(), &["resolve", "pkg.util.helper", "--json"]);
    assert_eq!(code(&no_chart), 2, "{no_chart:?}");
    assert!(stdout(&no_chart).is_empty(), "{}", stdout(&no_chart));

    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let miss = grid(dir.path(), &["resolve", "zzz_no_such_thing", "--json"]);
    assert_eq!(
        code(&miss),
        2,
        "a name that resolves to nothing is bad input in both forms:\n{miss:?}"
    );
    assert!(
        stdout(&miss).is_empty(),
        "the refusal must not be served as `[]`:\n{}",
        stdout(&miss)
    );
}

/// `grid scope` used to be the one verb that read its own silence as an
/// answer, and to call it complete while doing so.
#[test]
fn scope_refuses_a_pattern_that_matches_nothing() {
    let dir = repo();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["scope", "zzz_no_such_thing"]);
    assert_eq!(code(&out), 2, "{out:?}");
    assert!(stdout(&out).is_empty(), "{}", stdout(&out));
    let hint = stderr(&out);
    assert!(
        hint.contains("nothing is called") && hint.contains("closest"),
        "scope must refuse like resolve does:\n{hint}"
    );
    assert!(
        !hint.contains("complete through"),
        "an empty pack is not a complete one:\n{hint}"
    );
}

/// Budgets are rejected whole, never truncated — and never ignored. The
/// record form has no tier to spend a budget in, so asking for one there is
/// bad input rather than a flag that quietly does nothing.
#[test]
fn a_budget_that_cannot_bind_is_refused() {
    let dir = repo();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(
        dir.path(),
        &["scope", "pkg.util.helper", "--depth", "99", "--budget", "1"],
    );
    assert_eq!(code(&out), 2, "{out:?}");
    assert!(
        stdout(&out).is_empty(),
        "a budget that does not bind must not serve the whole answer:\n{}",
        stdout(&out)
    );
    // With a tier to spend it in, the same budget is honoured, not refused.
    let tiered = grid(
        dir.path(),
        &["scope", "pkg.util.helper", "--bodies", "1", "--budget", "1"],
    );
    assert_eq!(code(&tiered), 0, "{tiered:?}");
    assert!(
        stdout(&tiered).contains("# horizon:"),
        "{}",
        stdout(&tiered)
    );
}

/// `--under` is a path filter. A prefix no charted definition sits below is
/// bad input: an empty list under a hint that reads as a real answer is the
/// silence PORCELAIN forbids.
#[test]
fn an_under_prefix_that_matches_nothing_is_bad_input() {
    let dir = repo();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    for verb in ["orphans", "hotspots"] {
        // A callsign prefix is not a path, and neither is a path that is not
        // in the tree.
        for prefix in ["pkg.util", "zzzz"] {
            let out = grid(dir.path(), &[verb, "--under", prefix]);
            assert_eq!(code(&out), 2, "{verb} --under {prefix}:\n{out:?}");
            assert!(stdout(&out).is_empty(), "{}", stdout(&out));
            assert!(
                stderr(&out).contains("no charted definition under"),
                "{verb} --under {prefix} must say what it wanted:\n{}",
                stderr(&out)
            );
        }
        // The real prefix still answers.
        let real = grid(dir.path(), &[verb, "--under", "pkg"]);
        assert_eq!(code(&real), 0, "{real:?}");
        assert!(
            !stdout(&real).is_empty(),
            "{verb} --under pkg found nothing"
        );
    }
}
