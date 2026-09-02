//! `grid check` end to end: the composite is its components, byte for byte,
//! and it never answers with a section quietly missing.
//!
//! Every fixture is a real repository surveyed by the real frontends, so
//! every section here is the one the standalone verb would print.
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

fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--allow-empty", "-m", message]);
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

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(out)).unwrap_or_else(|e| panic!("--json: {e}\n{}", stdout(out)))
}

/// Records of one tag: `(first column, the rest joined)`.
fn records(out: &Output, tag: &str) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&tag))
        .filter_map(|c| Some((c.get(1)?.to_string(), c[2..].join(" "))))
        .collect()
}

fn ingest_trace(dir: &Path, id: &str, from: &str, to: &str) {
    let trace = serde_json::json!({
        "trace_id": id,
        "git_rev": "HEAD",
        "command": from,
        "observations": [{ "caller": { "symbol": from }, "callee": { "symbol": to } }]
    });
    std::fs::write(dir.join("trace.json"), trace.to_string()).expect("write trace");
    let out = grid(dir, &["trace", "ingest", "trace.json"]);
    assert_eq!(code(&out), 0, "ingest failed: {}", stderr(&out));
    assert!(
        stdout(&out).contains("1/1"),
        "the hop must resolve: {}",
        stdout(&out)
    );
    std::fs::remove_file(dir.join("trace.json")).expect("rm trace");
}

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
    assert_eq!(code(&out), 0, "record failed: {}", stderr(&out));
    assert!(stdout(&out).contains("1/1"), "{}", stdout(&out));
}

fn datum_of(dir: &Path, callsign: &str) -> String {
    let out = grid(dir, &["resolve", callsign, "--json"]);
    let refs: Vec<serde_json::Value> = serde_json::from_str(&stdout(&out)).expect("resolve --json");
    refs.first().expect("one definition")["datum"]
        .as_str()
        .expect("a datum")
        .to_owned()
}

const CFG_BEFORE: &str = "#[derive(serde::Deserialize)]\npub struct Cfg { pub a: u32 }\n\
     pub fn untouched() -> Cfg { Cfg { a: 1 } }\n";
// The constructor's text is byte-identical on both sides: it is the caller
// nobody touched, which is what makes it a suspect.
const CFG_AFTER: &str =
    "#[derive(serde::Deserialize)]\npub struct Cfg { pub a: u32, pub b: u32 }\n\
     pub fn untouched() -> Cfg { Cfg { a: 1 } }\n";

/// A repo that trips every gate at once and feeds every tail: a Rust
/// `Deserialize` type gains a required field (compat) with an untouched
/// constructor (drift suspect), and a traced, green Python test covers a
/// definition whose body then flips (stale verdict; weak once re-recorded).
fn loaded_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("crates/app/src/lib.rs", CFG_BEFORE),
            ("pkg/__init__.py", ""),
            ("pkg/lib.py", "def covered(x):\n    return x + 1\n"),
            ("pkg/other.py", "def alone():\n    return 1\n"),
            (
                "pkg/test_lib.py",
                "from pkg.lib import covered\n\n\ndef test_covered():\n    covered(1)\n",
            ),
        ],
    );
    commit(dir.path(), "before");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    ingest_trace(
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
    write(
        dir.path(),
        &[
            ("crates/app/src/lib.rs", CFG_AFTER),
            ("pkg/lib.py", "def covered(x):\n    return x - 1\n"),
        ],
    );
    commit(dir.path(), "after");
    dir
}

/// Differential (rung 5): each section of `check --json` is the standalone
/// verb's own `--json`, byte for byte. The drift section is `drift --tests
/// --compat`'s report with its three lists lifted out; stale and weak are
/// `drift --tests`'s; compat is `drift --compat`'s.
#[test]
fn each_section_is_the_standalone_verbs_own_json() {
    let dir = loaded_repo();
    let check = json(&grid(dir.path(), &["check", "--json"]));
    let mut full = json(&grid(
        dir.path(),
        &["drift", "--tests", "--compat", "--json"],
    ));
    let tests = json(&grid(dir.path(), &["drift", "--tests", "--json"]));
    let compat = json(&grid(dir.path(), &["drift", "--compat", "--json"]));
    let lifted: Vec<serde_json::Value> = ["stale_tests", "weak", "compat"]
        .iter()
        .map(|k| full.as_object_mut().expect("report").remove(*k).expect(k))
        .collect();
    let text = |v: &serde_json::Value| serde_json::to_string(v).expect("json");
    assert_eq!(text(&check["drift"]), text(&full), "drift section");
    assert_eq!(text(&check["stale"]), text(&tests["stale_tests"]), "stale");
    assert_eq!(text(&check["weak"]), text(&tests["weak"]), "weak");
    assert_eq!(text(&check["compat"]), text(&compat["compat"]), "compat");
    assert_eq!(text(&check["stale"]), text(&lifted[0]));
    assert_eq!(text(&check["compat"]), text(&lifted[2]));
    // And the sections say something: an empty differential proves nothing.
    assert!(!check["drift"]["suspects"]
        .as_array()
        .expect("suspects")
        .is_empty());
    assert!(!check["stale"].as_array().expect("stale").is_empty());
    assert!(!check["compat"].as_array().expect("compat").is_empty());
}

