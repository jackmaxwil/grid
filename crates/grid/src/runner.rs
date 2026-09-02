//! The real runners: cargo nextest (or `cargo test`) and pytest, spawned the
//! way a survey spawns git. Grid reads their inventories and their verdict
//! lines; it instruments nothing and ships no plugin.
//!
//! `ponytail:` runner output is captured and replayed to stderr after each
//! invocation rather than streamed, because the same bytes are parsed for
//! verdicts; a tee over the child's pipe lands when a long suite makes the
//! silence unbearable.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::testrun::{Outcome, Runner};
use crate::{hint, rpc};

/// A nextest binary, as the rollup needs it: `-p <package> --<kind> <name>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Target {
    package: String,
    kind: String,
    binary: String,
}

impl Target {
    /// The cargo target flag that compiles exactly this binary.
    fn flags(&self) -> Vec<String> {
        match self.kind.as_str() {
            "lib" => vec!["--lib".to_owned()],
            "bin" | "test" | "bench" | "example" => {
                vec![format!("--{}", self.kind), self.binary.clone()]
            }
            // proc-macro and anything nextest adds later: the package is
            // enough of a restriction, and a wider compile is never wrong.
            _ => Vec::new(),
        }
    }
}

/// Which Rust runner the machine has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rust {
    Nextest,
    /// `ponytail:` binary granularity: the inventory is names, and the run
    /// compiles the whole workspace with `-- --exact`. Install nextest to
    /// skip the links, which is the saving that matters.
    CargoTest,
}

/// The runners the repo has, detected once from its manifests.
pub struct Real {
    root: PathBuf,
    args: Vec<String>,
    rust: Option<Rust>,
    python: bool,
    /// nextest id -> its binary, from the inventory.
    targets: HashMap<String, Target>,
}

impl Real {
    /// A runner over the repo at `root`, with the user's extra runner args.
    #[must_use]
    pub fn new(root: &Path, args: Vec<String>) -> Self {
        let has = |names: &[&str]| names.iter().any(|n| root.join(n).exists());
        let rust = has(&["Cargo.toml"]).then(|| {
            if cargo(root, &["nextest", "--version"]).is_ok() {
                Rust::Nextest
            } else {
                Rust::CargoTest
            }
        });
        let python = has(&[
            "pyproject.toml",
            "pytest.ini",
            "setup.cfg",
            "tox.ini",
            "conftest.py",
        ]);
        Self {
            root: root.to_path_buf(),
            args,
            rust,
            python,
            targets: HashMap::new(),
        }
    }

    fn nextest_inventory(&mut self) -> Result<Vec<String>, String> {
        let out = cargo(&self.root, &["nextest", "list", "--message-format", "json"])?;
        let listing: serde_json::Value = serde_json::from_str(&out)
            .map_err(|e| format!("cargo nextest list did not answer JSON: {e}"))?;
        let mut ids = Vec::new();
        let Some(suites) = listing["rust-suites"].as_object() else {
            return Err("cargo nextest list: no rust-suites in its answer".into());
        };
        for (binary_id, suite) in suites {
            let field = |k: &str| suite[k].as_str().unwrap_or_default().to_owned();
            let target = Target {
                package: field("package-name"),
                kind: field("kind"),
                binary: field("binary-name"),
            };
            let Some(cases) = suite["testcases"].as_object() else {
                continue;
            };
            for (name, case) in cases {
                // An ignored test is not run by `nextest run`, so selecting
                // it would be a test the runner never reports on.
                if case["ignored"].as_bool() == Some(true) {
                    continue;
                }
                let id = format!("{binary_id} {name}");
                self.targets.insert(id.clone(), target.clone());
                ids.push(id);
            }
        }
        ids.sort();
        Ok(ids)
    }

