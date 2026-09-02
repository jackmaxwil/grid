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
    // `--allow-empty`: a fixture whose second commit changes nothing is a
    // real case (the covenant tests declare and then edit afterwards), and
    // git refuses an empty commit by default.
    git(
        dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "after"],
    );
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

/// Plan suppression is monotone: naming a promise can only remove suspects.
/// A plan that *added* one would make the gate and drift disagree about who
/// is responsible for a definition.
#[test]
fn a_plan_never_adds_a_suspect() {
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

    let plan = serde_json::json!({
        "plan_id": "plan:drift",
        "base": { "git_rev": "HEAD~1" },
        "delta": {
            "defs": [{ "action": "modify", "callsign": "app.promised" }],
            "edges": []
        }
    });
    let path = dir.path().join("plan.json");
    std::fs::write(&path, plan.to_string()).expect("write plan");

    let planned = grid(
        dir.path(),
        &["drift", "HEAD~1", "HEAD", "--plan", "plan.json"],
    );
    let after = suspects(&planned);
    assert!(
        after.iter().all(|s| before.contains(s)),
        "a plan can only remove: {before:?} -> {after:?}"
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
/// from the wire `Diff`. The reimplementation shares no code with the
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
            ) || edge["grade"].as_str() != Some("proven")
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

/// The `(dependent, changed, grade, reason?)` of every `drift` record — the
/// evidence column matters once observations join the walk.
fn graded(out: &Output) -> Vec<(String, String, String, Option<String>)> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&"drift"))
        .filter_map(|c| {
            Some((
                c.get(2)?.to_string(),
                c.get(4)?.to_string(),
                c.get(1)?.to_string(),
                c.get(6).filter(|r| **r != "-").map(|r| (*r).to_string()),
            ))
        })
        .collect()
}

/// Commit whatever is on disk under `message`.
fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", message]);
}

/// A trace asserting one call, by symbol, so it resolves against whatever the
/// live chart currently is.
fn ingest_wake(dir: &Path, id: &str, from: &str, to: &str) {
    let trace = serde_json::json!({
        "trace_id": id,
        "git_rev": "HEAD",
        // `command` is the test the trace came from — the same field
        // `grid trace ingest --lcov ... --test <name>` fills in, and the one
        // that makes a retained trace a test-registry entry.
        "command": from,
        "observations": [{
            "caller": { "symbol": from },
            "callee": { "symbol": to }
        }]
    });
    let path = dir.join("trace.json");
    std::fs::write(&path, trace.to_string()).expect("write trace");
    let out = grid(dir, &["trace", "ingest", "trace.json"]);
    assert_eq!(
        code(&out),
        0,
        "ingest failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout(&out).contains("1/1"),
        "the hop must resolve, or the fixture proves nothing: {}",
        stdout(&out)
    );
}

/// Ground truth for step 3: coupling only a trace knows about is a suspect
/// once the floor admits observations — and never at the proven default,
/// because a trace is not a proof.
#[test]
fn observed_coupling_implicates_a_dependent_only_at_or_below_the_observed_floor() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/api.py", "def window(retries=3):\n    return retries\n"),
            // Reaches `window` through a name the resolver cannot bind, so
            // the only evidence of the coupling is the trace.
            (
                "pkg/caller.py",
                "import importlib\n\n\ndef go():\n    return importlib.import_module(\"pkg.api\").window()\n",
            ),
        ],
    );
    commit(dir.path(), "before");
    // Survey, then trace: the trace resolves against the base chart.
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    ingest_wake(dir.path(), "trace:one", "caller.go", "api.window");

    write(
        dir.path(),
        &[("pkg/api.py", "def window(retries=5):\n    return retries\n")],
    );
    commit(dir.path(), "after");

    let proven = grid(
        dir.path(),
        &["drift", "HEAD~1", "HEAD", "--floor", "proven"],
    );
    assert_eq!(
        code(&proven),
        0,
        "a trace is not a proof: {}",
        stdout(&proven)
    );
    assert!(suspects(&proven).is_empty(), "{}", stdout(&proven));

    // `observed` is the default floor, so the everyday verb sees this.
    let observed = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    assert_eq!(
        code(&observed),
        3,
        "stderr:\n{}",
        String::from_utf8_lossy(&observed.stderr)
    );
    let rows = graded(&observed);
    assert_eq!(
        rows,
        vec![(
            "pkg.caller.go".to_owned(),
            "pkg.api.window".to_owned(),
            "observed".to_owned(),
            None,
        )],
        "the traced caller is a suspect at its honest grade"
    );
}

