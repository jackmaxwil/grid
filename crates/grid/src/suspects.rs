//! History-backed fault localization over retained traces.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use grid_gate::DeltaProvider as _;
use grid_wire::methods::SuspectsParams;
use grid_wire::{Datum, GitRev, SnapshotOrRev, Suspects, SuspectsRankedItem};

use crate::chart_provider::{self, ChartDelta};

#[derive(Debug)]
pub enum Error {
    UnknownTrace(String),
    InsufficientHistory(String),
    Chart(chart_provider::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTrace(id) => write!(f, "unknown trace: {id}"),
            Self::InsufficientHistory(why) => write!(f, "insufficient history: {why}"),
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

/// Rank changed definitions observed by the failing test using plain Ochiai.
pub fn rank(params: &SuspectsParams, root: &Path) -> Result<Suspects, Error> {
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
