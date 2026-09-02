//! `grid.test.plan` — test selection as data.
//!
//! The runner owns the inventory and the execution; grid owns exactly the
//! four facts selection needs: datum identity per definition (what changed),
//! edges with a grade (what can reach what), retained traces (what a test
//! actually reached), and closure-keyed verdicts (what was already proven in
//! this environment). This module joins them and nothing else.
//!
//! The rule, over a base and the worktree survey (head):
//!
//! - The changed set `C` is every definition whose *datum* moved between
//!   base and head — datum, not sig, because tests execute bodies.
//! - One inbound walk from `C` over base's chart, every relation and every
//!   grade (the `Kind::Test` reading: whatever can run can matter), plus the
//!   retained traces' observed edges, fuel [`grid_cache::FUEL`]. A test is
//!   selected when the walk reaches its definition, or when the test's own
//!   datum is not one base holds (it was edited or is new).
//! - A walk that runs aground selects **every** test and says so. Never a
//!   shorter list: a truncated walk looks exactly like a smaller change.
//! - A test with no retained trace still describing code base holds is
//!   selected regardless of the walk. The honesty number says the static
//!   closure is incomplete, so the closure alone never licenses a skip; a
//!   live trace is the evidence that the test's reach was actually watched.
//! - What is left is looked up under `Kind::Test` and the run's `env_hash`;
//!   a hit is replayed, a miss joins the run set as `not_recorded`.
//! - Definitions in `C` no selected test can reach are `uncovered` — a
//!   second, outbound walk from the reached tests, budgeted the same way.
//!   Heuristic, because "no charted path" is not "no path".
use std::collections::{HashMap, HashSet};

use grid_chart::{DatumId, Ordinal, Segment};
use grid_wire::{
    CacheHit, CacheHitVerdict, Callsign, Datum, DefRef, DriftUnchecked, SnapshotId, TestId,
    TestPlan, TestSelection, TestSelectionUnresolved, TestSelectionWhy,
};

/// Everything one plan is computed from. Bundled because every field is part
/// of what was asked, and so the pure function is testable without a store.
pub struct Inputs<'a, B: AsRef<[u8]>> {
    /// Base's live set — the chart the walk runs over.
    pub base: &'a [Segment<B>],
    /// Head's live set — where tests resolve and closure keys are frozen.
    pub head: &'a [Segment<B>],
    pub base_id: SnapshotId,
    pub head_id: SnapshotId,
    /// `C`: every definition whose datum moved, base side first.
    pub changed: &'a [Changed],
    pub traces: &'a [grid_trace::Stored],
    pub cache: &'a grid_cache::Cache,
    pub env_hash: &'a str,
    /// The runner's inventory, in the runner's order.
    pub tests: &'a [TestId],
    /// Whether unselected tests may be answered from the cache at all.
    pub replay: bool,
    /// Walk budget. A parameter so the refusal can be made to happen.
    pub fuel: u32,
    /// The head survey's declined split, or `None` when nobody surveyed.
    pub declined: Option<crate::drift::Declined>,
}

/// One definition whose datum moved between base and head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Changed {
    /// The datum base held — what the walk starts from.
    pub before: DatumId,
    /// The datum head holds under the same name, or `None` when the
    /// definition was removed. Needed for one thing: a test that was itself
    /// edited covers its own old self, and the pairing is how that is known.
    pub after: Option<DatumId>,
}

/// `C` from a diff: datum-level, base side. A move keeps its datum and an
/// addition has no base side; both are outside the set by construction.
#[must_use]
pub fn changed_of(diff: &grid_wire::Diff) -> Vec<Changed> {
    diff.defs
        .iter()
        .filter(|d| {
            matches!(
                d.action,
                grid_wire::DefChangeAction::Removed | grid_wire::DefChangeAction::Modified
            )
        })
        .filter_map(|d| {
            Some(Changed {
                before: DatumId::parse(&d.before.as_ref()?.0)?,
                after: d.after.as_ref().and_then(|after| DatumId::parse(&after.0)),
            })
        })
        .collect()
}

/// The callsign a runner's test id maps to, by the frontend's spelling.
///
/// nextest ids are `<binary-id> <name>`: `pkg` (lib), `pkg::bin/x` (bin),
/// `pkg::x` (integration test), `pkg::example/x`, `pkg::bench/x`; the name is
/// a `::` path. `lang-rs` roots a crate at its package name with `-` as `_`,
/// files integration tests under `tests.<binary>`, and spells `::` as `.`.
/// pytest ids are `path/to/test_x.py::TestY::test_z[param]`; `lang-py` roots
/// the file by path and drops the parametrization, which is one definition
/// however many times it runs. An id with neither shape maps to itself with
/// `::` as `.`, which is what a bare `cargo test -- --list` name is.
#[must_use]
pub fn callsign_of(test: &str) -> String {
    if let Some((binary, name)) = test.split_once(' ') {
        let name = name.replace("::", ".");
        let (package, target) = binary.split_once("::").unwrap_or((binary, ""));
        let root = package.replace('-', "_");
        return match target.split_once('/') {
            // `main.rs` folds into the crate root, as `lib.rs` does.
            Some(("bin", _)) => format!("{root}.{name}"),
            Some((kind, file)) => format!("{root}.{kind}.{file}.{name}"),
            None if target.is_empty() => format!("{root}.{name}"),
            None => format!("{root}.tests.{target}.{name}"),
        };
    }
    let (file, rest) = test.split_once("::").unwrap_or((test, ""));
    let module = file
        .strip_suffix(".py")
        .map_or_else(|| file.replace("::", "."), |m| m.replace('/', "."));
    let rest = rest.split('[').next().unwrap_or(rest).replace("::", ".");
    if rest.is_empty() {
        module
    } else {
        format!("{module}.{rest}")
    }
}

