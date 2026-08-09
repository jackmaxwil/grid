//! The CLI contract: records on stdout, hints on stderr, exit codes uniform
//! across verbs (`docs/PORCELAIN.md`).
//!
//! CI greps this output and branches on these exit codes, so both are as much
//! a contract as the wire is. Every assertion here is something a caller is
//! entitled to rely on.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::json;

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
    std::fs::write(dir.path().join("a.py"), "def a():\n    pass\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "one"]);
    std::fs::write(dir.path().join("b.py"), "def b():\n    pass\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "two"]);
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

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// Records are tag-then-columns, separated by runs of spaces. This is the
/// parse every downstream `awk` will write.
fn columns(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

#[test]
fn bare_grid_teaches_on_stderr_and_says_nothing_on_stdout() {
    let dir = repo();
    let out = grid(dir.path(), &[]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).is_empty(), "machines read stdout only");
    let hints = String::from_utf8(out.stderr).expect("utf-8");
    assert!(hints.contains("what your code promises vs what it does"));
}

#[test]
fn grid_hints_0_silences_stderr_entirely() {
    let dir = repo();
    let out = Command::new(env!("CARGO_BIN_EXE_grid"))
        .current_dir(dir.path())
        .env("GRID_HINTS", "0")
        .output()
        .expect("run grid");
    assert!(
        out.stderr.is_empty(),
        "GRID_HINTS=0 must silence the channel"
    );
}

fn write_track(dir: &Path, name: &str, value: &serde_json::Value) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.json"));
    std::fs::write(&path, value.to_string()).expect("write track");
    path
}

#[test]
fn gate_hold_is_exit_3_and_the_first_record_says_so() {
    let dir = repo();
    let track = write_track(
        dir.path(),
        "t",
        &json!({
            "track_id": "track:pr-1-v1",
            "base": {"git_rev": "HEAD~1"},
            "delta": {"defs": [], "edges": [{
                "action": "add",
                "src": "app.one",
                "relation": "calls",
                "dst": "app.two"
            }]}
        }),
    );
    let out = grid(dir.path(), &["gate", track.to_str().expect("path"), "HEAD"]);

    assert_eq!(code(&out), 3, "a hold is exit 3, uniformly");
    let text = stdout(&out);
    let mut lines = text.lines();
    let first = columns(lines.next().expect("a verdict record"));
    assert_eq!(first[0], "hold", "the verdict is the first record");
    assert_eq!(first[1], "track:pr-1-v1");
    // The promised edge no file-level provider can see comes back as a miss,
    // in the shape CI greps for.
    assert!(
        text.lines().any(|l| {
            let c = columns(l);
            c[0] == "edge" && c[1] == "miss" && c.contains(&"-calls->")
        }),
        "`grep '^edge  miss'` is the explanation:\n{text}"
    );
}

#[test]
fn gate_pass_is_exit_0() {
    let dir = repo();
    let track = write_track(
        dir.path(),
        "t",
        &json!({
            "track_id": "track:pr-2-v1",
            "base": {"git_rev": "HEAD~1"},
            "delta": {"defs": [{"action": "add", "callsign": "b"}], "edges": []}
        }),
    );
    let out = grid(dir.path(), &["gate", track.to_str().expect("path"), "HEAD"]);
    assert_eq!(code(&out), 0, "stdout was:\n{}", stdout(&out));
    assert_eq!(
        columns(stdout(&out).lines().next().expect("record"))[0],
        "pass"
    );
}

#[test]
fn a_missing_track_is_bad_input_not_a_divergence() {
    let dir = repo();
    let out = grid(dir.path(), &["gate", "track:nope"]);
    assert_eq!(code(&out), 2, "unreadable input is 2, never 3");
    assert!(
        stdout(&out).is_empty(),
        "no records when there is no verdict"
    );
    // The error ends in a pasteable command (CLI plan §4).
    let hint = String::from_utf8(out.stderr).expect("utf-8");
    assert!(hint.contains("grid track new"), "hint was: {hint}");
}

#[test]
fn track_new_then_lint_then_show_is_the_planning_loop() {
    let dir = repo();
    let out = grid(dir.path(), &["track", "new", "pr-3-v1"]);
    assert_eq!(code(&out), 0);
    assert!(dir.path().join(".grid/tracks/pr-3-v1.json").is_file());

    // A track that promises nothing is a warning, not an error: it is
    // authorable, just vacuous.
    let out = grid(dir.path(), &["track", "lint", "pr-3-v1"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("promises nothing"));

    let out = grid(dir.path(), &["track", "show", "track:pr-3-v1"]);
    assert_eq!(code(&out), 0);
    assert_eq!(
        columns(stdout(&out).lines().next().expect("record"))[0],
        "todo"
    );

    // Creating it twice is bad input, not a silent overwrite of a promise.
    assert_eq!(code(&grid(dir.path(), &["track", "new", "pr-3-v1"])), 2);
}

#[test]
fn lint_rejects_an_impossible_plan_with_exit_2() {
    let dir = repo();
    std::fs::create_dir_all(dir.path().join(".grid")).expect("mkdir");
    std::fs::write(
        dir.path().join(".grid/invariants.json"),
        json!([{"id": "no-cross-layer", "rule": {"forbid_edge": {
            "src": "app.boards.**", "relation": "calls", "dst": "app.db.**"
        }}}])
        .to_string(),
    )
    .expect("write");
    let track = write_track(
        dir.path(),
        "t",
        &json!({
            "track_id": "track:pr-4-v1",
            "base": {"git_rev": "HEAD"},
            "delta": {"defs": [], "edges": [{
                "action": "add", "src": "app.boards.render",
                "relation": "calls", "dst": "app.db.write"
            }]}
        }),
    );

    let out = grid(
        dir.path(),
        &["track", "lint", track.to_str().expect("path")],
    );
    assert_eq!(code(&out), 2, "an impossible plan dies at plan time");
    assert!(stdout(&out).contains("violates invariant no-cross-layer"));
}

#[test]
fn json_is_the_wire_result_verbatim() {
    let dir = repo();
    let track = write_track(
        dir.path(),
        "t",
        &json!({
            "track_id": "track:pr-5-v1",
            "base": {"git_rev": "HEAD~1"},
            "delta": {"defs": [], "edges": []}
        }),
    );
    let out = grid(
        dir.path(),
        &["gate", track.to_str().expect("path"), "HEAD", "--json"],
    );
    // --json does not change the verdict, only the rendering: still exit 3.
    assert_eq!(code(&out), 3);
    let verdict: grid_wire::XteVerdict =
        serde_json::from_str(stdout(&out).trim()).expect("--json emits a wire XteVerdict");
    assert_eq!(verdict.gate, grid_wire::XteVerdictGate::Hold);
}

#[test]
fn version_reports_the_porcelain_grammar_it_implements() {
    let dir = repo();
    let out = grid(dir.path(), &["version"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let declared = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/PORCELAIN.md"
    ))
    .expect("read PORCELAIN.md");
    let version = declared
        .lines()
        .find_map(|l| l.strip_prefix("version: "))
        .expect("PORCELAIN.md declares a version");
    assert!(
        text.contains(version),
        "grid version must report the grammar it implements ({version}):\n{text}"
    );
}

#[test]
fn an_unknown_verb_is_bad_input() {
    let dir = repo();
    let out = grid(dir.path(), &["frobnicate"]);
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).is_empty());
}

