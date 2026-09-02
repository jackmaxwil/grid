//! Coverage report -> trace: line coverage translated into test-reach
//! observations.
//!
//! Two report formats, one mapping. `cargo llvm-cov --lcov` (and lcov
//! proper) emit `SF:<path>` file records holding `DA:<line>,<count>`
//! execution counts; coverage.py's `coverage json` emits
//! `files.<path>.executed_lines`. Both are a set of executed lines per path,
//! and both go through [`Mapper`] — every executed line that lands inside a
//! charted definition becomes one observation `test -calls-> definition`,
//! deduplicated with executed-line counts summed; lines outside every
//! definition (module level, macro output) and files the chart does not hold
//! are counted and reported, never silently dropped — the same C16 honesty
//! rule ingestion itself holds. Of the lcov vocabulary only two record kinds
//! are read; the rest is skipped by design.
//!
//! A coverage trace is a *reach set*: hop order is file order and means
//! nothing, so `grid trace diff`'s first-divergence walk is not meaningful
//! over these — the edge sets are.
//!
//! One report is one test, so a per-test trace is a per-test report. The two
//! recipes that produce them are in the README's coverage section.
//! `ponytail:` the Rust one is a loop of one `cargo llvm-cov` export per
//! test, because a single profile cannot be attributed back to tests that
//! shared a process; a nextest wrapper script keying `LLVM_PROFILE_FILE` by
//! test name would make it one invocation, and that is nextest configuration
//! rather than grid code. It lands when the loop is measured too slow for CI.
use std::collections::BTreeMap;

use grid_chart::Ordinal;
use grid_wire::{SnapshotId, Trace, TraceEndpoint, TraceObservation};

use crate::chart_provider::{self, ChartDelta};

/// Everything one coverage report mapped to, before ingestion.
pub struct Mapped {
    pub trace: Trace,
    /// Executed lines that landed in no charted definition.
    pub unmapped_lines: u64,
    /// Reported files the chart holds no definitions for.
    pub unknown_files: u64,
}

/// coverage.py's `coverage json` report, at the two fields grid reads.
///
/// Deliberately not `deny_unknown_fields`: a real report also carries
/// `meta`, `totals`, per-file summaries, branch arcs and contexts, all of
/// which grid ignores and none of which make the report malformed.
#[derive(serde::Deserialize)]
struct CoverageReport {
    files: BTreeMap<String, CoverageFile>,
}

#[derive(serde::Deserialize)]
struct CoverageFile {
    executed_lines: Vec<u32>,
}

/// The line-to-definition mapping both report formats share: one charted
/// definition per (path, line span), innermost containment wins, and every
/// line or file it cannot place is counted rather than dropped.
struct Mapper {
    /// Every charted file, sorted by repo-relative path.
    files: Vec<File>,
    touched: BTreeMap<String, u64>,
    unmapped_lines: u64,
    unknown_files: u64,
}

/// One charted file's definitions, in the shape a line lookup wants.
struct File {
    path: String,
    defs: Vec<Span>,
}

/// One definition's line span and identity.
struct Span {
    line: u32,
    line_end: u32,
    datum: String,
}

impl Mapper {
    fn new(chart: &ChartDelta) -> Self {
        let mut by_path: BTreeMap<String, Vec<Span>> = BTreeMap::new();
        for segment in chart.head().segments() {
            for i in 0..segment.def_count() {
                let ord = Ordinal(i);
                let (Some(loc), Some(datum)) = (segment.location(ord), segment.datum(ord)) else {
                    continue;
                };
                by_path.entry(loc.path.to_owned()).or_default().push(Span {
                    line: loc.line,
                    line_end: loc.line_end,
                    datum: datum.to_string(),
                });
            }
        }
        Self {
            files: by_path
                .into_iter()
                .map(|(path, defs)| File { path, defs })
                .collect(),
            touched: BTreeMap::new(),
            unmapped_lines: 0,
            unknown_files: 0,
        }
    }

    /// The chart's index for a reported path, counting the file as unknown
    /// when the chart holds no definitions for it.
    ///
    /// Report paths are usually absolute (lcov) or runner-relative
    /// (coverage.py); chart paths are repo-relative. A suffix match on a
    /// path-segment boundary bridges the two.
    fn file(&mut self, reported: &str) -> Option<usize> {
        let hit = self.files.iter().position(|file| {
            reported == file.path
                || reported
                    .strip_suffix(file.path.as_str())
                    .is_some_and(|head| head.ends_with('/'))
        });
        if hit.is_none() {
            self.unknown_files += 1;
        }
        hit
    }

