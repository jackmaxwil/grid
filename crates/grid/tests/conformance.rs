//! D7 for wave 3: the three implemented methods, served over the real socket,
//! answering the contract's own examples.
//!
//! The wire crate's conformance test proves the *types* round-trip. This one
//! proves the *daemon* does: same JSON in, a response whose shape the contract
//! accepts out, over a unix socket with newline framing. A method that only
//! works when called in-process is not served.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde_json::{json, Value};

fn contract() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/wire-v0.openrpc.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("read contract"))
        .expect("parse contract")
}

/// The example params for a method, as a by-name object.
fn example_params(contract: &Value, method: &str) -> Value {
    let m = contract["methods"]
        .as_array()
        .expect("methods")
        .iter()
        .find(|m| m["name"] == method)
        .unwrap_or_else(|| panic!("contract has no method {method}"));
    m["examples"][0]["params"]
        .as_array()
        .expect("example params")
        .iter()
        .map(|p| {
            (
                p["name"].as_str().expect("param name").to_string(),
                p["value"].clone(),
            )
        })
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

/// A daemon on its own socket in its own directory, killed on drop so a failed
/// assertion cannot leak a process into the next test run.
struct Daemon {
    child: Child,
    socket: std::path::PathBuf,
}

impl Daemon {
    fn start(dir: &Path) -> Self {
        let socket = dir.join("grid.sock");
        let mut child = Command::new(env!("CARGO_BIN_EXE_grid"))
            .arg("serve")
            .arg(&socket)
            .current_dir(dir)
            .env("GRID_HINTS", "0")
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn grid serve");
        // Readiness is a connection that succeeds, not a file that exists:
        // bind(2) creates the socket file before listen(2) starts accepting,
        // so the file appearing is a promise, not an answer. Connecting is the
        // only signal that cannot be early.
        for _ in 0..200 {
            if UnixStream::connect(&socket).is_ok() {
                return Self { child, socket };
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        // Reap before failing: a daemon that never accepted is still a child.
        let _ = child.kill();
        let _ = child.wait();
        panic!("daemon never accepted on {}", socket.display());
    }

    fn call(&self, method: &str, params: &Value) -> Value {
        let stream = UnixStream::connect(&self.socket)
            .unwrap_or_else(|e| panic!("connect {}: {e}", self.socket.display()));
        let mut out = stream.try_clone().expect("clone");
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        writeln!(out, "{request}").expect("write request");
        out.flush().expect("flush");
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .expect("read response");
        serde_json::from_str(&line).expect("response is JSON")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A repo with two commits, so a gate has a real delta to check.
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

#[test]
fn version_example_is_served_over_the_socket() {
    let contract = contract();
    let dir = repo();
    let daemon = Daemon::start(dir.path());

    let response = daemon.call("grid.version", &example_params(&contract, "grid.version"));
    assert_eq!(response["jsonrpc"], "2.0");
    let result = &response["result"];
    // The contract's example fixes the shape; the values are this build's.
    let example = &contract["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "grid.version")
        .unwrap()["examples"][0]["result"]["value"];
    for key in ["wire", "engine", "porcelain"] {
        assert!(
            result.get(key).is_some_and(Value::is_string),
            "grid.version must answer with a string {key}, got {result}"
        );
    }
    assert_eq!(
        result["wire"], example["wire"],
        "wire version is the contract's"
    );
    assert_eq!(
        result["porcelain"], example["porcelain"],
        "porcelain version is PORCELAIN.md's"
    );
    // No survey has run, so there is no snapshot to report — and the field is
    // optional precisely for this state, not filled with a placeholder.
    assert!(result.get("snapshot").is_none());
}

#[test]
fn track_lint_example_is_served_verbatim() {
    let contract = contract();
    let dir = repo();
    // The example's finding cites a standing invariant, which lives in the
    // repo, not in the track: writing it here is what makes the example real.
    std::fs::create_dir_all(dir.path().join(".grid")).expect("mkdir");
    std::fs::write(
        dir.path().join(".grid/invariants.json"),
        json!([{
            "id": "boards-skip-persistence",
            "description": "Board code may not call persistence directly",
            "rule": {"forbid_edge": {
                "src": "cortana.boards.**",
                "relation": "calls",
                "dst": "cortana.persistence.**"
            }}
        }])
        .to_string(),
    )
    .expect("write invariants");

    let daemon = Daemon::start(dir.path());
    let response = daemon.call(
        "grid.track.lint",
        &example_params(&contract, "grid.track.lint"),
    );

    let expected = &contract["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "grid.track.lint")
        .unwrap()["examples"][0]["result"]["value"];
    assert_eq!(
        &response["result"], expected,
        "the contract's lint example must come back verbatim"
    );
}

#[test]
fn gate_check_answers_with_a_verdict_the_contract_accepts() {
    let contract = contract();
    let dir = repo();
    let daemon = Daemon::start(dir.path());

    // The example gates against a snapshot id, and there is no snapshot store
    // until wave 5 — so the example's *shape* is exercised against real revs.
    let mut params = example_params(&contract, "grid.gate.check");
    params["track"]["base"] = json!({"git_rev": "HEAD~1"});
    params["head"] = json!({"git_rev": "HEAD"});

    let response = daemon.call("grid.gate.check", &params);
    let result = &response["result"];
    assert!(
        response.get("error").is_none(),
        "gate.check errored: {response}"
    );
    // Every required field of XteVerdict is present.
    for key in [
        "gate",
        "track_id",
        "head",
        "matched",
        "missing",
        "surplus",
        "tolerated",
        "defs_unaccounted",
        "tests",
        "invariant_violations",
    ] {
        assert!(
            result.get(key).is_some(),
            "verdict is missing {key}: {result}"
        );
    }
    // The promised edge cannot be seen by a file-level provider, the expected
    // test never ran, and b.py is changed but unpromised: three hold causes,
    // itemized rather than summarized.
    assert_eq!(result["gate"], "hold");
    assert_eq!(result["missing"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["tests"][0]["actual"], "missing");
    assert!(!result["defs_unaccounted"]
        .as_array()
        .expect("array")
        .is_empty());
    // And the verdict deserializes into the generated type: the daemon's
    // answer is a wire value, not a lookalike.
    let typed: Result<grid_wire::XteVerdict, _> = serde_json::from_value(result.clone());
    assert!(typed.is_ok(), "verdict must deserialize: {typed:?}");
}

#[test]
fn a_bad_request_is_an_error_response_not_a_crash() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());

    assert_eq!(
        daemon.call("grid.nope", &json!({}))["error"]["code"],
        -32601
    );
    // Unknown params are a wire error: deny_unknown_fields is the point.
    let bad = daemon.call("grid.track.lint", &json!({"track": {"nope": 1}}));
    assert_eq!(bad["error"]["code"], -32602);
    // A snapshot id has no store to resolve against yet: a typed refusal.
    let unknown = daemon.call(
        "grid.gate.check",
        &json!({
            "track": {
                "track_id": "track:t",
                "base": {"snapshot": "snap:b3:00"},
                "delta": {"defs": [], "edges": []}
            },
            "head": {"snapshot": "snap:b3:00"}
        }),
    );
    assert_eq!(unknown["error"]["code"], -32003);
    // The daemon is still serving after all of that.
    assert_eq!(
        daemon.call("grid.version", &json!({}))["result"]["wire"],
        "0.1.0"
    );
}

#[test]
fn survey_run_publishes_a_snapshot_the_contract_accepts() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let response = daemon.call("grid.survey.run", &json!({}));
    assert!(
        response.get("error").is_none(),
        "survey.run errored: {response}"
    );
    let result = &response["result"];
    for key in ["snapshot_id", "git_rev", "created_at", "stats"] {
        assert!(
            result.get(key).is_some(),
            "snapshot missing {key}: {result}"
        );
    }
    // D5: the id is the hash of the manifest, so it is a content id.
    assert!(
        result["snapshot_id"]
            .as_str()
            .is_some_and(|s| s.starts_with("snap:b3:")),
        "snapshot id must be content-addressed: {result}"
    );
    assert!(result["stats"]["defs"].as_u64().is_some_and(|n| n > 0));
}

#[test]
fn surveying_twice_returns_the_same_snapshot_id() {
    // Idempotence by construction: an unchanged tree is the same snapshot,
    // so a caller cannot survey "too often".
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let first = daemon.call("grid.survey.run", &json!({}));
    let second = daemon.call("grid.survey.run", &json!({}));
    assert_eq!(
        first["result"]["snapshot_id"],
        second["result"]["snapshot_id"]
    );
}

#[test]
fn snapshot_current_reports_what_was_published() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let published = daemon.call("grid.survey.run", &json!({}));
    let current = daemon.call("grid.snapshot.current", &json!({}));
    assert_eq!(
        current["result"]["snapshot_id"], published["result"]["snapshot_id"],
        "the current snapshot is the one just published"
    );
}

#[test]
fn snapshot_current_before_any_survey_is_the_contracts_error() {
    // "Never surveyed" is a state with a wire code (-32002), not a crash and
    // not a null that a caller would have to special-case.
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let response = daemon.call("grid.snapshot.current", &json!({}));
    assert_eq!(response["error"]["code"], -32002, "got {response}");
}

#[test]
fn wave_ten_methods_reject_bad_params_and_absent_history() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());

    for method in ["grid.suspects", "grid.bisect"] {
        let bad = daemon.call(method, &json!({"nope": true}));
        assert_eq!(bad["error"]["code"], -32602, "got {bad}");
    }

    let suspects = daemon.call("grid.suspects", &json!({"test": "test::red"}));
    assert_eq!(suspects["error"]["code"], -32002, "got {suspects}");
    assert!(
        suspects["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("insufficient history")),
        "got {suspects}"
    );

    let bisect = daemon.call(
        "grid.bisect",
        &json!({
            "good": {"git_rev": "HEAD~1"},
            "bad": {"git_rev": "HEAD"},
            "test": "test::red"
        }),
    );
    assert_eq!(bisect["error"]["code"], -32002, "got {bisect}");
    assert!(
        bisect["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("insufficient history")),
        "got {bisect}"
    );
}

#[test]
fn delta_answers_with_a_made_good_the_contract_accepts() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let response = daemon.call(
        "grid.delta",
        &json!({"base": {"git_rev": "HEAD~1"}, "head": {"git_rev": "HEAD"}}),
    );
    assert!(response.get("error").is_none(), "delta errored: {response}");
    let result = &response["result"];
    for key in ["base", "head", "defs", "edges"] {
        assert!(
            result.get(key).is_some(),
            "MadeGood missing {key}: {result}"
        );
    }
    // `b.py` is added in the second commit of the fixture repo, and the
    // chart names definitions, not files.
    let defs = result["defs"].as_array().expect("defs");
    assert!(
        defs.iter()
            .any(|d| d["callsign"] == "b.b" && d["action"] == "added"),
        "the added definition is reported: {result}"
    );
}

/// The certified radius crosses the socket: a walk's answer says how far it
/// is *complete*, not only where it stopped.
#[test]
fn query_walk_reports_the_radius_it_finished() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    daemon.call("grid.survey.run", &json!({}));
    let response = daemon.call(
        "grid.query.walk",
        &json!({"query": {"from": [{"callsign": "b"}], "fuel": {"max_hops": 2}}}),
    );
    assert!(response.get("error").is_none(), "walk errored: {response}");
    let stats = &response["result"]["stats"];
    assert!(
        stats.get("complete_within").is_some(),
        "a walk that drained certifies its bound: {stats}"
    );
}

/// C10 for `grid.drift`: the daemon answers it over the real socket with the
/// same computation the CLI runs, refusals included.
#[test]
fn drift_answers_over_the_socket_with_its_refusals_attached() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let response = daemon.call(
        "grid.drift",
        &json!({"base": {"git_rev": "HEAD~1"}, "head": {"git_rev": "HEAD"}}),
    );
    assert!(response.get("error").is_none(), "drift errored: {response}");
    let result = &response["result"];
    for key in ["base", "head", "suspects", "unchecked"] {
        assert!(
            result.get(key).is_some(),
            "DriftReport missing {key}: {result}"
        );
    }
    // The refusal counts ride with every answer, including an empty one:
    // "no suspects" without them would be silence dressed as a result.
    assert!(
        result["unchecked"].get("budget_rejected").is_some(),
        "unchecked must state what was not walked: {result}"
    );
}

#[test]
fn resolve_finds_a_definition_by_unique_suffix() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    daemon.call("grid.survey.run", &json!({}));
    let response = daemon.call("grid.resolve", &json!({"locator": {"callsign": "b"}}));
    let refs = response["result"].as_array().expect("refs");
    assert_eq!(refs.len(), 1, "got {response}");
    assert_eq!(refs[0]["callsign"], "b.b");
    assert!(
        refs[0]["datum"]
            .as_str()
            .is_some_and(|d| d.starts_with("b3:")),
        "a DefRef carries the datum: {response}"
    );
}

