//! lcov -> wake: line coverage translated into test-reach observations.
//!
//! `cargo llvm-cov --lcov` (and lcov proper) emit `SF:<path>` file records
//! holding `DA:<line>,<count>` execution counts. Every executed line that
//! lands inside a charted definition becomes one observation
//! `test -calls-> definition`, deduplicated with executed-line counts
//! summed; lines outside every definition (module level, macro output) are
//! counted and reported, never silently dropped — the same C16 honesty rule
//! ingestion itself holds. Only two record kinds are read; the rest of the
//! lcov vocabulary is skipped by design.
//!
//! An lcov wake is a *reach set*: hop order is file order and means
//! nothing, so `grid wake diff`'s first-divergence walk is not meaningful
//! over these — the edge sets are.
use std::collections::BTreeMap;

use grid_chart::Ordinal;
use grid_wire::{Wake, WakeEndpoint, WakeObservation};

use crate::chart_provider::{self, ChartDelta};

/// Everything one lcov file mapped to, before ingestion.
pub struct LcovWake {
    pub wake: Wake,
    /// Executed lines that landed in no charted definition.
    pub unmapped_lines: u64,
    /// `SF:` files the chart holds no definitions for.
    pub unknown_files: u64,
}

/// Map an lcov report onto the live chart as observations from `test`.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn wake_of(
    root: &std::path::Path,
    lcov: &str,
    test: &str,
) -> Result<LcovWake, chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    // path -> [(line, line_end, datum)]; innermost containment = smallest
    // line span, so nested definitions win over their parents.
    let mut by_path: BTreeMap<String, Vec<(u32, u32, String)>> = BTreeMap::new();
    for segment in chart.head().segments() {
        for i in 0..segment.def_count() {
            let ord = Ordinal(i);
            let (Some(loc), Some(datum)) = (segment.location(ord), segment.datum(ord)) else {
                continue;
            };
            by_path.entry(loc.path.to_owned()).or_default().push((
                loc.line,
                loc.line_end,
                datum.to_string(),
            ));
        }
    }

    let mut touched: BTreeMap<String, u64> = BTreeMap::new();
    let (mut unmapped_lines, mut unknown_files) = (0u64, 0u64);
    let mut current: Option<&Vec<(u32, u32, String)>> = None;
    for line in lcov.lines() {
        if let Some(sf) = line.strip_prefix("SF:") {
            // lcov paths are usually absolute; chart paths are repo-relative.
            // Suffix match on a path-segment boundary bridges the two.
            current = by_path.iter().find_map(|(path, defs)| {
                let hit = sf == path
                    || sf
                        .strip_suffix(path.as_str())
                        .is_some_and(|head| head.ends_with('/'));
                hit.then_some(defs)
            });
            if current.is_none() {
                unknown_files += 1;
            }
            continue;
        }
        let Some(da) = line.strip_prefix("DA:") else {
            continue;
        };
        let Some((line_no, count)) = da.split_once(',') else {
            continue;
        };
        let (Ok(line_no), Ok(count)) = (line_no.parse::<u32>(), count.parse::<u64>()) else {
            continue;
        };
        if count == 0 {
            continue;
        }
        let Some(defs) = current else {
            continue;
        };
        let innermost = defs
            .iter()
            .filter(|(start, end, _)| *start <= line_no && line_no <= *end)
            .min_by_key(|(start, end, _)| end.saturating_sub(*start));
        match innermost {
            Some((_, _, datum)) => *touched.entry(datum.clone()).or_insert(0) += 1,
            None => unmapped_lines += 1,
        }
    }

    let observations = touched
        .into_iter()
        .map(|(datum, count)| WakeObservation {
            caller: WakeEndpoint::Symbol {
                symbol: test.to_owned(),
            },
            callee: WakeEndpoint::Datum {
                datum: grid_wire::Datum(datum),
            },
            count: Some(count),
        })
        .collect();
    Ok(LcovWake {
        wake: Wake {
            wake_id: format!("wake:lcov:{test}"),
            git_rev: grid_wire::GitRev(chart_provider::resolve_rev(root, "HEAD")),
            command: Some(test.to_owned()),
            started_at: None,
            observations,
        },
        unmapped_lines,
        unknown_files,
    })
}
