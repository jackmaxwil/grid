//! Fault localization, in two evidence modes.
//!
//! The first is history-backed: plain Ochiai over retained spectra, which
//! needs a populated verdict cache and refuses without one (C23). The second
//! is [`rank_closure`], which needs no history at all — the common failure is
//! "it passed at the last snapshot and fails now", and the static closure
//! plus a membership diff answer that in milliseconds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use grid_chart::{Change, DatumId, Segment};
use grid_gate::DeltaProvider as _;
use grid_wire::methods::SuspectsParams;
use grid_wire::{Datum, GitRev, SnapshotOrRev, Suspects, SuspectsRankedItem};

use crate::chart_provider::{self, ChartDelta};

#[derive(Debug)]
pub enum Error {
    UnknownTrace(String),
    InsufficientHistory(String),
    /// The test name does not resolve to exactly one definition in the live
    /// snapshot. Bad input: two definitions is not one closure, and none is
    /// not an empty closure.
    UnknownTest(String),
    /// The snapshot `--closure` named is not one this store can answer
    /// against. A typed refusal, never an empty ranking — "nothing moved"
    /// and "I have never held that snapshot" are opposite answers.
    UnknownSince(String),
    /// The closure walk ran past [`grid_cache::FUEL`]. Refused whole (C4's
    /// rule): a truncated closure would list fewer candidates and look
    /// cleaner for it.
    ClosureRejected(grid_cache::Reject),
    Chart(chart_provider::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTrace(id) => write!(f, "unknown trace: {id}"),
            Self::InsufficientHistory(why) => write!(f, "insufficient history: {why}"),
            Self::UnknownTest(why) | Self::UnknownSince(why) => f.write_str(why),
            Self::ClosureRejected(reject) => write!(f, "{reject} — refused whole"),
            Self::Chart(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<chart_provider::Error> for Error {
    fn from(error: chart_provider::Error) -> Self {
        Self::Chart(error)
    }
}

#[derive(serde::Deserialize)]
struct Retained {
    trace: grid_wire::Trace,
    hops: Vec<(String, String)>,
}

/// Rank changed definitions observed by the failing test using plain Ochiai,
/// or — when `closure_since` is set — by hop distance in the test's closure.
///
/// # Errors
/// Whatever the selected mode refuses.
pub fn rank(params: &SuspectsParams, root: &Path) -> Result<Suspects, Error> {
    if let Some(since) = &params.closure_since {
        return rank_closure(params, since, root).map(|found| found.suspects);
    }
    ochiai(params, root)
}

/// Rank changed definitions observed by the failing test using plain Ochiai.
fn ochiai(params: &SuspectsParams, root: &Path) -> Result<Suspects, Error> {
    let records = retained(root)?;
    let outcomes = outcomes(root)?;
    let usable: Vec<_> = records
        .iter()
        .filter_map(|record| {
            (record.trace.command.as_deref() == Some(params.test.as_str()))
                .then(|| {
                    outcomes
                        .get(&record.trace.trace_id)
                        .copied()
                        .map(|v| (record, v))
                })
                .flatten()
        })
        .collect();
    if usable.is_empty() {
        return Err(Error::InsufficientHistory(format!(
            "no retained spectra with verdicts for `{}`",
            params.test
        )));
    }

    let failing = if let Some(trace) = &params.trace {
        let record = records
            .iter()
            .find(|record| record.trace.trace_id == trace.0)
            .ok_or_else(|| Error::UnknownTrace(trace.0.clone()))?;
        if outcomes.get(&trace.0) != Some(&false) {
            return Err(Error::InsufficientHistory(format!(
                "trace `{}` has no failing verdict for `{}`",
                trace.0, params.test
            )));
        }
        record
    } else {
        usable
            .iter()
            .filter(|(_, passed)| !*passed)
            .map(|(record, _)| *record)
            .max_by(|a, b| a.trace.trace_id.cmp(&b.trace.trace_id))
            .ok_or_else(|| {
                Error::InsufficientHistory(format!("no failing trace for `{}`", params.test))
            })?
    };

    let base = params
        .base
        .clone()
        .unwrap_or_else(|| SnapshotOrRev::GitRev {
            git_rev: GitRev(format!("{}~1", failing.trace.git_rev.0)),
        });
    let head = SnapshotOrRev::GitRev {
        git_rev: failing.trace.git_rev.clone(),
    };
    let delta = ChartDelta::open(root)?.diff(&base, &head)?;
    let changed: BTreeSet<String> = delta
        .defs
        .iter()
        .flat_map(|change| [change.before.as_ref(), change.after.as_ref()])
        .flatten()
        .map(|datum| datum.0.clone())
        .collect();
    let failed_trace = spectrum(failing);
    let candidates: BTreeSet<_> = changed.intersection(&failed_trace).cloned().collect();
    if candidates.is_empty() {
        return Err(Error::InsufficientHistory(
            "failing trace observed no changed datum".into(),
        ));
    }

    let spectra: Vec<_> = usable
        .iter()
        .map(|(record, passed)| (spectrum(record), *passed))
        .collect();
    Ok(Suspects {
        ranked: score(candidates, &spectra)?,
    })
}

/// Plain Ochiai over retained spectra: descending suspicion, with datum
/// identity as the deterministic tie-break between equal scores.
fn score(
    candidates: impl IntoIterator<Item = String>,
    spectra: &[(BTreeSet<String>, bool)],
) -> Result<Vec<SuspectsRankedItem>, Error> {
    let total_failed = u32::try_from(spectra.iter().filter(|(_, passed)| !*passed).count())
        .map_err(|_| Error::InsufficientHistory("too many retained failing spectra".into()))?;
    let mut ranked = candidates
        .into_iter()
        .map(|datum| {
            let (failed, passed) = spectra.iter().fold((0_u32, 0_u32), |counts, (trace, ok)| {
                if trace.contains(&datum) {
                    if *ok {
                        (counts.0, counts.1 + 1)
                    } else {
                        (counts.0 + 1, counts.1)
                    }
                } else {
                    counts
                }
            });
            let score =
                f64::from(failed) / (f64::from(total_failed) * f64::from(failed + passed)).sqrt();
            SuspectsRankedItem {
                datum: Datum(datum),
                score,
                via: "changed in delta and observed in failing trace".into(),
                // Ochiai ranks by spectra, not by distance: no hop count to
                // report, and reporting one would invite a consumer to sort
                // on a number this mode never computed.
                hops: None,
            }
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.datum.0.cmp(&b.datum.0))
    });
    Ok(ranked)
}

/// What the closure mode found: the ranking, and — when two retained traces
/// of the test straddle `since` — where behaviour parted.
///
/// The fork is not on the wire: the plan's additive fields for this step are
/// `closure_since` and `hops`, so the divergence rides the porcelain only.
pub struct Closure {
    /// The ranking, `hops` set on every row.
    pub suspects: Suspects,
    /// The first divergence between the trace taken at `since` and the newest
    /// retained trace of the same test, when both exist.
    pub fork: Option<grid_wire::TraceDiff>,
}

/// Attribution with no run history: the definitions the test can reach at
/// head whose bodies moved since a named snapshot, ranked by hop distance.
///
/// Heuristic by construction — a candidate is a definition that changed and
/// is reachable, not a proven cause — so the caller exits `0` on any answer
/// this returns.
///
/// # Errors
/// [`Error::UnknownTest`] when the name is not exactly one live definition;
/// [`Error::UnknownSince`] when `since` is not a snapshot the retained chain
/// holds; [`Error::ClosureRejected`] when the walk ran past its fuel.
pub fn rank_closure(
    params: &SuspectsParams,
    since: &SnapshotOrRev,
    root: &Path,
) -> Result<Closure, Error> {
    let chart = ChartDelta::open(root)?;
    let head = chart.head();
    if head.segments().is_empty() {
        return Err(Error::UnknownSince(
            "no snapshot published: run `grid survey` first".into(),
        ));
    }
    let test = test_datum(&chart, &params.test)?;
    // The candidate set and its refusal are `grid_cache`'s: one definition of
    // "the closure under Kind::Test", so an answer here and a cache key
    // cannot disagree about what the test reaches.
    let closure = grid_cache::Chart::new(head.segments())
        .closure(test, grid_cache::Kind::Test)
        .map_err(Error::ClosureRejected)?;
    // The coordinate only: a membership diff reads its own image, so mapping
    // and validating the whole `since` epoch would be work this answer never
    // touches — and it is what put the question over its millisecond budget.
    let since_id = chart
        .snapshot_of(since)
        .map_err(|error| Error::UnknownSince(error.to_string()))?;
    let moved = moved_at(root, &since_id.0, head.snapshot_id().as_str())?;
    let depths = depths(head.segments(), test, &closure);

    let mut ranked: Vec<_> = closure
        .iter()
        .filter(|datum| moved.contains(datum))
        .filter_map(|datum| {
            let hops = *depths.get(datum)?;
            Some(SuspectsRankedItem {
                datum: Datum(datum.to_string()),
                // Hop distance rendered monotonically into the schema's
                // [0,1] score, so a consumer sorting on `score` and one
                // sorting on `hops` agree. The ranking is the distance;
                // this is not a probability and does not pretend to be.
                score: 1.0 / (1.0 + f64::from(u32::try_from(hops).unwrap_or(u32::MAX))),
                via: format!(
                    "via {hops} hop{}, datum moved",
                    if hops == 1 { "" } else { "s" }
                ),
                hops: Some(hops),
            })
        })
        .collect();
    ranked.sort_by(|a, b| a.hops.cmp(&b.hops).then_with(|| a.datum.0.cmp(&b.datum.0)));

    let fork = fork(root, &params.test, &since_id.0);
    Ok(Closure {
        suspects: Suspects { ranked },
        fork,
    })
}

/// The one live definition a test name binds to.
///
/// The `grid.resolve` rule, so the name a user can resolve is the name a
/// closure can be walked from. Ambiguity resolves to nothing.
fn test_datum(chart: &ChartDelta, test: &str) -> Result<DatumId, Error> {
    let hits = chart.resolve(&grid_wire::Locator::Callsign {
        callsign: grid_wire::Callsign(test.to_owned()),
    });
    match hits.as_slice() {
        [one] => DatumId::parse(&one.datum.0)
            .ok_or_else(|| Error::UnknownTest(format!("`{test}` resolves to an unreadable datum"))),
        [] => Err(Error::UnknownTest(format!(
            "`{test}` names no definition in the live snapshot"
        ))),
        many => Err(Error::UnknownTest(format!(
            "`{test}` is ambiguous: {} definitions carry that name",
            many.len()
        ))),
    }
}

/// Every datum the head snapshot holds that `since` did not — one merge-diff
/// over two sorted membership rows, nothing loaded and nothing compared but
/// ordinals.
///
/// The head-side half, because the closure was walked at head: a definition
/// whose body moved has a different datum at each end by construction, so the
/// base-side half of the same diff cannot intersect a head closure at all.
/// "The datum moved" and "head holds a datum since never held" are the same
/// fact read from the two ends, and this is the end the question is asked at.
fn moved_at(root: &Path, since: &str, head: &str) -> Result<BTreeSet<DatumId>, Error> {
    let store = grid_chart_io::Store::open(chart_provider::store_dir(root))
        .map_err(chart_provider::Error::Store)?;
    let membership = store
        .membership()
        .map_err(chart_provider::Error::Store)?
        .ok_or_else(|| {
            Error::UnknownSince(
                "no membership image beside the chart: run `grid survey` first".into(),
            )
        })?;
    Ok(membership
        .changed_between(since, head)
        .map_err(|error| Error::UnknownSince(error.to_string()))?
        .into_iter()
        .filter_map(|change| match change {
            Change::Added(datum) => Some(datum),
            Change::Removed(_) => None,
        })
        .collect())
}

/// Hop distance from `from` to every member of `within`, over the same edges
/// `Kind::Test`'s closure follows — every one, whatever its grade.
///
/// No fuel of its own: `within` is a closure [`grid_cache::Chart::closure`]
/// already accepted under `FUEL`, and this walk never leaves it, so it
/// expands each member exactly once and terminates in `within.len()` steps.
fn depths<B: AsRef<[u8]>>(
    segments: &[Segment<B>],
    from: DatumId,
    within: &BTreeSet<DatumId>,
) -> BTreeMap<DatumId, u64> {
    let mut depth = BTreeMap::from([(from, 0)]);
    let mut layer = vec![from];
    let mut hop = 0_u64;
    while !layer.is_empty() {
        hop = hop.saturating_add(1);
        let mut next = Vec::new();
        for current in layer {
            for segment in segments {
                let Some(ord) = segment.ordinal(&current) else {
                    continue;
                };
                for edge in segment.out_edges(ord) {
                    let Some(other) = segment.datum(edge.other) else {
                        continue;
                    };
                    if within.contains(&other) && !depth.contains_key(&other) {
                        depth.insert(other, hop);
                        next.push(other);
                    }
                }
            }
        }
        layer = next;
    }
    depth
}

/// Where the test's behaviour parted, when two retained traces of it straddle
/// `since`.
///
/// Green is the trace mapped against the `since` snapshot itself; red is the
/// newest other trace of the same test. Both must exist — a fork is never
/// inferred from one trace, and a missing green is silence, not a divergence.
fn fork(root: &Path, test: &str, since: &str) -> Option<grid_wire::TraceDiff> {
    let traces = crate::rpc::trace_dir(root)
        .and_then(grid_trace::Store::open)
        .and_then(|store| store.retained())
        .ok()?;
    let mine: Vec<_> = traces
        .iter()
        .filter(|stored| stored.trace.command.as_deref() == Some(test))
        .collect();
    let green = mine.iter().find(|stored| {
        stored
            .trace
            .snapshot
            .as_ref()
            .is_some_and(|id| id.0 == since)
    })?;
    let red = mine
        .iter()
        .filter(|stored| stored.trace.trace_id != green.trace.trace_id)
        .max_by(|a, b| {
            (&a.trace.started_at, &a.trace.trace_id).cmp(&(&b.trace.started_at, &b.trace.trace_id))
        })?;
    Some(grid_trace::diff(green, red))
}

fn spectrum(record: &Retained) -> BTreeSet<String> {
    record
        .hops
        .iter()
        .flat_map(|(caller, callee)| [caller.clone(), callee.clone()])
        .collect()
}

fn retained(root: &Path) -> Result<Vec<Retained>, Error> {
    let dir = crate::rpc::trace_dir(root)
        .map_err(|e| Error::Chart(chart_provider::Error::Config(e.to_string())))?;
    let entries = std::fs::read_dir(&dir).map_err(|error| {
        Error::InsufficientHistory(format!("cannot read {}: {error}", dir.display()))
    })?;
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| config(&error))?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|error| config(&error))?;
        records.push(serde_json::from_slice(&bytes).map_err(|error| {
            Error::Chart(chart_provider::Error::Config(format!(
                "{}: {error}",
                entry.path().display()
            )))
        })?);
    }
    Ok(records)
}

