//! The walk: budgeted, evidence-filtered breadth-first traversal.
//!
//! Breadth-first and not depth-first for one reason: a hop budget is only
//! meaningful if a hop is a layer. BFS makes "within 4 hops" the shortest-path
//! distance, so `max_hops` bounds the *answer*, not the search order — and the
//! first path found to a node is a shortest one, so path results do not depend
//! on expansion luck.
//!
//! ## Why this terminates (claim C3)
//!
//! Three independent reasons, any one of which suffices:
//!
//! 1. Every node is enqueued at most once — [`Walk::seen`] is checked before
//!    push, so the queue admits at most `def_count` elements in total.
//! 2. Every dequeue charges [`Budget::take_node`], which strictly decreases a
//!    counter that never grows; the loop exits when it refuses.
//! 3. Every layer charges [`Budget::take_hop`], likewise.
//!
//! The Kani harnesses in `proofs.rs` (compiled only under `cfg(kani)`, so
//! rustdoc cannot link them) prove (2) and (3) over a symbolic
//! graph: no sequence of steps, however adversarial the edges, re-enters the
//! loop with a budget that did not strictly decrease.
//!
//! ## Why an aground answer can certify a radius
//!
//! Because the walk is strictly layered, a layer that finished expanding has
//! discovered every node one hop past it — that is what expanding a layer
//! *is*. So the deepest fully-expanded layer is a radius the answer is
//! complete to, under this walk's filter and nothing broader. The certificate
//! is read off state the walk already keeps and spends no fuel, which is why
//! [`Walk::complete_within`] exists at all: a caller holding a partial answer
//! can otherwise not tell "every def within 3 hops is here" from "the walk
//! stopped mid-layer and even depth 1 may be short".

use std::collections::HashMap;

use grid_wire::{Grade, Relation};

use crate::fuel::Budget;
use crate::graph::{Direction, Graph, Ordinal, Step};

/// Evidence grade as a comparable rank; strongest is 0, matching the on-disk
/// order (`grid_chart::grade_tag`) and the porcelain column order.
const fn rank(z: Grade) -> u8 {
    match z {
        Grade::Proven => 0,
        Grade::Observed => 1,
        Grade::Modeled => 2,
        Grade::Heuristic => 3,
    }
}

/// What a hop must satisfy to be taken.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Restrict traversal to these edge kinds; empty means all.
    pub relations: Vec<Relation>,
    /// Weakest evidence a hop may carry.
    pub at_or_above: Option<Grade>,
}

impl Filter {
    fn admits(&self, step: Step) -> bool {
        let relation_ok = self.relations.is_empty() || self.relations.contains(&step.relation);
        let grade_ok = self
            .at_or_above
            .is_none_or(|floor| rank(step.grade) <= rank(floor));
        relation_ok && grade_ok
    }
}

/// How a node was first reached: its predecessor and the step that got there.
/// Roots have no predecessor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reached {
    /// Hops from the nearest root; roots are 0.
    pub depth: u64,
    /// The node this one was first reached from, if not a root.
    pub from: Option<Ordinal>,
    /// The step taken from `from`, if not a root.
    pub via: Option<Step>,
}

/// Why a walk stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The frontier emptied: the answer is the whole reachable set.
    Complete,
    /// Fuel ran out with work left. The answer is a lower bound.
    Aground,
}

/// The result of a walk over ordinals. Rendering to wire types is the
/// caller's job, in `grid`; this is the pure graph answer.
#[derive(Debug, Clone)]
pub struct Walk {
    /// Why it stopped.
    pub status: Status,
    /// Every node reached, with how it was first reached.
    pub seen: HashMap<Ordinal, Reached>,
    /// Nodes discovered but never expanded, because fuel ran out. Empty
    /// exactly when `status` is [`Status::Complete`].
    pub frontier: Vec<Ordinal>,
    /// Nodes dequeued and expanded.
    pub visited: u64,
    /// Layers completed.
    pub hops: u64,
    /// The radius this answer is *complete* to: every node reachable within
    /// this many hops, under this walk's filter, is in [`Walk::seen`].
    ///
    /// The walk is strictly layered, so a layer fully expanded before the
    /// fuel ran out has necessarily discovered every node one hop past it.
    /// The certificate is therefore free — it reads state the walk already
    /// keeps — and it is what upgrades an aground result from "here is where
    /// I stopped" to "here is the radius I finished".
    ///
    /// `None` when not even the roots' own layer was expanded: that answer
    /// certifies nothing, and says so rather than claiming `Some(0)`.
    pub complete_within: Option<u64>,
}

