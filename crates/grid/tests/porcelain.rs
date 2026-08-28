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

/// A repo whose second commit adds a definition *and* a call to it, so the
/// chart has a real edge to report. The file-level provider of wave 3 could
/// not have produced one.
fn repo_with_a_call() -> tempfile::TempDir {
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
    let write = |path: &str, body: &str| {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(full, body).expect("write");
    };
    git(&["init", "-q", "-b", "main"]);
    write("pkg/__init__.py", "");
    write("pkg/util.py", "def helper():\n    return 1\n");
    write(
        "pkg/main.py",
        "from pkg.util import helper\n\n\ndef sweep():\n    helper()\n",
    );
    git(&["add", "-A"]);
    git(&["commit", "-qm", "one"]);
    write(
        "pkg/util.py",
        "def helper():\n    return 1\n\n\ndef extra():\n    return 2\n",
    );
    write(
        "pkg/main.py",
        "from pkg.util import helper, extra\n\n\ndef sweep():\n    helper()\n    extra()\n",
    );
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
fn bare_grid_shows_the_diff_and_teaches_on_stderr() {
    // Bare `grid` is the everyday verb (CLI plan §4): what changed. The
    // teaching still happens, on stderr, so a machine reading stdout gets
    // only records and a human gets both.
    let dir = repo();
    let out = grid(dir.path(), &[]);
    assert_eq!(code(&out), 0);
    let hints = String::from_utf8(out.stderr.clone()).expect("utf-8");
    assert!(hints.contains("what your code promises vs what it does"));
    // `b.py` was added in the second commit, so the default HEAD~1..HEAD
    // diff is exactly that definition.
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| {
            let c = columns(l);
            c.first() == Some(&"def") && c.get(1) == Some(&"+") && c.get(2) == Some(&"b.b")
        }),
        "bare grid is the diff:\n{text}"
    );
}

#[test]
fn help_teaches_without_running_anything() {
    // `grid help` is the one that says nothing on stdout: it is the teaching
    // surface, not a query.
    let dir = repo();
    let out = grid(dir.path(), &["help"]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).is_empty(), "machines read stdout only");
    assert!(String::from_utf8(out.stderr)
        .expect("utf-8")
        .contains("what your code promises vs what it does"));
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
            // Callsigns are module-qualified now that the chart is real:
            // `b.py` defines `b.b`, not `b`.
            "delta": {"defs": [{"action": "add", "callsign": "b.b"}], "edges": []}
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