/// The gate section is `grid gate`'s own verdict, and `--why` still carries
/// the fulfilling paths the retired review verb carried.
#[test]
fn the_verdict_section_is_the_gates_own_and_why_carries_its_paths() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/util.py", "def helper():\n    return 1\n"),
            ("pkg/main.py", "def sweep():\n    return 0\n"),
        ],
    );
    commit(dir.path(), "before");
    let base = grid(dir.path(), &["survey", "--json"]);
    let base_id = json(&base)["snapshot_id"].as_str().expect("id").to_owned();
    write(
        dir.path(),
        &[(
            "pkg/main.py",
            "from pkg.util import helper\n\n\ndef sweep():\n    return helper()\n",
        )],
    );
    commit(dir.path(), "after");
    let plan = serde_json::json!({
        "plan_id": "plan:check/v1",
        "base": {"snapshot": base_id},
        "delta": {
            "defs": [{"action": "modify", "callsign": "pkg.main.sweep"}],
            "edges": [{"action": "add", "src": "pkg.main.sweep",
                       "relation": "calls", "dst": "pkg.util.helper"}]
        }
    });
    std::fs::write(dir.path().join("plan.json"), plan.to_string()).expect("plan");

    let check = json(&grid(
        dir.path(),
        &["check", "--plan", "plan.json", "--json"],
    ));
    let gate = json(&grid(dir.path(), &["gate", "plan.json", "--json"]));
    assert_eq!(
        serde_json::to_string(&check["verdict"]).expect("json"),
        serde_json::to_string(&gate).expect("json"),
        "the verdict is the gate's"
    );
    assert_eq!(check["verdict"]["gate"], "pass");

    let why = grid(dir.path(), &["check", "--plan", "plan.json", "--why"]);
    assert_eq!(code(&why), 0, "{}", stderr(&why));
    let text = stdout(&why);
    assert!(
        text.starts_with("pass"),
        "the verdict prints first:\n{text}"
    );
    assert!(
        text.lines().any(|l| l.starts_with("path")) && text.lines().any(|l| l.starts_with("hop")),
        "the fulfilling path rides under --why:\n{text}"
    );
    let without = stdout(&grid(dir.path(), &["check", "--plan", "plan.json"]));
    assert!(
        !without.lines().any(|l| l.starts_with("path")),
        "without --why the path stays home:\n{without}"
    );
}

/// Exit is the max over gates: a fixture tripping three of them at once
/// exits 3, every gating record is on stdout, and `--quick` — drift alone —
/// still exits 3 on the suspect while printing nothing it did not read.
#[test]
fn exit_is_the_max_over_gates_and_every_gate_prints() {
    let dir = loaded_repo();
    write(
        dir.path(),
        &[(
            ".grid/invariants.json",
            "[{\"id\": \"cfg-on-disk\", \"rule\": {\"persisted\": \"app.Cfg\"}}]",
        )],
    );
    let out = grid(dir.path(), &["check"]);
    assert_eq!(code(&out), 3, "{}\n{}", stdout(&out), stderr(&out));
    let drift = records(&out, "drift");
    assert!(
        drift.iter().any(|(_, rest)| rest.contains("app.untouched")),
        "the untouched constructor is a suspect:\n{}",
        stdout(&out)
    );
    assert!(
        records(&out, "miss").contains(&(
            "test_lib.test_covered".to_owned(),
            "closure-changed".to_owned()
        )),
        "the stale verdict is a miss:\n{}",
        stdout(&out)
    );
    assert!(
        records(&out, "inv")
            .iter()
            .any(|(status, rest)| status == "violated" && rest.contains("persisted:app.Cfg")),
        "the declared compat finding gates:\n{}",
        stdout(&out)
    );
    assert!(
        !stdout(&out).lines().any(|l| l.starts_with("done")),
        "no certificate while a gate holds:\n{}",
        stdout(&out)
    );

    let quick = grid(dir.path(), &["check", "--quick"]);
    assert_eq!(code(&quick), 3, "{}", stdout(&quick));
    let quick_json = json(&grid(dir.path(), &["check", "--quick", "--json"]));
    for absent in ["stale", "weak", "compat", "uncovered", "clones", "refactor"] {
        assert!(
            quick_json.get(absent).is_none(),
            "--quick reads no trace or verdict log, so no {absent}: {quick_json}"
        );
    }
    assert!(quick_json.get("drift").is_some());
}