impl Walk {
    /// The path from a root to `target`, as ordinals with the step between
    /// each pair. Empty if `target` was never reached.
    ///
    /// Walks backwards through the predecessor chain, which is acyclic by
    /// construction (a node's predecessor was reached strictly earlier), so
    /// this cannot loop even on a cyclic graph.
    #[must_use]
    pub fn path_to(&self, target: Ordinal) -> Vec<(Ordinal, Option<Step>)> {
        let mut chain = Vec::new();
        let mut at = target;
        // Bounded by `seen.len()`: each step moves to a strictly earlier node,
        // and the guard makes the bound structural rather than merely true.
        for _ in 0..=self.seen.len() {
            let Some(r) = self.seen.get(&at) else {
                return Vec::new();
            };
            chain.push((at, r.via));
            match r.from {
                Some(prev) => at = prev,
                None => break,
            }
        }
        chain.reverse();
        chain
    }
}

/// Walk `graph` from `roots`, in `dir`, spending `budget`.
///
/// `stop` is consulted for each node as it is expanded; returning `true` makes
/// the walk stop early with [`Status::Complete`] — the targets were found, and
/// the budget was not what ended it. Pass `|_| false` for a reachability sweep.
///
/// # Termination
/// Guaranteed, by the argument in the module docs: no input can make this
/// function run longer than `min(max_nodes, def_count)` expansions.
pub fn walk<G: Graph + ?Sized>(
    graph: &G,
    roots: &[Ordinal],
    dir: Direction,
    filter: &Filter,
    mut budget: Budget,
    mut stop: impl FnMut(Ordinal) -> bool,
) -> Walk {
    // The hop bound as asked for. A walk whose frontier drains is complete to
    // the whole bound, whatever depth it actually reached.
    let hop_bound = budget.hops();
    // The deepest layer fully expanded. Set only after a layer's expansion
    // finishes, so a refusal mid-layer never certifies that layer.
    let mut certified: Option<u64> = None;
    let mut seen: HashMap<Ordinal, Reached> = HashMap::new();
    let mut layer: Vec<Ordinal> = Vec::new();
    for &root in roots {
        if root.0 < graph.def_count()
            && seen
                .insert(
                    root,
                    Reached {
                        depth: 0,
                        from: None,
                        via: None,
                    },
                )
                .is_none()
        {
            layer.push(root);
        }
    }
    let mut visited = 0_u64;
    let mut hops = 0_u64;
    let mut depth = 0_u64;

    loop {
        if layer.is_empty() {
            return Walk {
                status: Status::Complete,
                seen,
                frontier: Vec::new(),
                visited,
                hops,
                complete_within: Some(hop_bound),
            };
        }
        // A hop buys the expansion of one layer. Charging before expanding is
        // what makes `max_hops` bound the depth of the *answer*: with three
        // hops, layers at depth 0, 1 and 2 are expanded, so nothing deeper
        // than depth 3 is ever discovered, and the layer we cannot afford is
        // reported as the frontier rather than half-explored.
        if !budget.take_hop() {
            return aground(seen, layer, visited, hops, certified);
        }
        hops += 1;
        let mut next = Vec::new();
        for (i, &node) in layer.iter().enumerate() {
            if !budget.take_node() {
                // The frontier is what was discovered but never expanded: the
                // unexpanded tail of this layer, then whatever it had found.
                let mut rest = layer[i..].to_vec();
                rest.extend(next);
                return aground(seen, rest, visited, hops, certified);
            }
            visited += 1;
            if stop(node) {
                return Walk {
                    status: Status::Complete,
                    seen,
                    frontier: Vec::new(),
                    visited,
                    hops,
                    // Stopped by the caller, mid-layer: the reach set is not
                    // the answer here and is not complete to the bound. The
                    // honest certificate is the last layer that finished.
                    complete_within: certified,
                };
            }
            for step in graph.steps(node, dir) {
                if step.other.0 >= graph.def_count() || !filter.admits(step) {
                    continue;
                }
                // First reach wins. BFS dequeues in depth order, so the first
                // predecessor recorded is on a shortest path — overwriting it
                // would splice a longer route into the chain and could make
                // the chain cyclic, which is exactly what `path_to` relies on
                // not happening.
                if let std::collections::hash_map::Entry::Vacant(slot) = seen.entry(step.other) {
                    slot.insert(Reached {
                        depth: depth + 1,
                        from: Some(node),
                        via: Some(step),
                    });
                    next.push(step.other);
                }
            }
        }
        // This layer finished: everything within `depth + 1` hops has been
        // discovered, but only `depth` is claimed — the layer at `depth + 1`
        // has not been expanded, so nothing past it is known.
        certified = Some(depth);
        if next.is_empty() {
            return Walk {
                status: Status::Complete,
                seen,
                frontier: Vec::new(),
                visited,
                hops,
                complete_within: Some(hop_bound),
            };
        }
        layer = next;
        depth += 1;
    }
}

fn aground(
    seen: HashMap<Ordinal, Reached>,
    frontier: Vec<Ordinal>,
    visited: u64,
    hops: u64,
    complete_within: Option<u64>,
) -> Walk {
    Walk {
        status: Status::Aground,
        seen,
        frontier,
        visited,
        hops,
        complete_within,
    }
}