/// The `grid.resolve` rule as an index: every dotted suffix of every callsign,
/// so resolving an inventory costs one hash lookup per test rather than one
/// scan of the chart per test.
struct Resolver {
    by_suffix: HashMap<String, Vec<DatumId>>,
    /// Every dotted suffix of every charted callsign's parent — the modules
    /// the survey did chart something in.
    modules: HashSet<String>,
}

impl Resolver {
    fn new<B: AsRef<[u8]>>(segments: &[Segment<B>]) -> Self {
        let mut by_suffix: HashMap<String, Vec<DatumId>> = HashMap::new();
        let mut modules: HashSet<String> = HashSet::new();
        for segment in segments {
            for ord in 0..segment.def_count() {
                let ord = Ordinal(ord);
                let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord))
                else {
                    continue;
                };
                let mut rest = callsign;
                loop {
                    let hits = by_suffix.entry(rest.to_owned()).or_default();
                    if !hits.contains(&datum) {
                        hits.push(datum);
                    }
                    match rest.split_once('.') {
                        Some((_, tail)) => rest = tail,
                        None => break,
                    }
                }
                // The same suffix chain over the callsign's *parent*, so the
                // reason an unresolved test gives can distinguish a name the
                // survey never charted from a name inside a module it did.
                if let Some((parent, _)) = callsign.rsplit_once('.') {
                    let mut rest = parent;
                    loop {
                        modules.insert(rest.to_owned());
                        match rest.split_once('.') {
                            Some((_, tail)) => rest = tail,
                            None => break,
                        }
                    }
                }
            }
        }
        Self { by_suffix, modules }
    }

    /// Exactly one definition, or the reason there is not one: two is an
    /// ambiguity, and an ambiguous test is one grid cannot say anything
    /// about. Absent splits again on whether the *module* is charted: a name
    /// missing from a module the survey did chart is a name the survey could
    /// not see, which on Rust means a macro invocation defines it — the
    /// frontend charts items, and macro output is not an item.
    fn resolve(&self, callsign: &str) -> Result<DatumId, TestSelectionUnresolved> {
        match self.by_suffix.get(callsign).map(Vec::as_slice) {
            Some([one]) => Ok(*one),
            Some([_, _, ..]) => Err(TestSelectionUnresolved::Ambiguous),
            _ if callsign
                .rsplit_once('.')
                .is_some_and(|(module, _)| self.modules.contains(module)) =>
            {
                Err(TestSelectionUnresolved::MacroDefined)
            }
            _ => Err(TestSelectionUnresolved::NotCharted),
        }
    }
}

/// The base chart plus the traces' edges, in both directions, as the two
/// walks need them. Built once per plan.
struct Graph<'a, B: AsRef<[u8]>> {
    segments: &'a [Segment<B>],
    observed_in: HashMap<DatumId, Vec<DatumId>>,
    observed_out: HashMap<DatumId, Vec<DatumId>>,
}

impl<B: AsRef<[u8]>> Graph<'_, B> {
    /// Breadth-first over the chart's edges in one direction plus the traces'
    /// edges in the same direction, from `roots`, charging one fuel per edge.
    /// `None` is the refusal: the walk ran out before it finished.
    fn walk(&self, roots: &[DatumId], inbound: bool, mut fuel: u32) -> Option<HashSet<DatumId>> {
        let mut seen: HashSet<DatumId> = roots.iter().copied().collect();
        let mut frontier: Vec<DatumId> = roots.to_vec();
        let observed = if inbound {
            &self.observed_in
        } else {
            &self.observed_out
        };
        while let Some(current) = frontier.pop() {
            let mut next: Vec<DatumId> = Vec::new();
            for segment in self.segments {
                let Some(ord) = segment.ordinal(&current) else {
                    continue;
                };
                let edges: Box<dyn Iterator<Item = grid_chart::Neighbour>> = if inbound {
                    Box::new(segment.in_edges(ord))
                } else {
                    Box::new(segment.out_edges(ord))
                };
                for edge in edges {
                    fuel = fuel.checked_sub(1)?;
                    // Every relation, every grade: a test executes whatever
                    // it can reach, and the closure key follows the same set.
                    if let Some(other) = segment.datum(edge.other) {
                        next.push(other);
                    }
                }
            }
            for other in observed.get(&current).into_iter().flatten() {
                fuel = fuel.checked_sub(1)?;
                next.push(*other);
            }
            for other in next {
                if seen.insert(other) {
                    frontier.push(other);
                }
            }
        }
        Some(seen)
    }
}