/// The refactor certificate: when nothing gates, one `done` record states
/// the three facts, and the weak test rides beneath it as a warning that
/// never moves the exit.
#[test]
fn a_green_change_gets_one_done_record_and_the_tail_ranks() {
    let dir = loaded_repo();
    // Undo the interface move: only the body flip stays.
    write(dir.path(), &[("crates/app/src/lib.rs", CFG_BEFORE)]);
    commit(dir.path(), "keep the interface");
    // Re-record the pass at head so the verdict is fresh, not stale.
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    record_pass(
        dir.path(),
        "env-1",
        &datum_of(dir.path(), "pkg.test_lib.test_covered"),
        "trace:t",
    );
    let out = grid(dir.path(), &["check", "HEAD~2", "HEAD"]);
    assert_eq!(code(&out), 0, "{}\n{}", stdout(&out), stderr(&out));
    assert_eq!(
        records(&out, "done"),
        vec![(
            "refactor".to_owned(),
            "interfaces held · 1 covering tests replayed pass · 0 changed defs uncovered"
                .to_owned()
        )],
        "{}",
        stdout(&out)
    );
    assert!(
        records(&out, "warn").contains(&(
            "test_lib.test_covered".to_owned(),
            "weak-for pkg.lib.covered".to_owned()
        )),
        "the weak test ranks:\n{}",
        stdout(&out)
    );
    assert!(
        !stdout(&out).contains("behavior-preserving") && !stdout(&out).contains("behaviour"),
        "{}",
        stdout(&out)
    );
    let answer = json(&grid(dir.path(), &["check", "HEAD~2", "HEAD", "--json"]));
    assert_eq!(answer["refactor"]["interfaces_moved"], 0);
    assert_eq!(answer["refactor"]["replayed_pass"], 1);
    assert_eq!(answer["refactor"]["uncovered"], 0);
    assert_eq!(answer["uncovered"], serde_json::json!([]));
}

/// Uncovered: a changed definition no traced test reaches is named, and the
/// certificate counts it. Without a retained trace nothing is claimed.
#[test]
fn a_changed_definition_no_trace_reaches_is_uncovered() {
    let dir = loaded_repo();
    write(
        dir.path(),
        &[
            ("crates/app/src/lib.rs", CFG_BEFORE),
            ("pkg/lib.py", "def covered(x):\n    return x + 1\n"),
            ("pkg/other.py", "def alone():\n    return 2\n"),
        ],
    );
    commit(dir.path(), "an untraced definition changes");
    // Base is the commit with the traced pass: only `alone` moved since.
    let out = grid(dir.path(), &["check", "HEAD~2", "HEAD"]);
    assert_eq!(code(&out), 0, "{}\n{}", stdout(&out), stderr(&out));
    assert!(
        records(&out, "warn").contains(&("pkg.other.alone".to_owned(), "uncovered".to_owned())),
        "{}",
        stdout(&out)
    );
    assert!(
        records(&out, "done")[0]
            .1
            .ends_with("1 changed defs uncovered"),
        "{}",
        stdout(&out)
    );

    // No trace, no claim.
    let bare = tempfile::tempdir().expect("tempdir");
    git(bare.path(), &["init", "-q", "-b", "main"]);
    write(
        bare.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "def f():\n    return 1\n"),
        ],
    );
    commit(bare.path(), "before");
    write(bare.path(), &[("pkg/a.py", "def f():\n    return 2\n")]);
    commit(bare.path(), "after");
    let answer = json(&grid(bare.path(), &["check", "--json"]));
    assert!(answer.get("uncovered").is_none(), "{answer}");
    assert_eq!(answer["refactor"]["interfaces_moved"], 0);
    assert!(answer["refactor"].get("uncovered").is_none(), "{answer}");
    let text = stdout(&grid(bare.path(), &["check"]));
    assert!(
        text.contains("done    refactor  interfaces held · no retained traces"),
        "{text}"
    );
}

/// Clone-of: two functions identical once their names are erased are the
/// pair, and a near-duplicate is not claimed.
#[test]
fn clone_of_finds_exactly_the_name_erased_identical_pair() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[(
            "crates/app/src/lib.rs",
            "pub fn a(x: u32) -> u32 { x + 1 }\n",
        )],
    );
    commit(dir.path(), "before");
    write(
        dir.path(),
        &[(
            "crates/app/src/lib.rs",
            "pub fn a(x: u32) -> u32 { x + 1 }\n\
             pub fn b(x: u32) -> u32 { x + 1 }\n\
             pub fn c(x: u32) -> u32 { x + 2 }\n",
        )],
    );
    commit(dir.path(), "after");
    let out = grid(dir.path(), &["check"]);
    assert_eq!(code(&out), 0, "clone-of never gates:\n{}", stderr(&out));
    let clones: Vec<(String, String)> = records(&out, "warn")
        .into_iter()
        .filter(|(_, rest)| rest.starts_with("clone-of"))
        .collect();
    assert_eq!(
        clones,
        vec![("app.b".to_owned(), "clone-of app.a".to_owned())],
        "{}",
        stdout(&out)
    );
    let answer = json(&grid(dir.path(), &["check", "--json"]));
    assert_eq!(answer["clones"].as_array().expect("clones").len(), 1);
    assert_eq!(answer["clones"][0]["def"]["callsign"], "app.b");
    assert_eq!(answer["clones"][0]["existing"]["callsign"], "app.a");
}

