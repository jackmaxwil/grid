//! The seam between the walk and the chart.
//!
//! Walk does no I/O and knows nothing about segment files: it walks
//! whatever answers [`Graph`]. The blanket impl over [`grid_chart::Segment`]
//! is the only one that ships, but tests build tiny graphs by hand and the
//! Kani harnesses drive a symbolic one, which is what keeps the termination
//! proof about the walk rather than about the store.

pub use grid_chart::{DatumId, Neighbour, Ordinal};
use grid_wire::{Grade, Lang, Relation};

/// Which way a hop is allowed to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Follow edges from source to destination.
    Out,
    /// Follow edges backwards: who reaches this.
    In,
    /// Both, as one undirected step.
    Both,
}

impl From<grid_wire::WalkQueryDirection> for Direction {
    fn from(d: grid_wire::WalkQueryDirection) -> Self {
        use grid_wire::WalkQueryDirection as D;
        match d {
            D::Out => Self::Out,
            D::In => Self::In,
            D::Both => Self::Both,
        }
    }
}

/// One hop as the walk sees it: the neighbour plus which way it was taken.
///
/// `Both` walks need the direction back to render a [`grid_wire::Edge`] with
/// its real `src` and `dst`; a path that reports a reversed edge is a lie
/// about the code even when the reachability answer is right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// The other endpoint.
    pub other: Ordinal,
    /// Typed edge kind.
    pub relation: Relation,
    /// Evidence grade.
    pub grade: Grade,
    /// `true` when the stored edge runs from the current node to `other`.
    pub forward: bool,
    /// Index of the modeled bridge this hop came from, if it is not a hop the
    /// language established. `None` is the only shape a proven hop can have:
    /// evidence travels with the step, so a modeled hop cannot be laundered
    /// into a proven one downstream (claim C18).
    pub bridge: Option<u32>,
}

/// A read-only graph the walk can traverse.
pub trait Graph {
    /// Number of definitions; ordinals are `0..def_count`.
    fn def_count(&self) -> u32;
    /// Content identity of a definition.
    fn datum(&self, ord: Ordinal) -> Option<DatumId>;
    /// Ordinal of a datum, if this graph holds it.
    fn ordinal(&self, datum: &DatumId) -> Option<Ordinal>;
    /// Human name at survey time.
    fn callsign(&self, ord: Ordinal) -> Option<&str>;
    /// Language adapter that found the definition.
    fn lang(&self, ord: Ordinal) -> Option<Lang>;
    /// Edges out of `ord`.
    fn out_edges(&self, ord: Ordinal) -> Vec<Neighbour>;
    /// Edges into `ord`.
    fn in_edges(&self, ord: Ordinal) -> Vec<Neighbour>;

    /// Every step legal from `ord` in `dir`, in stored order.
    ///
    /// `Both` yields out-edges before in-edges. The order is stored order,
    /// which `grid_chart::build` made content order, so a walk over the same
    /// segment expands the same frontier in the same sequence on every
    /// machine (claim C2 reaching through to query results).
    fn steps(&self, ord: Ordinal, dir: Direction) -> Vec<Step> {
        let out = || {
            self.out_edges(ord).into_iter().map(|n| Step {
                other: n.other,
                relation: n.relation,
                grade: n.grade,
                forward: true,
                bridge: None,
            })
        };
        let inc = || {
            self.in_edges(ord).into_iter().map(|n| Step {
                other: n.other,
                relation: n.relation,
                grade: n.grade,
                forward: false,
                bridge: None,
            })
        };
        match dir {
            Direction::Out => out().collect(),
            Direction::In => inc().collect(),
            Direction::Both => out().chain(inc()).collect(),
        }
    }

    /// The `via` label of a modeled hop, for rendering.
    ///
    /// Graphs that model nothing have no labels; the blanket segment impl is
    /// one of them, because a segment stores only what a language established.
    fn bridge_label(&self, bridge: u32) -> Option<String> {
        let _ = bridge;
        None
    }
}

impl<B: AsRef<[u8]>> Graph for grid_chart::Segment<B> {
    fn def_count(&self) -> u32 {
        Self::def_count(self)
    }
    fn datum(&self, ord: Ordinal) -> Option<DatumId> {
        Self::datum(self, ord)
    }
    fn ordinal(&self, datum: &DatumId) -> Option<Ordinal> {
        Self::ordinal(self, datum)
    }
    fn callsign(&self, ord: Ordinal) -> Option<&str> {
        Self::callsign(self, ord)
    }
    fn lang(&self, ord: Ordinal) -> Option<Lang> {
        Self::lang(self, ord)
    }
    fn out_edges(&self, ord: Ordinal) -> Vec<Neighbour> {
        Self::out_edges(self, ord).collect()
    }
    fn in_edges(&self, ord: Ordinal) -> Vec<Neighbour> {
        Self::in_edges(self, ord).collect()
    }
}
