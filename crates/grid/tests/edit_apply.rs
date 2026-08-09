//! Wave 6 assurance evidence (C15): an edit addressed at a stale coordinate
//! conflicts and writes nothing — it never lands against moved code.
//!
//! Every test runs the real binary against a throwaway git repo: the write
//! path is filesystem-shaped, so the tests are too.

use std::path::Path;
use std::process::{Command, Output};

/// A repo with one module defining one function, committed.
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

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let git = |args: &[&str]| git(dir.path(), args);
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("util.py"), "def helper():\n    return 1\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "one"]);
    dir
}

fn grid(dir: &Path, args: &[&str], stdin: &str) -> Output {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn grid");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf-8")
}

fn columns(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

/// The current datum of `util.helper`, read out of the tree the way the
/// wire result reports it.
fn current_datum(dir: &Path) -> String {
    let source = std::fs::read_to_string(dir.join("util.py")).expect("read");
    let defs = grid_survey::extract(&source).defs;
    assert_eq!(defs.len(), 1);
    defs[0].datum.as_str().to_string()
}

fn edit_request(target: &str, base: Option<&str>, new_text: &str) -> String {
    match base {
        Some(base) => serde_json::json!({"edits": [{
            "target": target, "base_datum": base, "new_text": new_text
        }]}),
        None => serde_json::json!({"edits": [{
            "target": target, "new_text": new_text
        }]}),
    }
    .to_string()
}

#[test]
fn a_matched_base_applies_and_reports_the_new_identity() {
    let dir = repo();
    let base = current_datum(dir.path());
    let request = edit_request("util.helper", Some(&base), "def helper():\n    return 2\n");
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(
        code(&out),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    let fields = columns(text.lines().next().expect("a record"));
    assert_eq!(fields[0], "def");
    assert_eq!(fields[1], "~");
    assert_eq!(fields[2], "util.helper");
    assert!(fields[3].contains('→'), "a transition: {fields:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("util.py")).expect("read"),
        "def helper():\n    return 2\n"
    );
    // The reported identity is the identity: recompute from the tree.
    let reported = stdout(&out);
    let now = current_datum(dir.path());
    assert!(
        reported.contains(&now[3..7]),
        "the record names the new datum: {reported}"
    );
    assert_ne!(base, now, "the content changed, so the datum moved");
}

#[test]
fn a_stale_base_conflicts_writes_nothing_and_recovers() {
    let dir = repo();
    let base = current_datum(dir.path());

    // Someone else moves the definition.
    let first = edit_request("util.helper", Some(&base), "def helper():\n    return 2\n");
    assert_eq!(code(&grid(dir.path(), &["edit", "apply", "-"], &first)), 0);

    // The stale edit: conflict records, exit 4, and the file is untouched.
    let before = std::fs::read_to_string(dir.path().join("util.py")).expect("read");
    let stale = edit_request("util.helper", Some(&base), "def helper():\n    return 3\n");
    let out = grid(dir.path(), &["edit", "apply", "-"], &stale);
    assert_eq!(code(&out), 4, "a stale coordinate is exit 4");
    let text = stdout(&out);
    let fields = columns(text.lines().next().expect("a conflict record"));
    assert_eq!(fields[0], "conflict");
    assert_eq!(fields[1], "util.helper");
    assert!(fields[2].contains('→'), "was→now: {fields:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("util.py")).expect("read"),
        before,
        "a conflicted request writes nothing"
    );

    // The recovery: the conflict's `now` is the current identity, so a retry
    // based on it lands.
    let now: serde_json::Value = serde_json::from_str(&stdout(&grid(
        dir.path(),
        &["edit", "apply", "-", "--json"],
        &stale,
    )))
    .expect("json");
    let fresh = now["conflicts"][0]["now"]
        .as_str()
        .expect("now")
        .to_string();
    let retry = edit_request("util.helper", Some(&fresh), "def helper():\n    return 3\n");
    let out = grid(dir.path(), &["edit", "apply", "-"], &retry);
    assert_eq!(
        code(&out),
        0,
        "the retry lands: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("util.py")).expect("read"),
        "def helper():\n    return 3\n"
    );
}

#[test]
fn a_stale_handle_prefix_is_a_conflict_not_an_unknown_target() {
    let dir = repo();
    let base = current_datum(dir.path());
    let stale_prefix = &base[3..7];
    // Move the def, then address it with the handle as read.
    let first = edit_request("util.helper", Some(&base), "def helper():\n    return 2\n");
    assert_eq!(code(&grid(dir.path(), &["edit", "apply", "-"], &first)), 0);
    let request = edit_request(
        &format!("util.helper@{stale_prefix}"),
        Some(&base),
        "def helper():\n    return 3\n",
    );
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(code(&out), 4, "a stale prefix is the same stale read");
    assert!(stdout(&out).contains("conflict"));
}

#[test]
fn an_ambiguous_or_unknown_target_is_bad_input_and_writes_nothing() {
    let dir = repo();
    std::fs::write(dir.path().join("other.py"), "def helper():\n    return 9\n").expect("write");
    // Discovery is `git ls-files`: an untracked file is invisible to edits.
    git(dir.path(), &["add", "other.py"]);
    let before = std::fs::read_to_string(dir.path().join("util.py")).expect("read");

    // No base would be a creation; a modify reaches resolution.
    let ambiguous = edit_request(
        "helper",
        Some("b3:0000000000000000000000000000000000000000000000000000000000000000"),
        "def helper():\n    return 0\n",
    );
    let out = grid(dir.path(), &["edit", "apply", "-"], &ambiguous);
    assert_eq!(code(&out), 2);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("ambiguous"), "stderr: {err}");
    assert!(
        err.contains("util.helper") && err.contains("other.helper"),
        "candidates: {err}"
    );

    let unknown = edit_request(
        "no.such.def",
        Some("b3:0000000000000000000000000000000000000000000000000000000000000000"),
        "def x():\n    pass\n",
    );
    let out = grid(dir.path(), &["edit", "apply", "-"], &unknown);
    assert_eq!(code(&out), 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("util.py")).expect("read"),
        before,
        "bad input writes nothing"
    );
}

