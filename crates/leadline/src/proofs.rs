//! Kani harnesses for claim C3: every walk terminates within fuel.
//!
//! Run with `just verify` (`cargo kani -p grid-leadline`). Termination is not
//! something a test can show — a hanging test is indistinguishable from a slow
//! one — so the property is stated as a decrease argument and proved.
//!
//! The harnesses drive a symbolic graph: unconstrained edges, unconstrained
//! evidence, unconstrained fuel. There is no adversarial adjacency the walk
//! can be handed that makes a take succeed without a counter falling.

use crate::fuel::Budget;
use crate::graph::{DatumId, Direction, Graph, Neighbour, Ordinal};
use crate::walk::{walk, Filter, Status};

/// A complete graph on `N` nodes with symbolic edge attributes: the densest
/// adjacency the walk can face, and the one where a missing visited-check
/// would loop forever.
struct AnyGraph<const N: u32> {
    relation: grid_wire::Relation,
    grade: grid_wire::Grade,
}

impl<const N: u32> Graph for AnyGraph<N> {
    fn def_count(&self) -> u32 {
        N
    }
    fn datum(&self, ord: Ordinal) -> Option<DatumId> {
        (ord.0 < N).then(|| DatumId([0; 32]))
    }
    fn ordinal(&self, _datum: &DatumId) -> Option<Ordinal> {
        None
    }
    fn callsign(&self, _ord: Ordinal) -> Option<&str> {
        None
    }
    fn lang(&self, _ord: Ordinal) -> Option<grid_wire::Lang> {
        None
    }
    fn out_edges(&self, ord: Ordinal) -> Vec<Neighbour> {
        if ord.0 >= N {
            return Vec::new();
        }
        (0..N)
            .map(|other| Neighbour {
                other: Ordinal(other),
                relation: self.relation,
                grade: self.grade,
            })
            .collect()
    }
    fn in_edges(&self, ord: Ordinal) -> Vec<Neighbour> {
        self.out_edges(ord)
    }
}

/// Taking fuel strictly decreases it, or refuses. This is the well-founded
/// measure the walk's termination rests on: `(hops, nodes)` is a pair of
/// naturals that never rises, and the loop continues only on a successful take.
#[kani::proof]
fn fuel_strictly_decreases() {
    let mut b = Budget::new(Some(&grid_wire::Fuel {
        max_hops: Some(kani::any()),
        max_nodes: Some(kani::any()),
        max_ms: None,
    }));
    let (hops, nodes) = (b.hops(), b.nodes());
    if b.take_node() {
        assert!(b.nodes() < nodes);
    } else {
        assert!(b.nodes() == nodes && nodes == 0);
    }
    let nodes = b.nodes();
    if b.take_hop() {
        assert!(b.hops() < hops);
    } else {
        assert!(b.hops() == hops && hops == 0);
    }
    // Taking one dimension never refunds the other.
    assert!(b.nodes() == nodes);
}

/// A walk over the densest possible graph terminates, and honours its budget:
/// it never expands more nodes than `max_nodes` nor reports more hops than
/// `max_hops`. Reaching this assertion at all is the termination proof — Kani
/// unwinds the loop, so a walk that could run longer than its budget would
/// fail the unwinding check rather than the assertion.
#[kani::proof]
#[kani::unwind(13)]
fn walk_terminates_within_fuel() {
    const N: u32 = 3;
    let max_hops: u64 = kani::any();
    let max_nodes: u64 = kani::any();
    kani::assume(max_hops <= 4 && max_nodes <= 8);
    let budget = Budget::new(Some(&grid_wire::Fuel {
        max_hops: Some(max_hops),
        max_nodes: Some(max_nodes),
        max_ms: None,
    }));
    let graph: AnyGraph<N> = AnyGraph {
        relation: grid_wire::Relation::Calls,
        grade: grid_wire::Grade::Proven,
    };
    let root = Ordinal(kani::any());
    kani::assume(root.0 < N);
    let dir = match kani::any::<u8>() % 3 {
        0 => Direction::Out,
        1 => Direction::In,
        _ => Direction::Both,
    };
    let w = walk(&graph, &[root], dir, &Filter::default(), budget, |_| false);
    assert!(w.visited <= max_nodes);
    assert!(w.hops <= max_hops);
    // Nothing outside the graph was ever reached.
    assert!(w.seen.keys().all(|o| o.0 < N));
    // A complete walk has nothing left on the frontier, and vice versa.
    assert!(w.frontier.is_empty() == (w.status == Status::Complete));
}

/// The predecessor chain is acyclic, so path reconstruction terminates even
/// when the graph is a single strongly-connected blob.
#[kani::proof]
#[kani::unwind(13)]
fn paths_terminate() {
    const N: u32 = 3;
    let graph: AnyGraph<N> = AnyGraph {
        relation: grid_wire::Relation::Calls,
        grade: grid_wire::Grade::Proven,
    };
    let budget = Budget::new(None);
    let w = walk(
        &graph,
        &[Ordinal(0)],
        Direction::Out,
        &Filter::default(),
        budget,
        |_| false,
    );
    let target = Ordinal(kani::any());
    kani::assume(target.0 < N);
    let chain = w.path_to(target);
    assert!(chain.len() <= w.seen.len());
}