    /// Attribute one executed line of `file` to the innermost definition
    /// containing it — smallest line span, so a nested definition wins over
    /// its parent — or count it as unmapped.
    fn line(&mut self, file: usize, line_no: u32) {
        let innermost = self.files.get(file).and_then(|file| {
            file.defs
                .iter()
                .filter(|span| span.line <= line_no && line_no <= span.line_end)
                .min_by_key(|span| span.line_end.saturating_sub(span.line))
                .map(|span| span.datum.clone())
        });
        match innermost {
            Some(datum) => *self.touched.entry(datum).or_insert(0) += 1,
            None => self.unmapped_lines += 1,
        }
    }

    fn finish(self, trace_id: String, test: &str, at: Coordinates) -> Mapped {
        let observations = self
            .touched
            .into_iter()
            .map(|(datum, count)| TraceObservation {
                caller: TraceEndpoint::Symbol {
                    symbol: test.to_owned(),
                },
                callee: TraceEndpoint::Datum {
                    datum: grid_wire::Datum(datum),
                },
                count: Some(count),
            })
            .collect();
        Mapped {
            trace: Trace {
                trace_id,
                git_rev: grid_wire::GitRev(at.git_rev),
                command: Some(test.to_owned()),
                started_at: None,
                snapshot: at.snapshot,
                observations,
            },
            unmapped_lines: self.unmapped_lines,
            unknown_files: self.unknown_files,
        }
    }
}

/// Where a mapping happened: the two coordinates the trace records.
///
/// The snapshot is the load-bearing one. `git_rev` needs git to mean
/// anything and a rewritten history stops answering; the snapshot id names
/// the chart the observations were mapped against, which is what a later
/// reader has to grade them under.
struct Coordinates {
    git_rev: String,
    snapshot: Option<SnapshotId>,
}

/// The live chart, its mapper, and where the mapping is happening.
fn open(root: &std::path::Path) -> Result<(Mapper, Coordinates), chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    let at = Coordinates {
        git_rev: chart_provider::resolve_rev(root, "HEAD"),
        snapshot: chart.current(),
    };
    Ok((Mapper::new(&chart), at))
}

/// Map an lcov report onto the live chart as observations from `test`.
///
/// # Errors
/// A message when the store cannot be opened. An lcov record grid does not
/// read is skipped, which is the format's own rule and not a parse failure.
pub fn trace_of_lcov(root: &std::path::Path, lcov: &str, test: &str) -> Result<Mapped, String> {
    let (mut mapper, at) = open(root).map_err(|e| e.to_string())?;
    let mut current: Option<usize> = None;
    for line in lcov.lines() {
        if let Some(sf) = line.strip_prefix("SF:") {
            current = mapper.file(sf);
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
        let Some(file) = current else {
            continue;
        };
        mapper.line(file, line_no);
    }
    Ok(mapper.finish(format!("trace:lcov:{test}"), test, at))
}

/// Map a coverage.py JSON report onto the live chart as observations from
/// `test`.
///
/// coverage.py records which lines ran, not how often, so an observation's
/// count is the number of executed lines that landed in the definition. It
/// is the same number the lcov path reports for a report whose counts are
/// all 1, and an observed edge claims reach either way.
///
/// # Errors
/// A message when the store cannot be opened, or when the text is not a
/// coverage.py JSON report. A report grid cannot parse is bad input; it is
/// never read as a test that reached nothing.
pub fn trace_of_coverage_json(
    root: &std::path::Path,
    json: &str,
    test: &str,
) -> Result<Mapped, String> {
    let report: CoverageReport = serde_json::from_str(json).map_err(|e| {
        format!("not a coverage.py JSON report: {e} (expected files.<path>.executed_lines)")
    })?;
    let (mut mapper, at) = open(root).map_err(|e| e.to_string())?;
    for (path, file) in &report.files {
        let Some(index) = mapper.file(path) else {
            continue;
        };
        for &line_no in &file.executed_lines {
            mapper.line(index, line_no);
        }
    }
    Ok(mapper.finish(format!("trace:cov:{test}"), test, at))
}
