//! `grid suspects --closure` end to end: attribution with no run history.
//!
//! Every fixture is a real repository surveyed by the real frontend, so the
//! closure walked here is the one the language's own binding rules
//! establish. The point of the verb is the case where nothing was retained —
//! the test passed at the last snapshot and fails now — so none of these
//! fixtures seeds a trace or a verdict.
use std::collections::BTreeSet;
use std::fmt::Write as _;
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

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(full, text).expect("write");
    }
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
    String::from_utf8(out.stderr.clone()).expect("stderr is utf-8")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// A repo with two surveyed snapshots: `before` committed and charted, then
/// `after` committed and charted. Returns the directory and the base rev,
/// which is what `--closure` is pointed at.
fn surveyed(before: &[(&str, &str)], after: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), before);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "before"]);
    let base = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf-8")
    .trim()
    .to_owned();
    let surveyed = grid(dir.path(), &["survey"]);
    assert_eq!(code(&surveyed), 0, "{}", stderr(&surveyed));
    write(dir.path(), after);
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "after"],
    );
    let surveyed = grid(dir.path(), &["survey"]);
    assert_eq!(code(&surveyed), 0, "{}", stderr(&surveyed));
    (dir, base)
}

/// The `(datum, via)` pairs of every ranked `hit` record.
fn hits(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .filter(|line| line.starts_with("hit"))
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            columns.next()?;
            let datum = columns.next()?.to_owned();
            let mode = columns.next()?;
            assert_eq!(mode, "closure", "closure mode names its evidence: {line}");
            Some((datum, columns.collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(out)).expect("--json is a Suspects")
}

/// Ground truth: the culprit is two hops down the test's closure, so it is
/// listed with its distance; a definition that changed just as much but sits
/// outside the closure is not listed at all.
#[test]
fn a_changed_definition_two_hops_down_is_ranked_and_one_outside_is_not() {
    let (dir, base) = surveyed(
        &[(
            "crates/app/src/lib.rs",
            "pub fn culprit() -> u32 { 1 }\n\
             pub fn middle() -> u32 { culprit() }\n\
             pub fn the_test() -> u32 { middle() }\n\
             pub fn outsider() -> u32 { 9 }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub fn culprit() -> u32 { 2 }\n\
             pub fn middle() -> u32 { culprit() }\n\
             pub fn the_test() -> u32 { middle() }\n\
             pub fn outsider() -> u32 { 8 }\n",
        )],
    );

    let out = grid(
        dir.path(),
        &["suspects", "app.the_test", "--closure", &base],
    );
    assert_eq!(
        code(&out),
        0,
        "attribution is heuristic and never gates:\n{}",
        stderr(&out)
    );
    let found = hits(&out);
    assert_eq!(
        found.len(),
        1,
        "one changed definition is reachable: {found:?}\n{}",
        stdout(&out)
    );
    assert_eq!(
        found[0].1, "via 2 hops, datum moved",
        "the culprit is two hops from the test: {found:?}"
    );

    // The datum is the culprit's, at head, and it is not the outsider's.
    let resolved = grid(dir.path(), &["resolve", "app.culprit", "--json"]);
    let culprit: serde_json::Value = serde_json::from_str(&stdout(&resolved)).expect("resolve");
    let culprit = culprit[0]["datum"].as_str().expect("datum").to_owned();
    assert_eq!(found[0].0, culprit, "the row names the culprit");

    let closing = stdout(&out);
    assert!(
        closing.lines().any(|l| l.starts_with("miss")),
        "the answer closes by saying what it could not follow:\n{closing}"
    );
}

/// Differential: the candidate set is exactly `grid scope` at full depth
/// intersected with `grid diff`, both computed here from the verbs' own
/// output rather than from the code under test.
#[test]
fn candidates_equal_an_independent_scope_intersect_diff() {
    let (dir, base) = surveyed(
        &[(
            "crates/app/src/lib.rs",
            "pub fn leaf_a() -> u32 { 1 }\n\
             pub fn leaf_b() -> u32 { 2 }\n\
             pub fn middle() -> u32 { leaf_a() + leaf_b() }\n\
             pub fn the_test() -> u32 { middle() }\n\
             pub fn apart() -> u32 { 7 }\n\
             pub fn apart_two() -> u32 { apart() }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub fn leaf_a() -> u32 { 11 }\n\
             pub fn leaf_b() -> u32 { 2 }\n\
             pub fn leaf_new() -> u32 { 3 }\n\
             pub fn middle() -> u32 { leaf_a() + leaf_b() + leaf_new() }\n\
             pub fn the_test() -> u32 { middle() }\n\
             pub fn apart() -> u32 { 77 }\n\
             pub fn apart_two() -> u32 { apart() }\n",
        )],
    );

    // Independent side one: every definition the test's pack reaches, at a
    // depth past the fixture's diameter, as a datum -> callsign map.
    let scoped = grid(
        dir.path(),
        &["scope", "app.the_test", "--depth", "9", "--json"],
    );
    assert_eq!(code(&scoped), 0, "{}", stderr(&scoped));
    let pack: serde_json::Value = serde_json::from_str(&stdout(&scoped)).expect("ScopePack");
    let names: std::collections::BTreeMap<String, String> = pack["defs"]
        .as_array()
        .expect("defs")
        .iter()
        .map(|d| {
            (
                d["datum"].as_str().expect("datum").to_owned(),
                d["callsign"].as_str().expect("callsign").to_owned(),
            )
        })
        .collect();

    // Independent side two: every definition head holds that base did not —
    // one whose body moved, and one that arrived — from the delta verb.
    let delta = grid(dir.path(), &["diff", &base, "HEAD", "--json"]);
    assert_eq!(code(&delta), 0, "{}", stderr(&delta));
    let delta: serde_json::Value = serde_json::from_str(&stdout(&delta)).expect("Diff");
    let moved: BTreeSet<String> = delta["defs"]
        .as_array()
        .expect("defs")
        .iter()
        .filter(|d| d["action"] == "modified" || d["action"] == "added")
        .map(|d| d["callsign"].as_str().expect("callsign").to_owned())
        .collect();

    let expected: BTreeSet<String> = names
        .values()
        .filter(|c| moved.contains(*c))
        .cloned()
        .collect();
    assert!(
        expected.contains("app.leaf_a")
            && expected.contains("app.middle")
            && expected.contains("app.leaf_new"),
        "the fixture is meant to move two reachable definitions and add one: {expected:?}"
    );
    assert!(
        !expected.contains("app.apart"),
        "and one unreachable one: {expected:?}"
    );

    let out = grid(
        dir.path(),
        &["suspects", "app.the_test", "--closure", &base, "--json"],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let actual: BTreeSet<String> = json(&out)["ranked"]
        .as_array()
        .expect("ranked")
        .iter()
        .map(|item| {
            let datum = item["datum"].as_str().expect("datum");
            names
                .get(datum)
                .unwrap_or_else(|| panic!("{datum} is outside the pack"))
                .clone()
        })
        .collect();
    assert_eq!(
        actual, expected,
        "the candidate set is scope ∩ diff, computed two ways"
    );
}

/// Conservatism: a `since` the chain never held is a typed refusal. An empty
/// ranking would read as "nothing in the closure moved", which is the
/// opposite of "I have never held that snapshot".
#[test]
fn an_unheld_since_refuses_rather_than_ranking_nothing() {
    let (dir, base) = surveyed(
        &[(
            "crates/app/src/lib.rs",
            "pub fn culprit() -> u32 { 1 }\n\
             pub fn the_test() -> u32 { culprit() }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub fn culprit() -> u32 { 2 }\n\
             pub fn the_test() -> u32 { culprit() }\n",
        )],
    );
    // A third commit nobody surveyed: a real rev, and a snapshot the chain
    // has never held.
    write(
        dir.path(),
        &[(
            "crates/app/src/lib.rs",
            "pub fn culprit() -> u32 { 3 }\n\
             pub fn the_test() -> u32 { culprit() }\n",
        )],
    );
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "unsurveyed"]);

    let out = grid(
        dir.path(),
        &["suspects", "app.the_test", "--closure", "HEAD"],
    );
    assert_eq!(
        code(&out),
        2,
        "an unheld snapshot is bad input:\n{}",
        stderr(&out)
    );
    assert_eq!(
        stdout(&out),
        "",
        "a refusal writes no records at all: {:?}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("no snapshot of the tree at HEAD"),
        "the refusal says which coordinate it could not answer: {}",
        stderr(&out)
    );
    // T-10: and the command that would produce it, in the tree to run it in.
    assert!(
        stderr(&out).contains("hint: run `grid survey HEAD` in ."),
        "the refusal names the command and the tree: {}",
        stderr(&out)
    );

    // And the same verb against the snapshot the chain does hold answers.
    let held = grid(
        dir.path(),
        &["suspects", "app.the_test", "--closure", &base],
    );
    assert_eq!(code(&held), 0, "{}", stderr(&held));
    assert_eq!(hits(&held).len(), 1, "{}", stdout(&held));
}