/// Conservatism: a drift section that refuses refuses the whole check —
/// exit 1 with the reason, an empty stdout — never a report with the drift
/// section quietly missing. A retained trace that does not parse is a store
/// with unknown contents, and drift declines to answer over it.
#[test]
fn a_refusing_drift_section_refuses_the_whole_check() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "def f():\n    return 1\n"),
        ],
    );
    commit(dir.path(), "before");
    write(dir.path(), &[("pkg/a.py", "def f():\n    return 2\n")]);
    commit(dir.path(), "after");
    assert_eq!(code(&grid(dir.path(), &["check"])), 0);
    write(dir.path(), &[(".grid/traces/bad.json", "{")]);

    let out = grid(dir.path(), &["check"]);
    assert_eq!(
        code(&out),
        1,
        "a refusing drift section refuses the whole check:\n{}",
        stderr(&out)
    );
    assert_eq!(stdout(&out), "", "and serves no report: {:?}", stdout(&out));
    assert!(
        stderr(&out).contains("bad.json"),
        "the refusal names its reason: {}",
        stderr(&out)
    );
    let json_form = grid(dir.path(), &["check", "--json"]);
    assert_eq!(code(&json_form), 1);
    assert_eq!(stdout(&json_form), "");
}

/// Conservatism, the aground case: a coverage walk over fuel refuses the
/// whole check with the budget named — not a report with `uncovered`
/// missing. The fixture is generated because the budget is a real one: a
/// hundred thousand edges, so the chart has to carry more.
#[test]
fn an_aground_coverage_walk_refuses_the_whole_check() {
    // 330 definitions, each calling all 330: the changed one has 330
    // callers, each expanding 330 edges, so the inbound walk examines
    // 108_900 — past fuel.
    const N: usize = 330;
    let clique = |seed: u32| {
        let mut source = String::with_capacity(N * N * 12);
        for i in 0..N {
            let _ = write!(source, "pub fn f{i:03}() -> u32 {{ {seed}");
            for j in 0..N {
                let _ = write!(source, " + f{j:03}()");
            }
            source.push_str(" }\n");
        }
        source.push_str("pub fn the_test() -> u32 { f000() }\n");
        source
    };
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), &[("crates/app/src/lib.rs", clique(0).as_str())]);
    commit(dir.path(), "wide");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    ingest_trace(dir.path(), "trace:t", "the_test", "f000");
    // Every body moves, no interface does: drift has nothing to walk and
    // the coverage walk has everything.
    write(dir.path(), &[("crates/app/src/lib.rs", clique(1).as_str())]);
    commit(dir.path(), "every body moves");

    let out = grid(dir.path(), &["check"]);
    assert_eq!(
        code(&out),
        1,
        "an aground coverage walk refuses the whole check:\n{}",
        stderr(&out)
    );
    assert_eq!(stdout(&out), "", "and serves no report: {:?}", stdout(&out));
    assert!(
        stderr(&out).contains("fuel"),
        "the refusal names the budget it ran past: {}",
        stderr(&out)
    );
    // Quick reads no trace, walks no coverage, and answers.
    let quick = grid(dir.path(), &["check", "--quick"]);
    assert_eq!(code(&quick), 0, "{}", stderr(&quick));
}