/// Ground truth for the decay half: once the traced *caller* has itself been
/// rewritten, the trace describes code the base chart does not hold. The edge
/// drops below the observed floor and appears only at `heuristic`, carrying
/// `stale-trace` — and re-tracing restores it.
#[test]
fn a_decayed_trace_falls_to_heuristic_with_a_named_reason_and_recovers() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/api.py", "def window(retries=3):\n    return retries\n"),
            (
                "pkg/caller.py",
                "import importlib\n\n\ndef go():\n    return importlib.import_module(\"pkg.api\").window()\n",
            ),
        ],
    );
    commit(dir.path(), "one");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    ingest_wake(dir.path(), "trace:one", "caller.go", "api.window");

    // The traced caller is rewritten: its datum moves, so the retained hop
    // now names a definition the next base does not intern.
    write(
        dir.path(),
        &[(
            "pkg/caller.py",
            "import importlib\n\n\ndef go():\n    return importlib.import_module(\"pkg.api\").window() + 1\n",
        )],
    );
    commit(dir.path(), "two");
    write(
        dir.path(),
        &[("pkg/api.py", "def window(retries=5):\n    return retries\n")],
    );
    commit(dir.path(), "three");

    let observed = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    assert_eq!(
        code(&observed),
        0,
        "a decayed trace does not gate: {}",
        stdout(&observed)
    );
    assert!(
        suspects(&observed).is_empty(),
        "a stale trace is below the observed floor:\n{}",
        stdout(&observed)
    );

    let weak = grid(
        dir.path(),
        &["drift", "HEAD~1", "HEAD", "--floor", "heuristic"],
    );
    let rows = graded(&weak);
    assert_eq!(rows.len(), 1, "{}", stdout(&weak));
    assert_eq!(rows[0].1, "pkg.api.window");
    assert_eq!(rows[0].2, "heuristic", "demoted, not silently observed");
    assert_eq!(
        rows[0].3.as_deref(),
        Some("stale-trace"),
        "the demotion says why: {}",
        stdout(&weak)
    );

    // Re-tracing against the current code restores the observation: the
    // retained log is append-only, and the fresh trace is simply believed.
    assert_eq!(code(&grid(dir.path(), &["survey", "HEAD~1"])), 0);
    ingest_wake(dir.path(), "trace:two", "caller.go", "api.window");
    let again = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    let rows = graded(&again);
    assert_eq!(
        rows.iter().map(|r| r.2.as_str()).collect::<Vec<_>>(),
        ["observed"],
        "a fresh trace is an observation again:\n{}",
        stdout(&again)
    );
}

/// Declare a covenant through the verb, asserting it took.
fn covenant(dir: &Path, a: &str, b: &str, via: &str) -> Output {
    grid(dir, &["covenant", a, b, "--via", via])
}

