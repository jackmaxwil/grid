//! Claim C3 evidence a test *can* give: a walk never exceeds its budget, and
//! says so when it runs out.
//!
//! Termination itself is Kani's job (`src/proofs.rs`) — a hanging test looks
//! like a slow one. What a test can show is that the budget is *respected*:
//! the counters in the result never exceed the fuel, on graphs including the
//! cyclic and densely connected ones where an unguarded BFS would not return.

use grid_chart::{build, DatumId, Def, Edge, Segment};
use grid_leadline::fuel::{Budget, DEFAULT_MAX_HOPS, DEFAULT_MAX_NODES};
use grid_leadline::graph::{Direction, Ordinal};
use grid_leadline::walk::{walk, Filter, Status};
use grid_wire::{Lang, Relation, Zoc};
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

fn edge(s: u8, d: u8) -> Edge {
    Edge {
        src: datum(s),
        dst: datum(d),
        relation: Relation::Calls,
        zoc: Zoc::Proven,
    }
}

/// A cycle over `n` nodes: the graph where a walk without a visited-set runs
/// forever, and where a walk without fuel runs forever even with one if the
/// hop budget is what is supposed to stop it.
fn ring(n: u8) -> Segment<Vec<u8>> {
    let defs: Vec<Def> = (0..n).map(def).collect();
    let edges: Vec<Edge> = (0..n).map(|i| edge(i, (i + 1) % n)).collect();
    Segment::parse(build(&defs, &edges).unwrap()).unwrap()
}

/// Every node points at every node, including itself.
fn complete(n: u8) -> Segment<Vec<u8>> {
    let defs: Vec<Def> = (0..n).map(def).collect();
    let edges: Vec<Edge> = (0..n)
        .flat_map(|i| (0..n).map(move |j| edge(i, j)))
        .collect();
    Segment::parse(build(&defs, &edges).unwrap()).unwrap()
}

fn fuel(hops: u64, nodes: u64) -> Budget {
    Budget::new(Some(&grid_wire::Fuel {
        max_hops: Some(hops),
        max_nodes: Some(nodes),
        max_ms: None,
    }))
}

#[test]
fn a_cycle_terminates_and_visits_each_node_once() {
    let seg = ring(8);
    let w = walk(
        &seg,
        &[Ordinal(0)],
        Direction::Out,
        &Filter::default(),
        fuel(100, 100),
        |_| false,
    );
    assert_eq!(w.status, Status::Complete);
    // Eight nodes, each dequeued exactly once, however many times the ring
    // leads back to them.
    assert_eq!(w.visited, 8);
    assert_eq!(w.seen.len(), 8);
    assert!(w.frontier.is_empty());
}

#[test]
fn hop_budget_bounds_the_answer_not_the_search_order() {
    let seg = ring(8);
    // BFS makes a hop a layer, so `max_hops: 3` from node 0 reaches exactly
    // nodes 1, 2, 3 — a depth-first walk would reach a different set.
    let w = walk(
        &seg,
        &[Ordinal(0)],
        Direction::Out,
        &Filter::default(),
        fuel(3, 100),
        |_| false,
    );
    assert_eq!(w.status, Status::Aground);
    let mut reached: Vec<u32> = w.seen.keys().map(|o| o.0).collect();
    reached.sort_unstable();
    assert_eq!(reached, vec![0, 1, 2, 3]);
    assert_eq!(w.hops, 3);
}

#[test]
fn running_out_of_fuel_is_aground_with_the_frontier_shown() {
    let seg = ring(8);
    let w = walk(
        &seg,
        &[Ordinal(0)],
        Direction::Out,
        &Filter::default(),
        fuel(100, 3),
        |_| false,
    );
    assert_eq!(w.status, Status::Aground);
    assert_eq!(w.visited, 3);
    // Aground is a lower bound with its edge shown, never a silent truncation.
    assert!(!w.frontier.is_empty());
}

#[test]
fn an_empty_budget_expands_nothing() {
    let seg = ring(8);
    let w = walk(
        &seg,
        &[Ordinal(0)],
        Direction::Out,
        &Filter::default(),
        fuel(1, 0),
        |_| false,
    );
    assert_eq!(w.status, Status::Aground);
    assert_eq!(w.visited, 0);
    assert_eq!(w.frontier, vec![Ordinal(0)]);
}