/// The compile record gates only when it describes the tree this check
/// surveyed; otherwise the warning says so and nothing gates on it.
#[test]
fn a_compile_record_from_another_snapshot_warns_and_gates_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "def f():\n    return 1\n"),
        ],
    );
    commit(dir.path(), "before");
    write(dir.path(), &[("pkg/a.py", "def f():\n    return 2\n")]);
    commit(dir.path(), "after");
    let absent = json(&grid(dir.path(), &["check", "--json"]));
    let head = absent["drift"]["head"].as_str().expect("head").to_owned();
    assert!(absent.get("compile").is_none());
    assert_eq!(absent["warnings"][0]["subject"], "-");
    assert_eq!(
        absent["warnings"][0]["message"],
        format!(
            "no compile record since @{}",
            &head["snap:b3:".len()..][..8]
        )
    );

    let record = |snapshot: &str| {
        std::fs::write(
            dir.path().join(".grid/compile.json"),
            serde_json::json!({
                "snapshot": snapshot, "features": [], "all_features": false,
                "no_default_features": false, "tests": false, "target": "x", "host": "x",
                "cfgs": [], "recorded_at": "1970-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .expect("write record");
    };
    record("snap:b3:0000000000000000000000000000000000000000000000000000000000000000");
    let stale = json(&grid(dir.path(), &["check", "--json"]));
    assert!(stale.get("compile").is_none(), "{stale}");
    assert!(stale["warnings"][0]["message"]
        .as_str()
        .expect("message")
        .starts_with("no compile record since"));
    record(&head);
    let fresh = json(&grid(dir.path(), &["check", "--json"]));
    assert!(fresh.get("compile").is_some(), "{fresh}");
    assert!(fresh.get("warnings").is_none(), "{fresh}");
}

/// `grid orphans --traced`: the trailing `trace N` column, zero where no
/// retained trace hits the definition.
#[test]
fn orphans_traced_counts_the_traces_that_hit_each_orphan() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/api.py", "def window():\n    return 1\n\n\ndef never():\n    return 2\n"),
            (
                "pkg/caller.py",
                "import importlib\n\n\ndef go():\n    return importlib.import_module(\"pkg.api\").window()\n",
            ),
        ],
    );
    commit(dir.path(), "before");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    ingest_trace(dir.path(), "trace:one", "caller.go", "api.window");
    let out = grid(dir.path(), &["orphans", "--traced"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let rows = records(&out, "orphan");
    let traces = |name: &str| {
        rows.iter().find(|(c, _)| c == name).map(|(_, rest)| {
            rest.rsplit_once("trace ")
                .expect("trace column")
                .1
                .to_owned()
        })
    };
    assert_eq!(
        traces("pkg.api.window").as_deref(),
        Some("1"),
        "{}",
        stdout(&out)
    );
    assert_eq!(
        traces("pkg.api.never").as_deref(),
        Some("0"),
        "{}",
        stdout(&out)
    );
    let rows: Vec<serde_json::Value> = serde_json::from_str(&stdout(&grid(
        dir.path(),
        &["orphans", "--traced", "--json"],
    )))
    .expect("json");
    assert!(rows.iter().all(|r| r.get("traces").is_some()), "{rows:?}");
    let plain: Vec<serde_json::Value> =
        serde_json::from_str(&stdout(&grid(dir.path(), &["orphans", "--json"]))).expect("json");
    assert!(
        plain.iter().all(|r| r.get("traces").is_none()),
        "plain orphans are DefRefs"
    );
}

/// Conservatism (T-24): one check opens the store *once*, and it opens it
/// **after** the head survey. Opening it earlier is the cheap-looking way to
/// open it once and it is the wrong one: the chart handed down to every
/// section would then be the epoch from before the edit, and the records
/// would describe a tree this check never judged — here, a caller whose
/// `file:line` is where it used to be. The uncommitted edit is exactly the
/// state a `PostToolUse` hook runs in.
#[test]
fn every_section_answers_about_the_tree_the_check_surveyed() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), &[("crates/app/src/lib.rs", CFG_BEFORE)]);
    commit(dir.path(), "one");
    // A second commit so `HEAD~1` — the default base — names the first one.
    commit(dir.path(), "two");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    // The edit stays in the worktree: nothing is committed after the survey,
    // so only a survey run inside the check can see it. It adds a required
    // field (so `Cfg`'s interface moves) and pushes the untouched caller two
    // lines down without changing what it is (so it stays a suspect, at a
    // new line).
    write(
        dir.path(),
        &[(
            "crates/app/src/lib.rs",
            "#[derive(serde::Deserialize)]\npub struct Cfg { pub a: u32, pub b: u32 }\n\n\n\
             pub fn untouched() -> Cfg { Cfg { a: 1 } }\n",
        )],
    );

    for args in [&["check"][..], &["check", "--quick"][..]] {
        let out = grid(dir.path(), args);
        let suspects = records(&out, "drift");
        let row = suspects
            .iter()
            .find(|(_, rest)| rest.contains("app.untouched"))
            .unwrap_or_else(|| {
                panic!(
                    "{args:?} must see the worktree edit and name its untouched caller: {}\n{}",
                    stdout(&out),
                    stderr(&out)
                )
            });
        assert!(
            row.1.ends_with("crates/app/src/lib.rs:5"),
            "{args:?} locates the caller in the tree it surveyed (line 5), not the one it \
             opened the store on (line 3): {}",
            row.1
        );
        assert_eq!(
            code(&out),
            3,
            "{args:?} gates on a suspect it can see: {}",
            stdout(&out)
        );
        assert!(
            !stdout(&out).contains("done    refactor"),
            "{args:?} certifies no refactor while a gate fires: {}",
            stdout(&out)
        );
    }
}

/// The two lines a `PostToolUse` hook would otherwise repeat verbatim on
/// every edit: the declined-reference count and the missing compile record.
fn repeated_lines(out: &Output) -> Vec<String> {
    stdout(out)
        .lines()
        .filter(|l| l.contains("unresolved references ") || l.contains("no compile record since @"))
        .map(str::to_owned)
        .collect()
}

/// A repo with something to decline (a `std` name) and no compile record, so
/// both repeated lines are emitted.
fn hook_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[(
            "crates/app/src/lib.rs",
            "pub fn one() -> String { String::new() }\n",
        )],
    );
    commit(dir.path(), "one");
    commit(dir.path(), "two");
    dir
}

/// T-02, ground truth: `--quick` says the unchanged counts once. The second
/// run in the same store drops them; the full `check` never does; and
/// `--json` is the record form, so it carries them whatever was printed.
#[test]
fn quick_says_the_unchanged_counts_once_and_the_full_check_always_says_them() {
    let dir = hook_repo();
    let first = grid(dir.path(), &["check", "--quick"]);
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    assert_eq!(
        repeated_lines(&first).len(),
        2,
        "the first --quick says both: {}",
        stdout(&first)
    );

    let second = grid(dir.path(), &["check", "--quick"]);
    assert!(
        repeated_lines(&second).is_empty(),
        "nothing moved, so nothing is repeated: {}",
        stdout(&second)
    );
    assert_eq!(code(&second), 0, "suppressing a line moves no exit code");

    let full = grid(dir.path(), &["check"]);
    assert_eq!(
        repeated_lines(&full).len(),
        2,
        "the full check is the record and always says them: {}",
        stdout(&full)
    );

    let quiet = grid(dir.path(), &["check", "--quick"]);
    assert!(
        repeated_lines(&quiet).is_empty(),
        "the full check does not disturb what --quick remembers: {}",
        stdout(&quiet)
    );
    let raw = json(&grid(dir.path(), &["check", "--quick", "--json"]));
    assert!(
        raw["drift"]["unchecked"]["unresolved_refs"]
            .as_u64()
            .is_some(),
        "--json is never filtered: {raw}"
    );
}