    fn cargo_test_inventory(&self) -> Result<Vec<String>, String> {
        let out = cargo(&self.root, &["test", "--workspace", "--", "--list"])?;
        let mut ids: Vec<String> = out
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .map(str::to_owned)
            .collect();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    fn pytest_inventory(&self) -> Result<Vec<String>, String> {
        let out = python(&self.root, &["-m", "pytest", "--collect-only", "-q"])?;
        Ok(out
            .lines()
            .map(str::trim)
            .filter(|l| l.contains("::") && !l.contains(' '))
            .map(str::to_owned)
            .collect())
    }

    /// Selected nextest tests, rolled up to their binaries: one invocation,
    /// `-p … --test … -E 'test(=a) + test(=b)'`, so unselected binaries are
    /// neither compiled nor linked.
    fn nextest_run(&self, tests: &[String]) -> Result<Vec<Outcome>, String> {
        let mut args: Vec<String> = vec!["nextest".into(), "run".into(), "--no-fail-fast".into()];
        args.extend(self.args.iter().cloned());
        args.extend(self.rollup(tests));
        args.push("-E".into());
        args.push(filter(tests));
        let stderr = cargo_passthrough(&self.root, &args)?;
        Ok(parse_nextest(&stderr, tests))
    }

    /// The step-1 recipe, per selected test: `cargo llvm-cov nextest --lcov
    /// -E 'test(=t)'`, then ingest as the test's trace.
    ///
    /// `ponytail:` one `cargo llvm-cov` invocation per test, because a
    /// single profile cannot be attributed back to the tests that shared a
    /// process. A nextest wrapper keying `LLVM_PROFILE_FILE` by test name
    /// makes it one invocation — nextest configuration, not grid code.
    fn nextest_run_traced(&self, tests: &[String]) -> Result<Vec<Outcome>, String> {
        let mut outcomes = Vec::new();
        let lcov = self.root.join(".grid/test/coverage.lcov");
        for test in tests {
            let mut args: Vec<String> = vec![
                "llvm-cov".into(),
                "nextest".into(),
                "--lcov".into(),
                "--output-path".into(),
                lcov.display().to_string(),
                "--no-fail-fast".into(),
            ];
            args.extend(self.args.iter().cloned());
            args.extend(self.rollup(std::slice::from_ref(test)));
            args.push("-E".into());
            args.push(filter(std::slice::from_ref(test)));
            let stderr = cargo_passthrough(&self.root, &args)?;
            let mut parsed = parse_nextest(&stderr, std::slice::from_ref(test));
            let report = std::fs::read_to_string(&lcov).unwrap_or_default();
            let trace_id = self.ingest(&report, test, true);
            for outcome in &mut parsed {
                outcome.trace_id.clone_from(&trace_id);
            }
            outcomes.extend(parsed);
        }
        let _ = std::fs::remove_file(&lcov);
        Ok(outcomes)
    }

    /// Map a coverage report onto the chart as the test's trace and retain
    /// it. `None` when the report could not be mapped or retained, said on
    /// stderr: a run whose trace was lost is still a run, and its verdict is
    /// still a verdict — just an untraced one.
    fn ingest(&self, report: &str, test: &str, lcov: bool) -> Option<String> {
        let callsign = crate::testplan::callsign_of(test);
        let mapped = if lcov {
            crate::coverage::trace_of_lcov(&self.root, report, &callsign)
        } else {
            crate::coverage::trace_of_coverage_json(&self.root, report, &callsign)
        };
        let mapped = match mapped {
            Ok(mapped) => mapped,
            Err(e) => {
                hint(&format!("grid: {test}: coverage not mapped: {e}"));
                return None;
            }
        };
        let id = mapped.trace.trace_id.clone();
        let params = grid_wire::methods::TraceIngestParams {
            trace: mapped.trace,
        };
        match rpc::trace_ingest(&params, &self.root) {
            Ok(_) => Some(id),
            Err(e) => {
                hint(&format!("grid: {test}: trace not retained: {e}"));
                None
            }
        }
    }

    fn rollup(&self, tests: &[String]) -> Vec<String> {
        let mut targets: Vec<&Target> = tests.iter().filter_map(|t| self.targets.get(t)).collect();
        targets.sort_by_key(|t| (&t.package, &t.kind, &t.binary));
        targets.dedup();
        let mut flags = Vec::new();
        let mut packages: Vec<&str> = targets.iter().map(|t| t.package.as_str()).collect();
        packages.dedup();
        for package in packages {
            flags.push("-p".to_owned());
            flags.push(package.to_owned());
        }
        // Cargo applies a target flag to every named package, and tolerates
        // a package that lacks the target; one flag per distinct binary.
        let mut seen: Vec<Vec<String>> = Vec::new();
        for target in targets {
            let flag = target.flags();
            if !seen.contains(&flag) {
                flags.extend(flag.iter().cloned());
                seen.push(flag);
            }
        }
        flags
    }

    fn cargo_test_run(&self, tests: &[String]) -> Result<Vec<Outcome>, String> {
        let mut args: Vec<String> = vec!["test".into(), "--workspace".into()];
        args.extend(self.args.iter().cloned());
        args.push("--".into());
        args.push("--exact".into());
        args.extend(tests.iter().cloned());
        let out = cargo_passthrough(&self.root, &args)?;
        let mut outcomes = Vec::new();
        for line in out.lines() {
            let Some(rest) = line.trim().strip_prefix("test ") else {
                continue;
            };
            let Some((name, status)) = rest.split_once(" ... ") else {
                continue;
            };
            if !tests.iter().any(|t| t == name) {
                continue;
            }
            let pass = match status.trim() {
                "ok" => true,
                s if s.starts_with("FAILED") => false,
                _ => continue,
            };
            outcomes.push(Outcome {
                test: name.to_owned(),
                pass,
                trace_id: None,
            });
        }
        Ok(outcomes)
    }

    fn pytest_run(&self, tests: &[String], trace: bool) -> Result<Vec<Outcome>, String> {
        let traced = trace && python(&self.root, &["-c", "import pytest_cov, coverage"]).is_ok();
        if trace && !traced {
            hint(
                "grid: pytest-cov is not importable — running untraced; traces refresh when it is",
            );
        }
        let mut args: Vec<String> = vec!["-m".into(), "pytest".into(), "-v".into()];
        if traced {
            args.push("--cov".into());
            args.push("--cov-context=test".into());
        }
        args.extend(self.args.iter().cloned());
        args.extend(tests.iter().cloned());
        let out = python_passthrough(&self.root, &args)?;
        let mut outcomes = Vec::new();
        for line in out.lines() {
            let mut tokens = line.split_whitespace();
            let (Some(id), Some(status)) = (tokens.next(), tokens.next()) else {
                continue;
            };
            if !tests.iter().any(|t| t == id) {
                continue;
            }
            let pass = match status {
                "PASSED" | "XFAIL" => true,
                "FAILED" | "ERROR" | "XPASS" => false,
                _ => continue,
            };
            let trace_id = traced.then(|| self.coverage_context(id)).flatten();
            outcomes.push(Outcome {
                test: id.to_owned(),
                pass,
                trace_id,
            });
        }
        Ok(outcomes)
    }

    /// One test's reach out of the shared coverage data file: no rerun.
    fn coverage_context(&self, test: &str) -> Option<String> {
        let path = self.root.join(".grid/test/coverage.json");
        let out = python(
            &self.root,
            &[
                "-m",
                "coverage",
                "json",
                &format!("--contexts={test}"),
                "-o",
                &path.display().to_string(),
            ],
        );
        if let Err(e) = out {
            hint(&format!("grid: {test}: coverage json: {e}"));
            return None;
        }
        let report = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        self.ingest(&report, test, false)
    }
}

impl Runner for Real {
    fn inventory(&mut self) -> Result<Vec<String>, String> {
        let mut ids = match self.rust {
            Some(Rust::Nextest) => self.nextest_inventory()?,
            Some(Rust::CargoTest) => {
                hint("grid: cargo-nextest not found — falling back to `cargo test`, which compiles every binary");
                self.cargo_test_inventory()?
            }
            None => Vec::new(),
        };
        if self.python {
            ids.extend(self.pytest_inventory()?);
        }
        if self.rust.is_none() && !self.python {
            return Err("no Cargo.toml and no pytest configuration in this tree".into());
        }
        Ok(ids)
    }

