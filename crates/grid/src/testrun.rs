//! `grid test` — run what the change can reach, replay the rest.
//!
//! The daemon answers the plan ([`crate::testplan`]); this module is the
//! CLI's half: it asks the runner for its inventory, hashes the environment,
//! asks for the plan, runs the selected tests through the runner, records
//! every verdict through `grid.cache.record`, and prints the records. The
//! runner is a trait so the whole loop can be driven by a deterministic fake
//! in tests — the differential the plan asks for compares selection against
//! running everything, and that comparison must not depend on cargo.
use std::collections::HashMap;

use grid_wire::methods::{CacheRecordKind, CacheRecordParams, TestPlanParams};
use grid_wire::{
    CacheResultEntry, CacheResultEntryVerdict, DriftWarning, SnapshotOrRev, TestId, TestOutcome,
    TestOutcomeVerdict, TestPlan, TestRun,
};

use crate::{exit, hint, porcelain, rpc};

/// What the runner reported for one test it ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub test: String,
    pub pass: bool,
    /// The retained trace the run produced, when it was traced.
    pub trace_id: Option<String>,
}

/// A test runner, as `grid test` needs one: an inventory, an environment,
/// and a way to run a named subset. Grid instruments nothing; the real
/// implementation ([`crate::runner::Real`]) spawns cargo and pytest the way
/// a survey spawns git.
pub trait Runner {
    /// The runner's own inventory. `Err` is a runner that could not be
    /// spawned, which is exit 1 — never an empty inventory.
    fn inventory(&mut self) -> Result<Vec<String>, String>;
    /// Bytes that identify the environment: toolchain, lockfile, target.
    fn environment(&self) -> Vec<u8>;
    /// Run exactly these tests. With `trace`, produce and ingest one trace
    /// per test. `Err` is a runner that could not be spawned.
    fn run(&mut self, tests: &[String], trace: bool) -> Result<Vec<Outcome>, String>;
}

/// The flags, parsed.
pub struct Opts {
    pub base: Option<SnapshotOrRev>,
    pub trace: bool,
    /// `false` under `--all` and `--no-replay`: nothing is looked up, so
    /// every test runs. The two flags coincide because an unselected test
    /// must either replay or run; there is no third thing to do with it.
    pub replay: bool,
    /// Everything after `--`, handed to the runner and folded into the
    /// environment hash — a different `--profile` is a different run.
    pub args: Vec<String>,
}

const USAGE: &str = "usage: grid test [<base>] [--trace] [--all] [--no-replay] [-- <runner args>]";

/// `grid test [base] [--trace] [--all] [--no-replay] [--json] [-- <runner args>]`.
pub fn main(args: &[&str], json: bool) -> u8 {
    let mut opts = Opts {
        base: None,
        trace: false,
        replay: true,
        args: Vec::new(),
    };
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match *arg {
            "--trace" => opts.trace = true,
            "--all" | "--no-replay" => opts.replay = false,
            "--" => {
                opts.args = rest.iter().map(|a| (*a).to_owned()).collect();
                break;
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{USAGE}"));
                return exit::BAD_INPUT;
            }
            other if opts.base.is_none() => opts.base = Some(crate::snapshot_or_rev(other)),
            other => {
                hint(&format!("grid: unexpected argument `{other}`\n{USAGE}"));
                return exit::BAD_INPUT;
            }
        }
    }
    let root = crate::root();
    let mut runner = crate::runner::Real::new(root, opts.args.clone());
    drive(root, &opts, &mut runner, json)
}

/// The environment a verdict is keyed under: the runner's bytes, the runner
/// args, and whether the run is traced — an instrumented binary is a
/// different environment.
fn env_hash(runner: &dyn Runner, opts: &Opts) -> String {
    let mut h = blake3::Hasher::new();
    h.update(b"grid-test-env/v1\n");
    let fields = [
        runner.environment(),
        opts.args.join("\0").into_bytes(),
        vec![u8::from(opts.trace)],
    ];
    for field in fields {
        h.update(&u64::try_from(field.len()).unwrap_or(u64::MAX).to_le_bytes());
        h.update(&field);
    }
    format!("env:b3:{}", h.finalize().to_hex())
}

