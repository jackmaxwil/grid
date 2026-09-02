//! grid-walk — query engine: budgeted multi-hop traversal, evidence-labeled.
//!
//! One entry point, [`query`]: a wire [`grid_wire::WalkQuery`] against a
//! graph, answering a wire [`grid_wire::WalkResult`]. Two shapes, chosen by
//! the query and not by the caller: `to` absent is a reachability sweep
//! (`reached`), `to` present is path-finding (`paths`).
//!
//! Everything here is pure. The walk sees a [`graph::Graph`], not a file, so
//! the daemon, the tests and the Kani harnesses all drive the same code.
//!
//! ## Fuel is not optional (claim C3)
//!
//! There is no unbudgeted path through this crate: [`fuel::Budget::new`]
//! applies the contract's defaults when a query names no fuel, and the walk
//! spends the budget on every node and every layer. A walk that runs out says
//! so — [`grid_wire::WalkResultStatus::Aground`] with the unexpanded
//! frontier — rather than truncating silently or hanging. `aground` is a
//! result, not an error: a lower bound with its edge shown is more useful than
//! a timeout, and the caller can raise the budget and ask again.

pub mod fuel;
pub mod graph;
pub mod walk;

use grid_wire::{
    Callsign, Datum, DefRef, Locator, Path, WalkQuery, WalkResult, WalkResultStatus, WalkStats,
};

use graph::{DatumId, Direction, Graph, Ordinal};
use walk::{Filter, Status};

/// Why a query could not be answered. Not a walk failure: running out of fuel
/// is a [`WalkResultStatus::Aground`] result, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A locator named nothing in this snapshot.
    Unresolved(String),
    /// A locator names something this crate cannot resolve without a survey
    /// index (a position or a `file:line` coordinate).
    Unsupported(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unresolved(what) => write!(f, "no definition matches {what}"),
            Self::Unsupported(what) => write!(f, "{what} locators need a coordinate index"),
        }
    }
}

impl std::error::Error for Error {}

/// Resolve one locator to an ordinal in `graph`.
///
/// Callsigns resolve by exact name, then by unique suffix — the display policy
/// in `docs/PORCELAIN.md`: any unique name suffix is accepted as input. An
/// ambiguous suffix is [`Error::Unresolved`], never an arbitrary pick.
///
/// # Errors
/// [`Error::Unresolved`] when nothing (or more than one thing) matches;
/// [`Error::Unsupported`] for coordinate locators.
pub fn resolve<G: Graph + ?Sized>(graph: &G, locator: &Locator) -> Result<Ordinal, Error> {
    match locator {
        Locator::Datum { datum } => DatumId::parse(&datum.0)
            .and_then(|d| graph.ordinal(&d))
            .ok_or_else(|| Error::Unresolved(datum.0.clone())),
        Locator::Callsign { callsign } => by_name(graph, &callsign.0),
        Locator::Handle { handle } => {
            by_name(graph, handle.0.split('@').next().unwrap_or(&handle.0))
        }
        Locator::Position { .. } => Err(Error::Unsupported("position")),
        Locator::FileLine { .. } => Err(Error::Unsupported("file:line")),
    }
}

/// Exact callsign, else unique suffix. One pass, no allocation per candidate.
fn by_name<G: Graph + ?Sized>(graph: &G, name: &str) -> Result<Ordinal, Error> {
    let mut suffix: Option<Ordinal> = None;
    let mut ambiguous = false;
    for i in 0..graph.def_count() {
        let ord = Ordinal(i);
        let Some(cs) = graph.callsign(ord) else {
            continue;
        };
        if cs == name {
            return Ok(ord);
        }
        // A suffix match must break on a segment boundary: `fetchCard` may
        // name `api.fetchCard`, but `Card` must not.
        if cs.len() > name.len()
            && cs.ends_with(name)
            && cs.as_bytes()[cs.len() - name.len() - 1] == b'.'
        {
            if suffix.is_some() {
                ambiguous = true;
            }
            suffix = Some(ord);
        }
    }
    match suffix {
        Some(ord) if !ambiguous => Ok(ord),
        _ => Err(Error::Unresolved(name.to_string())),
    }
}

