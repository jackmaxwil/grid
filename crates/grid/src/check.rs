//! `grid check` — the gate verdict and, on request, why it says so.
//!
//! A reviewer asks four questions about a change: did it do what it promised,
//! what did it do that it did not promise, what changed without being run,
//! and what is still open against this code. The gate answers the first two
//! already; this verb adds the *how*: for every promise the gate matched, the
//! paths through the chart that make it good.
//!
//! Only fulfilling paths ship. A promise the gate matched but for which no
//! path exists under the budget carries no `hops` entry rather than an empty
//! one — an item here means "this is why the gate said match", and an item
//! with nothing behind it would be a claim with no evidence.
//!
//! Traversal is [`grid_leadline::query`], the same engine `grid.query.walk`
//! serves, so a path a reviewer sees is a path the query verb would produce.
//! Nothing here re-implements a walk, and nothing here upgrades evidence: a
//! hop renders with the [`grid_wire::Grade`] the chart stored for it.
use std::path::Path;

use grid_wire::methods::CheckParams;
use grid_wire::{
    Callsign, CheckResult, CheckWhy, CheckWhyFulfillingItem, Datum, DefChangeAction, Fuel, Grade,
    Locator, PlanEdgePromise, SnapshotOrRev, WalkQuery, WalkQueryDirection,
};
use grid_xte::DeltaProvider as _;

use crate::chart_provider::{self, ChartDelta};
use crate::{rpc, standing};

/// Where traces live, relative to the repo root. The same directory
/// `grid.trace.ingest` writes: a review reads the log, never a second one.
const WAKE_STORE: &str = ".grid/wakes";

/// Budget for one promise's path search.
///
/// A review is interactive, so the walk is bounded by default rather than by
/// the caller remembering to bound it. Four hops is the honest reach for
/// "these calls make that promise good": beyond it a path stops being an
/// explanation a reviewer can hold in their head.
const MAX_HOPS: u64 = 4;
const MAX_NODES: u64 = 4096;
const MAX_MS: u64 = 250;

/// How many endpoints one pattern may expand to before the search stops
/// widening. A `**` pattern over a large repo would otherwise turn one
/// promise into a cross product; the cap is deterministic (candidates are
/// sorted before it applies), so two runs cut the same set.
const MAX_ENDPOINTS: usize = 32;