/// The whole verb, over any runner. Returns the exit code: 3 when a run
/// test failed or a replayed verdict is `fail`, 1 when the runner could not
/// be spawned, 0 otherwise.
pub fn drive(root: &std::path::Path, opts: &Opts, runner: &mut dyn Runner, json: bool) -> u8 {
    let tests = match runner.inventory() {
        Ok(tests) => tests,
        Err(e) => {
            hint(&format!("grid: runner could not be spawned: {e}"));
            return exit::INTERNAL;
        }
    };
    let env_hash = env_hash(runner, opts);
    let params = TestPlanParams {
        base: opts.base.clone(),
        env_hash: env_hash.clone(),
        tests: tests.iter().map(|t| TestId(t.clone())).collect(),
        replay: Some(opts.replay),
    };
    let mut chart = rpc::Chart::new(root);
    let plan = match rpc::test_plan_timed(&params, root, &mut chart) {
        Ok((plan, timing)) => {
            // The horizon line: what was selected out of what, and what it
            // cost to decide — the survey is the ledger's own row, the plan
            // is this verb's.
            hint(&format!(
                "grid: {} of {} tests selected, {} replayed — plan {}ms, survey {}ms",
                plan.selected.len(),
                tests.len(),
                plan.replayed.len(),
                timing.plan.as_millis(),
                timing.survey.as_millis()
            ));
            plan
        }
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    if plan.unchecked.budget_rejected > 0 {
        hint(&format!(
            "grid: the selection walk ran past its fuel ({}) — every test is selected, \
             because a truncated walk would look exactly like a smaller change",
            grid_cache::FUEL
        ));
    }
    let run_set: Vec<String> = plan.selected.iter().map(|s| s.test.0.clone()).collect();
    let outcomes = if run_set.is_empty() {
        Vec::new()
    } else {
        match runner.run(&run_set, opts.trace) {
            Ok(outcomes) => outcomes,
            Err(e) => {
                hint(&format!("grid: runner could not be spawned: {e}"));
                return exit::INTERNAL;
            }
        }
    };
    let run = finish(root, &plan, &env_hash, &run_set, &outcomes);
    // Remembered for the next run's default base. Best effort: a base that
    // cannot be remembered costs a wider selection next time, never a verdict.
    let last = crate::testplan::last_path(root);
    if let Err(e) = last
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&last, &run.plan.head.0))
    {
        hint(&format!(
            "grid: could not remember this base for the next run ({}): {e}",
            last.display()
        ));
    }
    let code = if failed(&run) > 0 {
        exit::DIVERGENCE
    } else {
        exit::OK
    };
    if json {
        return match crate::emit_json(&run) {
            exit::OK => code,
            failure => failure,
        };
    }
    print!("{}", porcelain::test_run(&run));
    code
}

/// How many tests this run leaves red: every run test the runner failed,
/// plus every verdict replayed from the cache as `fail`. One function, read
/// by the exit code and by the `done` line, so the two cannot disagree — and
/// a replay is a verdict about this exact closure, so a replayed `fail` is
/// as red as a fresh one.
#[must_use]
pub fn failed(run: &TestRun) -> usize {
    let ran = run
        .ran
        .iter()
        .filter(|o| o.verdict == TestOutcomeVerdict::Fail)
        .count();
    let replayed = run
        .plan
        .replayed
        .iter()
        .filter(|hit| hit.verdict == grid_wire::CacheHitVerdict::Fail)
        .count();
    ran.saturating_add(replayed)
}

