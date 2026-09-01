//! `grid drift` end to end: the dependents a changed interface left behind,
//! and — the half that decides whether the verb is usable — everything it
//! declines to claim.
//!
//! The ground-truth fixtures are real repositories surveyed by the real
//! frontends, so every edge here is one the language's own binding rules
//! establish. A fabricated segment could assert a cross-language proven edge;
//! the survey cannot produce one, and a fixture that pretended otherwise
//! would be testing the test.
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

/// A repo with two commits: `before` then `after`.
fn repo(before: &[(&str, &str)], after: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), before);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "before"]);
    write(dir.path(), after);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "after"]);
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

/// The `(dependent, changed)` callsign pairs of every `drift` record.
fn suspects(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&"drift"))
        .filter_map(|c| Some((c.get(2)?.to_string(), c.get(4)?.to_string())))
        .collect()
}

fn report(out: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(out)).expect("--json is a DriftReport")
}

/// Ground truth, Rust: a struct's serde attribute is the interface, and a
/// caller that did not change is a suspect while one that did is not.
#[test]
fn a_changed_serde_attribute_implicates_only_the_untouched_caller() {
    let dir = repo(
        &[(
            "crates/app/src/lib.rs",
            "pub struct Tag { pub name: u32 }\n\
             pub fn untouched() -> Tag { Tag { name: 1 } }\n\
             pub fn edited() -> Tag { Tag { name: 2 } }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub struct Tag {\n    #[serde(rename = \"label\")]\n    pub name: u32,\n}\n\
             pub fn untouched() -> Tag { Tag { name: 1 } }\n\
             pub fn edited() -> Tag { Tag { name: 3 } }\n",
        )],
    );
    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    assert_eq!(
        code(&out),
        3,
        "suspects exist, so drift diverges:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let found = suspects(&out);
    assert!(
        found.contains(&("app.untouched".to_owned(), "app.Tag".to_owned())),
        "the caller nobody touched is a suspect: {found:?}"
    );
    assert!(
        !found.iter().any(|(dep, _)| dep == "app.edited"),
        "a caller that changed is assumed handled: {found:?}"
    );
}

/// Ground truth, Python: a changed default is an interface change (every
/// caller that omits the argument silently gets a different value), and a
/// body-only edit next door is not.
#[test]
fn a_changed_python_default_implicates_its_caller_and_a_body_edit_does_not() {
    let dir = repo(
        &[
            ("pkg/__init__.py", ""),
            ("pkg/api.py", "def window(retries=3):\n    return retries\n"),
            ("pkg/body.py", "def helper():\n    return 1\n"),
            (
                "pkg/use.py",
                "from pkg.api import window\nfrom pkg.body import helper\n\n\n\
                 def calls_api():\n    return window()\n\n\n\
                 def calls_body():\n    return helper()\n",
            ),
        ],
        &[
            ("pkg/api.py", "def window(retries=5):\n    return retries\n"),
            ("pkg/body.py", "def helper():\n    return 2\n"),
        ],
    );
    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    assert_eq!(
        code(&out),
        3,
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let found = suspects(&out);
    assert!(
        found.contains(&("pkg.use.calls_api".to_owned(), "pkg.api.window".to_owned())),
        "a changed default is visible to every caller that omits it: {found:?}"
    );
    assert!(
        !found
            .iter()
            .any(|(_, changed)| changed == "pkg.body.helper"),
        "a body-only edit implicates nobody: {found:?}"
    );
}

/// The identity property: nothing drifted between a snapshot and itself.
#[test]
fn drift_of_a_snapshot_against_itself_is_empty() {
    let dir = repo(
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u32) -> u32 { x }\npub fn caller() -> u32 { callee(1) }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u64) -> u64 { x }\npub fn caller() -> u64 { callee(1) }\n",
        )],
    );
    // Survey first so both endpoints name the same published snapshot.
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let current = grid(dir.path(), &["version", "--json"]);
    let snap = report(&current)["snapshot"]
        .as_str()
        .expect("a published snapshot")
        .to_owned();

    let out = grid(dir.path(), &["drift", &snap, &snap]);
    assert_eq!(
        code(&out),
        0,
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(suspects(&out).is_empty(), "{}", stdout(&out));
}

