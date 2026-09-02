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
use std::collections::{BTreeMap, HashMap};

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
    /// Chart index by exact repo-relative path.
    by_path: HashMap<String, usize>,
    /// Chart indices by file name, the candidates for a suffix match.
    by_name: HashMap<String, Vec<usize>>,
    touched: BTreeMap<String, u64>,
    unmapped_lines: u64,
    unknown_files: u64,
    /// Where the mapping is happening: the two coordinates the trace
    /// records. The snapshot is the load-bearing one — `git_rev` needs git
    /// to mean anything and a rewritten history stops answering; the
    /// snapshot id names the chart the observations were mapped against,
    /// which is what a later reader has to grade them under.
    git_rev: String,
    snapshot: Option<SnapshotId>,
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
    /// The live chart's mapper, and where the mapping is happening.
    fn open(root: &std::path::Path) -> Result<Self, chart_provider::Error> {
        let chart = ChartDelta::open(root)?;
        let mut spans: BTreeMap<String, Vec<Span>> = BTreeMap::new();
        for segment in chart.head().segments() {
            for i in 0..segment.def_count() {
                let ord = Ordinal(i);
                let (Some(loc), Some(datum)) = (segment.location(ord), segment.datum(ord)) else {
                    continue;
                };
                spans.entry(loc.path.to_owned()).or_default().push(Span {
                    line: loc.line,
                    line_end: loc.line_end,
                    datum: datum.to_string(),
                });
            }
        }
        let files: Vec<File> = spans
            .into_iter()
            .map(|(path, defs)| File { path, defs })
            .collect();
        let by_path = files
            .iter()
            .enumerate()
            .map(|(i, file)| (file.path.clone(), i))
            .collect();
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, file) in files.iter().enumerate() {
            let name = file.path.rsplit('/').next().unwrap_or(&file.path);
            by_name.entry(name.to_owned()).or_default().push(i);
        }
        Ok(Self {
            files,
            by_path,
            by_name,
            touched: BTreeMap::new(),
            unmapped_lines: 0,
            unknown_files: 0,
            git_rev: chart_provider::resolve_rev(root, "HEAD"),
            snapshot: chart.current(),
        })
    }

    /// The chart's index for a reported path, counting the file as unknown
    /// when the chart holds no definitions for it.
    ///
    /// Report paths are usually absolute (lcov) or runner-relative
    /// (coverage.py); chart paths are repo-relative. An exact match wins;
    /// otherwise the *longest* charted path that is a suffix of the reported
    /// one on a segment boundary — `b/a/x.py` over `a/x.py` for
    /// `/repo/b/a/x.py`, where the first match in sort order would have
    /// filed the lines under the wrong module. Candidates come from the file
    /// name index, so a report over thousands of files is not a scan of the
    /// chart per file.
    fn file(&mut self, reported: &str) -> Option<usize> {
        let hit = self.by_path.get(reported).copied().or_else(|| {
            let name = reported.rsplit('/').next().unwrap_or(reported);
            self.by_name
                .get(name)?
                .iter()
                .copied()
                .filter(|&i| {
                    self.files.get(i).is_some_and(|file| {
                        reported
                            .strip_suffix(file.path.as_str())
                            .is_some_and(|head| head.ends_with('/'))
                    })
                })
                .max_by_key(|&i| self.files.get(i).map_or(0, |file| file.path.len()))
        });
        if hit.is_none() {
            self.unknown_files = self.unknown_files.saturating_add(1);
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
            Some(datum) => {
                let count = self.touched.entry(datum).or_insert(0);
                *count = count.saturating_add(1);
            }
            None => self.unmapped_lines = self.unmapped_lines.saturating_add(1),
        }
    }

    fn finish(self, trace_id: String, test: &str) -> Mapped {
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
                git_rev: grid_wire::GitRev(self.git_rev),
                command: Some(test.to_owned()),
                started_at: None,
                snapshot: self.snapshot,
                observations,
            },
            unmapped_lines: self.unmapped_lines,
            unknown_files: self.unknown_files,
        }
    }
}