#[test]
fn a_creation_writes_a_new_file_and_a_config_bootstrap_defines_nothing() {
    let dir = repo();
    // A new module: the new definition's identity is reported.
    let request = edit_request("extra.py", None, "def extra():\n    return 2\n");
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let fields = columns(text.lines().next().expect("a record"));
    assert_eq!(&fields[..3], ["def", "+", "extra.extra"]);
    assert!(dir.path().join("extra.py").is_file());

    // Grid's own config bootstrap shape (D6): a path that is not source.
    let request = edit_request(".grid/notes.json", None, "{}\n");
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(code(&out), 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".grid/notes.json")).expect("read"),
        "{}\n"
    );

    // Creating what exists is bad input, and creating it again did not append.
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(code(&out), 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".grid/notes.json")).expect("read"),
        "{}\n"
    );
}

#[test]
fn an_edit_that_would_not_parse_is_refused_before_any_write() {
    let dir = repo();
    let before = std::fs::read_to_string(dir.path().join("util.py")).expect("read");
    let base = current_datum(dir.path());
    let request = edit_request("util.helper", Some(&base), "def helper(:\n");
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(
        code(&out),
        2,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("parse"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("util.py")).expect("read"),
        before
    );
}

#[test]
fn one_request_many_files_applies_in_order() {
    let dir = repo();
    std::fs::write(dir.path().join("a.py"), "def a():\n    return 1\n").expect("write");
    git(dir.path(), &["add", "a.py"]);
    let helper = current_datum(dir.path());
    let a_source = std::fs::read_to_string(dir.path().join("a.py")).expect("read");
    let a_datum = grid_survey::extract(&a_source).defs[0]
        .datum
        .as_str()
        .to_string();
    let request = serde_json::json!({"edits": [
        {"target": "util.helper", "base_datum": helper, "new_text": "def helper():\n    return 2\n"},
        {"target": "a.a", "base_datum": a_datum, "new_text": "def a():\n    return 3\n"},
    ]})
    .to_string();
    let out = grid(dir.path(), &["edit", "apply", "-", "--json"], &request);
    assert_eq!(
        code(&out),
        0,
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(result["applied"].as_array().expect("applied").len(), 2);
    assert!(result["conflicts"]
        .as_array()
        .expect("conflicts")
        .is_empty());
    assert!(std::fs::read_to_string(dir.path().join("a.py"))
        .expect("read")
        .contains("return 3"));
}

#[test]
fn a_request_with_one_conflict_lands_nothing() {
    // Atomicity across the request: the good edit must not land either,
    // because a retry of the failed one alone would leave the tree in a
    // state the agent never validated.
    let dir = repo();
    std::fs::write(dir.path().join("a.py"), "def a():\n    return 1\n").expect("write");
    git(dir.path(), &["add", "a.py"]);
    let before_util = std::fs::read_to_string(dir.path().join("util.py")).expect("read");
    let before_a = std::fs::read_to_string(dir.path().join("a.py")).expect("read");
    let helper = current_datum(dir.path());
    let request = serde_json::json!({"edits": [
        {"target": "util.helper", "base_datum": helper, "new_text": "def helper():\n    return 2\n"},
        {"target": "a.a", "base_datum": "b3:0000000000000000000000000000000000000000000000000000000000000000", "new_text": "def a():\n    return 3\n"},
    ]})
    .to_string();
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(code(&out), 4);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("util.py")).expect("read"),
        before_util
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.py")).expect("read"),
        before_a
    );
}

#[test]
fn an_escaping_path_is_refused() {
    let dir = repo();
    let request = edit_request("../escape.py", None, "def escape():\n    pass\n");
    let out = grid(dir.path(), &["edit", "apply", "-"], &request);
    assert_eq!(code(&out), 2);
    assert!(!dir
        .path()
        .parent()
        .expect("parent")
        .join("escape.py")
        .exists());
}