/// The traces, read once: which test datums were watched by a trace that is
/// still live at base, and the observed edges in both directions.
///
/// A trace is live when every edge in it still describes code base holds;
/// one decayed edge is one definition the test's watched reach no longer
/// matches, and a reach set that is partly fiction licenses nothing. A trace
/// with no edges at all says the test reached nothing charted; it names its
/// test by command, and it is live while that test is one base holds.
fn read_traces<'a, B: AsRef<[u8]>>(
    inputs: &Inputs<'a, B>,
    resolver: &Resolver,
) -> (HashSet<DatumId>, Graph<'a, B>) {
    let mut traced: HashSet<DatumId> = HashSet::new();
    let mut graph = Graph {
        segments: inputs.base,
        observed_in: HashMap::new(),
        observed_out: HashMap::new(),
    };
    let held_at_base = |datum: DatumId| inputs.base.iter().any(|s| s.ordinal(&datum).is_some());
    for stored in inputs.traces {
        let edges = stored.edges();
        if edges.is_empty() {
            if let Some(test) = stored
                .trace
                .command
                .as_deref()
                .and_then(|name| resolver.resolve(name).ok())
                .filter(|test| held_at_base(*test))
            {
                traced.insert(test);
            }
            continue;
        }
        let live = edges
            .iter()
            .all(|edge| grid_trace::grade(inputs.base, *edge).grade == grid_wire::Grade::Observed);
        for edge in edges {
            if live {
                traced.insert(edge.caller);
            }
            graph
                .observed_in
                .entry(edge.callee)
                .or_default()
                .push(edge.caller);
            graph
                .observed_out
                .entry(edge.caller)
                .or_default()
                .push(edge.callee);
        }
    }
    (traced, graph)
}

/// One test's fate, given the walk's answer.
struct Sorted {
    selected: Vec<TestSelection>,
    replayed: Vec<CacheHit>,
    /// The reached tests' datums, for the uncovered walk.
    reached_tests: Vec<DatumId>,
}

fn sort_tests<B: AsRef<[u8]>>(
    inputs: &Inputs<'_, B>,
    resolver: &Resolver,
    reached: Option<&HashSet<DatumId>>,
    traced: &HashSet<DatumId>,
) -> Sorted {
    let held_at_base = |datum: DatumId| inputs.base.iter().any(|s| s.ordinal(&datum).is_some());
    let chart = grid_cache::Chart::new(inputs.head);
    let mut out = Sorted {
        selected: Vec::new(),
        replayed: Vec::new(),
        reached_tests: Vec::new(),
    };
    for test in inputs.tests {
        let datum = match resolver.resolve(&callsign_of(&test.0)) {
            Ok(datum) => datum,
            Err(unresolved) => {
                out.selected.push(TestSelection {
                    test: test.clone(),
                    datum: None,
                    why: TestSelectionWhy::Unresolved,
                    unresolved: Some(unresolved),
                });
                continue;
            }
        };
        let wire = Datum(datum.to_string());
        let why = match reached {
            None => TestSelectionWhy::Aground,
            // A datum base does not hold is a test that was edited or added:
            // its old self is in `C` (or never existed), so it runs.
            Some(_) if !held_at_base(datum) => TestSelectionWhy::Reached,
            Some(reached) if reached.contains(&datum) => TestSelectionWhy::Reached,
            Some(_) if !traced.contains(&datum) => TestSelectionWhy::NoTrace,
            Some(_) => {
                let hit = inputs
                    .replay
                    .then(|| {
                        chart
                            .key(datum, grid_cache::Kind::Test, inputs.env_hash)
                            .ok()
                    })
                    .flatten()
                    .and_then(|key| inputs.cache.get(key));
                match hit {
                    Some(entry) => {
                        out.replayed.push(CacheHit {
                            datum: wire,
                            verdict: match entry.verdict {
                                grid_cache::Verdict::Pass => CacheHitVerdict::Pass,
                                grid_cache::Verdict::Fail => CacheHitVerdict::Fail,
                            },
                            recorded_at: entry.recorded_at.clone(),
                            trace_id: entry.trace_id.clone(),
                            test: Some(test.clone()),
                        });
                        continue;
                    }
                    None => TestSelectionWhy::NotRecorded,
                }
            }
        };
        if why == TestSelectionWhy::Reached && held_at_base(datum) {
            out.reached_tests.push(datum);
        }
        out.selected.push(TestSelection {
            test: test.clone(),
            datum: Some(wire),
            why,
            // The column answers exactly one question, and this test is not
            // the one it asks.
            unresolved: None,
        });
    }
    out
}

