//! Certified aground: the radius a budgeted walk *finished*, not just where
//! it stopped.
//!
//! A caller holding a partial answer cannot otherwise tell "every def within
//! three hops is in here" from "the walk stopped mid-layer and even depth 1
//! may be short". The first is usable — a bounded blast radius, a scope pack
//! that admits its horizon — and the second is not.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use grid_chart::{build, DatumId, Def, Edge, Segment};
use grid_walk::fuel::Budget;
use grid_walk::graph::{Direction, Ordinal};
use grid_walk::walk::{walk, Filter, Status, Walk};
use grid_wire::{Grade, Lang, Relation};
use proptest::prelude::*;

fn datum(seed: u8) -> DatumId {
    DatumId([seed; 32])
}

fn def(seed: u8) -> Def {
    Def {
        datum: datum(seed),
        callsign: format!("pkg.mod.f{seed}"),
        lang: Some(Lang::Py),
        facts: None,
    }
}

fn edge(from: u8, to: u8, grade: Grade) -> Edge {
    Edge {
        src: datum(from),
        dst: datum(to),
        relation: Relation::Calls,
        grade,
    }
}

fn fuel(hops: u64, nodes: u64) -> Budget {
    Budget::new(Some(&grid_wire::Fuel {
        max_hops: Some(hops),
        max_nodes: Some(nodes),
        max_ms: None,
    }))
}

/// A chain `0 -> 1 -> 2 -> ...`, so depth is exactly node index and a layer
/// holds exactly one node — which makes "ran out mid-layer" arrangeable.
fn chain(n: u8) -> Segment<Vec<u8>> {
    let defs: Vec<Def> = (0..n).map(def).collect();
    let edges: Vec<Edge> = (0..n.saturating_sub(1))
        .map(|i| edge(i, i + 1, Grade::Proven))
        .collect();
    Segment::parse(build(&defs, &edges).expect("image")).expect("parse")
}

/// A tree whose layer 3 is wide, so a node budget can be made to run out
/// part-way through expanding it.
fn wide_at_three() -> Segment<Vec<u8>> {
    // 0 -> 1 -> 2 -> {3,4,5,6} -> each to a leaf of its own.
    let defs: Vec<Def> = (0..=10).map(def).collect();
    let mut edges = vec![edge(0, 1, Grade::Proven), edge(1, 2, Grade::Proven)];
    for i in 3..=6u8 {
        edges.push(edge(2, i, Grade::Proven));
        edges.push(edge(i, i + 4, Grade::Proven));
    }
    Segment::parse(build(&defs, &edges).expect("image")).expect("parse")
}

fn sweep<B: AsRef<[u8]>>(seg: &Segment<B>, budget: Budget, filter: &Filter) -> Walk {
    walk(seg, &[Ordinal(0)], Direction::Out, filter, budget, |_| {
        false
    })
}

/// The nodes a walk reached at depth `<= d`.
fn within(w: &Walk, d: u64) -> Vec<u32> {
    let mut out: Vec<u32> = w
        .seen
        .iter()
        .filter(|(_, r)| r.depth <= d)
        .map(|(o, _)| o.0)
        .collect();
    out.sort_unstable();
    out
}

/// Ground truth: a node budget that runs out part-way through layer 3
/// certifies 2 — and the depth-<=2 answer is exactly the unbudgeted walk's.
#[test]
fn a_walk_that_runs_out_mid_layer_certifies_the_layer_before_it() {
    let seg = wide_at_three();
    // Layers are {0}, {1}, {2}, {3,4,5,6}: three nodes to finish layer 2,
    // then one more gets part-way into layer 3 and stops.
    let budget = fuel(10, 4);
    let w = sweep(&seg, budget, &Filter::default());
    assert_eq!(w.status, Status::Aground);
    assert_eq!(w.complete_within, Some(2));

    let full = sweep(&seg, fuel(100, 100), &Filter::default());
    assert_eq!(full.status, Status::Complete);
    assert_eq!(
        within(&w, 2),
        within(&full, 2),
        "the certified prefix is the whole truth to that depth"
    );
}