fn outcomes(root: &Path) -> Result<BTreeMap<String, bool>, Error> {
    let path = root.join(".grid/cache/verdicts");
    let text = std::fs::read_to_string(&path).map_err(|error| {
        Error::InsufficientHistory(format!("cannot read {}: {error}", path.display()))
    })?;
    let mut outcomes = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        // `>= 4`: the verdict log grows at the end, and a line from a newer
        // engine still carries its trace id in the same column.
        if fields.len() >= 4 && fields[3] != "-" {
            match fields[1] {
                "pass" => {
                    outcomes.insert(fields[3].to_owned(), true);
                }
                "fail" => {
                    outcomes.insert(fields[3].to_owned(), false);
                }
                _ => {}
            }
        }
    }
    Ok(outcomes)
}

fn config(error: &std::io::Error) -> Error {
    Error::Chart(chart_provider::Error::Config(error.to_string()))
}

#[cfg(test)]
mod tests {
    //! Seeded spectra here are algorithm fixtures. They are not evidence of
    //! accumulated production CI history.
    use super::{score, BTreeSet};

    fn datum(name: &str) -> String {
        format!("b3:{name}")
    }

    fn trace(names: &[&str], passed: bool) -> (BTreeSet<String>, bool) {
        (names.iter().map(|name| datum(name)).collect(), passed)
    }