/// T-02, conservatism: a count that *moved* is printed, whatever the last
/// run printed. Suppressing on "I have said something before" rather than on
/// "I have said exactly this before" would hide the one direction that
/// matters — the honesty number getting worse.
#[test]
fn a_declined_count_that_moved_is_always_printed() {
    let dir = hook_repo();
    assert_eq!(
        repeated_lines(&grid(dir.path(), &["check", "--quick"])).len(),
        2
    );
    assert!(repeated_lines(&grid(dir.path(), &["check", "--quick"])).is_empty());

    // One more `std` name the resolver declines: the count moves by one.
    write(
        dir.path(),
        &[(
            "crates/app/src/lib.rs",
            "pub fn one() -> String { String::new() }\n\
             pub fn two() -> Vec<u8> { Vec::new() }\n",
        )],
    );
    let moved = grid(dir.path(), &["check", "--quick"]);
    let lines = repeated_lines(&moved);
    assert!(
        lines.iter().any(|l| l.contains("unresolved references ")),
        "a count that moved is said again: {}",
        stdout(&moved)
    );
    // And having said it, it goes quiet again on the next unchanged turn.
    assert!(
        repeated_lines(&grid(dir.path(), &["check", "--quick"])).is_empty(),
        "the new count is now the remembered one"
    );
}

/// T-10, ground truth: a verb that refuses for a base this store does not
/// hold prints one hint line with the command *and* the tree to run it in.
/// "run `grid survey` first" with no tree named is unfollowable from a hook,
/// a subdirectory, or a sibling worktree.
///
/// `check` and `drift` are not in the loop because they survey their base on
/// demand and so never reach this refusal; the two they can reach — a rev git
/// cannot name, a snapshot id the store never held — are not fixed by a
/// survey, and the last assertion pins that they do not pretend otherwise.
#[test]
fn a_missing_base_refusal_names_the_command_and_the_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), &[("crates/app/src/lib.rs", CFG_BEFORE)]);
    commit(dir.path(), "one");
    write(
        dir.path(),
        &[("crates/app/src/lib.rs", "pub fn only() -> u32 { 1 }\n")],
    );
    commit(dir.path(), "two");
    // Only the second commit is ever surveyed, so `HEAD~1` is a tree this
    // store does not hold — and neither verb below surveys on demand.
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);

    for args in [
        &["scope", "app.only", "--since", "HEAD~1"][..],
        &["suspects", "app.only", "--closure", "HEAD~1"][..],
    ] {
        let out = grid(dir.path(), args);
        assert_eq!(
            code(&out),
            2,
            "{args:?} refuses a base it does not hold: {}",
            stdout(&out)
        );
        assert!(
            stderr(&out).contains("no snapshot of the tree at HEAD~1"),
            "{args:?} says which coordinate it could not answer: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("hint: run `grid survey HEAD~1` in ."),
            "{args:?} names the command and the tree: {}",
            stderr(&out)
        );
    }

    // And a refusal a survey does not fix offers no survey: a hint that does
    // not work is worse than none.
    let bad = grid(dir.path(), &["check", "deadbeef"]);
    assert!(code(&bad) != 0, "{}", stdout(&bad));
    assert!(
        !stderr(&bad).contains("hint: run `grid survey"),
        "a rev git cannot name is git's answer, not a survey's: {}",
        stderr(&bad)
    );
}

