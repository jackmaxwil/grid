//! `grid.wake.diff`: two traces compared at call granularity.
//!
//! The first divergent hop is where behavior forked — the answer to "the
//! same test passed green and failed red, where did they part". The edge
//! sets answer the coarser question: what does each trace do that the other
//! never did.
use grid_chart::DatumId;
use grid_wire::{Wake, WakeEndpoint, WakeObservation, Zoc};
fn datum(n: u8) -> DatumId {
    let mut bytes = [0u8; 32];
    bytes[0] = n;
    DatumId(bytes)
}
fn no_chart() -> Vec<grid_chart::Segment<Vec<u8>>> {
    Vec::new()
}
fn stored(id: &str, hops: &[(u8, u8)]) -> grid_wake::Stored {
    let observations = hops
        .iter()
        .map(|(caller, callee)| WakeObservation {
            caller: WakeEndpoint::Datum {
                datum: grid_wire::Datum(datum(*caller).to_string()),
            },
            callee: WakeEndpoint::Datum {
                datum: grid_wire::Datum(datum(*callee).to_string()),
            },
            count: None,
        })
        .collect();
    let wake = Wake {
        wake_id: id.to_string(),
        git_rev: grid_wire::GitRev("4f2c9e1".to_string()),
        command: None,
        started_at: None,
        observations,
    };
    let ingested = grid_wake::ingest(&no_chart(), &wake);
    grid_wake::Stored {
        wake,
        trace: ingested.trace().to_vec(),
    }
}
#[test]
fn identical_traces_have_no_divergence_and_no_edge_changes() {
    let green = stored("wake:g", &[(1, 2), (2, 3)]);
    let red = stored("wake:r", &[(1, 2), (2, 3)]);
    let diff = grid_wake::diff(&green, &red);
    assert_eq!(diff.first_divergence, None);
    assert!(diff.added.is_empty());
    assert!(diff.removed.is_empty());
}
#[test]
fn the_first_divergence_is_where_the_traces_part() {
    // Same first hop, then green calls 3 where red calls 4.
    let green = stored("wake:g", &[(1, 2), (1, 3), (1, 2)]);
    let red = stored("wake:r", &[(1, 2), (1, 4), (1, 5)]);
    let diff = grid_wake::diff(&green, &red);
    let fork = diff.first_divergence.expect("the traces part at hop 1");
    assert_eq!(fork.caller.0, datum(1).to_string());
    assert_eq!(
        fork.green_callee.expect("green called").0,
        datum(3).to_string()
    );
    assert_eq!(fork.red_callee.expect("red called").0, datum(4).to_string());
    // The second difference (hop 2) is not reported: first means first.
    assert_eq!(diff.added.len(), 2, "red's (1,4) and (1,5)");
    assert_eq!(diff.removed.len(), 1, "green's (1,3); (1,2) is shared");
}
#[test]
fn a_trace_that_runs_long_forks_at_the_exhausted_hop() {
    let green = stored("wake:g", &[(1, 2)]);
    let red = stored("wake:r", &[(1, 2), (2, 3)]);
    let diff = grid_wake::diff(&green, &red);
    let fork = diff.first_divergence.expect("red kept going");
    assert_eq!(fork.caller.0, datum(2).to_string());
    assert_eq!(fork.green_callee, None, "green had no hop there");
    assert_eq!(fork.red_callee.expect("red called").0, datum(3).to_string());
    assert_eq!(diff.added.len(), 1);
    assert!(diff.removed.is_empty());
}
#[test]
fn edge_sets_are_observed_calls_sorted_by_endpoints() {
    let green = stored("wake:g", &[(3, 1), (1, 2)]);
    let red = stored("wake:r", &[(1, 2), (2, 3)]);
    let diff = grid_wake::diff(&green, &red);
    assert_eq!(diff.added.len(), 1);
    let added = &diff.added[0];
    assert_eq!(added.src.0, datum(2).to_string());
    assert_eq!(added.dst.0, datum(3).to_string());
    assert_eq!(added.relation, grid_wire::Relation::Calls);
    assert_eq!(added.zoc, Zoc::Observed, "a wake is observed evidence");
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.removed[0].src.0, datum(3).to_string());
}
#[test]
fn repeated_hops_do_not_multiply_edge_sets() {
    // A loop red runs twice is still one edge green lacks.
    let green = stored("wake:g", &[(1, 2)]);
    let red = stored("wake:r", &[(1, 2), (2, 1), (2, 1)]);
    let diff = grid_wake::diff(&green, &red);
    assert_eq!(diff.added.len(), 1, "got {:?}", diff.added);
}