#[test]
fn resolving_an_unknown_name_is_an_empty_list_not_an_error() {
    // The contract says so explicitly: "Unknown locators resolve to an empty
    // list, not an error."
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    daemon.call("grid.survey.run", &json!({}));
    let response = daemon.call(
        "grid.resolve",
        &json!({"locator": {"callsign": "no.such.name"}}),
    );
    assert!(response.get("error").is_none(), "got {response}");
    assert_eq!(response["result"].as_array().expect("refs").len(), 0);
}

/// Publish a one-segment chart into `dir/.grid/chart` and return the two datum
/// handles it interns: a caller and the callee it proves an edge to.
fn seed_chart(dir: &Path) -> (String, String) {
    use grid_chart::{build, DatumId, Def, Edge};

    let caller = DatumId([0x11; 32]);
    let target = DatumId([0x22; 32]);
    let image = build(
        &[
            Def {
                datum: caller,
                callsign: "app.caller".into(),
                lang: Some(grid_wire::Lang::Py),
                facts: None,
            },
            Def {
                datum: target,
                callsign: "app.callee".into(),
                lang: Some(grid_wire::Lang::Py),
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
    .expect("build segment");
    let mut store = grid_chart_io::Store::open(dir.join(".grid/chart")).expect("open store");
    store.publish("HEAD", None, &[image]).expect("publish");
    (caller.to_string(), target.to_string())
}

#[test]
fn cache_check_and_record_are_served_over_the_socket() {
    let dir = repo();
    let (caller, _callee) = seed_chart(dir.path());
    let daemon = Daemon::start(dir.path());
    let env = "env:b3:77aa77aa";

    // Cold: the datum is real, but nothing has been recorded, so it misses.
    let cold = daemon.call(
        "grid.cache.check",
        &json!({"request": {"kind": "test", "env_hash": env, "datums": [caller]}}),
    );
    let result = &cold["result"];
    assert_eq!(result["hits"].as_array().expect("hits").len(), 0);
    assert_eq!(result["misses"][0], json!(caller), "cold check misses");
    assert!(
        result["snapshot"]
            .as_str()
            .is_some_and(|s| s.starts_with("snap:b3:")),
        "a check answers with the snapshot it answered from: {result}"
    );

    // Record the miss, as CI would after running the tests.
    let recorded = daemon.call(
        "grid.cache.record",
        &json!({
            "kind": "test",
            "env_hash": env,
            "results": [{"datum": caller, "verdict": "pass"}]
        }),
    );
    assert_eq!(recorded["result"]["recorded"], 1);
    assert_eq!(
        recorded["result"]["rejected"]
            .as_array()
            .expect("rejected")
            .len(),
        0
    );

    // Warm: the same key now hits, with the verdict that was recorded.
    let warm = daemon.call(
        "grid.cache.check",
        &json!({"request": {"kind": "test", "env_hash": env, "datums": [caller]}}),
    );
    let hit = &warm["result"]["hits"][0];
    assert_eq!(hit["datum"], json!(caller));
    assert_eq!(hit["verdict"], "pass");
    assert!(
        hit["recorded_at"]
            .as_str()
            .is_some_and(|s| s.ends_with('Z')),
        "recorded_at is RFC 3339 UTC: {hit}"
    );
    assert_eq!(
        warm["result"]["misses"].as_array().expect("misses").len(),
        0
    );

    // A different environment is a different key, whatever the chart says.
    let other_env = daemon.call(
        "grid.cache.check",
        &json!({"request": {"kind": "lint", "env_hash": "env:b3:different", "datums": [caller]}}),
    );
    assert_eq!(other_env["result"]["misses"][0], json!(caller));
}

#[test]
fn a_datum_outside_the_snapshot_is_rejected_not_stored() {
    // C4's record-side face: no key can be frozen for a closure that cannot be
    // established, so the entry is refused with a reason rather than stored.
    let dir = repo();
    seed_chart(dir.path());
    let daemon = Daemon::start(dir.path());
    let stranger = format!("b3:{}", "ab".repeat(32));

    let recorded = daemon.call(
        "grid.cache.record",
        &json!({
            "kind": "test",
            "env_hash": "env:b3:77aa77aa",
            "results": [{"datum": stranger, "verdict": "pass"}]
        }),
    );
    assert_eq!(recorded["result"]["recorded"], 0);
    let rejected = &recorded["result"]["rejected"][0];
    assert_eq!(rejected["datum"], json!(stranger));
    assert!(
        rejected["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "a rejection carries its reason: {rejected}"
    );

    // And it never becomes a hit afterwards.
    let check = daemon.call(
        "grid.cache.check",
        &json!({"request": {"kind": "test", "env_hash": "env:b3:77aa77aa", "datums": [stranger]}}),
    );
    assert_eq!(check["result"]["hits"].as_array().expect("hits").len(), 0);
}

#[test]
fn the_contract_example_shape_is_served() {
    // The contract's own example params, against a real chart: the daemon must
    // answer the documented shape, not a shape only this test knows.
    let contract = contract();
    let dir = repo();
    let (caller, _) = seed_chart(dir.path());
    let daemon = Daemon::start(dir.path());

    let mut params = example_params(&contract, "grid.cache.check");
    // The example names a snapshot this build has never seen; the datums are
    // this chart's. Shape from the contract, coordinates from the fixture.
    params["request"]["snapshot"] = Value::Null;
    params["request"]
        .as_object_mut()
        .expect("obj")
        .remove("snapshot");
    params["request"]["datums"] = json!([caller]);

    let response = daemon.call("grid.cache.check", &params);
    let result = &response["result"];
    assert!(result["snapshot"].is_string());
    assert!(result["hits"].is_array());
    assert!(result["misses"].is_array());
}

/// Write a track file into the repo's store and return its id.
fn write_track(dir: &Path, id: &str, track: &Value) -> String {
    let tracks = dir.join(".grid/tracks");
    std::fs::create_dir_all(&tracks).expect("mkdir tracks");
    std::fs::write(tracks.join(format!("{id}.json")), track.to_string()).expect("write track");
    format!("track:{id}")
}

/// The wave 6 fixture: two functions, one caller, and an imported-but-
/// unused callee — so the promised edge can be built by editing a
/// definition alone (an import line has no datum; `edit apply` addresses
/// definitions).
fn repo_with_an_unused_import() -> tempfile::TempDir {
    let dir = repo();
    let write = |path: &str, body: &str| {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(full, body).expect("write");
    };
    write("pkg/__init__.py", "");
    write(
        "pkg/util.py",
        "def helper():\n    return 1\n\ndef extra():\n    return 2\n",
    );
    write(
        "pkg/main.py",
        "from pkg.util import helper, extra\n\ndef sweep():\n    helper()\n",
    );
    let ok = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output()
        .expect("git add")
        .status
        .success();
    assert!(ok);
    let ok = Command::new("git")
        .args(["commit", "-qm", "three"])
        .current_dir(dir.path())
        .env("GIT_AUTHOR_NAME", "grid")
        .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
        .env("GIT_COMMITTER_NAME", "grid")
        .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
        .output()
        .expect("git commit")
        .status
        .success();
    assert!(ok);
    dir
}

#[test]
fn scope_example_shape_is_served_over_the_socket() {
    let contract = contract();
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());
    let surveyed = daemon.call("grid.survey.run", &json!({}));
    let snapshot = surveyed["result"]["snapshot_id"].clone();

    let mut params = example_params(&contract, "grid.scope");
    params["callsigns"] = json!(["pkg.util.**"]);
    let response = daemon.call("grid.scope", &params);
    let result = &response["result"];
    assert!(response.get("error").is_none(), "scope errored: {response}");
    let typed: Result<grid_wire::ScopePack, _> = serde_json::from_value(result.clone());
    let pack = typed.expect("the answer is a wire ScopePack, not a lookalike");
    assert_eq!(pack.snapshot.0, snapshot.as_str().expect("snapshot id"));
    assert_eq!(pack.defs.len(), 2, "helper and extra: {pack:?}");
    // The pack saw the caller reaching in: one inbound proven edge each.
    assert_eq!(pack.in_edges.len(), 1, "sweep reaches helper: {pack:?}");
}

#[test]
fn todo_is_the_gate_verdict_as_a_worklist_over_the_socket() {
    let contract = contract();
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());
    let surveyed = daemon.call("grid.survey.run", &json!({}));
    let base = surveyed["result"]["snapshot_id"].clone();
    write_track(
        dir.path(),
        "todo",
        &json!({
            "track_id": "track:todo/v1",
            "base": {"snapshot": base},
            "delta": {
                "defs": [{"action": "modify", "callsign": "pkg.main.sweep"}],
                "edges": [{"action": "add", "src": "pkg.main.sweep",
                           "relation": "calls", "dst": "pkg.util.extra"}]
            }
        }),
    );

    let mut params = example_params(&contract, "grid.todo");
    params["track"] = json!("track:todo");
    let response = daemon.call("grid.todo", &params);
    let result = &response["result"];
    assert!(response.get("error").is_none(), "todo errored: {response}");
    let typed: Result<grid_wire::TodoList, _> = serde_json::from_value(result.clone());
    let list = typed.expect("the answer is a wire TodoList");
    // Nothing built yet: everything pending, fraction at zero.
    assert_eq!(list.edges_pending.len(), 1);
    assert_eq!(list.defs_pending.len(), 1);
    assert_eq!(list.fraction.num, 0);
    assert_eq!(list.fraction.den, 2);
}

#[test]
fn edit_apply_example_is_served_and_conflicts_are_typed_over_the_socket() {
    let contract = contract();
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());
    daemon.call("grid.survey.run", &json!({}));

    // The datum the example's handle pins, for the fixture's own `sweep`.
    let resolved = daemon.call(
        "grid.resolve",
        &json!({"locator": {"callsign": "pkg.main.sweep"}}),
    );
    let base = resolved["result"][0]["datum"].as_str().expect("datum");

    let mut params = example_params(&contract, "grid.edit.apply");
    params["edits"][0]["target"] = json!("pkg.main.sweep");
    params["edits"][0]["base_datum"] = json!(base);
    params["edits"][0]["new_text"] = json!("def sweep():\n    helper()\n    extra()\n");
    let response = daemon.call("grid.edit.apply", &params);
    let result = &response["result"];
    assert!(
        response.get("error").is_none(),
        "edit.apply errored: {response}"
    );
    let typed: Result<grid_wire::EditResult, _> = serde_json::from_value(result.clone());
    let applied = typed.expect("the answer is a wire EditResult");
    assert_eq!(applied.applied.len(), 1, "one edit landed: {applied:?}");
    assert!(applied.conflicts.is_empty());

    // Replaying the same edit is now stale: the answer is a typed conflict
    // whose `now` is the identity the first apply returned.
    let replay = daemon.call("grid.edit.apply", &params);
    let conflict: grid_wire::EditResult =
        serde_json::from_value(replay["result"].clone()).expect("typed conflict");
    assert!(conflict.applied.is_empty());
    assert_eq!(conflict.conflicts.len(), 1);
    assert_eq!(
        conflict.conflicts[0].now, applied.applied[0],
        "the conflict's now is the datum the first edit produced"
    );
}

#[test]
fn the_agent_loop_runs_over_the_socket_with_no_human_file_edits() {
    // The wave's accept criterion: plan, track, implement via edit.apply,
    // gate — the whole loop as socket calls.
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());

    // Plan: survey, author the track against that snapshot, lint it.
    let base = daemon.call("grid.survey.run", &json!({}))["result"]["snapshot_id"].clone();
    let track = json!({
        "track_id": "track:loop-v1",
        "base": {"snapshot": base},
        "delta": {
            "defs": [{"action": "modify", "callsign": "pkg.main.sweep"}],
            "edges": [{"action": "add", "src": "pkg.main.sweep",
                       "relation": "calls", "dst": "pkg.util.extra"}]
        }
    });
    let lint = daemon.call("grid.track.lint", &json!({"track": track}));
    assert!(
        lint["result"]["errors"]
            .as_array()
            .expect("errors")
            .is_empty(),
        "the plan is possible: {lint}"
    );

    // Gate before implementing: hold, with the promised edge itemized.
    let held = daemon.call(
        "grid.gate.check",
        &json!({"track": track, "head": {"snapshot": base}}),
    );
    assert_eq!(held["result"]["gate"], "hold");
    assert_eq!(
        held["result"]["missing"].as_array().expect("missing").len(),
        1
    );

    // Implement: one CAS edit, no human file edits.
    let resolved = daemon.call(
        "grid.resolve",
        &json!({"locator": {"callsign": "pkg.main.sweep"}}),
    );
    let base_datum = resolved["result"][0]["datum"].as_str().expect("datum");
    let applied = daemon.call(
        "grid.edit.apply",
        &json!({"edits": [{
            "target": "pkg.main.sweep",
            "base_datum": base_datum,
            "new_text": "def sweep():\n    helper()\n    extra()\n"
        }]}),
    );
    assert_eq!(
        applied["result"]["applied"]
            .as_array()
            .expect("applied")
            .len(),
        1,
        "the edit landed: {applied}"
    );

    // Gate at the resurveyed worktree: pass, and the worklist agrees.
    let resurveyed = daemon.call("grid.survey.run", &json!({}))["result"]["snapshot_id"].clone();
    let passed = daemon.call(
        "grid.gate.check",
        &json!({"track": track, "head": {"snapshot": resurveyed}}),
    );
    assert_eq!(
        passed["result"]["gate"], "pass",
        "the promised edge is now made good: {passed}"
    );
    write_track(dir.path(), "loop", &track);
    let todo = daemon.call("grid.todo", &json!({"track": "track:loop"}));
    assert_eq!(todo["result"]["fraction"], json!({"num": 2, "den": 2}));
}