/// T-11, ground truth: `--format md` is one document over the same judgment
/// the porcelain prints — headings for the gate-first order, one bullet per
/// finding as `callsign — reason — file:line`, the certificate, and what was
/// not checked as a footer. The records and `--json` are untouched by it.
#[test]
fn format_md_is_one_document_over_the_same_judgment() {
    let dir = loaded_repo();
    let records = stdout(&grid(dir.path(), &["check"]));
    let raw = json(&grid(dir.path(), &["check", "--json"]));

    let out = grid(dir.path(), &["check", "--format", "md"]);
    let doc = stdout(&out);
    assert_eq!(code(&out), 3, "the exit is the composite's own: {doc}");

    let head = doc.lines().next().expect("a heading");
    assert!(
        head.starts_with("# grid check @") && head.contains(" → @"),
        "the heading names base and head: {head}"
    );
    assert!(
        doc.contains("**Exit 3** — something this change did was not checked"),
        "and what the exit means: {doc}"
    );
    assert!(
        doc.contains("\n## Gates\n") && doc.contains("\n### Drift suspects\n"),
        "gates first, by section: {doc}"
    );
    assert!(
        doc.lines().any(
            |l| l.starts_with("- `app.untouched` — constructs `app.Cfg` (proven)")
                && l.ends_with("— `crates/app/src/lib.rs:3`")
        ),
        "one bullet per finding — callsign, reason, file:line: {doc}"
    );
    assert!(
        doc.contains("\n### Stale verdicts\n") && doc.contains("- `test_lib.test_covered`"),
        "every gating section is a section: {doc}"
    );
    assert!(
        doc.contains("\n## Tail\n") && doc.contains("\n### Compat\n"),
        "and the ranked findings are below the gates: {doc}"
    );
    assert!(
        !doc.contains("## Certificate"),
        "a gate fired, so there is no certificate: {doc}"
    );
    assert!(
        doc.contains("\n---\n\nNot checked: ") && doc.contains("unresolved references"),
        "the counts close the document as a footer: {doc}"
    );

    // The other two forms are what they were: three renderings, one judgment.
    assert_eq!(stdout(&grid(dir.path(), &["check"])), records);
    assert_eq!(json(&grid(dir.path(), &["check", "--json"])), raw);
    assert!(!records.contains("# grid check"), "porcelain is records");

    // A green check gets the certificate line and no Gates section.
    let green = tempfile::tempdir().expect("tempdir");
    git(green.path(), &["init", "-q", "-b", "main"]);
    write(
        green.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "def f():\n    return 1\n"),
        ],
    );
    commit(green.path(), "one");
    commit(green.path(), "two");
    let clean = grid(green.path(), &["check", "--format", "md"]);
    assert_eq!(code(&clean), 0, "{}", stderr(&clean));
    assert!(
        stdout(&clean).contains("**Exit 0** — no gate fired")
            && stdout(&clean).contains("## Certificate")
            && stdout(&clean).contains("**refactor** — interfaces held")
            && !stdout(&clean).contains("## Gates"),
        "{}",
        stdout(&clean)
    );

    // Two output forms at once is a contradiction, not a precedence rule.
    let both = grid(dir.path(), &["check", "--format", "md", "--json"]);
    assert_eq!(code(&both), 2, "{}", stdout(&both));
    let unknown = grid(dir.path(), &["check", "--format", "html"]);
    assert_eq!(code(&unknown), 2);
    assert!(
        stderr(&unknown).contains("unknown format `html`"),
        "{}",
        stderr(&unknown)
    );
}

/// T-11, conservatism: a section that refuses refuses the whole document.
/// The temptation is the opposite one — a pull-request comment that says
/// "grid could not read the trace store" is friendlier than no comment — and
/// it is exactly wrong: a document with a heading, no gates and a footer is
/// the same shape as a clean check, and a reviewer skims shape. So
/// `--format md` refuses the way the records do: the reason on stderr, the
/// exit code of the section that refused, and nothing at all on stdout.
#[test]
fn format_md_with_a_refusing_section_emits_no_document() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "def f():\n    return 1\n"),
        ],
    );
    commit(dir.path(), "before");
    write(dir.path(), &[("pkg/a.py", "def f():\n    return 2\n")]);
    commit(dir.path(), "after");
    assert_eq!(code(&grid(dir.path(), &["check", "--format", "md"])), 0);
    // A trace store grid cannot read: unknown contents, not empty contents.
    write(dir.path(), &[(".grid/traces/bad.json", "{")]);

    let out = grid(dir.path(), &["check", "--format", "md"]);
    assert_eq!(
        code(&out),
        1,
        "the refusing section's exit, not a friendlier one:\n{}{}",
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(
        stdout(&out),
        "",
        "and no document at all, not a document with the gates missing: {:?}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("bad.json"),
        "the reason is still said: {}",
        stderr(&out)
    );

    // A bad-input refusal is the same posture with the input's exit code.
    let bad = grid(dir.path(), &["check", "--format", "md", "deadbeef"]);
    assert_eq!(code(&bad), 2, "{}", stdout(&bad));
    assert_eq!(stdout(&bad), "", "{:?}", stdout(&bad));
}

/// T-11, conservatism: every `warn` the records carry reaches the document.
///
/// The document is a *review* surface, and the one thing a reviewer cannot
/// recover from it is a check that quietly stopped running. A covenant whose
/// pattern matches nothing at head is exactly that — the declaration is no
/// longer checked — and the records say so with a `warn`. A document that
/// showed a clean certificate over the same result would be the porcelain
/// and the document disagreeing about what was *checked*, which is the half
/// nobody notices.
#[test]
fn format_md_carries_every_warn_the_records_carry() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/writer.py",
                "def encode(tag):\n    return {\"n\": tag}\n",
            ),
            (
                "pkg/reader.py",
                "def decode(blob):\n    return blob[\"n\"]\n",
            ),
        ],
    );
    commit(dir.path(), "before");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let declared = grid(
        dir.path(),
        &[
            "covenant",
            "pkg.writer.encode",
            "pkg.reader.decode",
            "--via",
            "schema",
        ],
    );
    assert_eq!(code(&declared), 0, "{}", stderr(&declared));
    // Both sides are deleted out from under the declaration. Neither pattern
    // matches anything at head, so the covenant is no longer checked at all —
    // and because both sides moved together it fires nothing, which is what
    // makes the document look clean.
    std::fs::remove_file(dir.path().join("pkg/writer.py")).expect("remove");
    std::fs::remove_file(dir.path().join("pkg/reader.py")).expect("remove");
    commit(dir.path(), "drop both sides");

    let records = grid(dir.path(), &["check"]);
    let printed = stdout(&records);
    assert!(
        printed
            .lines()
            .any(|l| l.starts_with("warn ") && l.contains("matches nothing")),
        "the records say the declaration stopped being checked:\n{printed}"
    );

    let out = grid(dir.path(), &["check", "--format", "md"]);
    let doc = stdout(&out);
    assert!(
        doc.contains("covenant:schema matches nothing"),
        "and so must the document — a review surface that hides what stopped \
         being checked is worse than no review surface:\n{doc}"
    );
    assert!(
        doc.contains("pkg.reader.decode"),
        "naming the pattern that stopped matching, as the record does:\n{doc}"
    );
    // And the document is otherwise exactly the shape a reviewer skims past:
    // exit 0, a certificate, no gates. That is what makes the omission the
    // dangerous one rather than a cosmetic one.
    assert_eq!(code(&out), 0, "{doc}");
    assert!(
        doc.contains("## Certificate") && !doc.contains("## Gates"),
        "{doc}"
    );
}

