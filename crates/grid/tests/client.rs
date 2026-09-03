//! C53: `grid-client` against the daemon it was written for.
//!
//! Three questions the fake daemon in `crates/client/tests/protocol.rs`
//! cannot answer, because a fake answers whatever it was scripted to.
//! Does a typed call return what the CLI prints for the same question — the
//! CLI runs every verb in-process through the same `rpc::*` functions the
//! daemon dispatches to and opens no socket, so the comparison isolates
//! exactly what the SDK adds: framing, encode, match, decode. Is every
//! method the contract names answered over the socket. And does C2's
//! determinism survive the round trip.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Child, Command, Stdio};

use grid_client::{Client, Error};
use grid_wire::methods::{
    DeltaParams, DriftParams, QueryWalkParams, ResolveParams, ScopeParams, SurveyRunParams,
    VersionParams, METHODS,
};
use grid_wire::{CallsignPattern, GitRev, Locator, SnapshotOrRev, WalkQuery};
use serde_json::Value;

/// Contract methods `serve.rs` does not dispatch, so they answer `-32601`
/// (docs/TODOS.md T-64). A ratchet, not an allowlist: a method listed here
/// that starts being answered fails this test, so the list only shrinks.
const UNSERVED: [&str; 1] = ["grid.plan.estimate"];

const METHOD_NOT_FOUND: i32 = -32601;

/// The definition every differential asks about. It sits in a subdirectory
/// because `grid resolve`'s `path:line` locator is told apart from a pinned
/// name by the slash, so a top-level `a.py:1` would resolve as a callsign.
const FILE: &str = "src/a.py";

fn contract() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/wire-v1.openrpc.json"
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

/// A daemon on its own socket in its own directory, killed on drop so a
/// failed assertion cannot leak a process into the next test run.
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
        // so the file appearing is a promise, not an answer.
        for _ in 0..200 {
            if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
                return Self { child, socket };
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        // Reap before failing: a daemon that never accepted is still a child.
        let _ = child.kill();
        let _ = child.wait();
        panic!("daemon never accepted on {}", socket.display());
    }

    fn client(&self) -> Client {
        Client::connect(&self.socket).expect("connect to the daemon")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A repo with two commits, so `diff` and `drift` have a real delta.
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
    std::fs::create_dir(dir.path().join("src")).expect("mkdir");
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join(FILE), "def a():\n    pass\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "one"]);
    std::fs::write(dir.path().join("src/b.py"), "def b():\n    a()\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "two"]);
    dir
}

/// One CLI verb's `--json` stdout, parsed. The verb runs in-process against
/// the same store the daemon serves.
fn cli(dir: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .arg("--json")
        .current_dir(dir)
        .env("GRID_HINTS", "0")
        .stderr(Stdio::null())
        .output()
        .expect("run grid");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "grid {args:?} --json: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

/// A typed client result as JSON, so it can be compared with what the CLI
/// serialized from the same type.
fn typed<T: serde::Serialize>(result: Result<T, Error>) -> Value {
    serde_json::to_value(result.expect("client call")).expect("serialize the typed result")
}

/// The comparison surface contract convention 4 defines: the `stats` object
/// is excluded, everywhere it appears, and nothing else is.
fn without_stats(value: &Value) -> Value {
    match value {
        Value::Object(fields) => fields
            .iter()
            .filter(|(name, _)| name.as_str() != "stats")
            .map(|(name, v)| (name.clone(), without_stats(v)))
            .collect::<serde_json::Map<String, Value>>()
            .into(),
        Value::Array(items) => items.iter().map(without_stats).collect(),
        scalar => scalar.clone(),
    }
}

#[test]
fn the_client_answers_what_the_cli_prints() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let mut client = daemon.client();
    client
        .call(&SurveyRunParams {
            git_rev: None,
            worktree: None,
        })
        .expect("survey the fixture");

    assert_eq!(
        without_stats(&cli(dir.path(), &["version"])),
        without_stats(&typed(client.call(&VersionParams {}))),
        "grid version"
    );

    let at_line = format!("{FILE}:1");
    let defs = client
        .call(&ResolveParams {
            locator: Locator::FileLine {
                file: FILE.to_owned(),
                line: 1,
            },
            snapshot: None,
        })
        .expect("resolve");
    assert_eq!(
        without_stats(&cli(dir.path(), &["resolve", &at_line])),
        without_stats(&serde_json::to_value(&defs).expect("serialize")),
        "grid resolve"
    );

    // The callsign comes from the answer above rather than from a guess at
    // how the Python frontend spells one: the differential is about the two
    // paths agreeing, not about this fixture's naming.
    let callsign = defs
        .first()
        .and_then(|d| d.callsign.clone())
        .expect("the charted definition has a callsign");

    let rev = |s: &str| SnapshotOrRev::GitRev {
        git_rev: GitRev(s.to_owned()),
    };
    assert_eq!(
        without_stats(&cli(dir.path(), &["diff"])),
        without_stats(&typed(client.call(&DeltaParams {
            base: rev("HEAD~1"),
            head: rev("HEAD"),
        }))),
        "grid diff"
    );

    assert_eq!(
        without_stats(&cli(dir.path(), &["drift"])),
        without_stats(&typed(client.call(&DriftParams {
            base: rev("HEAD~1"),
            head: rev("HEAD"),
            floor: None,
            plan: None,
            tests: None,
            compat: None,
        }))),
        "grid drift"
    );

    assert_eq!(
        without_stats(&cli(dir.path(), &["scope", &callsign.0])),
        without_stats(&typed(client.call(&ScopeParams {
            callsigns: vec![CallsignPattern(callsign.0.clone())],
            depth: None,
            snapshot: None,
            bodies: None,
            sigs: None,
            budget: None,
            since: None,
        }))),
        "grid scope"
    );
}

#[test]
fn every_contract_method_is_answered_over_the_socket() {
    let contract = contract();
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let mut client = daemon.client();

    for method in METHODS {
        let answer = client.call_raw(method, &example_params(&contract, method));
        // Most of these refuse against a fixture with no snapshot, no plan
        // and no trace. A refusal is an answer; only -32601 says the daemon
        // does not know the method at all.
        let unknown = match &answer {
            Ok(_) => false,
            Err(Error::Rpc(e)) => e.code == METHOD_NOT_FOUND,
            Err(fault) => panic!("{method}: {fault}"),
        };
        if UNSERVED.contains(&method) {
            assert!(
                unknown,
                "{method} is on UNSERVED but the daemon answered it — shrink the list"
            );
        } else {
            assert!(!unknown, "{method}: the daemon dispatches no such method");
        }
    }
}

#[test]
fn the_same_pinned_walk_answers_the_same_thing_twice() {
    let dir = repo();
    let daemon = Daemon::start(dir.path());
    let mut client = daemon.client();
    let published = client
        .call(&SurveyRunParams {
            git_rev: None,
            worktree: None,
        })
        .expect("survey the fixture");

    // C2's own conditions: the snapshot pinned, and no `fuel.max_ms`, whose
    // wall-clock exhaustion the contract excludes from determinism.
    let query = WalkQuery {
        from: vec![Locator::FileLine {
            file: FILE.to_owned(),
            line: 1,
        }],
        to: None,
        direction: None,
        relations: None,
        at_or_above: None,
        fuel: None,
        snapshot: Some(published.snapshot_id),
    };
    let first = typed(client.call(&QueryWalkParams {
        query: query.clone(),
    }));
    let second = typed(client.call(&QueryWalkParams { query }));
    assert_eq!(
        without_stats(&first),
        without_stats(&second),
        "the same pinned walk answered differently through the client"
    );
}