/// Ground truth for step 4: a declared coupling fires on *datum* change, and
/// the partner that held is a suspect at `modeled` — which gates, because a
/// human declared the pair.
#[test]
fn a_covenant_implicates_the_side_that_held() {
    let dir = repo(
        &[
            ("pkg/__init__.py", ""),
            // No language rule couples these two. A human knows they are one
            // contract; the covenant is where that is written down.
            (
                "pkg/writer.py",
                "def encode(tag):\n    return {\"name\": tag}\n",
            ),
            (
                "pkg/reader.py",
                "def decode(blob):\n    return blob[\"name\"]\n",
            ),
        ],
        &[(
            "pkg/writer.py",
            "def encode(tag):\n    return {\"label\": tag}\n",
        )],
    );
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let bound = covenant(
        dir.path(),
        "pkg.writer.encode",
        "pkg.reader.decode",
        "schema",
    );
    assert_eq!(
        code(&bound),
        0,
        "stderr:\n{}",
        String::from_utf8_lossy(&bound.stderr)
    );

    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    assert_eq!(
        code(&out),
        3,
        "a declared coupling gates:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows = graded(&out);
    assert_eq!(
        rows,
        vec![(
            "pkg.reader.decode".to_owned(),
            "pkg.writer.encode".to_owned(),
            "modeled".to_owned(),
            Some("covenant:schema".to_owned()),
        )],
        "the side that held is the suspect, at modeled, saying why"
    );
}

/// Both sides moving is a covenant *kept* — which is the whole reason to
/// declare one — so it fires nothing.
#[test]
fn a_covenant_both_sides_honoured_is_silent() {
    let dir = repo(
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/writer.py",
                "def encode(tag):\n    return {\"name\": tag}\n",
            ),
            (
                "pkg/reader.py",
                "def decode(blob):\n    return blob[\"name\"]\n",
            ),
        ],
        &[
            (
                "pkg/writer.py",
                "def encode(tag):\n    return {\"label\": tag}\n",
            ),
            (
                "pkg/reader.py",
                "def decode(blob):\n    return blob[\"label\"]\n",
            ),
        ],
    );
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    assert_eq!(
        code(&covenant(
            dir.path(),
            "pkg.writer.encode",
            "pkg.reader.decode",
            "schema"
        )),
        0
    );
    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    assert_eq!(code(&out), 0, "{}", stdout(&out));
    assert!(suspects(&out).is_empty(), "{}", stdout(&out));
}

/// A covenant is a symmetric declaration: which side is written first must
/// not change the answer.
#[test]
fn a_covenant_is_symmetric() {
    let files = |writer: &str| {
        vec![
            ("pkg/__init__.py".to_owned(), String::new()),
            ("pkg/writer.py".to_owned(), writer.to_owned()),
            (
                "pkg/reader.py".to_owned(),
                "def decode(blob):\n    return blob[\"name\"]\n".to_owned(),
            ),
        ]
    };
    let run = |a: &str, b: &str| {
        let before = files("def encode(tag):\n    return {\"name\": tag}\n");
        let after = vec![(
            "pkg/writer.py".to_owned(),
            "def encode(tag):\n    return {\"label\": tag}\n".to_owned(),
        )];
        let as_refs = |v: &Vec<(String, String)>| -> Vec<(String, String)> { v.clone() };
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        for (path, text) in as_refs(&before) {
            write(dir.path(), &[(path.as_str(), text.as_str())]);
        }
        commit(dir.path(), "before");
        for (path, text) in as_refs(&after) {
            write(dir.path(), &[(path.as_str(), text.as_str())]);
        }
        commit(dir.path(), "after");
        assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
        assert_eq!(code(&covenant(dir.path(), a, b, "schema")), 0);
        let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
        (code(&out), suspects(&out))
    };
    let forward = run("pkg.writer.encode", "pkg.reader.decode");
    let backward = run("pkg.reader.decode", "pkg.writer.encode");
    assert_eq!(forward, backward, "declaration order is not an answer");
    assert_eq!(forward.0, 3, "and the fixture must actually fire");
}

/// Conservatism: `covenant` refuses a pattern that matches nothing. A
/// declaration about code that does not exist is a rule that will never
/// fire, and writing it down silently is worse than not writing it.
#[test]
fn bind_refuses_a_pattern_that_matches_nothing() {
    let dir = repo(&[("pkg/a.py", "def a():\n    pass\n")], &[]);
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = covenant(dir.path(), "pkg.a.a", "pkg.nothing.here", "schema");
    assert_eq!(code(&out), 2, "an unresolvable pattern is bad input");
    let hints = String::from_utf8_lossy(&out.stderr);
    assert!(
        hints.contains("nothing is called") && hints.contains("closest"),
        "the refusal lists the closest charted callsigns:\n{hints}"
    );
    assert!(
        !dir.path().join(".grid/covenants.json").exists(),
        "a refused declaration is not written"
    );
}

