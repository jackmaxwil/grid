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
    ///
    /// `test` is refused unless it names exactly one live definition: a
    /// trace whose caller resolves to nothing folds into no edge at all, and
    /// retaining it printed `done 0/2` at exit 0 — a CI loop recording
    /// nothing and reporting success. The accepted spellings are the
    /// runner's own and the callsign, which [`crate::testplan::callsign_of`]
    /// maps between; the refusal names them.
    fn open(root: &std::path::Path, test: &str) -> Result<Self, String> {
        let chart = ChartDelta::open(root).map_err(|e| e.to_string())?;
        let hits = chart.resolve(&grid_wire::Locator::Callsign {
            callsign: grid_wire::Callsign(test.to_owned()),
        });
        match hits.len() {
            1 => {}
            0 => {
                return Err(format!(
                    "`{test}` names no definition in the live snapshot — name the test as the \
                     runner does (`<binary-id> <name>` from `cargo nextest list`, \
                     `path/test_x.py::test_y` from pytest) or by its callsign"
                ))
            }
            n => {
                return Err(format!(
                    "`{test}` is ambiguous: {n} definitions carry that name — name the test as \
                     the runner does, which is unique"
                ))
            }
        }
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

    /// The mapped trace, or the refusal: a report that reached nothing
    /// charted is not a trace. Retained, it would say "this test reached
    /// nothing" and license a replay on the static closure alone — the same
    /// claim an empty report makes by a different route, refused the same way.
    fn finish(self, trace_id: String, test: &str) -> Result<Mapped, String> {
        if self.touched.is_empty() {
            return Err(format!(
                "nothing charted was reached: {} files not in the chart, {} executed lines \
                 outside any definition — not a trace of `{test}`, because a trace that reaches \
                 nothing would license a replay on the static closure alone",
                self.unknown_files, self.unmapped_lines
            ));
        }
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
        Ok(Mapped {
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
        })
    }
}

/// Map an lcov report onto the live chart as observations from `test` —
/// the runner's id or a callsign, either way mapped through
/// [`crate::testplan::callsign_of`].
///
/// # Errors
/// A message when the store cannot be opened, when `test` is not exactly one
/// live definition, when the text holds no `SF:` record at all, when no
/// `DA:` line has a count above zero, or when no executed line landed in a
/// charted definition — an empty, absent, all-zero or all-elsewhere report
/// is bad input, never a trace claiming the test reached nothing (the same
/// posture the JSON path takes). A test that ran executed at least its own
/// body, so an all-zero report is a profile that was written somewhere else
/// — a second `LLVM_PROFILE_FILE` in the environment — and retaining it
/// would license a replay on the static closure alone. An lcov record grid
/// does not read is skipped, which is the format's own rule and not a parse
/// failure.
pub fn trace_of_lcov(root: &std::path::Path, lcov: &str, test: &str) -> Result<Mapped, String> {
    if !lcov.lines().any(|line| line.starts_with("SF:")) {
        return Err("not an lcov report: no SF: record (an empty report is not a test that reached nothing)".to_owned());
    }
    if !lcov.lines().any(|line| executed(line).is_some()) {
        return Err("every DA: count is zero: a lost profile (the test ran its own body at least), not a test that reached nothing".to_owned());
    }
    let test = &crate::testplan::callsign_of(test);
    let mut mapper = Mapper::open(root, test)?;
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
    mapper.finish(format!("trace:lcov:{test}"), test)
}