/// Determinism: the same snapshot pair is the same bytes, every run.
#[test]
fn drift_is_deterministic_for_a_snapshot_pair() {
    let dir = repo(
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u32) -> u32 { x }\n\
             pub fn a() -> u32 { callee(1) }\n\
             pub fn b() -> u32 { callee(2) }\n\
             pub fn c() -> u32 { callee(3) }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u64) -> u64 { x }\n\
             pub fn a() -> u32 { callee(1) }\n\
             pub fn b() -> u32 { callee(2) }\n\
             pub fn c() -> u32 { callee(3) }\n",
        )],
    );
    let first = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--json"]);
    let again = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--json"]);
    assert_eq!(
        stdout(&first),
        stdout(&again),
        "a snapshot pair is one answer, byte for byte"
    );
}

/// Track suppression is monotone: naming a promise can only remove suspects.
/// A track that *added* one would make the gate and drift disagree about who
/// is responsible for a definition.
#[test]
fn a_track_never_adds_a_suspect() {
    let dir = repo(
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u32) -> u32 { x }\n\
             pub fn kept() -> u32 { callee(1) }\n\
             pub fn promised() -> u32 { callee(2) }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u64) -> u64 { x }\n\
             pub fn kept() -> u32 { callee(1) }\n\
             pub fn promised() -> u32 { callee(2) }\n",
        )],
    );
    let bare = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    let before: Vec<_> = suspects(&bare);
    assert!(before.len() >= 2, "two callers to start with: {before:?}");

    let track = serde_json::json!({
        "track_id": "track:drift",
        "base": { "git_rev": "HEAD~1" },
        "delta": {
            "defs": [{ "action": "modify", "callsign": "app.promised" }],
            "edges": []
        }
    });
    let path = dir.path().join("track.json");
    std::fs::write(&path, track.to_string()).expect("write track");

    let tracked = grid(
        dir.path(),
        &["drift", "HEAD~1", "HEAD", "--track", "track.json"],
    );
    let after = suspects(&tracked);
    assert!(
        after.iter().all(|s| before.contains(s)),
        "a track can only remove: {before:?} -> {after:?}"
    );
    assert!(
        !after.iter().any(|(dep, _)| dep == "app.promised"),
        "the promised dependent is the gate's problem now: {after:?}"
    );
    assert!(
        after.iter().any(|(dep, _)| dep == "app.kept"),
        "an unpromised dependent stays a suspect: {after:?}"
    );
}

