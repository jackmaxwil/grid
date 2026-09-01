//! grid-trace — trace ingestion: test-run traces -> observed edges.
//!
//! A trace is what a test run *did*: an ordered list of call hops, each naming
//! a caller and a callee. Ingestion resolves every endpoint against the live
//! chart — a datum handle resolves itself, a symbol resolves by the
//! `grid.resolve` rule (unique dotted-suffix match on a callsign) — and folds
//! the hops into observed edges: `Calls` at `Grade::Observed`, deduplicated with
//! counts summed.
//!
//! Claim C16 is the honesty rule, and it is one-directional like the cache's:
//! an edge is stored only where *both* endpoints resolve, and every endpoint
//! that does not resolve — unknown name, ambiguous suffix, malformed handle —
//! is returned in `unresolved_symbols`, never silently dropped. An observed
//! edge therefore always names two datums that could be pointed at; the count
//! of what could not be pointed at is always visible.
//!
//! Traces are retained (one file per trace, named by the blake3 of its id, so
//! no id is also a path) because `grid.trace.diff` compares two traces after
//! the fact: same-hop-order comparison for the first divergent hop, and edge
//! sets for what each trace has that the other lacks (C17).
//!
//! Retention is also why [`grade`] exists. An observed edge asserts coupling
//! *as the code was when the trace was taken*: its endpoints are datums, and
//! a datum is content. Ask about a chart in which either endpoint is no
//! longer interned and the edge is a claim about code that does not exist
//! there — still a signal, no longer an observation. [`grade`] is the one
//! place that decides, so drift, the walk engine and every later consumer demote
//! the same edges for the same reason. The stored trace is never rewritten:
//! decay is a property of the question, not of the log, and the log stays
//! append-only.
use grid_chart::{DatumId, Segment};
use grid_wire::{Datum, Edge, Grade, Trace, TraceDiff, TraceDiffFirstDivergence, TraceEndpoint};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
/// Every observed edge is a call: a trace watches execution, and what execution
/// crosses between definitions is a call.
const RELATION: grid_wire::Relation = grid_wire::Relation::Calls;
/// One observed call, deduplicated across the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedEdge {
    /// The calling definition.
    pub caller: DatumId,
    /// The called definition.
    pub callee: DatumId,
    /// How many hops in the trace were this call.
    pub count: u64,
}
/// What ingesting one trace produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingested {
    /// The distinct observed edges, sorted by `(caller, callee)` so two
    /// ingestions of the same trace agree byte for byte.
    pub edges: Vec<ObservedEdge>,
    /// Hops whose endpoints both resolved. Each is in `hops` and contributes
    /// to exactly one edge.
    pub resolved: u64,
    /// Endpoint texts that resolved to nothing or to more than one datum,
    /// sorted and deduplicated. Never silently dropped (C16).
    pub unresolved_symbols: Vec<String>,
    /// The resolved hops in trace order. Retained beside the trace: order is
    /// what `grid.trace.diff`'s first-divergence walk compares, and an edge
    /// set has no order.
    hops: Vec<(DatumId, DatumId)>,
}
/// Resolve one endpoint against the chart.
///
/// A datum handle resolves by parsing; a symbol by unique dotted-suffix match
/// over every def's callsign — the same rule `grid.resolve` answers with, so
/// the name a user can resolve is the name a trace can. Failure returns the
/// endpoint's own text for `unresolved_symbols`.
fn endpoint<B: AsRef<[u8]>>(
    segments: &[Segment<B>],
    ep: &TraceEndpoint,
) -> Result<DatumId, String> {
    match ep {
        TraceEndpoint::Datum { datum } => DatumId::parse(&datum.0).ok_or_else(|| datum.0.clone()),
        TraceEndpoint::Symbol { symbol } => {
            let mut hit = None;
            let mut ambiguous = false;
            for segment in segments {
                for ord in 0..segment.def_count() {
                    let ord = grid_chart::Ordinal(ord);
                    let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord))
                    else {
                        continue;
                    };
                    if suffix_matches(callsign, symbol) {
                        if hit.is_some_and(|prev| prev != datum) {
                            ambiguous = true;
                        }
                        hit = Some(datum);
                    }
                }
            }
            if ambiguous {
                return Err(symbol.clone());
            }
            hit.ok_or_else(|| symbol.clone())
        }
    }
}
/// Does `callsign` end with `want` on a dotted-segment boundary? The input
/// rule `grid.resolve` documents: `validate` finds `pkg.TagContract.validate`,
/// and `revalidate` finds nothing, because a suffix that starts mid-name is
/// not a name.
fn suffix_matches(callsign: &str, want: &str) -> bool {
    callsign == want
        || callsign
            .strip_suffix(want)
            .is_some_and(|head| head.ends_with('.'))
}
/// The reason a demoted observed edge carries: the trace described code that
/// is not in the chart being asked about.
pub const STALE_TRACE: &str = "stale-trace";