#[test]
fn the_contract_defaults_apply_when_a_query_names_no_fuel() {
    let b = Budget::new(None);
    assert_eq!((b.hops(), b.nodes()), (DEFAULT_MAX_HOPS, DEFAULT_MAX_NODES));
}

#[test]
fn direction_decides_who_is_reachable() {
    // 0 -> 1 -> 2, a line: out from 0 reaches 2, in from 0 reaches nobody.
    let defs: Vec<Def> = (0..3).map(def).collect();
    let seg = Segment::parse(build(&defs, &[edge(0, 1), edge(1, 2)]).unwrap()).unwrap();
    let out = walk(
        &seg,
        &[Ordinal(0)],
        Direction::Out,
        &Filter::default(),
        fuel(9, 9),
        |_| false,
    );
    assert_eq!(out.seen.len(), 3);
    let inc = walk(
        &seg,
        &[Ordinal(0)],
        Direction::In,
        &Filter::default(),
        fuel(9, 9),
        |_| false,
    );
    assert_eq!(inc.seen.len(), 1);
    let both = walk(
        &seg,
        &[Ordinal(2)],
        Direction::Both,
        &Filter::default(),
        fuel(9, 9),
        |_| false,
    );
    assert_eq!(both.seen.len(), 3);
}

#[test]
fn an_evidence_floor_refuses_weaker_hops() {
    let defs: Vec<Def> = (0..3).map(def).collect();
    let weak = Edge {
        zoc: Zoc::Heuristic,
        ..edge(1, 2)
    };
    let seg = Segment::parse(build(&defs, &[edge(0, 1), weak]).unwrap()).unwrap();
    let filter = Filter {
        relations: Vec::new(),
        at_or_above: Some(Zoc::Observed),
    };
    let w = walk(
        &seg,
        &[Ordinal(0)],
        Direction::Out,
        &filter,
        fuel(9, 9),
        |_| false,
    );
    // The proven hop is taken, the heuristic one is not: the answer is
    // labelled by the evidence that supports it, not by hope.
    assert_eq!(w.seen.len(), 2);
    assert_eq!(w.status, Status::Complete);
}

proptest! {
    /// The counters never exceed the fuel, on the densest graph there is.
    #[test]
    fn a_walk_never_exceeds_its_budget(
        n in 1u8..7,
        hops in 0u64..8,
        nodes in 0u64..8,
        root in 0u32..7,
        d in 0u8..3,
    ) {
        let seg = complete(n);
        let dir = [Direction::Out, Direction::In, Direction::Both][d as usize];
        let root = Ordinal(root % u32::from(n));
        let w = walk(&seg, &[root], dir, &Filter::default(), fuel(hops, nodes), |_| false);
        prop_assert!(w.visited <= nodes);
        prop_assert!(w.hops <= hops);
        prop_assert!(w.seen.keys().all(|o| o.0 < u32::from(n)));
        // Aground and a non-empty frontier are the same fact.
        prop_assert_eq!(w.frontier.is_empty(), w.status == Status::Complete);
    }

    /// A path is a real path: consecutive nodes are joined by an edge that
    /// exists in the segment, in the direction the walk recorded.
    #[test]
    fn every_reconstructed_path_is_a_real_walk(n in 2u8..7, target in 0u32..7) {
        let seg = ring(n);
        let target = Ordinal(target % u32::from(n));
        let w = walk(&seg, &[Ordinal(0)], Direction::Out, &Filter::default(), fuel(64, 64), |_| false);
        let chain = w.path_to(target);
        prop_assert!(!chain.is_empty());
        prop_assert_eq!(chain[0].0, Ordinal(0));
        prop_assert_eq!(chain.last().unwrap().0, target);
        for pair in chain.windows(2) {
            let (from, _) = pair[0];
            let (to, via) = pair[1];
            let step = via.expect("a non-root hop carries its step");
            prop_assert_eq!(step.other, to);
            prop_assert!(seg.out_edges(from).any(|nb| nb.other == to));
        }
    }
}