/// T-11, conservatism: a format word grid will not honour is bad input.
///
/// `--compile` with no `--plan` short-circuits into the compile record's own
/// report, which has no document form — so `--format md` in that position was
/// parsed and dropped, and the caller who asked for a pull-request comment
/// got padded porcelain columns to paste into one. That is the same sin
/// `is_markdown` refuses on purpose for an unknown format word, and the same
/// answer applies: exit 2, before the compile runs, rather than a silent fall
/// back to records.
///
/// `--quick` in the same position keeps its hint. It filters an output form
/// rather than choosing one, so ignoring it still hands the caller the thing
/// they asked for; ignoring `--format md` does not.
#[test]
fn a_format_word_the_compile_report_cannot_honour_is_bad_input() {
    let dir = loaded_repo();
    let out = grid(dir.path(), &["check", "--compile", "--format", "md"]);
    assert_eq!(
        code(&out),
        2,
        "a format grid will not honour is refused, not dropped:\n{}{}",
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(
        stdout(&out),
        "",
        "and nothing is printed in the form that was not asked for: {:?}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("--compile") && stderr(&out).contains("--plan"),
        "the refusal names both halves of the combination: {}",
        stderr(&out)
    );
}

/// T-11, conservatism: a word a human authored cannot break the document.
///
/// Every bullet is ``- `what` — why — `file:line` ``, and `why` carries
/// declaration-authored text — a covenant's `via` word, a `cfg` predicate, a
/// compat rule name. A backtick in one of those closes the code span it sits
/// in and swallows the rest of the line, so the reviewer reads a bullet that
/// says something other than what grid found. The document is a review
/// surface: a finding rendered wrong is worse than a finding rendered plain.
#[test]
fn a_backtick_in_a_declared_word_does_not_break_the_document() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            (
                "pkg/writer.py",
                "def encode(tag):\n    return {\"n\": tag}\n",
            ),
            (
                "pkg/reader.py",
                "def decode(blob):\n    return blob[\"n\"]\n",
            ),
        ],
    );
    commit(dir.path(), "before");
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    // A `via` word with a backtick in it. Nothing refuses this — it is a
    // human's note about a boundary, and grid does not police prose.
    let declared = grid(
        dir.path(),
        &[
            "covenant",
            "pkg.writer.encode",
            "pkg.reader.decode",
            "--via",
            "wire`schema",
        ],
    );
    assert_eq!(code(&declared), 0, "{}", stderr(&declared));
    // One side moves, so the covenant fires and the reason reaches a bullet.
    write(
        dir.path(),
        &[(
            "pkg/writer.py",
            "def encode(tag):\n    return {\"n\": str(tag)}\n",
        )],
    );
    commit(dir.path(), "after");

    let doc = stdout(&grid(dir.path(), &["check", "--format", "md"]));
    let fires: Vec<&str> = doc
        .lines()
        .filter(|l| l.starts_with("- ") && l.contains("covenant:"))
        .collect();
    assert_eq!(fires.len(), 1, "the covenant fired once:\n{doc}");
    assert_eq!(
        fires[0].matches('`').count() % 2,
        0,
        "every code span the bullet opened is closed: {}",
        fires[0]
    );
    assert!(
        fires[0].contains("covenant:wire'schema"),
        "and the word is still legible, substituted rather than dropped: {}",
        fires[0]
    );
}

/// T-11: `--why` is not silently swallowed by `--format md`.
///
/// The document has no section for the gate's fulfilling paths, so `--why`
/// adds nothing to it. That is the `--quick` case rather than the `--format
/// md`-beside-`--compile` case: `--why` adds content to a form the caller
/// still gets, so it is hinted, not refused. What it must not be is silent.
#[test]
fn why_beside_format_md_is_hinted_rather_than_swallowed() {
    let dir = loaded_repo();
    let plain = grid(dir.path(), &["check", "--format", "md"]);
    let with_why = grid(dir.path(), &["check", "--format", "md", "--why"]);
    assert_eq!(
        stdout(&with_why),
        stdout(&plain),
        "the document is the same one either way"
    );
    assert!(
        stderr(&with_why).contains("--why"),
        "and the flag that changed nothing says so: {}",
        stderr(&with_why)
    );
    assert!(
        !stderr(&plain).contains("--why"),
        "a caller who did not ask is not told: {}",
        stderr(&plain)
    );
}