/// An observed edge with the evidence it still deserves, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Graded {
    /// The edge itself, unchanged — grading never rewrites what was observed.
    pub edge: ObservedEdge,
    /// [`Grade::Observed`] while both endpoints are interned in the chart
    /// asked about, [`Grade::Heuristic`] once either is not.
    pub grade: Grade,
    /// [`STALE_TRACE`] on a demoted edge, `None` on a fresh one.
    pub reason: Option<&'static str>,
}

/// Grade an observed edge against a chart.
///
/// The rule is one line and deliberately one-directional: an edge whose two
/// endpoints are both interned in `segments` still describes code that
/// exists there, so it stays `observed`; anything else — an endpoint edited
/// since the trace was taken, an endpoint from a trace that named a datum no
/// chart ever held — is demoted to `heuristic` with [`STALE_TRACE`]. There
/// is no third answer: an endpoint that cannot be verified is not verified,
/// and a trace grid cannot check must never gate.
#[must_use]
pub fn grade<B: AsRef<[u8]>>(segments: &[Segment<B>], edge: ObservedEdge) -> Graded {
    let interned = |datum: DatumId| segments.iter().any(|s| s.ordinal(&datum).is_some());
    if interned(edge.caller) && interned(edge.callee) {
        Graded {
            edge,
            grade: Grade::Observed,
            reason: None,
        }
    } else {
        Graded {
            edge,
            grade: Grade::Heuristic,
            reason: Some(STALE_TRACE),
        }
    }
}

/// Distinct `(caller, callee)` pairs of a retained trace, with how many hops
/// each accounts for.
///
/// The count is the *trace's* multiplicity — how many retained hops were this
/// call — which is not always the `count` [`ingest`] reports: a trace may
/// declare `count: 5` for a hop it lists once, and retention keeps the hop
/// order, not the declaration. Nothing that consumes a retained edge reads
/// the count, and the pairs are identical either way.
#[must_use]
fn fold_edges(trace: &[(DatumId, DatumId)]) -> Vec<ObservedEdge> {
    let mut counts: BTreeMap<(DatumId, DatumId), u64> = BTreeMap::new();
    for hop in trace {
        *counts.entry(*hop).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|((caller, callee), count)| ObservedEdge {
            caller,
            callee,
            count,
        })
        .collect()
}

impl Stored {
    /// The observed edges this retained trace asserts, sorted and
    /// deduplicated. Grade them with [`grade`] before believing them.
    #[must_use]
    pub fn edges(&self) -> Vec<ObservedEdge> {
        fold_edges(&self.hops)
    }
}

