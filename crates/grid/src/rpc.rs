//! Method layer shared by CLI and daemon.
use grid_wire::methods::{
    CacheCheckParams, CacheRecordParams, CacheRecordResult, CacheRecordResultRejectedItem,
    DeltaParams, DeltaResult, GateCheckParams, GateCheckResult, ResolveParams, ResolveResult,
    SnapshotCurrentResult, SurveyRunParams, SurveyRunResult, TrackLintParams, TrackLintResult,
    VersionResult, WakeDiffParams, WakeIngestParams, WakeIngestResult,
};
use grid_wire::{
    CacheCheckResult, CacheHit, CacheHitVerdict, CacheResultEntryVerdict, GitRev, Snapshot,
    SnapshotId, SnapshotOrRev, SnapshotStats,
};

use grid_xte::DeltaProvider as _;

use crate::chart_provider::{self, ChartDelta};
use crate::{standing, ENGINE_VERSION, WIRE_VERSION};

/// `grid.version` — versions plus the current snapshot, if the repo has one.
///
/// A repo that has never been surveyed reports no snapshot rather than a
/// placeholder: the field means "what the chart currently holds".
#[must_use]
pub fn version(root: &std::path::Path) -> VersionResult {
    VersionResult {
        wire: WIRE_VERSION.to_string(),
        engine: ENGINE_VERSION.to_string(),
        porcelain: Some(crate::porcelain::VERSION.to_string()),
        snapshot: ChartDelta::open(root).ok().and_then(|c| c.current()),
    }
}

/// `grid.track.lint` — pure validation, no survey, no snapshot. Pure of the
/// chart, that is: the repo's standing invariants are part of the rules a
/// promise is judged against, and reading them touches no snapshot.
///
/// # Errors
/// Returns a message when the standing invariant file exists but is invalid;
/// a broken rule file must not be mistaken for an absence of rules.
pub fn track_lint(
    params: &TrackLintParams,
    root: &std::path::Path,
) -> Result<TrackLintResult, String> {
    let track = standing::fold(&params.track, standing::load(root)?);
    Ok(grid_xte::lint::lint(&track))
}

/// `grid.gate.check` — the track against the delta at head.
///
/// The provider is the chart now, not Wave 3's file-level stand-in, so a
/// promise about an edge is checked against a surveyed edge rather than
/// reported `missing` for want of one.
///
/// Both endpoints are surveyed on demand. A gate that failed because nobody
/// had run a survey would be a gate that reports the tool's state instead of
/// the code's, and CI would have to learn a second command to avoid it.
///
/// # Errors
/// Propagates the provider's error (unknown snapshot, failed survey).
pub fn gate_check(
    params: &GateCheckParams,
    root: &std::path::Path,
) -> Result<GateCheckResult, chart_provider::Error> {
    let standing = standing::load(root).map_err(chart_provider::Error::Config)?;
    let track = standing::fold(&params.track, standing);
    let mut chart = Chart::new(root);
    chart.ensure(&track.base)?;
    chart.ensure(&params.head)?;
    grid_xte::check(&track, &params.head, &ChartDelta::open(root)?)
}

/// The repo's segment store, plus the survey cache that lives beside it.
///
/// The cache is in-process and per-call for the CLI (one survey per process,
/// so nothing to reuse) and held by the daemon across calls, which is what
/// makes a warm survey warm.
pub struct Chart {
    root: std::path::PathBuf,
    cache: grid_survey::Cache,
}

impl Chart {
    /// A chart rooted at `root`.
    #[must_use]
    pub fn new(root: &std::path::Path) -> Self {
        Self {
            root: root.to_path_buf(),
            cache: grid_survey::Cache::new(),
        }
    }