/// har-verify rung 5: the segment walk against an independent recomputation
/// from the wire `MadeGood`. The reimplementation shares no code with the
/// engine's — it reads `grid diff --json` and `grid uses --json` and joins
/// them itself — so agreement is evidence, not a tautology.
#[test]
fn drift_agrees_with_an_independent_recomputation_from_the_wire() {
    let dir = repo(
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u32) -> u32 { x }\n\
             pub fn body_only(x: u32) -> u32 { x }\n\
             pub fn a() -> u32 { callee(1) }\n\
             pub fn b() -> u32 { body_only(2) }\n",
        )],
        &[(
            "crates/app/src/lib.rs",
            "pub fn callee(x: u64) -> u64 { x }\n\
             pub fn body_only(x: u32) -> u32 { x + 1 }\n\
             pub fn a() -> u32 { callee(1) }\n\
             pub fn b() -> u32 { body_only(2) }\n",
        )],
    );
    let engine = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--json"]);
    let mine = recompute(dir.path(), "crates/app/src/lib.rs");
    let mut theirs: Vec<(String, String)> = report(&engine)["suspects"]
        .as_array()
        .expect("suspects array")
        .iter()
        .map(|s| {
            (
                s["dependent"]["callsign"].as_str().unwrap_or("").to_owned(),
                s["changed"]["callsign"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    theirs.sort();
    assert_eq!(mine, theirs, "the two computations must agree");
    assert!(!mine.is_empty(), "a vacuous agreement proves nothing");
}

/// Drift, recomputed from the wire by hand: the changed set from `grid diff
/// --json`, the dependent edges from `grid uses --json`, the antijoin against
/// the changed callsigns done here. Deliberately naive, and deliberately a
/// different formulation — it reads the *head* chart's inbound edges rather
/// than walking base's segments. The two coincide by construction: a suspect
/// is a definition whose datum did not move, so the callsign-space edge from
/// it to the changed definition is present on both sides, and a dependent
/// that did move is excluded either way.
fn recompute(dir: &Path, file: &str) -> Vec<(String, String)> {
    let delta = report(&grid(dir, &["diff", "HEAD~1", "HEAD", "--json"]));
    let defs = delta["defs"].as_array().expect("defs").clone();
    let touched: Vec<&str> = defs.iter().filter_map(|d| d["callsign"].as_str()).collect();
    let changed: Vec<&str> = defs
        .iter()
        .filter(|d| {
            let action = d["action"].as_str().unwrap_or("");
            action == "removed"
                || (action == "modified" && d["sig_changed"].as_bool() != Some(false))
        })
        .filter_map(|d| d["callsign"].as_str())
        .collect();

    // Datum -> callsign, straight off `grid resolve <file>`: the wire's own
    // answer to "what is in here", not a second index.
    let listing = grid(dir, &["resolve", file, "--json"]);
    let names: Vec<serde_json::Value> =
        serde_json::from_str(&stdout(&listing)).expect("resolve --json is a DefRef list");
    let callsign_of = |datum: &str| -> Option<String> {
        names.iter().find(|d| d["datum"].as_str() == Some(datum))?["callsign"]
            .as_str()
            .map(str::to_owned)
    };

    let mut out = Vec::new();
    for name in changed {
        let uses = grid(dir, &["uses", name, "--json"]);
        let edges: Vec<serde_json::Value> =
            serde_json::from_str(&stdout(&uses)).unwrap_or_default();
        for edge in edges {
            if !matches!(
                edge["relation"].as_str(),
                Some("calls" | "constructs" | "implements" | "writes_field")
            ) || edge["zoc"].as_str() != Some("proven")
            {
                continue;
            }
            let Some(dependent) = edge["src"].as_str().and_then(callsign_of) else {
                continue;
            };
            if touched.contains(&dependent.as_str()) {
                continue;
            }
            out.push((dependent, name.to_owned()));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Conservatism: a dependent reachable only through a glob import is not a
/// suspect — the survey never proved that edge — and the reference it
/// declined is *counted*, so the report says how much it could not see.
/// Silence here would read as "nothing depends on this".
#[test]
fn a_glob_import_dependent_is_declined_and_counted_not_claimed() {
    let dir = repo(
        &[
            (
                "crates/app/src/lib.rs",
                "mod other;\nuse crate::other::*;\npub fn f() -> u32 { mystery(1) }\n",
            ),
            (
                "crates/app/src/other.rs",
                "pub fn mystery(x: u32) -> u32 { x }\n",
            ),
        ],
        &[(
            "crates/app/src/other.rs",
            "pub fn mystery(x: u64) -> u64 { x }\n",
        )],
    );
    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    assert_eq!(
        code(&out),
        0,
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let found = suspects(&out);
    assert!(
        found.is_empty(),
        "a glob-supplied name was never a proven edge: {found:?}"
    );
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            c.first() == Some(&"miss")
                && c.get(1)
                    .and_then(|n| n.parse::<u64>().ok())
                    .is_some_and(|n| n > 0)
                && l.contains("unresolved references")
        }),
        "the declined reference is counted, not silent:\n{text}"
    );
}

/// Conservatism: a head nobody re-surveyed cannot report a declined count,
/// and reports it as unknown rather than as zero. Zero would claim the
/// report saw everything.
#[test]
fn an_unsurveyed_head_reports_its_count_as_unknown_not_zero() {
    let dir = repo(
        &[("pkg/a.py", "def a():\n    pass\n")],
        &[("pkg/b.py", "def b():\n    pass\n")],
    );
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let snap = report(&grid(dir.path(), &["version", "--json"]))["snapshot"]
        .as_str()
        .expect("a published snapshot")
        .to_owned();
    let out = grid(dir.path(), &["drift", &snap, &snap]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    assert!(
        text.contains("miss") && text.contains("uncounted"),
        "an uncounted refusal still prints:\n{text}"
    );
    let json = report(&grid(dir.path(), &["drift", &snap, &snap, "--json"]));
    assert!(
        json["unchecked"].get("unresolved_refs").is_none(),
        "unknown is absent on the wire, never zero: {json}"
    );
}