/// Why a review pack could not be produced.
#[derive(Debug)]
pub enum Error {
    /// The chart could not answer (unknown snapshot, failed survey, unknown
    /// plan).
    Chart(chart_provider::Error),
    /// A trace id was supplied and the log does not hold it. Coverage without
    /// the trace it names would be a different question silently answered.
    UnknownTrace(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chart(e) => write!(f, "{e}"),
            Self::UnknownTrace(id) => write!(f, "no trace `{id}` in {WAKE_STORE}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<chart_provider::Error> for Error {
    fn from(e: chart_provider::Error) -> Self {
        Self::Chart(e)
    }
}

/// `grid.check` — verdict, and why: fulfilling paths, surplus, coverage, anchors.
///
/// The verdict is the gate's, computed here the way `grid.todo` computes it:
/// one engine, so a review and a gate can never disagree about whether a
/// promise was kept.
///
/// # Errors
/// [`Error::Chart`] when the plan, the base or the head cannot be resolved;
/// [`Error::UnknownTrace`] when a supplied trace id names nothing retained.
pub fn answer(params: &CheckParams, root: &Path) -> Result<CheckResult, Error> {
    let plan = crate::plan::load(&params.plan.0).map_err(chart_provider::Error::Plan)?;
    let plan = standing::fold(
        &plan,
        standing::load(root).map_err(chart_provider::Error::Config)?,
    );
    let mut chart = rpc::Chart::new(root);
    chart.ensure(&plan.base)?;
    if let SnapshotOrRev::GitRev { .. } = params.head {
        chart.ensure(&params.head)?;
    }
    let provider = ChartDelta::open(root)?;
    let diff = provider.diff(&plan.base, &params.head)?;
    let verdict = rpc::gate_check(
        &grid_wire::methods::GateCheckParams {
            plan: plan.clone(),
            head: params.head.clone(),
        },
        root,
    )?;

    let snapshot = provider
        .current()
        .ok_or(chart_provider::Error::NoSnapshot)?;
    let names = callsigns(&provider);
    let fulfilling = verdict
        .matched
        .iter()
        .filter_map(|promise| {
            let hops = hops_for(&provider, &names, promise, &snapshot);
            (!hops.is_empty()).then(|| CheckWhyFulfillingItem {
                promise: promise.clone(),
                hops,
            })
        })
        .collect();

    Ok(CheckResult {
        why: Some(CheckWhy {
            surplus: verdict.surplus.clone(),
            coverage: coverage(&diff, params.trace.as_ref(), root)?,
            // Grid stores no anchors: nothing writes them, so the honest
            // answer is none, not an invented one. The field is the
            // consumer's (an Artifacts review, say) once a store exists.
            open_anchors: Vec::new(),
            fulfilling,
        }),
        verdict,
    })
}

/// Every callsign the live epoch knows, sorted — the name-space side of the
/// pattern match, gathered once rather than per promise.
fn callsigns(provider: &ChartDelta) -> Vec<String> {
    let mut out: Vec<String> = provider
        .head()
        .segments()
        .iter()
        .flat_map(|segment| {
            (0..segment.def_count())
                .filter_map(|ord| segment.callsign(grid_chart::Ordinal(ord)))
                .map(str::to_owned)
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The concrete endpoints a promise pattern names, capped deterministically.
fn endpoints(names: &[String], pattern: &str) -> Vec<Locator> {
    names
        .iter()
        .filter(|name| grid_xte::pattern::matches(pattern, name))
        .take(MAX_ENDPOINTS)
        .map(|name| Locator::Callsign {
            callsign: Callsign(name.clone()),
        })
        .collect()
}

/// The paths that make one matched promise good.
///
/// Hops are filtered to the promised relation: a chain of `calls` is why a
/// `calls` promise holds, and a path that reached the destination by some
/// other relation would be a different claim wearing this promise's name.
fn hops_for(
    provider: &ChartDelta,
    names: &[String],
    promise: &PlanEdgePromise,
    snapshot: &grid_wire::SnapshotId,
) -> Vec<grid_wire::Path> {
    let from = endpoints(names, &promise.src.0);
    let to = endpoints(names, &promise.dst.0);
    if from.is_empty() || to.is_empty() {
        return Vec::new();
    }
    let query = WalkQuery {
        from,
        to: Some(to),
        direction: Some(WalkQueryDirection::Out),
        relations: Some(vec![promise.relation]),
        at_or_above: None,
        fuel: Some(Fuel {
            max_hops: Some(MAX_HOPS),
            max_nodes: Some(MAX_NODES),
            max_ms: Some(MAX_MS),
        }),
        snapshot: None,
    };
    let mut paths = Vec::new();
    for segment in provider.head().segments() {
        // A segment that holds none of the endpoints answers nothing about
        // this promise; the walk engine would call the unresolved root an
        // error,
        // and across segments it is simply an absence.
        let Ok(result) = grid_leadline::query(segment, &query, snapshot.clone()) else {
            continue;
        };
        paths.extend(result.paths.into_iter().flatten());
    }
    paths.sort_by(|a, b| {
        (a.nodes.len(), &a.nodes.first().map(|d| d.0.clone()))
            .cmp(&(b.nodes.len(), &b.nodes.first().map(|d| d.0.clone())))
    });
    paths.dedup();
    paths
}

/// Changed but not reached by the supplied trace — the reviewer's red flags.
///
/// No trace, no claim: coverage is empty rather than "everything is uncovered",
/// because an absent test run is not evidence that code never ran.
fn coverage(
    diff: &grid_wire::Diff,
    trace: Option<&grid_wire::TraceId>,
    root: &Path,
) -> Result<Vec<Datum>, Error> {
    let Some(trace_id) = trace else {
        return Ok(Vec::new());
    };
    let store = grid_wake::Store::open(root.join(WAKE_STORE))
        .map_err(|e| Error::Chart(chart_provider::Error::Config(e.to_string())))?;
    let stored = store
        .get(&trace_id.0)
        .map_err(|e| Error::Chart(chart_provider::Error::Config(e.to_string())))?
        .ok_or_else(|| Error::UnknownTrace(trace_id.0.clone()))?;
    let mut reached: Vec<String> = stored
        .hops
        .iter()
        .flat_map(|(caller, callee)| [caller.to_string(), callee.to_string()])
        .collect();
    reached.sort();
    reached.dedup();

    let mut out: Vec<Datum> = diff
        .defs
        .iter()
        .filter(|d| d.action != DefChangeAction::Removed)
        .filter_map(|d| d.after.clone())
        .filter(|datum| reached.binary_search(&datum.0).is_err())
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup();
    Ok(out)
}

/// Render a pack as porcelain records (`docs/PORCELAIN.md`).
///
/// The verdict renders exactly as `grid gate` renders it — a review that
/// spelled the verdict differently would be a second grammar for one judgment.
/// What follows is the review's own: a `path` per fulfilling route, its `hop`
/// records beneath it, and a `miss` per changed-but-unexecuted definition.
#[must_use]
pub fn porcelain(answer: &CheckResult, name: &crate::porcelain::NameOf) -> String {
    let show = |d: &Datum| name(d).unwrap_or_else(|| format!("@{}", prefix(d)));
    let mut out = crate::porcelain::verdict(&answer.verdict, name);
    let Some(why) = answer.why.as_ref() else {
        return out;
    };
    for item in &why.fulfilling {
        for path in &item.hops {
            let head = path.nodes.first().map_or_else(String::new, &show);
            record(
                &mut out,
                "path",
                &[&head, &path.edges.len().to_string(), weakest(&path.edges)],
            );
            for edge in &path.edges {
                record(
                    &mut out,
                    "hop",
                    &[
                        grid_xte::lint::relation_name(edge.relation),
                        &show(&edge.dst),
                        grade(edge.grade),
                    ],
                );
            }
        }
    }
    for datum in &why.coverage {
        record(&mut out, "miss", &[&show(datum), "unexecuted"]);
    }
    out
}

/// The weakest evidence on a path: a chain is only as good as its worst hop,
/// so that is what the `path` record reports.
fn weakest(edges: &[grid_wire::Edge]) -> &'static str {
    edges
        .iter()
        .map(|e| e.grade)
        .max_by_key(|z| rank(*z))
        .map_or("proven", grade)
}

const fn rank(z: Grade) -> u8 {
    match z {
        Grade::Proven => 0,
        Grade::Observed => 1,
        Grade::Modeled => 2,
        Grade::Heuristic => 3,
    }
}

const fn grade(z: Grade) -> &'static str {
    match z {
        Grade::Proven => "proven",
        Grade::Observed => "observed",
        Grade::Modeled => "modeled",
        Grade::Heuristic => "heuristic",
    }
}

fn prefix(datum: &Datum) -> String {
    datum.0.rsplit_once(':').map_or_else(
        || datum.0.clone(),
        |(_, hash)| hash[..8.min(hash.len())].to_string(),
    )
}

/// One record, in the shared grammar: tag padded to eight, columns separated
/// by two spaces.
fn record(out: &mut String, tag: &str, columns: &[&str]) {
    out.push_str(tag);
    for _ in 0..8usize.saturating_sub(tag.len()).max(2) {
        out.push(' ');
    }
    out.push_str(&columns.join("  "));
    out.push('\n');
}