fn wake_json(wake_id: &str, hops: &[(&str, &str)]) -> Value {
    json!({
        "wake_id": wake_id,
        "git_rev": "4f2c9e1",
        "observations": hops.iter().map(|(caller, callee)| json!({
            "caller": { "datum": caller },
            "callee": { "datum": callee },
        })).collect::<Vec<_>>(),
    })
}
#[test]
#[allow(clippy::similar_names)] // caller/callee are the domain words
fn wake_ingest_and_diff_are_served_over_the_socket() {
    let dir = repo();
    seed_chart(dir.path());
    let daemon = Daemon::start(dir.path());
    let (caller, callee) = (
        "b3:1111111111111111111111111111111111111111111111111111111111111111",
        "b3:2222222222222222222222222222222222222222222222222222222222222222",
    );
    // A symbol endpoint resolves against the same chart `grid.resolve` uses.
    let response = daemon.call(
        "grid.wake.ingest",
        &json!({ "wake": {
            "wake_id": "wake:g",
            "git_rev": "4f2c9e1",
            "observations": [{ "caller": { "symbol": "caller" }, "callee": { "datum": callee } }],
        }}),
    );
    let result = &response["result"];
    assert_eq!(result["resolved"], 1, "got {response}");
    assert_eq!(result["observed_edges"], 1);
    assert_eq!(result["unresolved_symbols"], json!([]));
    // Red calls back; the diff must find the fork and the added edge.
    daemon.call(
        "grid.wake.ingest",
        &json!({ "wake": wake_json("wake:r", &[(caller, callee), (callee, caller)]) }),
    );
    let response = daemon.call(
        "grid.wake.diff",
        &json!({ "green": "wake:g", "red": "wake:r" }),
    );
    let diff = &response["result"];
    // After the shared first hop, red continues callee -> caller: the fork's
    // caller is the callee, and its red callee is the original caller.
    assert_eq!(diff["first_divergence"]["caller"], callee, "got {response}");
    assert_eq!(diff["first_divergence"]["red_callee"], caller);
    assert!(
        diff["first_divergence"].get("green_callee").is_none(),
        "green had no second hop: {response}"
    );
    assert_eq!(diff["added"].as_array().expect("added").len(), 1);
    assert_eq!(diff["added"][0]["zoc"], "observed");
    assert_eq!(diff["removed"].as_array().expect("removed").len(), 0);
}
#[test]
fn an_unknown_wake_id_is_the_contracts_error() {
    let dir = repo();
    seed_chart(dir.path());
    let daemon = Daemon::start(dir.path());
    let response = daemon.call(
        "grid.wake.diff",
        &json!({ "green": "wake:nope", "red": "wake:alsono" }),
    );
    assert_eq!(
        response["error"]["code"],
        serde_json::json!(-32008),
        "got {response}"
    );
}
#[test]
fn ingest_resolved_nothing_is_counts_not_silence() {
    let dir = repo();
    seed_chart(dir.path());
    let daemon = Daemon::start(dir.path());
    let response = daemon.call(
        "grid.wake.ingest",
        &json!({ "wake": {
            "wake_id": "wake:u",
            "git_rev": "4f2c9e1",
            "observations": [
                { "caller": { "symbol": "no.such" }, "callee": { "symbol": "nor.this" } },
            ],
        }}),
    );
    let result = &response["result"];
    assert_eq!(result["resolved"], 0, "got {response}");
    assert_eq!(result["observed_edges"], 0);
    assert_eq!(result["unresolved_symbols"], json!(["no.such", "nor.this"]));
}