#[test]
fn survey_emits_one_snapshot_record() {
    let dir = repo();
    let out = grid(dir.path(), &["survey"]);
    assert_eq!(
        code(&out),
        0,
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    let c = columns(text.lines().next().expect("a record"));
    assert_eq!(c[0], "snapshot");
    // Displayed identity is the rev, never the raw content id (CLI plan §2).
    assert!(c[1].starts_with('@'), "snapshot displays @rev: {text}");
    assert!(text.contains("defs "), "stats are on the record: {text}");
}

#[test]
fn diff_records_the_definition_that_changed() {
    let dir = repo();
    let out = grid(dir.path(), &["diff", "HEAD~1", "HEAD"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| {
            let c = columns(l);
            c[0] == "def" && c[1] == "+" && c[2] == "b.b"
        }),
        "the added definition is a `def +` record:\n{text}"
    );
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
                facts: None,
            },
            Def {
                datum: target,
                callsign: "app.callee".into(),
                lang: None,
                facts: None,
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
fn diff_prints_edges_in_callsign_space_not_as_hashes() {
    // The display policy: callsigns are identity, hashes appear only where
    // identity beyond the name matters. An arrow is not one of those places.
    let dir = repo_with_a_call();
    let out = grid(dir.path(), &["diff", "HEAD~1", "HEAD"]);
    let text = stdout(&out);
    let arrow = text
        .lines()
        .find(|l| l.starts_with("edge"))
        .unwrap_or_else(|| panic!("no edge record:\n{text}"));
    assert!(
        arrow.contains("pkg.main.sweep -calls-> pkg.util.extra"),
        "edges read as names: {arrow}"
    );
    assert!(
        !arrow.contains("b3:"),
        "a raw digest is neither readable nor typeable: {arrow}"
    );
}

#[test]
fn resolve_finds_a_definition_by_suffix_and_refuses_an_unknown_name_loudly() {
    let dir = repo();
    grid(dir.path(), &["survey"]);
    let out = grid(dir.path(), &["resolve", "b"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let c = columns(text.lines().next().expect("record"));
    // `ref`, not `def`: a resolved definition exists, it did not change, so
    // there is no glyph column and the callsign is first.
    assert_eq!((c[0], c[1]), ("ref", "b.b"));

    // A miss is bad input with an empty stdout: silence read as "does not
    // exist" was the worst agent failure mode, so the refusal is loud now.
    let out = grid(dir.path(), &["resolve", "no.such.name"]);
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).is_empty());
}

#[test]
fn a_gate_over_a_real_edge_promise_passes_when_the_edge_is_built() {
    // The wave's headline: a promise about a *call* is now checked against a
    // surveyed call, not reported `missing` for want of a chart.
    let dir = repo_with_a_call();
    let track = write_track(
        dir.path(),
        "edge",
        &json!({
            "track_id": "track:edge/v1",
            "base": {"git_rev": "HEAD~1"},
            "delta": {
                "defs": [
                    {"action": "add", "callsign": "pkg.util.extra"},
                    {"action": "modify", "callsign": "pkg.main.sweep"}
                ],
                "edges": [{
                    "action": "add",
                    "src": "pkg.main.sweep",
                    "relation": "calls",
                    "dst": "pkg.util.extra"
                }]
            }
        }),
    );
    let out = grid(dir.path(), &["gate", track.to_str().expect("path"), "HEAD"]);
    let text = stdout(&out);
    assert_eq!(code(&out), 0, "stdout:\n{text}");
    assert!(
        text.lines().any(|l| {
            let c = columns(l);
            c[0] == "edge" && c[1] == "match"
        }),
        "the promised edge is matched, not missing:\n{text}"
    );
}

#[test]
fn a_gate_holds_when_the_promised_edge_was_not_built() {
    // The inverse, and the one CI branches on: exit 3, with the divergence
    // itemized as records.
    let dir = repo_with_a_call();
    let track = write_track(
        dir.path(),
        "unbuilt",
        &json!({
            "track_id": "track:unbuilt/v1",
            "base": {"git_rev": "HEAD~1"},
            "delta": {
                "defs": [],
                "edges": [{
                    "action": "add",
                    "src": "pkg.main.sweep",
                    "relation": "calls",
                    "dst": "pkg.util.never_written"
                }]
            }
        }),
    );
    let out = grid(dir.path(), &["gate", track.to_str().expect("path"), "HEAD"]);
    let text = stdout(&out);
    assert_eq!(code(&out), 3, "stdout:\n{text}");
    assert!(
        text.lines().any(|l| {
            let c = columns(l);
            c[0] == "edge" && c[1] == "miss"
        }),
        "`grep '^edge  miss'` is the explanation:\n{text}"
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

#[test]
fn scope_prints_defs_edges_and_invariants_as_records() {
    let dir = repo_with_a_call();
    // A standing invariant that reaches the scoped name, and one that does
    // not: only the first is in scope.
    std::fs::create_dir_all(dir.path().join(".grid")).expect("mkdir");
    std::fs::write(
        dir.path().join(".grid/invariants.json"),
        serde_json::json!([
            {"id": "util-pure", "rule": {"forbid_edge": {
                "src": "pkg.util.**", "relation": "calls", "dst": "pkg.main.**"}}},
            {"id": "unrelated", "rule": {"forbid_edge": {
                "src": "nowhere.**", "relation": "calls", "dst": "elsewhere.**"}}}
        ])
        .to_string(),
    )
    .expect("write invariants");
    grid(dir.path(), &["survey"]);

    let out = grid(dir.path(), &["scope", "pkg.util.extra"]);
    assert_eq!(code(&out), 0, "stdout:\n{}", stdout(&out));
    let text = stdout(&out);
    let tags: Vec<&str> = text.lines().map(|l| columns(l)[0]).collect();
    assert!(tags.contains(&"ref"), "a def record: {text}");
    assert!(tags.contains(&"edge"), "an edge record: {text}");
    assert!(tags.contains(&"inv"), "an invariant record: {text}");
    // The edge record's first column is the evidence grade, and the arrow is
    // callsign-space.
    let edge = text
        .lines()
        .find(|l| columns(l)[0] == "edge")
        .expect("edge");
    let fields = columns(edge);
    assert_eq!(fields[1], "proven");
    assert!(
        edge.contains("pkg.main.sweep -calls-> pkg.util.extra"),
        "{edge}"
    );
    // Only the invariant whose pattern reaches the scope is listed.
    let inv = text.lines().find(|l| columns(l)[0] == "inv").expect("inv");
    assert!(inv.contains("util-pure"), "{inv}");
    assert!(!text.contains("unrelated"), "{text}");
}

#[test]
fn uses_answers_who_reaches_a_name_with_proven_edges_only() {
    let dir = repo_with_a_call();
    grid(dir.path(), &["survey"]);
    let out = grid(dir.path(), &["uses", "extra"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let fields = columns(text.lines().next().expect("an edge record"));
    assert_eq!(fields[0], "edge");
    assert_eq!(fields[1], "proven");
    assert!(
        text.contains("pkg.main.sweep -calls-> pkg.util.extra"),
        "{text}"
    );

    // An unknown name is bad input, not an empty answer.
    let out = grid(dir.path(), &["uses", "no_such_name"]);
    assert_eq!(code(&out), 2);
}

#[test]
fn uses_on_an_ambiguous_name_lists_pasteable_commands() {
    let dir = repo_with_a_call();
    // A second `sweep` makes the short name ambiguous.
    std::fs::write(
        dir.path().join("pkg/other.py"),
        "def sweep():\n    return 0\n",
    )
    .expect("write");
    let add = Command::new("git")
        .args(["add", "pkg/other.py"])
        .current_dir(dir.path())
        .output()
        .expect("git add");
    assert!(add.status.success());
    grid(dir.path(), &["survey"]);
    let out = grid(dir.path(), &["uses", "sweep"]);
    assert_eq!(code(&out), 2);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("grid uses pkg.main.sweep"), "{err}");
    assert!(err.contains("grid uses pkg.other.sweep"), "{err}");
}

#[test]
fn todo_renders_the_fraction_then_the_pending_promises() {
    let dir = repo_with_a_call();
    let track = write_track(
        dir.path(),
        "todo",
        &json!({
            "track_id": "track:todo-v1",
            "base": {"git_rev": "HEAD~1"},
            "delta": {
                "defs": [{"action": "add", "callsign": "pkg.util.extra"}],
                "edges": [
                    {"action": "add", "src": "pkg.main.sweep",
                     "relation": "calls", "dst": "pkg.util.extra"},
                    {"action": "add", "src": "pkg.main.sweep",
                     "relation": "calls", "dst": "pkg.util.never_built"}
                ]
            }
        }),
    );
    let out = grid(dir.path(), &["todo", track.to_str().expect("path"), "HEAD"]);
    assert_eq!(code(&out), 0, "a worklist is a report, not a verdict");
    let text = stdout(&out);
    let first = columns(text.lines().next().expect("the fraction record"));
    assert_eq!(first[0], "done");
    // Two promises made good (one def, one edge), one edge pending: 2/3.
    assert_eq!(first[1], "2/3", "{text}");
    assert!(text.contains("edges 1/2 · defs 1/1 · tests 0/0"), "{text}");
    // The pending promise prints as the same record a fresh track would show.
    let pending: Vec<&str> = text.lines().filter(|l| l.contains("never_built")).collect();
    assert_eq!(pending.len(), 1, "{text}");
    assert_eq!(columns(pending[0])[0], "edge");
}

#[test]
fn explain_answers_from_the_generated_corpus() {
    let dir = repo();
    let out = grid(dir.path(), &["explain", "hold"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(text.starts_with("hold — "), "{text}");
    assert!(text.contains("diverge"), "{text}");

    // A wire word the ontology does not name still answers, from the
    // contract's own description.
    let out = grid(dir.path(), &["explain", "callsignpattern"]);
    assert_eq!(code(&out), 0);

    // An unknown token is bad input with a hint, not a shrug.
    let out = grid(dir.path(), &["explain", "xyzzy"]);
    assert_eq!(code(&out), 2);
}

#[test]
fn explain_handle_names_the_definition_the_prefix_pins() {
    let dir = repo_with_a_call();
    grid(dir.path(), &["survey"]);
    let resolved = grid(dir.path(), &["resolve", "extra", "--json"]);
    let defs: serde_json::Value = serde_json::from_str(&stdout(&resolved)).expect("json");
    let datum = defs[0]["datum"].as_str().expect("datum");
    let prefix = &datum[3..7];
    let out = grid(dir.path(), &["explain", &format!("@{prefix}")]);
    assert_eq!(code(&out), 0);
    assert!(stdout(&out).contains("pkg.util.extra"), "{}", stdout(&out));
}
/// Write a wake JSON into `dir` and return its path for `grid wake ingest`.
fn wake_file(dir: &Path, name: &str, wake_id: &str, hops: &[(&str, &str)]) -> std::path::PathBuf {
    let observations: Vec<serde_json::Value> = hops
        .iter()
        .map(|(caller, callee)| {
            serde_json::json!({
                "caller": { "datum": caller },
                "callee": { "datum": callee },
            })
        })
        .collect();
    let path = dir.join(name);
    std::fs::write(
        &path,
        serde_json::json!({
            "wake_id": wake_id,
            "git_rev": "4f2c9e1",
            "observations": observations,
        })
        .to_string(),
    )
    .expect("write wake");
    path
}
#[test]
#[allow(clippy::similar_names)] // caller/callee are the domain words
fn wake_ingest_then_diff_is_the_ci_loop() {
    let dir = repo();
    let (caller, callee) = seed_chart(dir.path(), 0x22);
    let green = wake_file(dir.path(), "green.json", "wake:g", &[(&caller, &callee)]);
    let red = wake_file(
        dir.path(),
        "red.json",
        "wake:r",
        &[(&caller, &callee), (&callee, &caller)],
    );
    for (file, want) in [(&green, "1/1"), (&red, "2/2")] {
        let out = grid(
            dir.path(),
            &["wake", "ingest", file.to_str().expect("utf-8 path")],
        );
        assert_eq!(code(&out), 0, "{}", stdout(&out));
        let text = stdout(&out);
        let cols = columns(text.lines().next().expect("done record"));
        assert_eq!(cols[0], "done");
        assert_eq!(cols[1], want);
    }
    // Same trace twice: no fork, exit 0.
    let same = grid(dir.path(), &["wake", "diff", "wake:g", "wake:g"]);
    assert_eq!(code(&same), 0);
    assert!(stdout(&same).is_empty(), "no records without a difference");
    // Green against red: red kept calling, exit 3, fork then edge records.
    let forked = grid(dir.path(), &["wake", "diff", "wake:g", "wake:r"]);
    assert_eq!(code(&forked), 3, "a forked trace is a divergence");
    let text = stdout(&forked);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(columns(lines[0])[0], "fork");
    assert_eq!(columns(lines[0])[2], "-", "green had no second hop");
    assert_eq!(columns(lines[1])[0], "edge");
    assert_eq!(columns(lines[1])[1], "+");
    assert_eq!(columns(lines[1])[5], "observed");
}
#[test]
fn wake_ingest_reports_unresolved_symbols_never_drops_them() {
    let dir = repo();
    seed_chart(dir.path(), 0x22);
    let path = dir.path().join("wake.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "wake_id": "wake:u",
            "git_rev": "4f2c9e1",
            "observations": [
                { "caller": { "symbol": "caller" }, "callee": { "symbol": "no.such" } },
            ],
        })
        .to_string(),
    )
    .expect("write wake");
    let out = grid(
        dir.path(),
        &["wake", "ingest", path.to_str().expect("utf-8")],
    );
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let cols = columns(text.lines().next().expect("done record"));
    assert_eq!(cols[1], "0/1", "the hop resolved to nothing: {text}");
    let warn = columns(text.lines().nth(1).expect("warn record"));
    assert_eq!(warn[0], "warn");
    assert_eq!(warn[1], "no.such");
}
#[test]
fn a_diff_against_an_unknown_wake_is_bad_input() {
    let dir = repo();
    seed_chart(dir.path(), 0x22);
    let out = grid(dir.path(), &["wake", "diff", "wake:nope", "wake:alsono"]);
    assert_eq!(code(&out), 2);
}

#[test]
fn orphans_lists_the_uncalled_with_their_location_and_never_the_called() {
    // repo(): a.py defines a (never called), b.py defines b (never called);
    // add a caller of b so exactly one def has a proven inbound edge.
    let dir = repo();
    std::fs::write(
        dir.path().join("c.py"),
        "from b import b\ndef c():\n    return b()\n",
    )
    .expect("write");
    // Discovery is `git ls-files`: untracked files are invisible on purpose.
    let ok = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output()
        .expect("git add")
        .status
        .success();
    assert!(ok);
    let out = grid(dir.path(), &["survey"]);
    assert_eq!(code(&out), 0);
    let out = grid(dir.path(), &["orphans"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let rows: Vec<&str> = text.lines().collect();
    assert!(
        rows.iter().any(|l| columns(l)[..2] == ["orphan", "a.a"]),
        "the uncalled def is an orphan row:\n{text}"
    );
    assert!(
        !rows.iter().any(|l| columns(l).get(1) == Some(&"b.b")),
        "a def with a proven caller is not an orphan:\n{text}"
    );
    // The location column is what makes the row pasteable.
    assert!(
        rows.iter()
            .any(|l| columns(l).last().is_some_and(|c| c.starts_with("a.py:"))),
        "orphan rows carry file:line:\n{text}"
    );
}

#[test]
fn lcov_ingests_as_a_reach_set_and_omits_what_it_cannot_map() {
    let dir = repo();
    // b.py: `def b():` line 1, body line 2. Line 1-2 executed inside b.b;
    // a DA outside any def (no such line) is omitted, not guessed.
    let out = grid(dir.path(), &["survey"]);
    assert_eq!(code(&out), 0);
    let lcov = format!(
        "TN:\nSF:{}/b.py\nDA:1,4\nDA:2,4\nDA:9,4\nend_of_record\n",
        dir.path().display()
    );
    std::fs::write(dir.path().join("cov.lcov"), lcov).expect("write");
    let out = grid(
        dir.path(),
        &["wake", "ingest", "--lcov", "cov.lcov", "--test", "a"],
    );
    assert_eq!(
        code(&out),
        0,
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // One observation: test a -> def b.b (both lines fold into one datum).
    let text = stdout(&out);
    assert!(
        text.lines()
            .next()
            .is_some_and(|l| columns(l)[..2] == ["done", "1/1"]),
        "one deduplicated observation ingested:\n{text}"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("1 executed lines outside any definition"),
        "the unmappable line is counted, never silently dropped:\n{err}"
    );
}

#[test]
fn hotspots_rank_the_branchy_and_edited_above_the_large_and_flat() {
    let dir = repo();
    // branchy.py: complexity from real branches. flat.py: long but straight.
    std::fs::write(
        dir.path().join("branchy.py"),
        "def hot(x):\n    if x:\n        return 1\n    if not x:\n        return 2\n    for i in [1]:\n        pass\n    return 3\n",
    )
    .expect("write");
    let flat = format!(
        "def cold(x):\n{}    return x\n",
        "    x = x + 1\n".repeat(30)
    );
    std::fs::write(dir.path().join("flat.py"), flat).expect("write");
    let ok = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output()
        .expect("git add")
        .status
        .success();
    assert!(ok);
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    // Edit the branchy def twice so the log gives it churn.
    for body in ["return 4", "return 5"] {
        std::fs::write(
            dir.path().join("branchy.py"),
            format!("def hot(x):\n    if x:\n        return 1\n    if not x:\n        return 2\n    for i in [1]:\n        pass\n    {body}\n"),
        )
        .expect("write");
        assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    }
    let out = grid(dir.path(), &["hotspots", "--limit", "10"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let pos = |name: &str| text.lines().position(|l| columns(l).get(1) == Some(&name));
    let (hot, cold) = (pos("branchy.hot"), pos("flat.cold"));
    assert!(
        hot.is_some() && (cold.is_none() || hot < cold),
        "branchy-and-edited outranks large-and-flat:\n{text}"
    );
    assert!(
        text.lines().next().is_some_and(|l| l.contains("legs 2")),
        "churn is the multiplier and is reported:\n{text}"
    );
}

#[test]
fn under_scopes_orphans_and_hotspots_to_a_subtree() {
    let dir = repo();
    std::fs::create_dir_all(dir.path().join("pkg")).expect("mkdir");
    std::fs::write(dir.path().join("pkg/c.py"), "def c():\n    pass\n").expect("write");
    let ok = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output()
        .expect("git add")
        .status
        .success();
    assert!(ok);
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["orphans", "--under", "pkg"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| columns(l).get(1) == Some(&"pkg.c.c")),
        "the subtree's orphan is listed:\n{text}"
    );
    assert!(
        !text.lines().any(|l| columns(l).get(1) == Some(&"a.a")),
        "rows outside --under are filtered:\n{text}"
    );
    let out = grid(dir.path(), &["hotspots", "--under", "pkg"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.lines().all(|l| columns(l).get(1) == Some(&"pkg.c.c")),
        "hotspot rows outside --under are filtered:\n{text}"
    );
}

#[test]
fn scope_defs_carry_their_location() {
    let dir = repo_with_a_call();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["scope", "pkg.util.extra"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    // The ref row is pasteable, same as resolve and orphans: file:line last.
    let row = text
        .lines()
        .find(|l| columns(l)[0] == "ref")
        .expect("a ref row");
    assert!(
        columns(row)
            .last()
            .is_some_and(|c| c.starts_with("pkg/util.py:")),
        "scope refs carry file:line:\n{text}"
    );
}

#[test]
fn resolving_an_unknown_name_is_bad_input_with_the_closest_callsigns() {
    let dir = repo_with_a_call();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    // `pkg.helper` skips a path segment, so it is not a suffix of
    // `pkg.util.helper`; the refusal must name the near miss instead of
    // silently printing nothing.
    let out = grid(dir.path(), &["resolve", "pkg.helper"]);
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).is_empty(), "no records for a non-answer");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("pkg.util.helper"),
        "the near miss is suggested:\n{err}"
    );
    let out = grid(dir.path(), &["resolve", "helpr"]);
    assert_eq!(code(&out), 2);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("pkg.util.helper"),
        "a typo still finds its neighbour:\n{err}"
    );
}

#[test]
fn uses_with_no_proven_callers_says_so_instead_of_silence() {
    let dir = repo();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["uses", "a.a"]);
    assert_eq!(code(&out), 0);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("no proven callers"),
        "zero edges get a voice:\n{err}"
    );
}

#[test]
fn help_lists_every_verb() {
    let dir = repo();
    let out = grid(dir.path(), &["help"]);
    assert_eq!(code(&out), 0);
    let err = String::from_utf8_lossy(&out.stderr);
    for verb in [
        "survey", "resolve", "uses", "scope", "orphans", "hotspots", "log", "bisect", "gate",
        "cache", "edit", "wake", "roots",
    ] {
        assert!(err.contains(verb), "`{verb}` missing from help:\n{err}");
    }
}

#[test]
fn roots_lists_import_name_to_directory() {
    let dir = repo();
    std::fs::create_dir_all(dir.path().join("crates/app/src")).expect("mkdir");
    std::fs::write(
        dir.path().join("crates/app/Cargo.toml"),
        "[package]\nname = \"my-app\"\n",
    )
    .expect("write");
    std::fs::write(
        dir.path().join("crates/app/src/lib.rs"),
        "pub fn a() -> u32 { 1 }\n",
    )
    .expect("write");
    let ok = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output()
        .expect("git add")
        .status
        .success();
    assert!(ok);
    let out = grid(dir.path(), &["roots"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| {
            let c = columns(l);
            c.first() == Some(&"root")
                && c.get(1) == Some(&"my_app")
                && c.get(2) == Some(&"crates/app")
        }),
        "the import-name -> directory row:\n{text}"
    );
}

#[test]
fn resolving_a_bare_file_lists_its_definitions() {
    let dir = repo();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["resolve", "a.py"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let c = columns(text.lines().next().expect("a record"));
    assert_eq!((c[0], c[1]), ("ref", "a.a"), "{text}");
    assert!(
        c.last().is_some_and(|l| l.starts_with("a.py:")),
        "rows are pasteable:\n{text}"
    );
}

#[test]
fn a_fresh_checkout_seeds_churn_from_git_history() {
    // repo(): two commits, one file each. No grid-recorded legs exist, so
    // the ranking would otherwise be complexity-only with every row legs 0.
    let dir = repo();
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["hotspots", "--limit", "10"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.lines()
            .any(|l| columns(l).windows(2).any(|w| w == ["legs", "1"])),
        "file-level git churn seeds the ranking:\n{text}"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("git file history"),
        "the churn source is named:\n{err}"
    );
}