/// Conservatism: a covenant whose pattern matches nothing *at drift time* —
/// the definition was deleted or renamed out from under the declaration — is
/// a `warn` record. A rule that quietly stops being checked is worse than no
/// rule at all.
#[test]
fn a_covenant_that_matches_nothing_at_drift_time_warns() {
    let dir = repo(
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/writer.py",
                "def encode(tag):\n    return {\"name\": tag}\n",
            ),
            (
                "pkg/reader.py",
                "def decode(blob):\n    return blob[\"name\"]\n",
            ),
        ],
        &[],
    );
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    assert_eq!(
        code(&covenant(
            dir.path(),
            "pkg.writer.encode",
            "pkg.reader.decode",
            "schema"
        )),
        0
    );
    // The reader is deleted after the declaration was made.
    std::fs::remove_file(dir.path().join("pkg/reader.py")).expect("remove");
    commit(dir.path(), "drop the reader");

    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD"]);
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            c.first() == Some(&"warn")
                && c.get(1) == Some(&"pkg.reader.decode")
                && l.contains("matches nothing")
        }),
        "a vacuous covenant is reported, never silently satisfied:\n{text}"
    );
}

/// `grid covenant --list` prints what was declared, one `covenant` record each.
#[test]
fn covenant_list_prints_the_declarations() {
    let dir = repo(
        &[
            ("pkg/__init__.py", ""),
            ("pkg/writer.py", "def encode(t):\n    return t\n"),
            ("pkg/reader.py", "def decode(b):\n    return b\n"),
        ],
        &[],
    );
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    assert_eq!(
        code(&covenant(
            dir.path(),
            "pkg.writer.encode",
            "pkg.reader.decode",
            "schema"
        )),
        0
    );
    let out = grid(dir.path(), &["covenant", "--list"]);
    assert_eq!(code(&out), 0);
    let text = stdout(&out);
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    assert_eq!(
        rows,
        vec![vec![
            "covenant",
            "pkg.writer.encode",
            "pkg.reader.decode",
            "schema"
        ]]
    );
}

/// `miss` rows from a drift report: `(subject, reason)`.
fn misses(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&"miss"))
        .filter_map(|c| Some((c.get(1)?.to_string(), c[2..].join(" "))))
        .collect()
}

/// Record a `pass` for `datum` in environment `env`, tied to `trace` — which
/// is how the verdict is attributed to the test that produced it.
fn record_pass(dir: &Path, env: &str, datum: &str, trace: &str) {
    let out = grid(
        dir,
        &[
            "cache",
            "record",
            "test",
            env,
            &format!("{datum}=pass"),
            "--trace",
            trace,
        ],
    );
    assert_eq!(
        code(&out),
        0,
        "record failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout(&out).contains("1/1"),
        "the verdict must be stored, or the fixture proves nothing: {}",
        stdout(&out)
    );
}

/// The datum a callsign currently answers to.
fn datum_of(dir: &Path, callsign: &str) -> String {
    let out = grid(dir, &["resolve", callsign, "--json"]);
    let refs: Vec<serde_json::Value> = serde_json::from_str(&stdout(&out)).expect("resolve --json");
    refs.first().expect("one definition")["datum"]
        .as_str()
        .expect("a datum")
        .to_owned()
}

/// A repo with a test that covers `pkg.lib.covered`, a trace recording that
/// coverage, and a recorded pass. Returns the directory.
fn repo_with_a_recorded_verdict() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/lib.py",
                "def covered():\n    return 1\n\n\ndef elsewhere():\n    return 1\n",
            ),
            (
                "pkg/test_lib.py",
                "from pkg.lib import covered\n\n\ndef test_covered():\n    assert covered() == 1\n",
            ),
        ],
    );
    commit(dir.path(), "before");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    ingest_wake(
        dir.path(),
        "trace:t",
        "test_lib.test_covered",
        "lib.covered",
    );
    let test_datum = datum_of(dir.path(), "pkg.test_lib.test_covered");
    record_pass(dir.path(), "env-1", &test_datum, "trace:t");
    dir
}

/// Ground truth for step 5: editing a definition inside the test's closure
/// invalidates the recorded verdict, and `drift --tests` names it.
#[test]
fn a_change_inside_a_test_closure_stales_its_verdict() {
    let dir = repo_with_a_recorded_verdict();
    write(
        dir.path(),
        &[(
            "pkg/lib.py",
            "def covered():\n    return 2\n\n\ndef elsewhere():\n    return 1\n",
        )],
    );
    commit(dir.path(), "edit inside the closure");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);

    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--tests"]);
    assert_eq!(
        code(&out),
        3,
        "verdict-staleness is identity math, so it gates:\n{}",
        stdout(&out)
    );
    assert!(
        misses(&out).contains(&(
            "test_lib.test_covered".to_owned(),
            "closure-changed".to_owned()
        )),
        "the stale test is named:\n{}",
        stdout(&out)
    );
}

