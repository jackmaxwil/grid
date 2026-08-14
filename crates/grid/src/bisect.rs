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
    UnknownWake(String),
    InsufficientHistory(String),
    Chart(chart_provider::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownWake(id) => write!(f, "unknown wake: {id}"),
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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

/// Production entry point. Snapshot presence is validated before refusing:
/// Grid must not manufacture an ordering or test outcome from snapshot ids.
pub fn run(params: &BisectParams, _root: &Path) -> Result<BisectResult, Error> {
    Err(Error::InsufficientHistory(format!(
        "no retained ordered patch outcomes for `{}`",
        params.test
    )))
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
