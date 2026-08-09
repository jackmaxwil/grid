//! The method layer: one function per wire method, shared verbatim by the CLI
//! and the daemon.
//!
//! The CLI is not a second implementation of the daemon — both call these
//! functions, so `grid gate --json` and a `grid.gate.check` request over the
//! socket cannot drift.

use grid_wire::methods::{
    CacheCheckParams, CacheRecordParams, CacheRecordResult, CacheRecordResultRejectedItem,
    GateCheckParams, GateCheckResult, TrackLintParams, TrackLintResult, VersionResult,
};
use grid_wire::{CacheCheckResult, CacheHit, CacheHitVerdict, CacheResultEntryVerdict};

use crate::provider::{self, FileDelta};
use crate::{standing, ENGINE_VERSION, WIRE_VERSION};

/// `grid.version` — versions plus the current snapshot, if any. There is no
/// snapshot store until Wave 4, so the field is honestly absent.
#[must_use]
pub fn version() -> VersionResult {
    VersionResult {
        wire: WIRE_VERSION.to_string(),
        engine: ENGINE_VERSION.to_string(),
        porcelain: Some(crate::porcelain::VERSION.to_string()),
        snapshot: None,
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
/// # Errors
/// Propagates the provider's error (unknown snapshot, failed diff).
pub fn gate_check(
    params: &GateCheckParams,
    root: &std::path::Path,
) -> Result<GateCheckResult, provider::Error> {
    let standing = standing::load(root).map_err(provider::Error::Config)?;
    let track = standing::fold(&params.track, standing);
    grid_xte::check(&track, &params.head, &FileDelta::new(root))
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