/// Wave 9: the leadline walk, served. The fixture's proven edge is
/// `pkg.main.sweep -> pkg.util.helper`, so an inbound walk from the callee
/// must reach the caller and answer with a wire `LeadlineResult`.
#[test]
fn query_walk_answers_with_a_leadline_result_the_contract_accepts() {
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());
    let snapshot = daemon.call("grid.survey.run", &json!({}))["result"]["snapshot_id"].clone();

    let response = daemon.call(
        "grid.query.walk",
        &json!({"query": {
            "from": [{"callsign": "pkg.util.helper"}],
            "direction": "in",
            "at_or_above": "proven",
            "fuel": {"max_hops": 4},
        }}),
    );
    assert!(response.get("error").is_none(), "walk errored: {response}");
    let typed: Result<grid_wire::LeadlineResult, _> =
        serde_json::from_value(response["result"].clone());
    let walk = typed.expect("the answer is a wire LeadlineResult, not a lookalike");
    assert_eq!(walk.snapshot.0, snapshot.as_str().expect("snapshot id"));
    let reached = walk
        .reached
        .expect("a reachability sweep reports what it reached");
    assert!(
        reached
            .iter()
            .any(|r| r.callsign.as_ref().is_some_and(|c| c.0 == "pkg.main.sweep")),
        "sweep calls helper, so it is inbound-reachable: {reached:?}"
    );
}