/// The property the plan states: an edit outside every test closure stales
/// no test. A verb that flagged everything would be as useless as one that
/// flagged nothing.
#[test]
fn a_change_outside_every_closure_stales_nothing() {
    let dir = repo_with_a_recorded_verdict();
    write(
        dir.path(),
        &[(
            "pkg/lib.py",
            "def covered():\n    return 1\n\n\ndef elsewhere():\n    return 99\n",
        )],
    );
    commit(dir.path(), "edit outside the closure");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);

    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--tests"]);
    let stale: Vec<_> = misses(&out)
        .into_iter()
        .filter(|(_, reason)| reason.starts_with("closure-"))
        .collect();
    assert!(
        stale.is_empty(),
        "nothing in the closure moved:\n{}",
        stdout(&out)
    );
}

/// Conservatism: a verdict no retained trace claims cannot be attributed to a
/// test, so it is *counted*, not quietly treated as covering something.
#[test]
fn an_unattributed_verdict_is_counted_not_assumed() {
    let dir = repo_with_a_recorded_verdict();
    // A second verdict with no trace behind it: nobody can say which test it
    // belongs to.
    let datum = datum_of(dir.path(), "pkg.lib.covered");
    let out = grid(
        dir.path(),
        &["cache", "record", "test", "env-1", &format!("{datum}=pass")],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    // A base to diff against; the fixture's own commit is the only one.
    commit(dir.path(), "second");

    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--tests"]);
    assert_ne!(
        code(&out),
        2,
        "drift must answer: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        misses(&out)
            .iter()
            .any(|(n, reason)| n == "1" && reason == "unattributed entries"),
        "the unattributable verdict is counted:\n{}",
        stdout(&out)
    );
}

/// Conservatism: a test the head chart cannot resolve has verdicts nobody
/// can re-key. Reported as a `warn`, never counted as fresh — and never as a
/// gate, because the test being gone is not a divergence in the code.
#[test]
fn a_test_that_no_longer_resolves_warns_rather_than_passing_silently() {
    let dir = repo_with_a_recorded_verdict();
    std::fs::remove_file(dir.path().join("pkg/test_lib.py")).expect("remove");
    commit(dir.path(), "drop the test");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);

    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--tests"]);
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            c.first() == Some(&"warn")
                && c.get(1) == Some(&"test_lib.test_covered")
                && l.contains("cannot be re-keyed")
        }),
        "a test that vanished is said out loud:\n{text}"
    );
}

/// Without `--tests` the report says nothing about tests at all — including
/// nothing that could be read as "all fresh".
#[test]
fn without_the_flag_no_test_claim_is_made() {
    let dir = repo_with_a_recorded_verdict();
    write(
        dir.path(),
        &[(
            "pkg/lib.py",
            "def covered():\n    return 2\n\n\ndef elsewhere():\n    return 1\n",
        )],
    );
    commit(dir.path(), "edit inside the closure");
    let json = report(&grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--json"]));
    assert!(
        json.get("stale_tests").is_none(),
        "an unasked question has no answer: {json}"
    );
    assert!(
        json["unchecked"].get("unattributed_entries").is_none(),
        "and no count either: {json}"
    );
}

/// `warn` rows from a drift report: `(subject, message)`.
fn warns(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&"warn"))
        .filter_map(|c| Some((c.get(1)?.to_string(), c[2..].join(" "))))
        .collect()
}

/// A repo where a test covers `pkg.lib.covered`, a trace records the
/// coverage, and a `pass` is recorded — with the whole thing surveyed and
/// retained so both endpoints of the later patch are in the chain.
fn repo_with_a_green_test() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/lib.py", "def covered(x):\n    return x + 1\n"),
            (
                "pkg/test_lib.py",
                // Assertion-free on purpose: it executes `covered` and can
                // never fail because of it. That is what "weak" means.
                "from pkg.lib import covered\n\n\ndef test_covered():\n    covered(1)\n",
            ),
        ],
    );
    commit(dir.path(), "before");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    ingest_wake(
        dir.path(),
        "trace:t",
        "test_lib.test_covered",
        "lib.covered",
    );
    record_pass(
        dir.path(),
        "env-1",
        &datum_of(dir.path(), "pkg.test_lib.test_covered"),
        "trace:t",
    );
    dir
}