    fn environment(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if self.rust.is_some() {
            out.extend(
                cargo(&self.root, &["--version"])
                    .unwrap_or_default()
                    .into_bytes(),
            );
            out.extend(rustc(&self.root).into_bytes());
            out.extend(std::fs::read(self.root.join("Cargo.lock")).unwrap_or_default());
        }
        if self.python {
            out.extend(
                python(&self.root, &["--version"])
                    .unwrap_or_default()
                    .into_bytes(),
            );
            for lock in ["uv.lock", "poetry.lock", "Pipfile.lock", "requirements.txt"] {
                if let Ok(bytes) = std::fs::read(self.root.join(lock)) {
                    out.extend(bytes);
                    break;
                }
            }
        }
        out
    }

    fn run(&mut self, tests: &[String], trace: bool) -> Result<Vec<Outcome>, String> {
        let (rust, py): (Vec<String>, Vec<String>) =
            tests.iter().cloned().partition(|t| !t.contains(".py::"));
        let mut outcomes = Vec::new();
        if !rust.is_empty() {
            outcomes.extend(match self.rust {
                Some(Rust::Nextest) => {
                    let llvm_cov = trace && cargo(&self.root, &["llvm-cov", "--version"]).is_ok();
                    if trace && !llvm_cov {
                        hint("grid: cargo-llvm-cov not found — running untraced; traces refresh when it is");
                    }
                    if llvm_cov {
                        self.nextest_run_traced(&rust)?
                    } else {
                        self.nextest_run(&rust)?
                    }
                }
                Some(Rust::CargoTest) => {
                    if trace {
                        hint("grid: tracing needs cargo-nextest and cargo-llvm-cov — running untraced");
                    }
                    self.cargo_test_run(&rust)?
                }
                None => Vec::new(),
            });
        }
        if !py.is_empty() {
            outcomes.extend(self.pytest_run(&py, trace)?);
        }
        Ok(outcomes)
    }
}

/// nextest's filter for exactly these tests: `test(=a) + test(=b)`.
/// `ponytail:` one expression per run, so a very large selection is bounded
/// by the platform's argument length; a `--filterset-file` lands if a repo
/// ever selects enough tests to hit it.
fn filter(tests: &[String]) -> String {
    tests
        .iter()
        .map(|t| {
            let name = t.split_once(' ').map_or(t.as_str(), |(_, n)| n);
            format!("test(={name})")
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Verdict lines out of nextest's stderr: `PASS [ 0.01s] (1/3) <binary> <name>`,
/// with `FAIL`, `TIMEOUT`, signals, and `TRY n` retries in the same shape.
/// The last line for a test wins, which is what a retry means.
fn parse_nextest(stderr: &str, tests: &[String]) -> Vec<Outcome> {
    let mut verdicts: HashMap<&str, bool> = HashMap::new();
    for line in stderr.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(status) = tokens
            .windows(2)
            .find(|w| w[1].starts_with('[') && verdict_of(w[0]).is_some())
            .and_then(|w| verdict_of(w[0]))
        else {
            continue;
        };
        let [.., binary, name] = tokens.as_slice() else {
            continue;
        };
        let id = format!("{binary} {name}");
        if let Some(test) = tests.iter().find(|t| **t == id) {
            verdicts.insert(test.as_str(), status);
        }
    }
    tests
        .iter()
        .filter_map(|t| {
            Some(Outcome {
                test: t.clone(),
                pass: *verdicts.get(t.as_str())?,
                trace_id: None,
            })
        })
        .collect()
}

const fn verdict_of(token: &str) -> Option<bool> {
    match token.as_bytes() {
        b"PASS" | b"LEAK" | b"FLAKY" => Some(true),
        b"FAIL" | b"TIMEOUT" | b"ABORT" | b"SIGSEGV" | b"SIGABRT" | b"SIGBUS" | b"SIGILL"
        | b"SIGKILL" => Some(false),
        _ => None,
    }
}

/// Run cargo and return its stdout; a non-zero exit is an error with stderr.
fn cargo(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo {} exited {}: {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn rustc(root: &Path) -> String {
    Command::new("rustc")
        .arg("-Vv")
        .current_dir(root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Run cargo, replay both streams to stderr, and return them concatenated
/// for parsing. Test failures are not spawn failures: a non-zero exit with
/// output is a run that happened.
fn cargo_passthrough(root: &Path, args: &[String]) -> Result<String, String> {
    passthrough("cargo", root, args)
}

fn python(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("python3")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("python3: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "python3 {} exited {}: {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn python_passthrough(root: &Path, args: &[String]) -> Result<String, String> {
    passthrough("python3", root, args)
}

fn passthrough(program: &str, root: &Path, args: &[String]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    // The runner's output is the user's; grid's records stay on stdout.
    eprint!("{stdout}{stderr}");
    if out.stdout.is_empty() && out.stderr.is_empty() && !out.status.success() {
        return Err(format!(
            "{program} {} exited {}",
            args.join(" "),
            out.status
        ));
    }
    Ok(format!("{stdout}\n{stderr}"))
}