    /// Publish a snapshot for `target` if the store has none.
    ///
    /// A snapshot id must already exist — it names content, so it cannot be
    /// conjured — but a rev can always be surveyed, and doing it here is what
    /// lets `grid gate` be the only command CI runs.
    ///
    /// # Errors
    /// [`chart_provider::Error`] if the rev cannot be surveyed or published.
    pub fn ensure(&mut self, target: &SnapshotOrRev) -> Result<(), chart_provider::Error> {
        let SnapshotOrRev::GitRev { git_rev } = target else {
            return Ok(());
        };
        // Already surveyed at this rev: surveying again would produce the same
        // bytes and the same id (D5), so it is pure cost.
        if ChartDelta::open(&self.root)?.has_rev(&git_rev.0)? {
            return Ok(());
        }
        self.survey_run(&SurveyRunParams {
            git_rev: Some(git_rev.clone()),
            worktree: None,
        })
        .map(|_| ())
    }

    /// `grid.survey.run` — survey the tree and publish a snapshot.
    ///
    /// Idempotent by construction (D5): an unchanged tree produces the same
    /// segment bytes, so the same manifest, so the same id. Surveying twice
    /// is not a second snapshot.
    ///
    /// # Errors
    /// [`chart_provider::Error`] if the tree cannot be surveyed or the store
    /// cannot be written.
    pub fn survey_run(
        &mut self,
        params: &SurveyRunParams,
    ) -> Result<(SurveyRunResult, u64), chart_provider::Error> {
        // Surveying "at a rev" means the tree at that rev, read out of the
        // object store; with no rev it is the working tree, labelled with the
        // rev it was checked out from.
        let (survey, rev) = match params.git_rev.as_ref() {
            Some(rev) => (
                grid_survey::run_at(&self.root, &rev.0, &mut self.cache)?,
                chart_provider::resolve_rev(&self.root, &rev.0),
            ),
            None => (
                grid_survey::run(&self.root, &mut self.cache)?,
                chart_provider::resolve_rev(&self.root, "HEAD"),
            ),
        };
        // The daemon is the single writer (D6); `publish` takes `&mut self`,
        // which is that rule in the type system.
        let mut store = grid_chart_io::Store::open(chart_provider::store_dir(&self.root))
            .map_err(chart_provider::Error::Store)?;
        let id = store
            .publish(&rev, None, &[survey.image])
            .map_err(chart_provider::Error::Store)?;
        Ok((
            Snapshot {
                snapshot_id: SnapshotId(id.as_str().to_owned()),
                git_rev: GitRev(rev),
                created_at: String::new(),
                worktree_diff_hash: None,
                stats: SnapshotStats {
                    defs: survey.defs,
                    edges: survey.edges,
                    langs: Some(survey.langs),
                },
            },
            // How many files came from the cache rather than the parser. Not
            // a wire field — it describes this *run*, not the snapshot, which
            // is why it is returned beside the snapshot instead of inside it.
            survey.reread,
        ))
    }
}

/// `grid.snapshot.current` — the published snapshot, or `None` for a store
/// that has never been surveyed.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn snapshot_current(
    root: &std::path::Path,
) -> Result<Option<SnapshotCurrentResult>, chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    let Some(id) = chart.current() else {
        return Ok(None);
    };
    Ok(Some(Snapshot {
        snapshot_id: id,
        git_rev: GitRev(chart.git_rev().to_owned()),
        created_at: String::new(),
        worktree_diff_hash: None,
        stats: chart.stats(),
    }))
}

/// `grid.delta` — the actual delta between two snapshots or revs.
///
/// # Errors
/// [`chart_provider::Error`] if either endpoint does not resolve.
pub fn delta(
    params: &DeltaParams,
    root: &std::path::Path,
) -> Result<DeltaResult, chart_provider::Error> {
    let mut chart = Chart::new(root);
    chart.ensure(&params.base)?;
    chart.ensure(&params.head)?;
    ChartDelta::open(root)?.made_good(&params.base, &params.head)
}

/// `grid.resolve` — a locator to the definitions it names.
///
/// An unknown locator resolves to an empty list, not an error: "nothing is
/// called that" is an answer, and the wire contract says so.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn resolve(
    params: &ResolveParams,
    root: &std::path::Path,
) -> Result<ResolveResult, chart_provider::Error> {
    Ok(ChartDelta::open(root)?.resolve(&params.locator))
}

