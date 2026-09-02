//! The budget. Every walk carries fuel; there are no unbounded queries.
//!
//! This module is deliberately free of clocks and allocation: it is pure
//! arithmetic over two counters, which is what makes claim C3 provable rather
//! than merely tested. [`Budget::take_node`] and [`Budget::take_hop`] are the
//! only ways to make progress, and each one either refuses or strictly
//! decreases a counter that never grows — so a loop that only continues while
//! a take succeeds cannot run forever.
//!
//! The wall-clock guard (`max_ms`) lives in the walk, not here: it is
//! nondeterministic, so results truncated by it are outside the determinism
//! claim, and it must not be part of the termination argument.

/// Defaults from the wire contract (`Fuel`).
pub const DEFAULT_MAX_HOPS: u64 = 6;
/// Defaults from the wire contract (`Fuel`).
pub const DEFAULT_MAX_NODES: u64 = 10_000;

/// A traversal budget in flight: what is left, never what was spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    hops: u64,
    nodes: u64,
}

impl Budget {
    /// A budget from a wire [`grid_wire::Fuel`], applying the contract's
    /// defaults for absent fields.
    #[must_use]
    pub fn new(fuel: Option<&grid_wire::Fuel>) -> Self {
        Self {
            hops: fuel.and_then(|f| f.max_hops).unwrap_or(DEFAULT_MAX_HOPS),
            nodes: fuel.and_then(|f| f.max_nodes).unwrap_or(DEFAULT_MAX_NODES),
        }
    }

    /// Hops left.
    #[must_use]
    pub const fn hops(self) -> u64 {
        self.hops
    }

    /// Nodes left.
    #[must_use]
    pub const fn nodes(self) -> u64 {
        self.nodes
    }

    /// Is the budget spent in either dimension?
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.hops == 0 || self.nodes == 0
    }

    /// Charge one node visit. Returns `false` when there was nothing left to
    /// charge, in which case the budget is unchanged.
    pub fn take_node(&mut self) -> bool {
        match self.nodes.checked_sub(1) {
            Some(left) => {
                self.nodes = left;
                true
            }
            None => false,
        }
    }

    /// Charge one hop (one BFS layer). Same contract as [`Self::take_node`].
    pub fn take_hop(&mut self) -> bool {
        match self.hops.checked_sub(1) {
            Some(left) => {
                self.hops = left;
                true
            }
            None => false,
        }
    }
}