/// Re-survey, re-record the same green verdict at head, and return the
/// report `grid drift --tests` gives for the patch.
fn after_head_pass(dir: &Path) -> Output {
    assert_eq!(code(&grid(dir, &["survey"])), 0);
    record_pass(
        dir,
        "env-1",
        &datum_of(dir, "pkg.test_lib.test_covered"),
        "trace:t",
    );
    grid(dir, &["drift", "HEAD~1", "HEAD", "--tests"])
}

/// Ground truth for step 7: the covered definition's arithmetic flips, the
/// assertion-free test stays green, and nobody touched the test — so the
/// test proved nothing about the change, and grid says so.
#[test]
fn a_green_test_over_a_changed_definition_is_flagged_weak() {
    let dir = repo_with_a_green_test();
    write(
        dir.path(),
        &[("pkg/lib.py", "def covered(x):\n    return x - 1\n")],
    );
    commit(dir.path(), "flip the arithmetic");
    let out = after_head_pass(dir.path());
    assert!(
        warns(&out).contains(&(
            "test_lib.test_covered".to_owned(),
            "weak-for pkg.lib.covered".to_owned()
        )),
        "the weak test is named:\n{}",
        stdout(&out)
    );
    // Heuristic: a ranking, never a gate. The exit code must not move for it.
    assert_ne!(
        code(&out),
        3,
        "a weak-test finding must not gate:\n{}",
        stdout(&out)
    );
}

/// A test edited alongside the change is not weak: somebody looked at it.
#[test]
fn a_test_edited_alongside_the_change_is_not_weak() {
    let dir = repo_with_a_green_test();
    write(
        dir.path(),
        &[
            ("pkg/lib.py", "def covered(x):\n    return x - 1\n"),
            (
                "pkg/test_lib.py",
                "from pkg.lib import covered\n\n\ndef test_covered():\n    assert covered(1) == 0\n",
            ),
        ],
    );
    commit(dir.path(), "change both");
    let out = after_head_pass(dir.path());
    assert!(
        !warns(&out).iter().any(|(_, m)| m.starts_with("weak-for")),
        "an edited test is not weak:\n{}",
        stdout(&out)
    );
}

/// A test with no recorded pass at head is not weak either: the finding is
/// "it passed before, it passes now, and the thing it covers is different".
/// Without the second pass there is no claim to make.
#[test]
fn a_test_with_no_verdict_at_head_is_not_weak() {
    let dir = repo_with_a_green_test();
    write(
        dir.path(),
        &[("pkg/lib.py", "def covered(x):\n    return x - 1\n")],
    );
    commit(dir.path(), "flip the arithmetic");
    // Survey, but record nothing at head.
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["drift", "HEAD~1", "HEAD", "--tests"]);
    assert!(
        !warns(&out).iter().any(|(_, m)| m.starts_with("weak-for")),
        "no pass at head, no claim:\n{}",
        stdout(&out)
    );
}

/// Conservatism: coverage carried by a decayed trace produces no `weak`
/// record at all — a heuristic built on a demoted heuristic is noise — and
/// the declining is counted rather than silent.
#[test]
fn coverage_from_a_decayed_trace_is_declined_and_counted() {
    let dir = repo_with_a_green_test();
    // Rewriting the test moves its datum, so the retained trace now names a
    // caller base does not hold: the coverage fact has decayed.
    write(
        dir.path(),
        &[(
            "pkg/test_lib.py",
            "from pkg.lib import covered\n\n\ndef test_covered():\n    covered(2)\n",
        )],
    );
    commit(dir.path(), "rewrite the test");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    write(
        dir.path(),
        &[("pkg/lib.py", "def covered(x):\n    return x - 1\n")],
    );
    commit(dir.path(), "flip the arithmetic");
    let out = after_head_pass(dir.path());
    assert!(
        !warns(&out).iter().any(|(_, m)| m.starts_with("weak-for")),
        "a decayed coverage fact proves nothing:\n{}",
        stdout(&out)
    );
    assert!(
        misses(&out)
            .iter()
            .any(|(n, reason)| n != "0" && reason.contains("weak-checks skipped")),
        "the declining is counted:\n{}",
        stdout(&out)
    );
}