/// Where the cache log and the chart store live under a repo root. Both are
/// engine state, so both sit under `.grid/`.
fn cache_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".grid/cache/verdicts")
}

fn chart_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".grid/chart")
}

/// Open the live epoch. An empty store is not an error — it is a snapshot with
/// no defs, against which every cache check misses and every record is
/// rejected, which is the honest answer before a survey has run.
fn epoch(root: &std::path::Path) -> Result<std::sync::Arc<grid_chart_io::Epoch>, String> {
    grid_chart_io::Store::open(chart_dir(root))
        .map(|store| store.load())
        .map_err(|e| e.to_string())
}

/// `grid.cache.check` — look up frozen verdicts for a set of datums.
///
/// A datum is a hit only if its closure, recomputed against the live chart
/// *now*, keys to something recorded. Anything else is a miss: an unknown
/// datum, an unresolvable closure, and an absent key are all just "run it".
///
/// # Errors
/// Returns a message when the chart store or the cache log cannot be read. A
/// cache that cannot be read is an error, not a universal miss: silently
/// missing would hide a broken store behind a slow CI.
pub fn cache_check(
    params: &CacheCheckParams,
    root: &std::path::Path,
) -> Result<CacheCheckResult, String> {
    let epoch = epoch(root)?;
    let request = &params.request;
    // The request may name a snapshot; the engine holds exactly one live
    // epoch, so a request for any other snapshot cannot be answered from it.
    if let Some(want) = &request.snapshot {
        if want.0 != epoch.snapshot_id().as_str() {
            return Err(format!("snapshot {} is not the live epoch", want.0));
        }
    }
    let cache = grid_cache::Cache::open(cache_path(root)).map_err(|e| e.to_string())?;
    let chart = grid_cache::Chart::new(epoch.segments());
    let kind = grid_cache::Kind::from(request.kind);

    let mut hits = Vec::new();
    let mut misses = Vec::new();
    for datum in &request.datums {
        let hit = grid_chart::DatumId::parse(&datum.0)
            .and_then(|id| chart.key(id, kind, &request.env_hash).ok())
            .and_then(|key| cache.get(key).cloned());
        match hit {
            Some(entry) => hits.push(CacheHit {
                datum: datum.clone(),
                verdict: match entry.verdict {
                    grid_cache::Verdict::Pass => CacheHitVerdict::Pass,
                    grid_cache::Verdict::Fail => CacheHitVerdict::Fail,
                },
                recorded_at: entry.recorded_at,
                wake_id: entry.wake_id,
            }),
            None => misses.push(datum.clone()),
        }
    }
    Ok(CacheCheckResult {
        snapshot: grid_wire::SnapshotId(epoch.snapshot_id().as_str().to_string()),
        hits,
        misses,
    })
}