/// A walk whose frontier drains is complete to its whole hop bound, whatever
/// depth it actually reached: there was nothing further to find.
#[test]
fn a_drained_walk_certifies_its_whole_bound() {
    let seg = chain(4);
    let w = sweep(&seg, fuel(9, 100), &Filter::default());
    assert_eq!(w.status, Status::Complete);
    assert_eq!(w.complete_within, Some(9));
}

/// Conservatism: fuel refused expanding the roots' own layer certifies
/// nothing. `Some(0)` would claim a layer finished that never started.
#[test]
fn a_walk_that_never_expanded_a_layer_certifies_nothing() {
    let seg = chain(4);
    // No hop at all.
    let w = sweep(&seg, fuel(0, 100), &Filter::default());
    assert_eq!(w.status, Status::Aground);
    assert_eq!(w.complete_within, None);

    // And no node budget either: the hop is charged, the first expansion is
    // refused, and the roots' layer still did not finish.
    let w = sweep(&seg, fuel(9, 0), &Filter::default());
    assert_eq!(w.status, Status::Aground);
    assert_eq!(w.complete_within, None);
}

/// The certificate composes with the filter and nothing broader: it is a
/// claim about *this* query's relations and evidence floor.
#[test]
fn the_certificate_is_scoped_to_the_query_filter() {
    let defs: Vec<Def> = (0..4).map(def).collect();
    let edges = vec![
        edge(0, 1, Grade::Proven),
        // A weaker hop the filter below will refuse.
        edge(1, 2, Grade::Heuristic),
        edge(2, 3, Grade::Proven),
    ];
    let seg = Segment::parse(build(&defs, &edges).expect("image")).expect("parse");
    let proven_only = Filter {
        relations: Vec::new(),
        at_or_above: Some(Grade::Proven),
    };
    let w = sweep(&seg, fuel(100, 100), &proven_only);
    assert_eq!(w.status, Status::Complete);
    // Complete under *this* filter: node 2 is unreachable through a proven
    // edge, and the certificate says nothing about a weaker walk.
    assert_eq!(within(&w, 100), vec![0, 1]);
    assert_eq!(w.complete_within, Some(100));
}

proptest! {
    /// The property the plan states: for any graph, budget and filter, the
    /// certified prefix of an aground answer equals the same query's
    /// unbudgeted answer truncated to that depth. A differential against the
    /// walk with no budget at all — har-verify rung 5.
    #[test]
    fn a_certified_prefix_equals_the_unbudgeted_walk_truncated(
        n in 2u8..12,
        hops in 0u64..6,
        nodes in 0u64..12,
        extra in proptest::collection::vec((0u8..12, 0u8..12), 0..12),
        floor in prop::option::of(prop::sample::select(vec![Grade::Proven, Grade::Observed])),
    ) {
        let defs: Vec<Def> = (0..n).map(def).collect();
        let mut edges: Vec<Edge> = (0..n.saturating_sub(1))
            .map(|i| edge(i, i + 1, Grade::Proven))
            .collect();
        for (a, b) in extra {
            if a < n && b < n {
                // A mix of grades, so the filter has something to refuse.
                let grade = if (a + b) % 2 == 0 { Grade::Proven } else { Grade::Observed };
                edges.push(edge(a, b, grade));
            }
        }
        let seg = Segment::parse(build(&defs, &edges).expect("image")).expect("parse");
        let filter = Filter { relations: Vec::new(), at_or_above: floor };

        let budgeted = sweep(&seg, fuel(hops, nodes), &filter);
        let full = sweep(&seg, fuel(1_000, 1_000), &filter);
        prop_assert_eq!(full.status, Status::Complete);

        if let Some(d) = budgeted.complete_within {
            prop_assert_eq!(
                within(&budgeted, d),
                within(&full, d),
                "certified to {} hops, so that prefix must be whole", d
            );
        }
    }
}