/// Conservatism: a closure that runs past its fuel refuses the whole answer.
/// A truncated closure would list fewer candidates and read as a cleaner
/// result, which is exactly the confusion C4 forbids.
///
/// The fixture is generated because the budget is a real one: `grid_cache`'s
/// `FUEL` is a hundred thousand edges, so the chart has to carry more.
#[test]
fn a_closure_over_fuel_refuses_the_whole_answer() {
    // 330 definitions, each calling all 330: every one of them is reachable
    // from the test and expands 330 edges, so the walk examines 108_900 —
    // comfortably past FUEL, and one pass for the survey.
    const N: usize = 330;
    let mut source = String::with_capacity(N * N * 12);
    for i in 0..N {
        let _ = write!(source, "pub fn f{i:03}() -> u32 {{ 0");
        for j in 0..N {
            let _ = write!(source, " + f{j:03}()");
        }
        source.push_str(" }\n");
    }
    source.push_str("pub fn the_test() -> u32 { f000() }\n");

    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), &[("crates/app/src/lib.rs", source.as_str())]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "wide"]);
    let base = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(dir.path())
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf-8")
    .trim()
    .to_owned();
    let surveyed = grid(dir.path(), &["survey"]);
    assert_eq!(code(&surveyed), 0, "{}", stderr(&surveyed));

    let out = grid(
        dir.path(),
        &["suspects", "app.the_test", "--closure", &base],
    );
    assert_eq!(
        code(&out),
        1,
        "a closure over fuel is refused whole:\n{}",
        stderr(&out)
    );
    assert_eq!(
        stdout(&out),
        "",
        "and serves no partial ranking: {:?}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("fuel"),
        "the refusal names the budget it ran past: {}",
        stderr(&out)
    );
}