/// Publish a chart with a caller and its proven callee, so the cache verbs
/// have real closures to key over.
fn seed_chart(dir: &Path, callee_byte: u8) -> (String, String) {
    use grid_chart::{build, DatumId, Def, Edge};

    let caller = DatumId([0x11; 32]);
    let target = DatumId([callee_byte; 32]);
    let image = build(
        &[
            Def {
                datum: caller,
                callsign: "app.caller".into(),
                lang: None,
            },
            Def {
                datum: target,
                callsign: "app.callee".into(),
                lang: None,
            },
        ],
        &[Edge {
            src: caller,
            dst: target,
            relation: grid_wire::Relation::Calls,
            zoc: grid_wire::Zoc::Proven,
        }],
    )
    .expect("build");
    let mut store = grid_chart_io::Store::open(dir.join(".grid/chart")).expect("store");
    store.publish("HEAD", None, &[image]).expect("publish");
    (caller.to_string(), target.to_string())
}

#[test]
fn cache_miss_then_record_then_hit() {
    // The CI loop the wave is for: check, run what missed, record, and the
    // next run hits.
    let dir = repo();
    let (caller, _) = seed_chart(dir.path(), 0x22);
    let env = "env:b3:aa";

    let cold = grid(dir.path(), &["cache", "check", "test", env, &caller]);
    assert_eq!(code(&cold), 0, "a miss is not a failure");
    let line = stdout(&cold);
    let fields = columns(line.lines().next().expect("a record"));
    assert_eq!(fields[0], "miss");
    assert_eq!(fields[1], caller);

    let recorded = grid(
        dir.path(),
        &["cache", "record", "test", env, &format!("{caller}=pass")],
    );
    assert_eq!(code(&recorded), 0);
    assert_eq!(
        columns(stdout(&recorded).lines().next().unwrap())[..2],
        ["done", "1/1"]
    );

    let warm = stdout(&grid(dir.path(), &["cache", "check", "test", env, &caller]));
    let fields = columns(warm.lines().next().expect("a record"));
    assert_eq!(fields[0], "hit");
    assert_eq!(fields[1], caller);
    assert_eq!(fields[2], "pass");
}