/// `grid.cache.record` — freeze keys and store verdicts.
///
/// Freezing happens here, at record time, against the chart the verdict was
/// produced on. A definition whose closure cannot be fully established is
/// rejected with a reason rather than stored under a partial key (claim C4):
/// a partial key is precisely how a stale verdict outlives the change it
/// depended on.
///
/// # Errors
/// Returns a message when the chart store or the cache log cannot be written.
pub fn cache_record(
    params: &CacheRecordParams,
    root: &std::path::Path,
) -> Result<CacheRecordResult, String> {
    let epoch = epoch(root)?;
    if let Some(want) = &params.snapshot {
        if want.0 != epoch.snapshot_id().as_str() {
            return Err(format!("snapshot {} is not the live epoch", want.0));
        }
    }
    let mut cache = grid_cache::Cache::open(cache_path(root)).map_err(|e| e.to_string())?;
    let chart = grid_cache::Chart::new(epoch.segments());
    let kind = grid_cache::Kind::from(params.kind);
    let recorded_at = grid_cache::now_rfc3339();

    let mut recorded = 0u64;
    let mut rejected = Vec::new();
    for entry in &params.results {
        let Some(id) = grid_chart::DatumId::parse(&entry.datum.0) else {
            rejected.push(CacheRecordResultRejectedItem {
                datum: entry.datum.clone(),
                reason: "not a b3 datum handle".to_string(),
            });
            continue;
        };
        match chart.key(id, kind, &params.env_hash) {
            Ok(key) => {
                cache
                    .put(
                        key,
                        grid_cache::Entry {
                            verdict: match entry.verdict {
                                CacheResultEntryVerdict::Pass => grid_cache::Verdict::Pass,
                                CacheResultEntryVerdict::Fail => grid_cache::Verdict::Fail,
                            },
                            recorded_at: recorded_at.clone(),
                            wake_id: params.wake_id.clone(),
                        },
                    )
                    .map_err(|e| e.to_string())?;
                recorded += 1;
            }
            Err(reject) => rejected.push(CacheRecordResultRejectedItem {
                datum: entry.datum.clone(),
                reason: reject.to_string(),
            }),
        }
    }
    Ok(CacheRecordResult { recorded, rejected })
}

/// `grid.wake.ingest` — resolve a test-run wake against the live chart,
/// retain it, and report what it meant.
///
/// Resolution is the `grid.resolve` rule (unique dotted-suffix), so the name
/// a user can resolve is the name a wake can. What cannot resolve is
/// returned, never dropped (C15): an observed edge always names two datums
/// that could be pointed at.
///
/// # Errors
/// Returns a message when the chart store or the wake log cannot be read or
/// written. Ingesting into an unsurveyed repo is an error even for an
/// all-datum wake: symbol resolution must never silently see an empty chart.
pub fn wake_ingest(
    params: &WakeIngestParams,
    root: &std::path::Path,
) -> Result<WakeIngestResult, String> {
    let epoch = epoch(root)?;
    let ingested = grid_wake::ingest(epoch.segments(), &params.wake);
    let store = grid_wake::Store::open(wake_dir(root)).map_err(|e| e.to_string())?;
    store
        .put(&params.wake, &ingested)
        .map_err(|e| e.to_string())?;
    #[allow(clippy::cast_possible_truncation)]
    let observed_edges = ingested.edges.len() as u64;
    Ok(WakeIngestResult {
        observed_edges,
        resolved: ingested.resolved,
        unresolved_symbols: ingested.unresolved_symbols,
    })
}
/// Why `grid.wake.diff` could not answer.
#[derive(Debug)]
pub enum WakeDiffError {
    /// A named wake was never ingested under that id.
    UnknownWake(String),
    /// The retention log could not be read or is corrupt — suspect, the way a
    /// corrupt segment is, not silently empty.
    Store(String),
}
impl std::fmt::Display for WakeDiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownWake(id) => write!(f, "unknown wake id {id}: ingest it first"),
            Self::Store(msg) => f.write_str(msg),
        }
    }
}
/// `grid.wake.diff` — two retained traces compared at call granularity.
///
/// # Errors
/// [`WakeDiffError::UnknownWake`] when either id names nothing retained;
/// [`WakeDiffError::Store`] when the log itself fails.
pub fn wake_diff(
    params: &WakeDiffParams,
    root: &std::path::Path,
) -> Result<grid_wire::WakeDiff, WakeDiffError> {
    let store =
        grid_wake::Store::open(wake_dir(root)).map_err(|e| WakeDiffError::Store(e.to_string()))?;
    let load = |id: &grid_wire::WakeId| {
        store
            .get(&id.0)
            .map_err(|e| WakeDiffError::Store(e.to_string()))?
            .ok_or_else(|| WakeDiffError::UnknownWake(id.0.clone()))
    };
    let (green, red) = (load(&params.green)?, load(&params.red)?);
    Ok(grid_wake::diff(&green, &red))
}
fn wake_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".grid/wakes")
}