impl Ingested {
    /// The resolved hops in trace order — what [`Store::put`] retains and
    /// what [`diff`] walks.
    #[must_use]
    pub fn hops(&self) -> &[(DatumId, DatumId)] {
        &self.hops
    }
}
/// Resolve a trace against the chart and fold it into observed edges.
///
/// Pure: the chart is read, nothing is written. Retention is [`Store::put`],
/// so the same function serves a dry run and a real ingest, and the two
/// cannot disagree about what a trace means.
#[must_use]
pub fn ingest<B: AsRef<[u8]>>(segments: &[Segment<B>], trace: &Trace) -> Ingested {
    let mut counts: BTreeMap<(DatumId, DatumId), u64> = BTreeMap::new();
    let mut hops = Vec::new();
    let mut unresolved = BTreeSet::new();
    for obs in &trace.observations {
        match (
            endpoint(segments, &obs.caller),
            endpoint(segments, &obs.callee),
        ) {
            (Ok(from), Ok(to)) => {
                hops.push((from, to));
                // A hop without a count happened once.
                *counts.entry((from, to)).or_insert(0) += obs.count.unwrap_or(1);
            }
            (from, to) => {
                // C16: report every endpoint that resolved to nothing. The
                // hop itself yields no edge — half an edge is not an edge.
                if let Err(text) = from {
                    unresolved.insert(text);
                }
                if let Err(text) = to {
                    unresolved.insert(text);
                }
            }
        }
    }
    let edges = counts
        .into_iter()
        .map(|((caller, callee), count)| ObservedEdge {
            caller,
            callee,
            count,
        })
        .collect::<Vec<_>>();
    #[allow(clippy::cast_possible_truncation)]
    let resolved = hops.len() as u64;
    Ingested {
        resolved,
        unresolved_symbols: unresolved.into_iter().collect(),
        edges,
        hops,
    }
}
/// Trace retention: one file per trace under one directory.
///
/// The file name is the blake3 hex of the trace id, so an id is never also a
/// path — `trace:01J4…` and stranger strings all land inside the directory.
/// Writes go through a temporary file and a rename, so a reader never meets
/// half a trace.
pub struct Store {
    dir: PathBuf,
}
/// A retained trace: what arrived, plus the resolved hop order ingestion gave
/// it. The trace is what diffing needs; the trace itself is kept for
/// provenance (`git_rev`, `command`) and so `unresolved_symbols` stays
/// reproducible without the chart that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    /// The trace as it arrived.
    pub trace: Trace,
    /// Resolved hops in trace order.
    pub hops: Vec<(DatumId, DatumId)>,
}
/// The on-disk shape. Traces are datum handles, not bytes: the log stays
/// human-readable, and [`DatumId::parse`] is the only way back in.
#[derive(serde::Serialize, serde::Deserialize)]
struct Record {
    trace: Trace,
    hops: Vec<(String, String)>,
}
/// Why a store operation failed.
#[derive(Debug)]
pub enum StoreError {
    /// The directory or a file could not be read or written.
    Io(std::io::Error),
    /// A retained trace does not parse or disagrees with the id it was filed
    /// under. The log is suspect, the way a corrupt segment is.
    Corrupt(String),
    /// Two candidate store directories exist and nothing says which one is
    /// current. Retained traces are user data; guessing could drop a run.
    Ambiguous(String),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "trace store: {e}"),
            Self::Corrupt(why) => write!(f, "trace store: corrupt record: {why}"),
            Self::Ambiguous(why) => write!(f, "trace store: {why}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl Store {
    /// Open (creating if needed) the trace directory.
    ///
    /// # Errors
    /// [`StoreError::Io`] if the directory cannot be created.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        std::fs::create_dir_all(dir.as_ref())?;
        Ok(Self {
            dir: dir.as_ref().to_path_buf(),
        })
    }
    fn path(&self, trace_id: &str) -> PathBuf {
        self.dir.join(format!(
            "{}.json",
            blake3::hash(trace_id.as_bytes()).to_hex()
        ))
    }
    /// Retain a trace and its resolved trace. Putting the same id twice
    /// replaces: a trace id names a run, and the later report of that run is
    /// the one to believe.
    ///
    /// # Errors
    /// [`StoreError::Io`] if the file cannot be written.
    pub fn put(&self, trace: &Trace, ingested: &Ingested) -> Result<(), StoreError> {
        let record = Record {
            trace: trace.clone(),
            hops: ingested
                .hops
                .iter()
                .map(|(caller, callee)| (caller.to_string(), callee.to_string()))
                .collect(),
        };
        let json = serde_json::to_vec(&record).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let path = self.path(&trace.trace_id);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
    /// Every retained trace, in a deterministic order (by trace id).
    ///
    /// A file that does not parse is a corrupt log, not an absence: it is
    /// returned as an error, the same posture [`Store::get`] takes, because
    /// a consumer that silently skipped it would answer "no observations"
    /// for a store that has them.
    ///
    /// # Errors
    /// [`StoreError::Io`] if the directory cannot be read;
    /// [`StoreError::Corrupt`] if a retained record does not parse.
    pub fn retained(&self) -> Result<Vec<Stored>, StoreError> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            // A store nobody has written to holds no traces. That is a state,
            // not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(StoreError::Io(e)),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let record: Record = serde_json::from_slice(&bytes)
                .map_err(|e| StoreError::Corrupt(format!("{}: {e}", path.display())))?;
            let mut hops = Vec::with_capacity(record.hops.len());
            for (caller, callee) in &record.hops {
                let parse = |text: &str| {
                    DatumId::parse(text).ok_or_else(|| {
                        StoreError::Corrupt(format!("{text} is not a b3 datum handle"))
                    })
                };
                hops.push((parse(caller)?, parse(callee)?));
            }
            out.push(Stored {
                trace: record.trace,
                hops,
            });
        }
        out.sort_by(|a, b| a.trace.trace_id.cmp(&b.trace.trace_id));
        Ok(out)
    }

    /// Load a retained trace, if the id is known.
    ///
    /// # Errors
    /// [`StoreError::Corrupt`] if a retained record does not parse, parses to
    /// a different trace id than it was filed under, or holds a datum handle
    /// that is not one — the log is suspect, not the caller.
    pub fn get(&self, trace_id: &str) -> Result<Option<Stored>, StoreError> {
        let path = self.path(trace_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(StoreError::Io(e)),
        };
        let record: Record =
            serde_json::from_slice(&bytes).map_err(|e| StoreError::Corrupt(e.to_string()))?;
        if record.trace.trace_id != trace_id {
            return Err(StoreError::Corrupt(format!(
                "filed under {trace_id}, holds {}",
                record.trace.trace_id
            )));
        }
        let mut hops = Vec::with_capacity(record.hops.len());
        for (caller, callee) in &record.hops {
            let parse = |text: &str| {
                DatumId::parse(text)
                    .ok_or_else(|| StoreError::Corrupt(format!("{text} is not a b3 datum handle")))
            };
            hops.push((parse(caller)?, parse(callee)?));
        }
        Ok(Some(Stored {
            trace: record.trace,
            hops,
        }))
    }
}
/// Compare two retained traces.
///
/// Two comparisons, because they answer two questions. *Where did behavior
/// fork* is the first index at which the ordered traces disagree — one side
/// calling what the other did not, or running out first. *What differs* is
/// the edge sets: `added` is what red does that green never did, `removed`
/// what green did that red never does. Both are datum-space; the trace store
/// has no callsigns to dress them in.
#[must_use]
pub fn diff(green: &Stored, red: &Stored) -> TraceDiff {
    let first_divergence = (0..=green.hops.len().min(red.hops.len())).find_map(|i| {
        let (g, r) = (green.hops.get(i), red.hops.get(i));
        if g == r {
            return None;
        }
        // The caller is green's where green reaches this hop — green is the
        // reference the diff reads from — else red's, the side still running.
        // `g != r` above, so at least one side reaches hop `i`.
        let (caller, _) = g.or(r)?;
        Some(TraceDiffFirstDivergence {
            caller: Datum(caller.to_string()),
            green_callee: g.map(|hop| Datum(hop.1.to_string())),
            red_callee: r.map(|hop| Datum(hop.1.to_string())),
        })
    });
    let edge_set =
        |stored: &Stored| -> BTreeSet<(DatumId, DatumId)> { stored.hops.iter().copied().collect() };
    let wire_edges = |pairs: BTreeSet<(DatumId, DatumId)>| -> Vec<Edge> {
        pairs
            .into_iter()
            .map(|(src, dst)| Edge {
                src: Datum(src.to_string()),
                relation: RELATION,
                dst: Datum(dst.to_string()),
                grade: Grade::Observed,
                via: None,
            })
            .collect()
    };
    let (g, r) = (edge_set(green), edge_set(red));
    TraceDiff {
        first_divergence,
        added: wire_edges(&r - &g),
        removed: wire_edges(&g - &r),
    }
}