/// Run a query against a graph.
///
/// # Errors
/// [`Error`] when a `from` locator resolves to nothing. A `to` locator that
/// resolves to nothing is not an error — it is a target that was not reached,
/// which the empty `paths` array already says.
pub fn query<G: Graph + ?Sized>(
    graph: &G,
    q: &WalkQuery,
    snapshot: grid_wire::SnapshotId,
) -> Result<WalkResult, Error> {
    let roots = q
        .from
        .iter()
        .map(|l| resolve(graph, l))
        .collect::<Result<Vec<_>, _>>()?;
    let dir = q.direction.map_or(Direction::Out, Direction::from);
    let filter = Filter {
        relations: q.relations.clone().unwrap_or_default(),
        at_or_above: q.at_or_above,
    };
    let budget = fuel::Budget::new(q.fuel.as_ref());

    // Targets that do not resolve are not reachable by definition; dropping
    // them here keeps "unreached target" and "unknown name" one answer.
    let targets: Option<Vec<Ordinal>> =
        q.to.as_ref()
            .map(|ls| ls.iter().filter_map(|l| resolve(graph, l).ok()).collect());

    let started = std::time::Instant::now();
    let mut remaining: Vec<Ordinal> = targets.clone().unwrap_or_default();
    let walk = walk::walk(graph, &roots, dir, &filter, budget, |ord| {
        // Path-finding stops as soon as every target has been dequeued: BFS
        // has then found a shortest path to each, and further expansion could
        // only cost fuel to learn nothing.
        if targets.is_none() {
            return false;
        }
        remaining.retain(|t| *t != ord);
        remaining.is_empty()
    });

    let status = match walk.status {
        Status::Complete => WalkResultStatus::Complete,
        Status::Aground => WalkResultStatus::Aground,
    };
    let counters = WalkStats {
        visited: walk.visited,
        hops: walk.hops,
        ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        // Not a diagnostic: a claim about the answer, and a pure function of
        // the graph and the query. See `Walk::complete_within`.
        complete_within: walk.complete_within,
    };
    let frontier = (!walk.frontier.is_empty()).then(|| {
        walk.frontier
            .iter()
            .filter_map(|&o| datum_of(graph, o))
            .collect()
    });

    let (paths, reached) = match &targets {
        Some(targets) => (Some(paths_to(graph, &walk, targets)), None),
        None => (None, Some(reached(graph, &walk, &roots))),
    };

    Ok(WalkResult {
        status,
        snapshot,
        paths,
        reached,
        frontier,
        stats: counters,
    })
}

fn datum_of<G: Graph + ?Sized>(graph: &G, ord: Ordinal) -> Option<Datum> {
    graph.datum(ord).map(|d| Datum(d.to_string()))
}

fn def_ref<G: Graph + ?Sized>(graph: &G, ord: Ordinal) -> Option<DefRef> {
    Some(DefRef {
        datum: datum_of(graph, ord)?,
        callsign: graph.callsign(ord).map(|c| Callsign(c.to_string())),
        lang: graph.lang(ord),
        signature: None,
        cfg: None,
        materialization: None,
    })
}

/// Everything reached that is not a root, in ordinal order.
///
/// Ordinal order is content order (`grid_chart::build` sorts by datum), so the
/// answer is byte-identical for the same snapshot on every machine — the walk's
/// `HashMap` never reaches the output.
fn reached<G: Graph + ?Sized>(graph: &G, walk: &walk::Walk, roots: &[Ordinal]) -> Vec<DefRef> {
    let mut ords: Vec<Ordinal> = walk
        .seen
        .keys()
        .copied()
        .filter(|o| !roots.contains(o))
        .collect();
    ords.sort_unstable();
    ords.iter().filter_map(|&o| def_ref(graph, o)).collect()
}

/// One shortest path per reached target, in target order.
fn paths_to<G: Graph + ?Sized>(graph: &G, walk: &walk::Walk, targets: &[Ordinal]) -> Vec<Path> {
    targets
        .iter()
        .filter_map(|&t| path_of(graph, walk, t))
        .collect()
}

fn path_of<G: Graph + ?Sized>(graph: &G, walk: &walk::Walk, target: Ordinal) -> Option<Path> {
    let chain = walk.path_to(target);
    if chain.is_empty() {
        return None;
    }
    let mut nodes = Vec::with_capacity(chain.len());
    let mut edges = Vec::with_capacity(chain.len().saturating_sub(1));
    let mut prev: Option<Datum> = None;
    for (ord, via) in chain {
        let datum = datum_of(graph, ord)?;
        if let (Some(prev), Some(step)) = (prev.clone(), via) {
            // `forward` says which endpoint is the source: a `both` walk may
            // have crossed this edge against its direction, and the rendered
            // edge must still name the real src and dst.
            let (src, dst) = if step.forward {
                (prev, datum.clone())
            } else {
                (datum.clone(), prev)
            };
            edges.push(grid_wire::Edge {
                src,
                relation: step.relation,
                dst,
                grade: step.grade,
                via: None,
            });
        }
        nodes.push(datum.clone());
        prev = Some(datum);
    }
    Some(Path { nodes, edges })
}

#[cfg(kani)]
mod proofs;