/// Params are typed at the door: an unknown field is -32602, not a walk over
/// a query the caller did not write.
#[test]
fn query_walk_rejects_params_the_wire_does_not_define() {
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());
    daemon.call("grid.survey.run", &json!({}));

    let response = daemon.call(
        "grid.query.walk",
        &json!({"query": {"from": [{"callsign": "pkg.util.helper"}], "depth": 3}}),
    );
    assert_eq!(response["error"]["code"], -32602, "got {response}");
    assert!(response.get("result").is_none());
}

/// Wave 9: the reviewer's bundle, served. Same track the gate holds on, so
/// the pack's verdict is the gate's.
#[test]
fn review_pack_answers_with_a_review_pack_the_contract_accepts() {
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());
    let base = daemon.call("grid.survey.run", &json!({}))["result"]["snapshot_id"].clone();
    let track_value = json!({
        "track_id": "track:review/v1",
        "base": {"snapshot": base},
        "delta": {
            "defs": [{"action": "modify", "callsign": "pkg.main.sweep"}],
            "edges": [{"action": "add", "src": "pkg.main.sweep",
                       "relation": "calls", "dst": "pkg.util.extra"}]
        }
    });
    let track = write_track(dir.path(), "review", &track_value);

    let response = daemon.call(
        "grid.review.pack",
        &json!({"track": track, "head": {"snapshot": base}}),
    );
    assert!(response.get("error").is_none(), "pack errored: {response}");
    let typed: Result<grid_wire::ReviewPack, _> =
        serde_json::from_value(response["result"].clone());
    let pack = typed.expect("the answer is a wire ReviewPack, not a lookalike");
    // Nothing was implemented, so the gate holds and the promise is unmet:
    // a pack never disagrees with `grid.gate.check` about that.
    let gate = daemon.call(
        "grid.gate.check",
        &json!({"track": track_value, "head": {"snapshot": base}}),
    );
    assert_eq!(
        serde_json::to_value(&pack.verdict).expect("verdict"),
        gate["result"],
        "the pack's verdict is the gate's"
    );
}

/// A wake the log does not hold is the contract's unknown-wake error, not a
/// pack computed as if coverage had been asked for and found empty.
#[test]
fn review_pack_with_an_unknown_wake_is_the_contracts_error() {
    let dir = repo_with_an_unused_import();
    let daemon = Daemon::start(dir.path());
    let base = daemon.call("grid.survey.run", &json!({}))["result"]["snapshot_id"].clone();
    let track = write_track(
        dir.path(),
        "review-wake",
        &json!({
            "track_id": "track:review-wake/v1",
            "base": {"snapshot": base},
            "delta": {"defs": [{"action": "modify", "callsign": "pkg.main.sweep"}], "edges": []}
        }),
    );

    let response = daemon.call(
        "grid.review.pack",
        &json!({"track": track, "head": {"snapshot": base}, "wake": "wake:nope"}),
    );
    assert_eq!(response["error"]["code"], -32008, "got {response}");
    assert!(response.get("result").is_none());
}
