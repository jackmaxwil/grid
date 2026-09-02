//! `grid trace export <dir>` / `grid trace import <dir>` at the CLI: the
//! records, and the exit code a refusal carries.
//!
//! The transport itself is proved in `grid-trace`'s own suite. What this
//! proves is the wiring: which store the verb opens, what it prints, and that
//! a refused import is exit 2 — bad input, because a directory and a store
//! disagreeing about what a run did is not code diverging from a promise.
use std::path::Path;
use std::process::Command;

use grid_trace::{ingest, Store};
use grid_wire::{Trace, TraceEndpoint, TraceObservation};

fn trace(id: &str, callee: &str) -> Trace {
    Trace {
        trace_id: id.to_owned(),
        git_rev: grid_wire::GitRev("abc123".into()),
        command: Some("tests::t".into()),
        started_at: None,
        snapshot: None,
        observations: vec![TraceObservation {
            caller: TraceEndpoint::Datum {
                datum: grid_wire::Datum(format!("b3:{}", "1".repeat(64))),
            },
            callee: TraceEndpoint::Datum {
                datum: grid_wire::Datum(callee.to_owned()),
            },
            count: Some(1),
        }],
    }
}

/// Retain `traces` in a repo root's own store, the way `grid test --trace`
/// would have.
fn retain(root: &Path, traces: &[(&str, &str)]) {
    let store = Store::open(root.join(".grid/traces")).expect("store");
    let empty: [grid_chart::Segment<Vec<u8>>; 0] = [];
    for (id, callee) in traces {
        let w = trace(id, callee);
        store.put(&w, &ingest(&empty, &w)).expect("retain");
    }
}

/// The verb, run in `root`. Returns `(code, stdout)`.
fn grid(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(root)
        .env("GRID_HINTS", "0")
        .output()
        .expect("spawn grid");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn traces_travel_as_a_directory_and_a_conflicting_import_is_refused() {
    let ci = tempfile::tempdir().expect("tempdir");
    let artifact = tempfile::tempdir().expect("tempdir");
    let local = tempfile::tempdir().expect("tempdir");
    let out = artifact.path().join("traces");
    let a = format!("b3:{}", "2".repeat(64));
    let b = format!("b3:{}", "3".repeat(64));
    retain(ci.path(), &[("trace:one", &a), ("trace:two", &b)]);

    let (code, stdout) = grid(ci.path(), &["trace", "export", &out.display().to_string()]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("traces 2 exported · 0 unchanged · 0 refused"),
        "{stdout}"
    );

    let (code, stdout) = grid(
        local.path(),
        &["trace", "import", &out.display().to_string()],
    );
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("traces 2 imported · 0 unchanged · 0 refused"),
        "{stdout}"
    );

    // Idempotent: the same directory again writes nothing.
    let (code, stdout) = grid(
        local.path(),
        &["trace", "import", &out.display().to_string()],
    );
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("traces 0 imported · 2 unchanged · 0 refused"),
        "{stdout}"
    );

    // `--json` is the wire result, verbatim: the contract's own type reads
    // it back with unknown fields denied, in the contract's own words.
    let (code, stdout) = grid(
        local.path(),
        &["trace", "import", &out.display().to_string(), "--json"],
    );
    assert_eq!(code, 0, "{stdout}");
    let wire: grid_wire::TraceTransportResult =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout}"));
    assert_eq!(
        wire,
        grid_wire::TraceTransportResult {
            written: Vec::new(),
            unchanged: vec!["trace:one".into(), "trace:two".into()],
            refused: Vec::new(),
        }
    );

    // The far side re-ran `trace:one` and it watched something else. Two
    // records, one id: refused, whole, and the id is on stdout.
    let elsewhere = tempfile::tempdir().expect("tempdir");
    retain(elsewhere.path(), &[("trace:one", &b), ("trace:three", &a)]);
    let (code, _) = grid(
        elsewhere.path(),
        &["trace", "export", &out.display().to_string()],
    );
    assert_eq!(code, 0);
    let (code, stdout) = grid(
        local.path(),
        &["trace", "import", &out.display().to_string()],
    );
    assert_eq!(code, 2, "a refusal is bad input:\n{stdout}");
    assert!(
        stdout.contains("miss    trace:one  differs from the trace already there"),
        "{stdout}"
    );
    assert!(stdout.contains("traces 0 imported"), "{stdout}");
    // And `trace:three`, which conflicted with nothing, was not written
    // either: the refusal is whole.
    let held = Store::open(local.path().join(".grid/traces"))
        .expect("store")
        .retained()
        .expect("read");
    let ids: Vec<&str> = held.iter().map(|s| s.trace.trace_id.as_str()).collect();
    assert_eq!(ids, ["trace:one", "trace:two"], "nothing was added");
}