    /// C21: the injected culprit is the only candidate exclusive to failing
    /// spectra, so plain Ochiai must rank it first regardless of input order.
    #[test]
    fn seeded_spectra_rank_injected_culprit_first() {
        let spectra = [
            trace(&["culprit", "shared", "rare"], false),
            trace(&["culprit", "shared"], false),
            trace(&["culprit", "shared"], false),
            trace(&["shared"], true),
            trace(&["shared"], true),
        ];
        // Reverse-sorted input: a first-place culprit cannot come from order.
        let candidates = vec![datum("shared"), datum("rare"), datum("culprit")];

        let ranked = score(candidates, &spectra).expect("seeded spectra rank");

        let order: Vec<_> = ranked.iter().map(|item| item.datum.0.as_str()).collect();
        assert_eq!(
            order,
            [datum("culprit"), datum("shared"), datum("rare")]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        assert!(
            (ranked[0].score - 1.0).abs() < 1e-12,
            "culprit exclusive to every failing spectrum scores 1.0: {}",
            ranked[0].score
        );
        assert!(
            ranked[0].score > ranked[1].score,
            "first place must be strictly above the next suspect"
        );
    }

    /// C21: equal scores break on datum identity, ascending, not on input or
    /// map iteration order.
    #[test]
    fn equal_scores_break_deterministically_on_datum_identity() {
        let spectra = [
            trace(&["zzz", "aaa"], false),
            trace(&["zzz", "aaa"], false),
            trace(&["zzz", "aaa"], true),
        ];
        let ranked = score(vec![datum("zzz"), datum("aaa")], &spectra).expect("seeded spectra");

        assert_eq!(ranked[0].datum.0, datum("aaa"));
        assert_eq!(ranked[1].datum.0, datum("zzz"));
        assert!(
            (ranked[0].score - ranked[1].score).abs() < f64::EPSILON,
            "the tie-break only matters when the scores are equal"
        );

        let reversed = score(vec![datum("aaa"), datum("zzz")], &spectra).expect("seeded spectra");
        assert_eq!(
            reversed.iter().map(|i| &i.datum.0).collect::<Vec<_>>(),
            ranked.iter().map(|i| &i.datum.0).collect::<Vec<_>>(),
            "ranking must not depend on candidate input order"
        );
    }
}
