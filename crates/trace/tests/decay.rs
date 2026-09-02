//! Observed-edge decay: an edge is an observation only while the code it
//! observed still exists.
//!
//! A trace taken at snapshot S asserts coupling *as the code was at S*. Ask
//! about a chart where either endpoint has since been edited and the edge is
//! a claim about code that is not there. Left graded `observed` it would rot
//! into false positives within weeks — which is why the grade is recomputed
//! per question and the retained log is never rewritten.
use grid_chart::{build, DatumId, Def, DefFacts, Segment};
use grid_trace::{grade, ObservedEdge, Store, STALE_TRACE};
use grid_wire::{Grade, Trace, TraceEndpoint, TraceObservation};

fn def(callsign: &str, fill: u8) -> Def {
    Def {
        datum: DatumId([fill; 32]),
        callsign: callsign.to_owned(),
        lang: Some(grid_wire::Lang::Py),
        facts: Some(DefFacts {
            path: "m.py".into(),
            line: 1,
            line_end: 1,
            span: (0, 1),
            shape: [fill; 32],
            complexity: 1,
            sig: [fill; 32],
            cfg: String::new(),
        }),
    }
}

fn chart(defs: &[Def]) -> Segment<Vec<u8>> {
    Segment::parse(build(defs, &[]).expect("image")).expect("parse")
}

fn edge(from: u8, to: u8) -> ObservedEdge {
    ObservedEdge {
        caller: DatumId([from; 32]),
        callee: DatumId([to; 32]),
        count: 1,
    }
}

#[test]
fn an_edge_whose_endpoints_both_still_exist_stays_observed() {
    let segments = [chart(&[def("m.caller", 1), def("m.callee", 2)])];
    let graded = grade(&segments, edge(1, 2));
    assert_eq!(graded.grade, Grade::Observed);
    assert_eq!(graded.reason, None);
    assert_eq!(graded.edge, edge(1, 2), "grading never rewrites the edge");
}

#[test]
fn an_edited_endpoint_demotes_the_edge_with_a_named_reason() {
    // The callee's body moved, so its datum moved: the trace describes a
    // definition this chart does not hold.
    let segments = [chart(&[def("m.caller", 1), def("m.callee", 9)])];
    let graded = grade(&segments, edge(1, 2));
    assert_eq!(graded.grade, Grade::Heuristic);
    assert_eq!(graded.reason, Some(STALE_TRACE));

    // Symmetrically for the caller: either endpoint is enough.
    let segments = [chart(&[def("m.caller", 9), def("m.callee", 2)])];
    assert_eq!(grade(&segments, edge(1, 2)).grade, Grade::Heuristic);
}

/// Conservatism: an endpoint no chart ever held — the "old trace with unknown
/// datums" case, and equally a trace that named a raw datum handle out of
/// nowhere — is demoted, never trusted. An endpoint that cannot be verified
/// is not verified.
#[test]
fn an_unverifiable_endpoint_is_demoted_not_trusted() {
    let segments = [chart(&[def("m.caller", 1)])];
    let graded = grade(&segments, edge(1, 200));
    assert_eq!(graded.grade, Grade::Heuristic);
    assert_eq!(graded.reason, Some(STALE_TRACE));

    // And with nothing in the chart at all, which is what an empty store
    // looks like: still demoted, never silently observed.
    let empty: [Segment<Vec<u8>>; 0] = [];
    assert_eq!(grade(&empty, edge(1, 2)).grade, Grade::Heuristic);
}

/// Decay is a property of the question, not of the log: the same edge is
/// `observed` against one chart and `heuristic` against another, and the
/// retained trace is untouched either way.
#[test]
fn the_same_edge_grades_differently_against_two_charts() {
    let before = [chart(&[def("m.caller", 1), def("m.callee", 2)])];
    let after = [chart(&[def("m.caller", 1), def("m.callee", 3)])];
    assert_eq!(grade(&before, edge(1, 2)).grade, Grade::Observed);
    assert_eq!(grade(&after, edge(1, 2)).grade, Grade::Heuristic);
}

fn trace(id: &str, hops: &[(&str, &str)]) -> Trace {
    Trace {
        trace_id: id.to_owned(),
        git_rev: grid_wire::GitRev("rev".into()),
        command: None,
        started_at: None,
        snapshot: None,
        observations: hops
            .iter()
            .map(|(from, to)| TraceObservation {
                caller: TraceEndpoint::Symbol {
                    symbol: (*from).to_string(),
                },
                callee: TraceEndpoint::Symbol {
                    symbol: (*to).to_string(),
                },
                count: None,
            })
            .collect(),
    }
}

#[test]
fn retained_wakes_come_back_in_a_deterministic_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path()).expect("store");
    let segments = [chart(&[def("m.caller", 1), def("m.callee", 2)])];

    for id in ["trace:b", "trace:a"] {
        let w = trace(id, &[("m.caller", "m.callee")]);
        let ingested = grid_trace::ingest(&segments, &w);
        assert_eq!(ingested.resolved, 1, "the fixture's hop must resolve");
        store.put(&w, &ingested).expect("retain");
    }

    let retained = store.retained().expect("read back");
    let ids: Vec<&str> = retained.iter().map(|s| s.trace.trace_id.as_str()).collect();
    assert_eq!(
        ids,
        ["trace:a", "trace:b"],
        "sorted by id, not by directory"
    );
    assert_eq!(retained[0].edges(), vec![edge(1, 2)]);
}

/// A store nobody has written to holds no traces; a store whose log does not
/// parse is *suspect*, not empty. Answering "no observations" for a broken
/// store would be a fabricated all-clear.
#[test]
fn an_empty_store_is_empty_and_a_corrupt_one_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path().join("never-written")).expect("store");
    assert!(store.retained().expect("empty is not an error").is_empty());

    let broken = tempfile::tempdir().expect("tempdir");
    std::fs::write(broken.path().join("x.json"), b"{ not a record }").expect("write");
    let store = Store::open(broken.path()).expect("store");
    assert!(
        store.retained().is_err(),
        "a corrupt log is not an empty one"
    );
}