/// Compute the plan. Pure: nothing is written, and the same inputs give the
/// same plan (the determinism property).
#[must_use]
pub fn compute<B: AsRef<[u8]>>(inputs: &Inputs<'_, B>) -> TestPlan {
    let resolver = Resolver::new(inputs.head);
    let (traced, graph) = read_traces(inputs, &resolver);
    let roots: Vec<DatumId> = inputs.changed.iter().map(|c| c.before).collect();
    // The one inbound walk. `None` is aground, and aground selects everything.
    let reached = graph.walk(&roots, true, inputs.fuel);
    let mut budget_rejected = u64::from(reached.is_none());
    let Sorted {
        selected,
        replayed,
        mut reached_tests,
    } = sort_tests(inputs, &resolver, reached.as_ref(), &traced);

    // The uncovered set is `C` minus what reached tests reach. Only reached
    // tests need walking — an unreached test reaches nothing in `C`, or the
    // inbound walk would have found it. A test that was itself edited walks
    // from its old self, which is the one base's edges leave from, and
    // covers that old self by running. Aground upstream means nothing can
    // be said here either.
    let mut uncovered = Vec::new();
    if reached.is_some() && !inputs.changed.is_empty() {
        let selected_datums: HashSet<&str> = selected
            .iter()
            .filter_map(|s| s.datum.as_ref().map(|d| d.0.as_str()))
            .collect();
        let edited_tests: Vec<DatumId> = inputs
            .changed
            .iter()
            .filter(|c| {
                c.after
                    .is_some_and(|d| selected_datums.contains(d.to_string().as_str()))
            })
            .map(|c| c.before)
            .collect();
        reached_tests.extend(edited_tests.iter().copied());
        match graph.walk(&reached_tests, false, inputs.fuel) {
            Some(covered) => {
                for change in inputs.changed {
                    if !covered.contains(&change.before) {
                        uncovered.push(def_ref(inputs.base, change.before));
                    }
                }
                let key =
                    |d: &DefRef| (d.callsign.as_ref().map(|c| c.0.clone()), d.datum.0.clone());
                uncovered.sort_by_key(key);
            }
            None => budget_rejected = budget_rejected.saturating_add(1),
        }
    }

    TestPlan {
        base: inputs.base_id.clone(),
        head: inputs.head_id.clone(),
        selected,
        replayed,
        uncovered,
        unchecked: DriftUnchecked {
            unresolved_refs: inputs.declined.map(crate::drift::Declined::total),
            unresolved_external: inputs.declined.map(|d| d.external),
            unresolved_method: inputs.declined.map(|d| d.method),
            unresolved_internal: inputs.declined.map(|d| d.internal),
            budget_rejected,
            unattributed_entries: None,
            weak_skipped: None,
            compat_unchecked: None,
        },
    }
}

/// A changed definition, named where base can name it.
fn def_ref<B: AsRef<[u8]>>(segments: &[Segment<B>], datum: DatumId) -> DefRef {
    let named = segments.iter().find_map(|s| {
        let ord = s.ordinal(&datum)?;
        Some((s.callsign(ord)?.to_owned(), s.location(ord)))
    });
    DefRef {
        datum: Datum(datum.to_string()),
        callsign: named.as_ref().map(|(c, _)| Callsign(c.clone())),
        lang: None,
        signature: None,
        cfg: None,
        materialization: named.and_then(|(_, loc)| loc).map(|loc| {
            grid_wire::DefRefMaterialization {
                file: loc.path.to_owned(),
                line: u64::from(loc.line),
            }
        }),
    }
}

/// Where the last `grid test` recorded its head, for the default base.
pub(crate) fn last_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".grid/test/last")
}