/// Map an lcov report onto the live chart as observations from `test`.
///
/// # Errors
/// A message when the store cannot be opened, when the text holds no `SF:`
/// record at all, or when no `DA:` line has a count above zero — an empty,
/// absent or all-zero report is bad input, never a trace claiming the test
/// reached nothing (the same posture the JSON path takes). A test that ran
/// executed at least its own body, so an all-zero report is a profile that
/// was written somewhere else — a second `LLVM_PROFILE_FILE` in the
/// environment — and retaining it would license a replay on the static
/// closure alone. An lcov record grid does not read is skipped, which is the
/// format's own rule and not a parse failure.
pub fn trace_of_lcov(root: &std::path::Path, lcov: &str, test: &str) -> Result<Mapped, String> {
    if !lcov.lines().any(|line| line.starts_with("SF:")) {
        return Err("not an lcov report: no SF: record (an empty report is not a test that reached nothing)".to_owned());
    }
    if !lcov.lines().any(|line| executed(line).is_some()) {
        return Err("every DA: count is zero: a profile lost to another LLVM_PROFILE_FILE, not a test that reached nothing".to_owned());
    }
    let mut mapper = Mapper::open(root).map_err(|e| e.to_string())?;
    let mut current: Option<usize> = None;
    for line in lcov.lines() {
        if let Some(sf) = line.strip_prefix("SF:") {
            current = mapper.file(sf);
            continue;
        }
        let (Some(line_no), Some(file)) = (executed(line), current) else {
            continue;
        };
        mapper.line(file, line_no);
    }
    Ok(mapper.finish(format!("trace:lcov:{test}"), test))
}

/// The line number of a `DA:<line>,<count>` record whose count is above
/// zero; `None` for any other line, an unparsable one included.
fn executed(line: &str) -> Option<u32> {
    let (line_no, count) = line.strip_prefix("DA:")?.split_once(',')?;
    (count.parse::<u64>().ok()? > 0).then(|| line_no.parse().ok())?
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
/// A message when the store cannot be opened, when the text is not a
/// coverage.py JSON report, or when no file has an executed line. A report
/// grid cannot parse is bad input, and one with nothing executed is a lost
/// context — the test ran its own body; neither is read as a test that
/// reached nothing.
pub fn trace_of_coverage_json(
    root: &std::path::Path,
    json: &str,
    test: &str,
) -> Result<Mapped, String> {
    let report: CoverageReport = serde_json::from_str(json).map_err(|e| {
        format!("not a coverage.py JSON report: {e} (expected files.<path>.executed_lines)")
    })?;
    if report
        .files
        .values()
        .all(|file| file.executed_lines.is_empty())
    {
        return Err(
            "no executed line in any file: a lost context, not a test that reached nothing"
                .to_owned(),
        );
    }
    let mut mapper = Mapper::open(root).map_err(|e| e.to_string())?;
    for (path, file) in &report.files {
        let Some(index) = mapper.file(path) else {
            continue;
        };
        for &line_no in &file.executed_lines {
            mapper.line(index, line_no);
        }
    }
    Ok(mapper.finish(format!("trace:cov:{test}"), test))
}

#[cfg(test)]
mod tests {
    use super::trace_of_lcov;

    /// An empty or SF-less report is refused: `cargo llvm-cov` writes no
    /// report when the test command fails, and an empty text read in its
    /// place must not become a trace that says the test reached nothing.
    #[test]
    fn an_lcov_text_with_no_sf_record_is_refused_not_an_empty_trace() {
        let dir = tempfile::tempdir().expect("tempdir");
        for text in ["", "TN:\nend_of_record\n"] {
            let out = trace_of_lcov(dir.path(), text, "t");
            assert!(
                out.is_err(),
                "{text:?} is not a report: {:?}",
                out.map(|m| m.trace.observations.len())
            );
        }
    }

    /// A report with `SF:` records and every `DA:` count zero is a lost
    /// profile — a test that ran executed at least its own body — never a
    /// test that reached nothing charted. Retained, it would license a
    /// replay on the static closure alone; refused, the verdict is kept
    /// untraced. The same for a coverage.py report whose every
    /// `executed_lines` is empty.
    #[test]
    fn an_all_zero_report_is_a_lost_profile_not_an_empty_trace() {
        use crate::testplan::fixtures::{owned, python, repo};
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        let core = dir.path().join("pkg/core.py").display().to_string();
        let lcov = format!("SF:{core}\nDA:1,0\nDA:2,0\nDA:5,0\nend_of_record\n");
        let out = trace_of_lcov(dir.path(), &lcov, "t");
        assert!(
            out.is_err(),
            "every DA: count is zero: a lost profile, not a trace of {:?} observations",
            out.map(|m| m.trace.observations.len())
        );
        let json = format!(r#"{{"files":{{"{core}":{{"executed_lines":[]}}}}}}"#);
        let out = super::trace_of_coverage_json(dir.path(), &json, "t");
        assert!(
            out.is_err(),
            "no executed line at all: a lost profile, not a trace of {:?} observations",
            out.map(|m| m.trace.observations.len())
        );
        // And one executed line is still a report.
        let lcov = format!("SF:{core}\nDA:1,0\nDA:2,1\nend_of_record\n");
        assert!(trace_of_lcov(dir.path(), &lcov, "t").is_ok());
    }
}