/// Refuse a trace naming a snapshot this store never held.
///
/// A trace records the snapshot its observations were mapped against, and
/// that is what a later reader grades them under. One the store never held
/// names a chart nobody can name back: refused, never retained as a live
/// trace that grades nothing. A trace naming no snapshot predates the column
/// and is taken as it is.
///
/// # Errors
/// A message naming the trace and the snapshot, or the store's own when it
/// cannot be read.
pub fn refuse_unheld_snapshot(root: &std::path::Path, trace: &Trace) -> Result<(), String> {
    let Some(snapshot) = &trace.snapshot else {
        return Ok(());
    };
    let store =
        grid_chart_io::Store::open(chart_provider::store_dir(root)).map_err(|e| e.to_string())?;
    let held = store.load().snapshot_id().as_str() == snapshot.0
        || store
            .retained_snapshots()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|id| id.as_str() == snapshot.0);
    if held {
        return Ok(());
    }
    Err(format!(
        "trace `{}` was mapped against {}, a snapshot this store never held — survey the tree \
         it was taken on before ingesting it",
        trace.trace_id, snapshot.0
    ))
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
/// A message when the store cannot be opened, when `test` is not exactly one
/// live definition, when the text is not a coverage.py JSON report, when no
/// file has an executed line, or when no executed line landed in a charted
/// definition. A report grid cannot parse is bad input, and one with nothing
/// executed is a lost context — the test ran its own body; neither is read
/// as a test that reached nothing.
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
    let test = &crate::testplan::callsign_of(test);
    let mut mapper = Mapper::open(root, test)?;
    for (path, file) in &report.files {
        let Some(index) = mapper.file(path) else {
            continue;
        };
        for &line_no in &file.executed_lines {
            mapper.line(index, line_no);
        }
    }
    mapper.finish(format!("trace:cov:{test}"), test)
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
        assert!(trace_of_lcov(dir.path(), &lcov, "tests.test_core.test_a").is_ok());
    }

    /// Conservatism: a report every file of which is outside the chart, or
    /// every executed line of which sits in no definition, reached nothing
    /// charted — and that is not a trace. Retained with zero observations it
    /// would license a replay on the static closure alone, the same claim an
    /// empty report makes by another route; both are refused, with the counts.
    #[test]
    fn a_report_that_reaches_nothing_charted_is_refused_not_retained_empty() {
        use crate::testplan::fixtures::{owned, python, repo};
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        let out = trace_of_lcov(
            dir.path(),
            "SF:/nowhere/ghost.py\nDA:1,1\nend_of_record\n",
            "tests.test_core.test_a",
        );
        let err = out
            .map(|m| m.trace.observations.len())
            .expect_err("a report whose every file is outside the chart is not a trace");
        assert!(
            err.contains("1 files not in the chart"),
            "the refusal carries the count: {err}"
        );
        // A line inside a charted file but outside every definition: the same.
        let core = dir.path().join("pkg/core.py").display().to_string();
        let out = trace_of_lcov(
            dir.path(),
            &format!("SF:{core}\nDA:4,1\nend_of_record\n"),
            "tests.test_core.test_a",
        );
        let err = out
            .map(|m| m.trace.observations.len())
            .expect_err("a report whose every line is outside every definition is not a trace");
        assert!(
            err.contains("1 executed lines outside any definition"),
            "{err}"
        );
    }

    /// Conservatism: a trace naming a snapshot this store never held is
    /// refused at ingest. It records the chart its observations were mapped
    /// against, and a chart nobody can name back grades nothing — retaining
    /// it at exit 0 with no observations is the empty-report claim again.
    #[test]
    fn a_trace_naming_a_snapshot_never_held_is_refused_at_ingest() {
        use crate::testplan::fixtures::{owned, python, repo};
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        let phantom = "snap:b3:0000000000000000000000000000000000000000000000000000000000000000";
        let trace = grid_wire::Trace {
            trace_id: "trace:phantom".into(),
            git_rev: grid_wire::GitRev("HEAD".into()),
            command: Some("tests.test_core.test_a".into()),
            started_at: None,
            snapshot: Some(grid_wire::SnapshotId(phantom.into())),
            observations: Vec::new(),
        };
        let out =
            crate::rpc::trace_ingest(&grid_wire::methods::TraceIngestParams { trace }, dir.path());
        let err = out
            .map(|r| r.resolved)
            .expect_err("a snapshot the store never held is a refusal, not a retained trace");
        assert!(
            err.contains(phantom),
            "the refusal names the snapshot: {err}"
        );
        assert!(
            crate::rpc::trace_dir(dir.path())
                .and_then(grid_trace::Store::open)
                .and_then(|s| s.get("trace:phantom"))
                .expect("store")
                .is_none(),
            "nothing was retained"
        );
    }

    /// The test is named as the runner names it or by callsign, and either
    /// way it is one live definition: the nextest id, the `cargo test` path
    /// and the pytest node id all map through [`crate::testplan::callsign_of`]
    /// to the same trace, and a name that resolves to nothing is refused with
    /// the accepted forms — not retained as `done 0/2` at exit 0.
    #[test]
    fn a_runner_spelled_test_maps_to_its_callsign_and_an_unknown_name_is_refused() {
        use crate::testplan::fixtures::{owned, python, repo};
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        let core = dir.path().join("pkg/core.py").display().to_string();
        let lcov = format!("SF:{core}\nDA:1,1\nDA:2,1\nend_of_record\n");
        for spelling in ["tests/test_core.py::test_a", "tests.test_core.test_a"] {
            let mapped = trace_of_lcov(dir.path(), &lcov, spelling)
                .unwrap_or_else(|e| panic!("{spelling} is an accepted spelling: {e}"));
            assert_eq!(
                mapped.trace.command.as_deref(),
                Some("tests.test_core.test_a"),
                "{spelling} maps to the callsign"
            );
            assert_eq!(mapped.trace.trace_id, "trace:lcov:tests.test_core.test_a");
            assert_eq!(mapped.trace.observations.len(), 1, "{spelling}: pkg.core.a");
        }
        let err = trace_of_lcov(dir.path(), &lcov, "nope::test_a")
            .map(|m| m.trace.observations.len())
            .expect_err("a test the chart cannot name is refused, not retained as 0/N");
        assert!(
            err.contains("`nope.test_a` names no definition") && err.contains("nextest list"),
            "the refusal names the accepted forms: {err}"
        );
    }

    /// The mapping reaches every definition kind the frontends chart: a
    /// trait impl method, an inherent method, and a Python class method map
    /// by their own line spans exactly as a free function does. Guarded
    /// because a sweep read a misfiled report as this mapping missing trait
    /// impls; it did not, and this is what says so.
    #[test]
    fn impl_methods_and_class_methods_map_like_free_functions() {
        use crate::testplan::fixtures::{datum_of, owned, repo};
        let files: Vec<(&'static str, String)> = vec![
            (
                "Cargo.toml",
                "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n".to_owned(),
            ),
            (
                "src/lib.rs",
                "pub trait Shape {\n    fn area(&self) -> f64;\n}\n\
                 pub struct Rect {\n    pub w: f64,\n}\n\
                 impl Rect {\n    pub fn new(w: f64) -> Self {\n        Self { w }\n    }\n}\n\
                 impl Shape for Rect {\n    fn area(&self) -> f64 {\n        self.w * self.w\n    }\n}\n\
                 #[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn rect() {\n        assert_eq!(Rect::new(2.0).area(), 4.0);\n    }\n}\n"
                    .to_owned(),
            ),
            (
                "pkg/__init__.py",
                String::new(),
            ),
            (
                "pkg/shapes.py",
                "class Rect:\n    def __init__(self, w):\n        self.w = w\n\n    def area(self):\n        return self.w * self.w\n\n\ndef test_rect():\n    assert Rect(2).area() == 4\n"
                    .to_owned(),
            ),
        ];
        let dir = repo(&owned(&files));
        let lib = dir.path().join("src/lib.rs").display().to_string();
        // `new` is lines 8-10, `area` 13-15, the test 20-23.
        let lcov = format!("SF:{lib}\nDA:9,1\nDA:14,1\nDA:22,1\nend_of_record\n");
        let mapped = trace_of_lcov(dir.path(), &lcov, "fx tests::rect").expect("maps");
        let reached: std::collections::BTreeSet<String> = mapped
            .trace
            .observations
            .iter()
            .filter_map(|o| match &o.callee {
                grid_wire::TraceEndpoint::Datum { datum } => Some(datum.0.clone()),
                grid_wire::TraceEndpoint::Symbol { .. } => None,
            })
            .collect();
        for callsign in ["fx.Rect.new", "fx.Rect@Shape.area", "fx.tests.rect"] {
            assert!(
                reached.contains(&datum_of(dir.path(), callsign)),
                "{callsign} is reached: {reached:?}"
            );
        }
        assert_eq!(mapped.unmapped_lines, 0);
        let shapes = dir.path().join("pkg/shapes.py").display().to_string();
        let json = format!(r#"{{"files":{{"{shapes}":{{"executed_lines":[6,10]}}}}}}"#);
        let mapped = super::trace_of_coverage_json(dir.path(), &json, "pkg/shapes.py::test_rect")
            .expect("maps");
        let reached: std::collections::BTreeSet<String> = mapped
            .trace
            .observations
            .iter()
            .filter_map(|o| match &o.callee {
                grid_wire::TraceEndpoint::Datum { datum } => Some(datum.0.clone()),
                grid_wire::TraceEndpoint::Symbol { .. } => None,
            })
            .collect();
        for callsign in ["pkg.shapes.Rect.area", "pkg.shapes.test_rect"] {
            assert!(
                reached.contains(&datum_of(dir.path(), callsign)),
                "{callsign} is reached: {reached:?}"
            );
        }
    }
}