/// Real repositories surveyed by the real frontends, so every edge a plan
/// walks is one the language's own binding rules establish. Shared with the
/// `grid test` driver's tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::Path;

    use grid_wire::{GitRev, SnapshotOrRev};

    pub(crate) fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "grid")
            .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
            .env("GIT_COMMITTER_NAME", "grid")
            .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
            .output()
            .expect("run git")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    pub(crate) fn write(dir: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let full = dir.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(full, text).expect("write");
        }
    }

    pub(crate) fn commit(dir: &Path, message: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "--allow-empty", "-m", message]);
    }

    /// A committed repo, surveyed once so the live epoch is `HEAD`.
    pub(crate) fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        write(dir.path(), files);
        commit(dir.path(), "base");
        survey(dir.path());
        dir
    }

    pub(crate) fn survey(dir: &Path) {
        crate::rpc::Chart::new(dir)
            .survey_run(&grid_wire::methods::SurveyRunParams {
                git_rev: None,
                worktree: None,
            })
            .expect("survey");
    }

    /// The Python fixture: three definitions, three tests that call one
    /// each (`c` calls `b`), and one definition no test reaches.
    pub(crate) fn python(a: i64, b: i64, d: i64, expect: [i64; 3]) -> Vec<(&'static str, String)> {
        vec![
            ("pkg/__init__.py", String::new()),
            ("pytest.ini", "[pytest]\n".to_owned()),
            (
                "pkg/core.py",
                format!(
                    "def a():\n    return {a}\n\n\ndef b():\n    return {b}\n\n\n\
                     def c():\n    return b() + 1\n\n\ndef d():\n    return {d}\n"
                ),
            ),
            (
                "tests/test_core.py",
                format!(
                    "from pkg.core import a, b, c\n\n\ndef test_a():\n    assert a() == {}\n\n\n\
                     def test_b():\n    assert b() == {}\n\n\ndef test_c():\n    assert c() == {}\n\n\n\
                     def test_d():\n    assert True\n",
                    expect[0], expect[1], expect[2]
                ),
            ),
        ]
    }

    pub(crate) fn owned<'a>(files: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
        files.iter().map(|(p, t)| (*p, t.as_str())).collect()
    }

    /// The Rust fixture: the same shape as the Python one, as a crate.
    pub(crate) fn rust(a: i64, b: i64, d: i64) -> Vec<(&'static str, String)> {
        vec![
            (
                "Cargo.toml",
                "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n".to_owned(),
            ),
            (
                "src/lib.rs",
                format!(
                    "pub fn a() -> i64 {{ {a} }}\npub fn b() -> i64 {{ {b} }}\n\
                     pub fn c() -> i64 {{ b() + 1 }}\npub fn d() -> i64 {{ {d} }}\n\n\
                     #[cfg(test)]\nmod tests {{\n    use super::*;\n\
                     #[test]\n    fn test_a() {{ assert_eq!(a(), 1); }}\n\
                     #[test]\n    fn test_b() {{ assert_eq!(b(), 2); }}\n\
                     #[test]\n    fn test_c() {{ assert_eq!(c(), 3); }}\n\
                     #[test]\n    fn test_d() {{ assert!(true); }}\n}}\n"
                ),
            ),
        ]
    }

    /// Retain a trace for `test` reaching `callees`, by symbol, against the
    /// live epoch — what `grid trace ingest --coverage-json` retains.
    pub(crate) fn ingest(dir: &Path, test: &str, callees: &[&str]) {
        let trace = grid_wire::Trace {
            trace_id: format!("trace:fx:{test}"),
            git_rev: GitRev("fx".into()),
            command: Some(test.to_owned()),
            started_at: None,
            snapshot: None,
            observations: callees
                .iter()
                .map(|callee| grid_wire::TraceObservation {
                    caller: grid_wire::TraceEndpoint::Symbol {
                        symbol: test.to_owned(),
                    },
                    callee: grid_wire::TraceEndpoint::Symbol {
                        symbol: (*callee).to_owned(),
                    },
                    count: Some(1),
                })
                .collect(),
        };
        let result =
            crate::rpc::trace_ingest(&grid_wire::methods::TraceIngestParams { trace }, dir)
                .expect("ingest");
        assert!(
            result.unresolved_symbols.is_empty(),
            "fixture trace resolves: {:?}",
            result.unresolved_symbols
        );
    }

    /// The datum a callsign answers to in the live epoch.
    pub(crate) fn datum_of(dir: &Path, callsign: &str) -> String {
        let hits = crate::chart_provider::ChartDelta::open(dir)
            .expect("open")
            .resolve(&grid_wire::Locator::Callsign {
                callsign: grid_wire::Callsign(callsign.to_owned()),
            });
        assert_eq!(hits.len(), 1, "{callsign} resolves to one definition");
        hits[0].datum.0.clone()
    }

    /// Record a pass for each callsign under `env` against the live epoch.
    pub(crate) fn record_pass(dir: &Path, env: &str, callsigns: &[&str]) {
        let result = crate::rpc::cache_record(
            &grid_wire::methods::CacheRecordParams {
                kind: grid_wire::methods::CacheRecordKind::Test,
                env_hash: env.to_owned(),
                snapshot: None,
                trace_id: None,
                results: callsigns
                    .iter()
                    .map(|c| grid_wire::CacheResultEntry {
                        datum: grid_wire::Datum(datum_of(dir, c)),
                        verdict: grid_wire::CacheResultEntryVerdict::Pass,
                    })
                    .collect(),
            },
            dir,
        )
        .expect("record");
        assert!(result.rejected.is_empty(), "{:?}", result.rejected);
    }

    pub(crate) fn plan(
        dir: &Path,
        base: Option<&str>,
        env: &str,
        tests: &[&str],
    ) -> grid_wire::TestPlan {
        let params = grid_wire::methods::TestPlanParams {
            base: base.map(|rev| SnapshotOrRev::GitRev {
                git_rev: GitRev(rev.to_owned()),
            }),
            env_hash: env.to_owned(),
            tests: tests
                .iter()
                .map(|t| grid_wire::TestId((*t).to_owned()))
                .collect(),
            replay: None,
        };
        crate::rpc::test_plan(&params, dir, &mut crate::rpc::Chart::new(dir)).expect("plan")
    }

    /// `(test, why)` pairs, in plan order.
    pub(crate) fn selected(
        plan: &grid_wire::TestPlan,
    ) -> Vec<(String, grid_wire::TestSelectionWhy)> {
        plan.selected
            .iter()
            .map(|s| (s.test.0.clone(), s.why))
            .collect()
    }

    pub(crate) fn replayed(plan: &grid_wire::TestPlan) -> Vec<String> {
        plan.replayed
            .iter()
            .map(|h| h.test.clone().expect("a replayed hit names its test").0)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::callsign_of;
    use super::fixtures::{
        commit, ingest, owned, plan, python, record_pass, replayed, repo, rust, selected, survey,
        write,
    };
    use grid_wire::TestSelectionWhy as Why;

    const ENV: &str = "env:fx";
    const PY_TESTS: [&str; 4] = [
        "tests/test_core.py::test_a",
        "tests/test_core.py::test_b",
        "tests/test_core.py::test_c",
        "tests/test_core.py::test_d",
    ];
    const RS_TESTS: [&str; 4] = [
        "fx tests::test_a",
        "fx tests::test_b",
        "fx tests::test_c",
        "fx tests::test_d",
    ];

    /// Ground truth, Python: three traced tests, a fourth never traced.
    /// Change one definition's body and exactly its test is selected; the
    /// untraced one is selected every time; the other two replay.
    #[test]
    fn a_body_edit_selects_exactly_the_test_that_reaches_it() {
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        for (test, callee) in [("test_a", "pkg.core.a"), ("test_b", "pkg.core.b")] {
            ingest(dir.path(), &format!("tests.test_core.{test}"), &[callee]);
        }
        ingest(
            dir.path(),
            "tests.test_core.test_c",
            &["pkg.core.c", "pkg.core.b"],
        );
        record_pass(
            dir.path(),
            ENV,
            &[
                "tests.test_core.test_a",
                "tests.test_core.test_b",
                "tests.test_core.test_c",
            ],
        );
        write(dir.path(), &owned(&python(5, 2, 9, [1, 2, 3])));
        let plan = plan(dir.path(), Some("HEAD"), ENV, &PY_TESTS);
        assert_eq!(
            selected(&plan),
            [
                ("tests/test_core.py::test_a".to_owned(), Why::Reached),
                ("tests/test_core.py::test_d".to_owned(), Why::NoTrace),
            ]
        );
        assert_eq!(
            replayed(&plan),
            ["tests/test_core.py::test_b", "tests/test_core.py::test_c"]
        );
        assert!(plan.uncovered.is_empty(), "{:?}", plan.uncovered);
        assert_eq!(plan.unchecked.budget_rejected, 0);
    }

    /// The same ground truth on the Rust fixture, addressed by nextest ids.
    /// `c` calls `b`, so a `b` edit selects both `test_b` and `test_c`
    /// through the proven edge, and `test_a` replays.
    #[test]
    fn a_rust_body_edit_selects_through_the_proven_call_chain() {
        let dir = repo(&owned(&rust(1, 2, 9)));
        for (test, callees) in [
            ("test_a", vec!["fx.a"]),
            ("test_b", vec!["fx.b"]),
            ("test_c", vec!["fx.c", "fx.b"]),
        ] {
            ingest(dir.path(), &format!("fx.tests.{test}"), &callees);
        }
        record_pass(
            dir.path(),
            ENV,
            &["fx.tests.test_a", "fx.tests.test_b", "fx.tests.test_c"],
        );
        write(dir.path(), &owned(&rust(1, 7, 9)));
        let plan = plan(dir.path(), Some("HEAD"), ENV, &RS_TESTS);
        assert_eq!(
            selected(&plan),
            [
                ("fx tests::test_b".to_owned(), Why::Reached),
                ("fx tests::test_c".to_owned(), Why::Reached),
                ("fx tests::test_d".to_owned(), Why::NoTrace),
            ]
        );
        assert_eq!(replayed(&plan), ["fx tests::test_a"]);
    }

    /// Conservatism: a test whose only trace has decayed is selected, not
    /// skipped. The trace watched `b` as it was two snapshots ago; a verdict
    /// recorded since keys to the current closure, so a plan that took any
    /// trace as a licence would replay it — and the reach it would be
    /// trusting is one nobody watched.
    #[test]
    fn a_test_whose_only_trace_decayed_is_selected_not_skipped() {
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        ingest(dir.path(), "tests.test_core.test_b", &["pkg.core.b"]);
        // `b` moves and is committed: the trace now describes code base
        // no longer holds. A verdict is recorded against the new base too.
        write(dir.path(), &owned(&python(1, 7, 9, [1, 2, 3])));
        commit(dir.path(), "b moved");
        survey(dir.path());
        record_pass(dir.path(), ENV, &["tests.test_core.test_b"]);
        // An unrelated edit, so the walk does not reach `test_b` either.
        write(dir.path(), &owned(&python(5, 7, 9, [1, 2, 3])));
        let plan = plan(dir.path(), Some("HEAD"), ENV, &PY_TESTS[1..2]);
        assert!(
            plan.replayed.is_empty(),
            "a decayed trace licenses no replay: {:?}",
            replayed(&plan)
        );
        assert_eq!(
            selected(&plan),
            [("tests/test_core.py::test_b".to_owned(), Why::NoTrace)]
        );
    }

    /// Conservatism: a walk over fuel selects every test and says so. Two
    /// tests, fuel for one edge: a truncating walk would select one and
    /// look like a smaller change.
    #[test]
    fn a_walk_over_fuel_selects_every_test() {
        use grid_chart::{build, DatumId, Def, Edge, Segment};
        let def = |callsign: &str, datum: u8| Def {
            datum: DatumId([datum; 32]),
            callsign: callsign.to_owned(),
            lang: Some(grid_wire::Lang::Py),
            facts: None,
        };
        let calls = |src: u8, dst: u8| Edge {
            src: DatumId([src; 32]),
            dst: DatumId([dst; 32]),
            relation: grid_wire::Relation::Calls,
            grade: grid_wire::Grade::Proven,
        };
        let parse = |image: Vec<u8>| Segment::parse(image).expect("parse");
        let base = [parse(
            build(
                &[def("m.f", 1), def("m.test_x", 2), def("m.test_y", 3)],
                &[calls(2, 1), calls(3, 1)],
            )
            .expect("base"),
        )];
        let head = [parse(
            build(
                &[def("m.f", 4), def("m.test_x", 2), def("m.test_y", 3)],
                &[calls(2, 4), calls(3, 4)],
            )
            .expect("head"),
        )];
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = grid_cache::Cache::open(dir.path().join("verdicts")).expect("cache");
        let tests = [
            grid_wire::TestId("m.test_x".into()),
            grid_wire::TestId("m.test_y".into()),
        ];
        let inputs = super::Inputs {
            base: &base,
            head: &head,
            base_id: grid_wire::SnapshotId("snap:base".into()),
            head_id: grid_wire::SnapshotId("snap:head".into()),
            changed: &[super::Changed {
                before: DatumId([1; 32]),
                after: Some(DatumId([4; 32])),
            }],
            traces: &[],
            cache: &cache,
            env_hash: ENV,
            tests: &tests,
            replay: true,
            fuel: 1,
            declined: None,
        };
        let plan = super::compute(&inputs);
        assert_eq!(
            selected(&plan),
            [
                ("m.test_x".to_owned(), Why::Aground),
                ("m.test_y".to_owned(), Why::Aground),
            ],
            "a walk over fuel selects everything, not a shorter list"
        );
        assert_eq!(plan.unchecked.budget_rejected, 1);
        assert!(plan.replayed.is_empty());
        // With fuel to finish, the same inputs select both by reach and
        // report nothing rejected — the refusal was the budget's, not the
        // chart's.
        let plan = super::compute(&super::Inputs {
            fuel: 100,
            ..inputs
        });
        assert_eq!(
            selected(&plan),
            [
                ("m.test_x".to_owned(), Why::Reached),
                ("m.test_y".to_owned(), Why::Reached),
            ]
        );
        assert_eq!(plan.unchecked.budget_rejected, 0);
    }

    /// Property: `plan(S, S)` selects only the untraced and the unresolved.
    #[test]
    fn a_plan_of_a_snapshot_against_itself_selects_only_untraced_and_unresolved() {
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        ingest(dir.path(), "tests.test_core.test_a", &["pkg.core.a"]);
        ingest(dir.path(), "tests.test_core.test_b", &["pkg.core.b"]);
        record_pass(
            dir.path(),
            ENV,
            &["tests.test_core.test_a", "tests.test_core.test_b"],
        );
        let tests = [
            PY_TESTS[0],
            PY_TESTS[1],
            PY_TESTS[2],
            "nope.py::test_nothing",
        ];
        let plan = plan(dir.path(), Some("HEAD"), ENV, &tests);
        assert_eq!(
            selected(&plan),
            [
                ("tests/test_core.py::test_c".to_owned(), Why::NoTrace),
                ("nope.py::test_nothing".to_owned(), Why::Unresolved),
            ]
        );
        assert_eq!(
            replayed(&plan),
            ["tests/test_core.py::test_a", "tests/test_core.py::test_b"]
        );
        assert!(plan.uncovered.is_empty());
    }

    /// Property: selection is deterministic for a snapshot pair, and adding
    /// a trace never *adds* a selected test.
    #[test]
    fn selection_is_deterministic_and_a_new_trace_never_adds_a_selection() {
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        ingest(dir.path(), "tests.test_core.test_a", &["pkg.core.a"]);
        record_pass(
            dir.path(),
            ENV,
            &[
                "tests.test_core.test_a",
                "tests.test_core.test_b",
                "tests.test_core.test_c",
            ],
        );
        write(dir.path(), &owned(&python(1, 7, 9, [1, 2, 3])));
        let before = plan(dir.path(), Some("HEAD"), ENV, &PY_TESTS);
        assert_eq!(before, plan(dir.path(), Some("HEAD"), ENV, &PY_TESTS));
        // `test_c` was untraced; now it is traced, and its trace reaches `b`
        // — which moved — so it stays selected, by reach instead of by
        // absence. `test_d` gains a trace and drops out of the selection.
        ingest(
            dir.path(),
            "tests.test_core.test_c",
            &["pkg.core.c", "pkg.core.b"],
        );
        ingest(dir.path(), "tests.test_core.test_d", &[]);
        record_pass(dir.path(), ENV, &["tests.test_core.test_d"]);
        let after = plan(dir.path(), Some("HEAD"), ENV, &PY_TESTS);
        let names = |p: &grid_wire::TestPlan| {
            p.selected
                .iter()
                .map(|s| s.test.0.clone())
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert!(
            names(&after).is_subset(&names(&before)),
            "before {:?}, after {:?}",
            names(&before),
            names(&after)
        );
        assert!(names(&after).contains("tests/test_core.py::test_c"));
    }

    /// Ground truth for the `unresolved` reason, over the real Rust frontend.
    /// A `proptest!` block is a macro invocation, so the survey charts no
    /// definition for the tests it generates: the name is missing from a
    /// module the survey *did* chart. That is a different fact from a module
    /// nothing was charted in, and a different one again from a name two
    /// definitions answer to — and the reader of `miss` gets told which.
    #[test]
    fn an_unresolved_test_says_why_the_chart_cannot_name_it() {
        use grid_wire::TestSelectionUnresolved as U;

        let files: Vec<(&'static str, String)> = vec![
            (
                "Cargo.toml",
                "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n".to_owned(),
            ),
            (
                "src/lib.rs",
                "pub fn a() -> i64 { 1 }\n\
                 pub mod p { pub mod t { pub fn dup() -> i64 { 1 } } }\n\
                 pub mod q { pub mod t { pub fn dup() -> i64 { 2 } } }\n\n\
                 #[cfg(test)]\n\
                 mod tests {\n    \
                 use super::*;\n    \
                 #[test]\n    fn charted() { assert_eq!(a(), 1); }\n\n    \
                 proptest! {\n        \
                 #[test]\n        fn generated(x in 0u8..) { let _ = x; }\n    }\n}\n"
                    .to_owned(),
            ),
        ];
        let dir = repo(&owned(&files));
        let ids = [
            "fx tests::charted",
            "fx tests::generated",
            "fx nowhere::absent",
            "t::dup",
        ];
        let plan = plan(dir.path(), Some("HEAD"), ENV, &ids);
        let reason = |id: &str| {
            plan.selected
                .iter()
                .find(|s| s.test.0 == id)
                .map_or_else(|| panic!("{id} is in the plan"), |s| (s.why, s.unresolved))
        };
        assert_eq!(
            reason("fx tests::charted").1,
            None,
            "a charted test is not unresolved"
        );
        assert_eq!(
            reason("fx tests::generated"),
            (Why::Unresolved, Some(U::MacroDefined)),
            "the module is charted and the name is not: a macro defines it"
        );
        assert_eq!(
            reason("fx nowhere::absent"),
            (Why::Unresolved, Some(U::NotCharted)),
            "nothing under that module is charted at all"
        );
        assert_eq!(
            reason("t::dup"),
            (Why::Unresolved, Some(U::Ambiguous)),
            "two definitions answer to that name"
        );
        // And the record says so where a reader will meet it.
        let run = grid_wire::TestRun {
            env_hash: ENV.to_owned(),
            plan,
            ran: Vec::new(),
            warnings: Vec::new(),
        };
        let out = crate::porcelain::test_run(&run);
        assert!(
            out.contains("miss    fx tests::generated  unresolved:macro-defined\n"),
            "{out}"
        );
        assert!(
            out.contains("miss    fx nowhere::absent  unresolved:not-charted\n"),
            "{out}"
        );
        // Conservatism: a plan from before the column existed carries no
        // reason, and no reason is printed. Unknown is not `not-charted`,
        // which is a claim about the survey nobody made.
        let mut older = run;
        for sel in &mut older.plan.selected {
            sel.unresolved = None;
        }
        let out = crate::porcelain::test_run(&older);
        assert!(
            out.contains("miss    fx tests::generated  unresolved\n"),
            "an absent reason prints no reason:\n{out}"
        );
        assert!(!out.contains("unresolved:"), "{out}");
    }

    #[test]
    fn runner_ids_map_to_the_frontends_spellings() {
        assert_eq!(
            callsign_of("grid-cache tests::keys_agree"),
            "grid_cache.tests.keys_agree"
        );
        assert_eq!(
            callsign_of("grid::bin/grid drift::tests::over_budget"),
            "grid.drift.tests.over_budget"
        );
        assert_eq!(
            callsign_of("grid-cache::closure a_closure_past_fuel_is_rejected"),
            "grid_cache.tests.closure.a_closure_past_fuel_is_rejected"
        );
        assert_eq!(
            callsign_of("tests/test_x.py::TestY::test_z"),
            "tests.test_x.TestY.test_z"
        );
        assert_eq!(
            callsign_of("tests/test_x.py::test_z[1-2]"),
            "tests.test_x.test_z"
        );
        assert_eq!(callsign_of("drift::tests::t"), "drift.tests.t");
    }
}