/// Record every verdict and assemble the run: the plan verbatim, what the
/// runner said, and the findings — nondeterministic from the record, and
/// uncovered from the plan.
fn finish(
    root: &std::path::Path,
    plan: &TestPlan,
    env_hash: &str,
    run_set: &[String],
    outcomes: &[Outcome],
) -> TestRun {
    let datum_of: HashMap<&str, &str> = plan
        .selected
        .iter()
        .filter_map(|s| Some((s.test.0.as_str(), s.datum.as_ref()?.0.as_str())))
        .collect();
    let test_of: HashMap<&str, &str> = datum_of.iter().map(|(t, d)| (*d, *t)).collect();
    let mut warnings = Vec::new();
    let mut ran: Vec<TestOutcome> = Vec::new();

    // One record call per trace id: a traced run is one trace per test, an
    // untraced run is one batch. Grouped rather than one call per test so
    // the untraced CI form is one log append per run.
    let mut batches: Vec<(Option<String>, Vec<CacheResultEntry>)> = Vec::new();
    for outcome in outcomes {
        ran.push(TestOutcome {
            test: TestId(outcome.test.clone()),
            verdict: if outcome.pass {
                TestOutcomeVerdict::Pass
            } else {
                TestOutcomeVerdict::Fail
            },
            trace_id: outcome.trace_id.clone(),
        });
        // An unresolved test has no datum to key a verdict under; it ran,
        // and that is all grid can say about it.
        let Some(datum) = datum_of.get(outcome.test.as_str()) else {
            continue;
        };
        let entry = CacheResultEntry {
            datum: grid_wire::Datum((*datum).to_owned()),
            verdict: if outcome.pass {
                CacheResultEntryVerdict::Pass
            } else {
                CacheResultEntryVerdict::Fail
            },
        };
        match batches.iter_mut().find(|(id, _)| *id == outcome.trace_id) {
            Some((_, entries)) => entries.push(entry),
            None => batches.push((outcome.trace_id.clone(), vec![entry])),
        }
    }
    // A selected test the runner never reported on is not green. Counted as
    // failed and named, never recorded: the runner gave no verdict to record.
    for test in run_set {
        if !outcomes.iter().any(|o| &o.test == test) {
            ran.push(TestOutcome {
                test: TestId(test.clone()),
                verdict: TestOutcomeVerdict::Fail,
                trace_id: None,
            });
            warnings.push(DriftWarning {
                subject: test.clone(),
                message: "not reported by the runner — counted as failed, nothing recorded".into(),
            });
        }
    }
    for (trace_id, results) in batches {
        let params = CacheRecordParams {
            kind: CacheRecordKind::Test,
            env_hash: env_hash.to_owned(),
            snapshot: None,
            trace_id,
            results,
        };
        match rpc::cache_record(&params, root) {
            Ok(result) => {
                let name = |datum: &str| test_of.get(datum).map_or(datum, |t| *t).to_owned();
                for item in result.nondeterministic.iter().flatten() {
                    warnings.push(DriftWarning {
                        subject: name(&item.datum.0),
                        message: porcelain::nondeterministic_message(item),
                    });
                }
                for item in &result.rejected {
                    warnings.push(DriftWarning {
                        subject: name(&item.datum.0),
                        message: format!("verdict not recorded: {}", item.reason),
                    });
                }
            }
            Err(e) => warnings.push(DriftWarning {
                subject: "-".into(),
                message: format!("verdicts not recorded: {e}"),
            }),
        }
    }
    for def in &plan.uncovered {
        warnings.push(DriftWarning {
            subject: def
                .callsign
                .as_ref()
                .map_or_else(|| def.datum.0.clone(), |c| c.0.clone()),
            message: "uncovered".into(),
        });
    }
    TestRun {
        env_hash: env_hash.to_owned(),
        plan: plan.clone(),
        ran,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    //! The verb driven by a deterministic fake runner that evaluates the
    //! Python fixture from source, so the differential below compares
    //! selection against running everything without spawning pytest.
    use std::path::{Path, PathBuf};

    use super::{drive, Opts, Outcome, Runner};
    use crate::testplan::fixtures::{commit, ingest, owned, python, repo, write};

    const TESTS: [&str; 4] = [
        "tests/test_core.py::test_a",
        "tests/test_core.py::test_b",
        "tests/test_core.py::test_c",
        "tests/test_core.py::test_d",
    ];

    /// Evaluates `pkg/core.py` and `tests/test_core.py` as the fixture
    /// writes them: a function returns a literal or `<name>() + <literal>`,
    /// a test asserts `<name>() == <literal>`. `test_d` always passes.
    struct Fake {
        root: PathBuf,
        /// Every test the fake was asked to run, in order.
        ran: Vec<Vec<String>>,
    }

    impl Fake {
        fn call(&self, name: &str) -> i64 {
            let core = std::fs::read_to_string(self.root.join("pkg/core.py")).expect("core");
            let body = core
                .split("def ")
                .find_map(|chunk| chunk.strip_prefix(&format!("{name}():\n    return ")))
                .unwrap_or_else(|| panic!("{name} is defined"))
                .lines()
                .next()
                .expect("a return line");
            match body.split_once("() + ") {
                Some((callee, add)) => self.call(callee) + add.trim().parse::<i64>().expect("int"),
                None => body.trim().parse().expect("int"),
            }
        }

        /// The verdict a full run at head gives this test.
        fn evaluate(&self, test: &str) -> bool {
            let name = test.rsplit("::test_").next().expect("a test name");
            if name == "d" {
                return true;
            }
            let tests =
                std::fs::read_to_string(self.root.join("tests/test_core.py")).expect("tests");
            let expected: i64 = tests
                .split("def test_")
                .find_map(|chunk| {
                    chunk.strip_prefix(&format!("{name}():\n    assert {name}() == "))
                })
                .expect("an assertion")
                .lines()
                .next()
                .expect("a line")
                .trim()
                .parse()
                .expect("int");
            self.call(name) == expected
        }

        /// What the test reaches, transitively, as callsigns.
        fn reach(&self, name: &str, out: &mut Vec<String>) {
            out.push(format!("pkg.core.{name}"));
            let core = std::fs::read_to_string(self.root.join("pkg/core.py")).expect("core");
            if let Some(callee) = core
                .split("def ")
                .find_map(|chunk| chunk.strip_prefix(&format!("{name}():\n    return ")))
                .and_then(|body| body.lines().next())
                .and_then(|body| body.split_once("() + "))
                .map(|(callee, _)| callee)
            {
                self.reach(callee, out);
            }
        }
    }

    impl Runner for Fake {
        fn inventory(&mut self) -> Result<Vec<String>, String> {
            Ok(TESTS.iter().map(|t| (*t).to_owned()).collect())
        }

        fn environment(&self) -> Vec<u8> {
            b"fake".to_vec()
        }

        fn run(&mut self, tests: &[String], trace: bool) -> Result<Vec<Outcome>, String> {
            self.ran.push(tests.to_vec());
            Ok(tests
                .iter()
                .map(|test| {
                    let name = test.rsplit("::test_").next().expect("a name");
                    let trace_id = trace.then(|| {
                        let mut callees = Vec::new();
                        if name != "d" {
                            self.reach(name, &mut callees);
                        }
                        let refs: Vec<&str> = callees.iter().map(String::as_str).collect();
                        let callsign = format!("tests.test_core.test_{name}");
                        ingest(&self.root, &callsign, &refs);
                        format!("trace:fx:{callsign}")
                    });
                    Outcome {
                        test: test.clone(),
                        pass: self.evaluate(test),
                        trace_id,
                    }
                })
                .collect())
        }
    }

    fn opts(replay: bool) -> Opts {
        Opts {
            base: None,
            trace: true,
            replay,
            args: Vec::new(),
        }
    }

    /// One `grid test` over the fake, returning the exit code and the plan
    /// (read back from what the driver records, via a fresh plan against
    /// the same base — the driver's own plan is not returned).
    fn run(dir: &Path, fake: &mut Fake, replay: bool) -> u8 {
        drive(dir, &opts(replay), fake, false)
    }

    /// The differential (rung 5): over a seeded sequence of edits, every
    /// test the plan skipped has a recorded verdict at base equal to what a
    /// full run at head produces. Selection is compared against running
    /// everything, edit by edit, through the real cache and the real chart.
    #[test]
    fn every_skipped_verdict_equals_a_full_run_at_head() {
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        let mut fake = Fake {
            root: dir.path().to_path_buf(),
            ran: Vec::new(),
        };
        // Edit sequence: a breaks, a's test is fixed, b breaks (taking c
        // with it through the call), both fixed, an untested def moves.
        // `(a, b, d, expectations, exit)` per step.
        let edits: [(i64, i64, i64, [i64; 3], u8); 6] = [
            (1, 2, 9, [1, 2, 3], 0),
            (5, 2, 9, [1, 2, 3], 3),
            (5, 2, 9, [5, 2, 3], 0),
            (5, 7, 9, [5, 2, 3], 3),
            (5, 7, 9, [5, 7, 8], 0),
            (5, 7, 10, [5, 7, 8], 0),
        ];
        let mut skipped_total = 0usize;
        for (step, (a, b, d, expect, expected_exit)) in edits.iter().enumerate() {
            write(dir.path(), &owned(&python(*a, *b, *d, *expect)));
            let code = run(dir.path(), &mut fake, true);
            assert_eq!(code, *expected_exit, "step {step}: exit");
            let ran = fake.ran.last().cloned().unwrap_or_default();
            // Everything not run was skipped: its recorded verdict must be
            // the verdict a full run would give now.
            let cache = grid_cache::Cache::open(crate::rpc::cache_path(dir.path())).expect("cache");
            let epoch = grid_chart_io::Store::open(crate::chart_provider::store_dir(dir.path()))
                .expect("store")
                .load();
            let chart = grid_cache::Chart::new(epoch.segments());
            let env = super::env_hash(&fake, &opts(true));
            for test in TESTS {
                if ran.iter().any(|r| r == test) {
                    continue;
                }
                skipped_total += 1;
                let callsign = crate::testplan::callsign_of(test);
                let datum = crate::testplan::fixtures::datum_of(dir.path(), &callsign);
                let key = chart
                    .key(
                        grid_chart::DatumId::parse(&datum).expect("datum"),
                        grid_cache::Kind::Test,
                        &env,
                    )
                    .expect("closure");
                let recorded = cache
                    .get(key)
                    .unwrap_or_else(|| panic!("step {step}: {test} was skipped without a verdict"));
                assert_eq!(
                    recorded.verdict == grid_cache::Verdict::Pass,
                    fake.evaluate(test),
                    "step {step}: {test} was skipped, and its recorded verdict differs from a full run"
                );
            }
            if step > 0 {
                assert!(
                    ran.len() < TESTS.len(),
                    "step {step}: selection ran everything ({ran:?}), so the differential proved nothing"
                );
            }
        }
        assert!(skipped_total > 0, "the sequence skipped something");
        // Step 4 (b fixed) reaches test_b and test_c through the call and
        // leaves test_a replayed; step 5 (untested d) runs nothing.
        assert_eq!(fake.ran[4], [TESTS[1], TESTS[2]]);
        assert!(
            fake.ran.get(5).is_none_or(Vec::is_empty),
            "{:?}",
            fake.ran.get(5)
        );
    }

    /// Conservatism: a changed definition no test reaches is `uncovered`,
    /// reported as a `warn`, and the exit code does not move for it.
    #[test]
    fn an_uncovered_definition_warns_without_moving_the_exit() {
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        let mut fake = Fake {
            root: dir.path().to_path_buf(),
            ran: Vec::new(),
        };
        assert_eq!(run(dir.path(), &mut fake, true), 0);
        commit(dir.path(), "traced");
        write(dir.path(), &owned(&python(1, 2, 10, [1, 2, 3])));
        let runner: &mut dyn Runner = &mut fake;
        let inventory = runner.inventory().expect("inventory");
        let plan = crate::rpc::test_plan(
            &grid_wire::methods::TestPlanParams {
                base: None,
                env_hash: super::env_hash(runner, &opts(true)),
                tests: inventory.into_iter().map(grid_wire::TestId).collect(),
                replay: Some(true),
            },
            dir.path(),
            &mut crate::rpc::Chart::new(dir.path()),
        )
        .expect("plan");
        let uncovered: Vec<String> = plan
            .uncovered
            .iter()
            .filter_map(|d| d.callsign.as_ref().map(|c| c.0.clone()))
            .collect();
        assert_eq!(uncovered, ["pkg.core.d"]);
        assert!(plan.selected.is_empty(), "{:?}", plan.selected);
        let code = run(dir.path(), &mut fake, true);
        assert_eq!(code, 0, "uncovered is heuristic: it never moves the exit");
        let out = crate::porcelain::test_run(&super::finish(dir.path(), &plan, "env", &[], &[]));
        assert!(
            out.contains("warn    pkg.core.d  uncovered\n"),
            "the warn record names the definition:\n{out}"
        );
    }

    /// The same test twice, opposite verdicts, nothing changed: reported as
    /// `nondeterministic`, in those words, from the last entry.
    #[test]
    fn an_opposite_verdict_under_the_same_key_is_nondeterministic() {
        struct Flip {
            inner: Fake,
            runs: usize,
        }
        impl Runner for Flip {
            fn inventory(&mut self) -> Result<Vec<String>, String> {
                self.inner.inventory()
            }
            fn environment(&self) -> Vec<u8> {
                self.inner.environment()
            }
            fn run(&mut self, tests: &[String], trace: bool) -> Result<Vec<Outcome>, String> {
                self.runs += 1;
                let flip = self.runs.is_multiple_of(2);
                Ok(self
                    .inner
                    .run(tests, trace)?
                    .into_iter()
                    .map(|o| Outcome {
                        pass: o.pass != (flip && o.test.ends_with("test_a")),
                        ..o
                    })
                    .collect())
            }
        }
        let dir = repo(&owned(&python(1, 2, 9, [1, 2, 3])));
        let mut flip = Flip {
            inner: Fake {
                root: dir.path().to_path_buf(),
                ran: Vec::new(),
            },
            runs: 0,
        };
        assert_eq!(drive(dir.path(), &opts(true), &mut flip, false), 0);
        // Nothing changed, so with replay on nothing would run; `--no-replay`
        // is how the same closure gets a second verdict.
        assert_eq!(drive(dir.path(), &opts(false), &mut flip, false), 3);
        let params = grid_wire::methods::CacheRecordParams {
            kind: grid_wire::methods::CacheRecordKind::Test,
            env_hash: super::env_hash(&flip, &opts(true)),
            snapshot: None,
            trace_id: None,
            results: vec![grid_wire::CacheResultEntry {
                datum: grid_wire::Datum(crate::testplan::fixtures::datum_of(
                    dir.path(),
                    "tests.test_core.test_a",
                )),
                verdict: grid_wire::CacheResultEntryVerdict::Pass,
            }],
        };
        let result = crate::rpc::cache_record(&params, dir.path()).expect("record");
        let flagged = result
            .nondeterministic
            .expect("the third verdict flips again");
        assert_eq!(flagged.len(), 1);
        let text = crate::porcelain::nondeterministic_message(&flagged[0]);
        assert!(
            text.starts_with("nondeterministic  same closure key, same env: fail "),
            "{text}"
        );
        assert!(!text.contains("flake"));
    }

    /// A verdict replayed from the cache as `fail` is as red as a fresh one:
    /// nothing changed, so the suite still fails under this exact closure,
    /// and the exit code and the `done` line both say so.
    #[test]
    fn a_replayed_fail_verdict_moves_the_exit() {
        // `a` returns 5 where `test_a` expects 1.
        let dir = repo(&owned(&python(5, 2, 9, [1, 2, 3])));
        let mut fake = Fake {
            root: dir.path().to_path_buf(),
            ran: Vec::new(),
        };
        assert_eq!(
            run(dir.path(), &mut fake, true),
            3,
            "first run: test_a fails"
        );
        let code = run(dir.path(), &mut fake, true);
        assert_eq!(
            fake.ran.len(),
            1,
            "nothing changed, so the runner was not asked a second time: {:?}",
            fake.ran
        );
        assert_eq!(code, 3, "a replayed fail is still a failure");

        // And the `done` line counts it, so the line and the exit agree.
        let plan = grid_wire::TestPlan {
            base: grid_wire::SnapshotId("snap:b3:base".into()),
            head: grid_wire::SnapshotId("snap:b3:head".into()),
            selected: Vec::new(),
            replayed: vec![grid_wire::CacheHit {
                datum: grid_wire::Datum("b3:00".into()),
                verdict: grid_wire::CacheHitVerdict::Fail,
                recorded_at: "2026-09-01T00:00:00Z".into(),
                trace_id: None,
                test: Some(grid_wire::TestId(TESTS[0].to_owned())),
            }],
            uncovered: Vec::new(),
            unchecked: grid_wire::DriftUnchecked {
                unresolved_refs: None,
                unresolved_external: None,
                unresolved_method: None,
                unresolved_internal: None,
                budget_rejected: 0,
                unattributed_entries: None,
                weak_skipped: None,
                compat_unchecked: None,
            },
        };
        let replayed_only = grid_wire::TestRun {
            env_hash: "env".into(),
            plan,
            ran: Vec::new(),
            warnings: Vec::new(),
        };
        assert_eq!(super::failed(&replayed_only), 1);
        let out = crate::porcelain::test_run(&replayed_only);
        assert!(
            out.ends_with("done    tests 0/1 ran · 1 replayed · 1 failed\n"),
            "the done line counts the replayed fail:\n{out}"
        );
    }
}
