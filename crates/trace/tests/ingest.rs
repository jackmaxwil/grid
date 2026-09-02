//! C16: an ingested trace yields observed edges only where both endpoints
//! resolve; every endpoint that does not resolve is returned, never dropped.
//!
//! The claim is the honesty of the `observed` evidence grade: an `observed`
//! edge must mean "these two datums were seen calling, at least once", and
//! the unresolved bucket must mean "this is everything we could not point
//! at". One direction without the other is a trace that lies by omission.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use grid_chart::{build, DatumId, Def, Segment};
use grid_trace::{ingest, Store};
use grid_wire::{Lang, Trace, TraceEndpoint, TraceObservation};
/// Digest `n`: distinct datums that are cheap to talk about in a failure.
fn datum(n: u8) -> DatumId {
    let mut bytes = [0u8; 32];
    bytes[0] = n;
    DatumId(bytes)
}
fn handle(n: u8) -> TraceEndpoint {
    TraceEndpoint::Datum {
        datum: grid_wire::Datum(datum(n).to_string()),
    }
}
fn symbol(name: &str) -> TraceEndpoint {
    TraceEndpoint::Symbol {
        symbol: name.to_string(),
    }
}
fn hop(from: TraceEndpoint, to: TraceEndpoint, count: Option<u64>) -> TraceObservation {
    TraceObservation {
        caller: from,
        callee: to,
        count,
    }
}
fn trace(hops: Vec<TraceObservation>) -> Trace {
    Trace {
        trace_id: "trace:test".to_string(),
        git_rev: grid_wire::GitRev("4f2c9e1".to_string()),
        command: None,
        started_at: None,
        snapshot: None,
        observations: hops,
    }
}
/// No chart at all: datum handles resolve without one.
fn no_chart() -> Vec<Segment<Vec<u8>>> {
    Vec::new()
}
/// A chart of three defs: `app.main` (1), `app.helper` (2), `lib.helper` (3).
fn segments() -> Vec<Segment<Vec<u8>>> {
    let image = build(
        &[
            Def {
                datum: datum(1),
                callsign: "app.main".into(),
                lang: Some(Lang::Py),
                facts: None,
            },
            Def {
                datum: datum(2),
                callsign: "app.helper".into(),
                lang: Some(Lang::Py),
                facts: None,
            },
            Def {
                datum: datum(3),
                callsign: "lib.helper".into(),
                lang: Some(Lang::Py),
                facts: None,
            },
        ],
        &[],
    )
    .expect("build segment");
    vec![Segment::parse(image).expect("parse segment")]
}
#[test]
fn datum_endpoints_resolve_with_no_chart_at_all() {
    let w = trace(vec![hop(handle(1), handle(2), None)]);
    let got = ingest(&no_chart(), &w);
    assert_eq!(got.resolved, 1);
    assert!(got.unresolved_symbols.is_empty());
    assert_eq!(got.edges.len(), 1);
    assert_eq!(got.edges[0].caller, datum(1));
    assert_eq!(got.edges[0].callee, datum(2));
    assert_eq!(got.edges[0].count, 1, "a hop without a count happened once");
}
#[test]
fn a_symbol_resolves_by_unique_dotted_suffix() {
    let w = trace(vec![
        hop(symbol("main"), symbol("app.helper"), None),
        hop(symbol("app.main"), symbol("lib.helper"), None),
    ]);
    let got = ingest(&segments(), &w);
    assert_eq!(got.resolved, 2, "got {got:?}");
    assert!(got.unresolved_symbols.is_empty());
    assert_eq!(got.edges.len(), 2);
}
#[test]
fn a_suffix_that_starts_mid_name_is_not_a_name() {
    // `per` is a suffix of `app.helper` the string but not the name: matching
    // it would resolve a trace to a definition it never named.
    let w = trace(vec![hop(symbol("per"), handle(1), None)]);
    let got = ingest(&segments(), &w);
    assert_eq!(got.resolved, 0);
    assert_eq!(got.unresolved_symbols, ["per"]);
    assert!(got.edges.is_empty(), "half an edge is not an edge");
}
#[test]
fn an_ambiguous_symbol_yields_no_edge_and_is_returned() {
    // `helper` names both app.helper and lib.helper; picking one would invent
    // an observation the trace never made.
    let w = trace(vec![hop(symbol("helper"), handle(1), None)]);
    let got = ingest(&segments(), &w);
    assert_eq!(got.resolved, 0);
    assert_eq!(got.unresolved_symbols, ["helper"]);
    assert!(got.edges.is_empty());
}
#[test]
fn unknown_symbols_and_bad_handles_are_returned_sorted_and_deduped() {
    let w = trace(vec![
        hop(symbol("no.such"), symbol("also.missing"), None),
        hop(symbol("no.such"), handle(1), None),
        hop(
            TraceEndpoint::Datum {
                datum: grid_wire::Datum("not a handle".to_string()),
            },
            handle(1),
            None,
        ),
    ]);
    let got = ingest(&segments(), &w);
    assert_eq!(got.resolved, 0);
    assert_eq!(
        got.unresolved_symbols,
        ["also.missing", "no.such", "not a handle"]
    );
}
#[test]
fn repeated_hops_fold_into_one_edge_with_summed_counts() {
    let w = trace(vec![
        hop(handle(1), handle(2), Some(3)),
        hop(handle(1), handle(2), None),
        hop(handle(2), handle(1), Some(2)),
    ]);
    let got = ingest(&no_chart(), &w);
    assert_eq!(got.resolved, 3);
    assert_eq!(got.edges.len(), 2);
    // Edges are sorted by (caller, callee): ingestion of the same trace always
    // renders the same list.
    assert_eq!(got.edges[0].caller, datum(1));
    assert_eq!(got.edges[0].count, 4, "counts sum, got {got:?}");
    assert_eq!(got.edges[1].caller, datum(2));
    assert_eq!(got.edges[1].count, 2);
}
#[test]
fn retention_round_trips_and_replaces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path().join("traces")).expect("open store");
    let w = trace(vec![hop(handle(1), handle(2), Some(2))]);
    let first = ingest(&no_chart(), &w);
    store.put(&w, &first).expect("put");
    let loaded = store.get("trace:test").expect("get").expect("retained");
    assert_eq!(loaded.trace, w);
    assert_eq!(loaded.hops, vec![(datum(1), datum(2))]);
    // Same id, new run: the later report of the run is the one to believe.
    let w2 = Trace {
        observations: vec![hop(handle(3), handle(1), None)],
        ..trace(vec![])
    };
    store.put(&w2, &ingest(&no_chart(), &w2)).expect("replace");
    let loaded = store.get("trace:test").expect("get").expect("retained");
    assert_eq!(loaded.hops, vec![(datum(3), datum(1))]);
}
#[test]
fn an_unknown_wake_id_is_none_not_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path()).expect("open store");
    assert_eq!(store.get("trace:nope").expect("get"), None);
}
#[test]
fn a_wake_id_is_never_a_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path().join("traces")).expect("open store");
    let mut w = trace(vec![hop(handle(1), handle(2), None)]);
    w.trace_id = "../../etc/passwd".to_string();
    store.put(&w, &ingest(&no_chart(), &w)).expect("put");
    assert_eq!(
        store.get("../../etc/passwd").expect("get"),
        Some(grid_trace::Stored {
            trace: w,
            hops: vec![(datum(1), datum(2))],
        })
    );
    let entries = std::fs::read_dir(dir.path().join("traces"))
        .expect("read_dir")
        .count();
    assert_eq!(entries, 1, "the id stayed out of the filesystem shape");
}
#[test]
fn a_corrupt_record_is_an_error_not_a_silent_miss() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path().join("traces")).expect("open store");
    let w = trace(vec![hop(handle(1), handle(2), None)]);
    store.put(&w, &ingest(&no_chart(), &w)).expect("put");
    // Corrupt whatever file the id produced: one trace in, one file.
    let files = std::fs::read_dir(dir.path().join("traces"))
        .expect("read_dir")
        .map(|e| e.expect("entry").path())
        .collect::<Vec<_>>();
    std::fs::write(files.first().expect("one trace file"), b"{ not json").expect("corrupt");
    let err = store.get("trace:test").expect_err("corrupt is not a trace");
    assert!(
        matches!(err, grid_trace::StoreError::Corrupt(_)),
        "got {err}"
    );
}
