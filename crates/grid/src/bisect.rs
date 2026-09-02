//! `grid.bisect` — cache-assisted search over retained patch history.
//!
//! Grid currently retains snapshots, not an ordered patch DAG with test
//! outcomes. The wire method therefore fails closed until that history exists;
//! the pure search below is the algorithm used once a trustworthy chain is
//! available and is exercised with seeded history.

use std::path::Path;

use grid_wire::methods::BisectParams;
use grid_wire::{BisectResult, Datum};

use crate::chart_provider;

#[derive(Debug)]
pub enum Error {
    #[allow(dead_code)]
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

/// Outcome for one retained patch probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub culprit: Datum,
    pub fails: bool,
    pub cached: bool,
}

/// Find the first failing patch in an ordered good-to-bad chain.
///
/// The boundary entries must be good then bad. `cache_hits` counts only
/// midpoint outcomes supplied by retained cache data.
pub fn search(chain: &[Probe]) -> Result<BisectResult, Error> {
    if chain.len() < 2 || chain[0].fails || !chain[chain.len() - 1].fails {
        return Err(Error::InsufficientHistory(
            "bisect needs an ordered good-to-bad patch chain".into(),
        ));
    }
    let (mut good, mut bad) = (0usize, chain.len() - 1);
    let (mut steps, mut cache_hits) = (0_u64, 0_u64);
    while bad - good > 1 {
        let probe = good + (bad - good) / 2;
        steps += 1;
        cache_hits += u64::from(chain[probe].cached);
        if chain[probe].fails {
            bad = probe;
        } else {
            good = probe;
        }
    }
    Ok(BisectResult {
        culprit: Some(chain[bad].culprit.clone()),
        steps,
        cache_hits,
    })
}

/// Production entry point: the retained patch log in append order, each edit
/// probed with the outcome recorded for `(its datum, this test)`. A edit
/// with no recorded outcome cannot be probed and is skipped; a chain whose
/// probed ends are not good-then-bad still refuses — fail closed stays the
/// contract, the condition just became satisfiable.
pub fn run(params: &BisectParams, root: &Path) -> Result<BisectResult, Error> {
    let outcomes = crate::history::outcomes(root);
    let verdict_of = |datum: &str| {
        outcomes
            .iter()
            .rev()
            .find(|o| o.test == params.test && o.datum == datum)
            .map(|o| o.fails)
    };
    let mut chain = Vec::new();
    for edit in crate::history::read(root) {
        if chain.is_empty() {
            if let Some(fails) = verdict_of(&edit.from.0) {
                chain.push(Probe {
                    culprit: edit.from.clone(),
                    fails,
                    cached: true,
                });
            }
        }
        if let Some(fails) = verdict_of(&edit.to.0) {
            chain.push(Probe {
                culprit: edit.to.clone(),
                fails,
                cached: true,
            });
        }
    }
    if chain.len() < 2 {
        return Err(Error::InsufficientHistory(format!(
            "no retained ordered patch outcomes for `{}`",
            params.test
        )));
    }
    search(&chain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datum(n: u8) -> Datum {
        Datum(format!("b3:{n:064x}"))
    }

    #[test]
    fn seeded_chain_finds_first_failure_with_measured_cache_hits() {
        let chain = [
            Probe {
                culprit: datum(0),
                fails: false,
                cached: false,
            },
            Probe {
                culprit: datum(1),
                fails: false,
                cached: true,
            },
            Probe {
                culprit: datum(2),
                fails: false,
                cached: true,
            },
            Probe {
                culprit: datum(3),
                fails: true,
                cached: true,
            },
            Probe {
                culprit: datum(4),
                fails: true,
                cached: false,
            },
        ];
        let result = search(&chain).expect("ordered seeded history");
        assert_eq!(result.culprit, Some(datum(3)));
        assert!(result.steps > 0);
        assert!(result.cache_hits * 2 > result.steps);
    }

    #[test]
    fn invalid_history_refuses() {
        let chain = [
            Probe {
                culprit: datum(0),
                fails: false,
                cached: false,
            },
            Probe {
                culprit: datum(1),
                fails: false,
                cached: false,
            },
        ];
        assert!(matches!(search(&chain), Err(Error::InsufficientHistory(_))));
    }
}