/// The live snapshot id, as `grid survey --json` reports it.
fn snapshot_id(dir: &Path) -> String {
    let out = grid(dir, &["survey", "--json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("survey --json");
    value["snapshot_id"]
        .as_str()
        .expect("a snapshot id")
        .to_owned()
}

/// A definition's datum, as `grid resolve --json` reports it.
fn datum_of(dir: &Path, callsign: &str) -> String {
    let out = grid(dir, &["resolve", callsign, "--json"]);
    let refs: Vec<serde_json::Value> = serde_json::from_str(&stdout(&out)).expect("resolve --json");
    refs.first().expect("one definition")["datum"]
        .as_str()
        .expect("a datum")
        .to_owned()
}

/// Retain a trace of `test` taken against `snapshot`, and record a passing
/// verdict that names it — the join the default `--closure` walks back.
fn seed_pass(dir: &Path, test: &str, reached: &str, snapshot: &str, trace_id: &str) {
    let trace = serde_json::json!({
        "trace_id": trace_id,
        "git_rev": "HEAD",
        "command": test,
        "snapshot": snapshot,
        "observations": [{ "caller": { "symbol": test }, "callee": { "symbol": reached } }]
    });
    std::fs::write(dir.join("trace.json"), trace.to_string()).expect("write trace");
    let out = grid(dir, &["trace", "ingest", "trace.json"]);
    assert_eq!(code(&out), 0, "ingest: {}", stderr(&out));
    std::fs::remove_file(dir.join("trace.json")).expect("rm trace");
    let out = grid(
        dir,
        &[
            "cache",
            "record",
            "test",
            "env-1",
            &format!("{}=pass", datum_of(dir, test)),
            "--trace",
            trace_id,
        ],
    );
    assert_eq!(code(&out), 0, "record: {}", stderr(&out));
}

const CLOSURE_BEFORE: &str = "pub fn culprit() -> u32 { 1 }\n\
     pub fn middle() -> u32 { culprit() }\n\
     pub fn the_test() -> u32 { middle() }\n";
const CLOSURE_AFTER: &str = "pub fn culprit() -> u32 { 2 }\n\
     pub fn middle() -> u32 { culprit() }\n\
     pub fn the_test() -> u32 { middle() }\n";

/// A repo surveyed at `before` with a passing verdict recorded against that
/// snapshot, then edited and surveyed again. Returns the directory and the
/// snapshot the pass was recorded at.
fn passed_then_edited() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), &[("crates/app/src/lib.rs", CLOSURE_BEFORE)]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "before"]);
    let green = snapshot_id(dir.path());
    seed_pass(
        dir.path(),
        "app.the_test",
        "app.middle",
        &green,
        "trace:green",
    );
    write(dir.path(), &[("crates/app/src/lib.rs", CLOSURE_AFTER)]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "after"]);
    let out = grid(dir.path(), &["survey"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    (dir, green)
}

/// T-09, ground truth: a bare `--closure` answers the same question the
/// snapshot the test last passed at answers, and says which one it used.
#[test]
fn a_bare_closure_uses_the_last_snapshot_the_test_passed_at() {
    let (dir, green) = passed_then_edited();

    let named = grid(
        dir.path(),
        &["suspects", "app.the_test", "--closure", &green],
    );
    let bare = grid(dir.path(), &["suspects", "app.the_test", "--closure"]);
    assert_eq!(code(&bare), 0, "attribution never gates: {}", stderr(&bare));
    assert_eq!(
        stdout(&bare),
        stdout(&named),
        "the default is the snapshot, not a different question"
    );
    assert!(
        !hits(&bare).is_empty(),
        "an empty ranking would prove nothing: {}",
        stdout(&bare)
    );
    assert!(
        stderr(&bare).contains(&green) && stderr(&bare).contains("recorded a pass at"),
        "the snapshot it used is part of the answer: {}",
        stderr(&bare)
    );
    // And the flag still takes an explicit snapshot when one follows it.
    assert!(
        !stderr(&named).contains("recorded a pass at"),
        "a named snapshot needs no default: {}",
        stderr(&named)
    );
}

/// T-09, conservatism: with no passing verdict on record the bare flag
/// refuses. Falling back to head — the tempting default, since head is
/// always there — would rank nothing, and an empty ranking reads as
/// "nothing in the closure moved", which is the opposite claim.
#[test]
fn a_bare_closure_with_no_recorded_pass_refuses_rather_than_ranking_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), &[("crates/app/src/lib.rs", CLOSURE_BEFORE)]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "before"]);
    let out = grid(dir.path(), &["survey"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));

    let bare = grid(dir.path(), &["suspects", "app.the_test", "--closure"]);
    assert_eq!(
        code(&bare),
        2,
        "no snapshot on record is bad input, not an empty answer: {}\n{}",
        stdout(&bare),
        stderr(&bare)
    );
    assert!(
        stdout(&bare).is_empty(),
        "a refusal prints no records: {}",
        stdout(&bare)
    );
    assert!(
        stderr(&bare).contains("--closure <snapshot|rev>"),
        "the refusal names the way out: {}",
        stderr(&bare)
    );
}