#[test]
fn a_callee_change_invalidates_the_caller_but_not_its_lint() {
    // The wave's acceptance criterion, at the CLI: a synthetic callee change
    // invalidates its callers for kinds whose closure covers the callee, and
    // only those kinds.
    let dir = repo();
    let (caller, _) = seed_chart(dir.path(), 0x22);
    let env = "env:b3:aa";

    for kind in ["test", "lint"] {
        let out = grid(
            dir.path(),
            &["cache", "record", kind, env, &format!("{caller}=pass")],
        );
        assert_eq!(code(&out), 0);
    }

    // Same caller, different callee: republish the chart with the callee moved.
    seed_chart(dir.path(), 0x33);

    let test_check = stdout(&grid(dir.path(), &["cache", "check", "test", env, &caller]));
    assert!(
        test_check.starts_with("miss"),
        "a test verdict must not survive its callee changing: {test_check}"
    );
    let lint_check = stdout(&grid(dir.path(), &["cache", "check", "lint", env, &caller]));
    assert!(
        lint_check.starts_with("hit"),
        "lint reads one span, so a callee change cannot invalidate it: {lint_check}"
    );
}

#[test]
fn an_unresolvable_closure_is_warned_about_and_not_recorded() {
    let dir = repo();
    seed_chart(dir.path(), 0x22);
    let stranger = format!("b3:{}", "ab".repeat(32));

    let out = grid(
        dir.path(),
        &[
            "cache",
            "record",
            "test",
            "env:b3:aa",
            &format!("{stranger}=pass"),
        ],
    );
    // Refusing to store is the cache working, so the exit code is success.
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let mut lines = text.lines();
    assert_eq!(columns(lines.next().expect("done"))[..2], ["done", "0/1"]);
    let warn = columns(lines.next().expect("a warn record"));
    assert_eq!(warn[0], "warn");
    assert_eq!(warn[1], stranger);
}

#[test]
fn cache_json_is_the_wire_result_verbatim() {
    let dir = repo();
    let (caller, _) = seed_chart(dir.path(), 0x22);
    let out = grid(
        dir.path(),
        &["cache", "check", "--json", "test", "env:b3:aa", &caller],
    );
    assert_eq!(code(&out), 0);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(value["misses"][0], json!(caller));
    assert!(value["snapshot"]
        .as_str()
        .is_some_and(|s| s.starts_with("snap:b3:")));
}

#[test]
fn a_bad_cache_kind_is_bad_input() {
    let dir = repo();
    let out = grid(
        dir.path(),
        &["cache", "check", "typo", "env:b3:aa", "b3:00"],
    );
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).is_empty(), "errors go to stderr, not stdout");
}
