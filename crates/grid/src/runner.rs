//! The real runners: cargo nextest (or `cargo test`) and pytest, spawned the
//! way a survey spawns git. Grid reads their inventories and their verdict
//! lines; it instruments nothing and ships no plugin.
//!
//! Runner output is streamed: each of the child's two pipes is drained by its
//! own thread, written to stderr as it arrives, and kept — the same bytes the
//! verdict parse reads. Two threads rather than one because a child that
//! fills one pipe while grid reads the other never finishes, and a nextest
//! run fills both.
use std::collections::HashMap;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    /// process. The nextest wrapper template in `scripts/templates/` fixes
    /// the attribution half — one `.profraw` per test, named by the test id
    /// — and this loop says so when it sees those files; turning them into
    /// traces without a second export is `docs/TODOS.md` T-45.
    fn nextest_run_traced(&self, tests: &[String]) -> Result<Vec<Outcome>, String> {
        let mut outcomes = Vec::new();
        let profiles = profile_dir(&self.root);
        match per_test_profiles(&profiles) {
            0 => {}
            found => hint(&format!(
                "grid: {found} per-test profiles under {} — the nextest wrapper is doing its \
                 half; grid still exports one report per test (docs/TODOS.md T-45)",
                profiles.display()
            )),
        }
        let lcov = self.root.join(".grid/test/coverage.lcov");
        for test in tests {
            // One report per test, so the previous test's must be gone
            // before this one runs: `cargo llvm-cov` writes nothing when the
            // test command fails, and reading the stale file would file the
            // last test's reach under this test's name.
            let _absent = std::fs::remove_file(&lcov);
            let mut args: Vec<String> = vec![
                "llvm-cov".into(),
                "nextest".into(),
                "--lcov".into(),
                "--output-path".into(),
                lcov.display().to_string(),
                // A failing test still ran, and its reach is still its
                // trace: without this the report is withheld on failure.
                "--ignore-run-fail".into(),
                "--no-fail-fast".into(),
            ];
            args.extend(self.args.iter().cloned());
            args.extend(self.rollup(std::slice::from_ref(test)));
            args.push("-E".into());
            args.push(filter(std::slice::from_ref(test)));
            let stderr = cargo_passthrough(&self.root, &args)?;
            let mut parsed = parse_nextest(&stderr, std::slice::from_ref(test));
            // No report is no trace — never an empty trace, which would read
            // as "this test reached nothing charted" and license a replay on
            // the static closure alone.
            let trace_id = match std::fs::read_to_string(&lcov) {
                Ok(report) if !report.trim().is_empty() => self.ingest(&report, test, true),
                _ => {
                    hint(&format!(
                        "grid: {test}: cargo llvm-cov wrote no report — verdict kept, no trace"
                    ));
                    None
                }
            };
            for outcome in &mut parsed {
                outcome.trace_id.clone_from(&trace_id);
            }
            outcomes.extend(parsed);
        }
        let _absent = std::fs::remove_file(&lcov);
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
                &format!("--contexts={}", context_pattern(test)),
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

/// Where the nextest wrapper template writes one profile per test:
/// `GRID_LLVM_PROFILE_DIR` when the environment names one, else
/// `target/grid-profiles` — the same default the template's wrapper uses.
fn profile_dir(root: &Path) -> PathBuf {
    std::env::var_os("GRID_LLVM_PROFILE_DIR")
        .map_or_else(|| root.join("target/grid-profiles"), PathBuf::from)
}

/// How many per-test profiles are sitting in `dir`. A directory that is not
/// there holds none, which is the ordinary case: the template is a template.
fn per_test_profiles(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "profraw"))
            .count()
    })
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

/// coverage.py's `--contexts` takes a Python regex. A pytest-cov context is
/// `<nodeid>|<phase>`, and a node id carries regex metacharacters of its own
/// (`test_z[1-2]` is a character class, `.` matches anything, an unanchored
/// `test_a` also matches `test_ab`), so the id is escaped and anchored to
/// the start and to the phase separator.
fn context_pattern(test: &str) -> String {
    let mut out = String::with_capacity(test.len().saturating_add(4));
    out.push('^');
    for c in test.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push_str("\\|");
    out
}

/// `text` minus ANSI escape sequences. nextest colours its stderr under
/// `CARGO_TERM_COLOR=always` (or its own colour setting), and a coloured
/// `PASS` token is not the word `PASS`; the runner is asked for plain output
/// and this is the belt to that suspender.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            // CSI: parameter and intermediate bytes, then one final byte.
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Verdict lines out of nextest's stderr: `PASS [ 0.01s] (1/3) <binary> <name>`,
/// with `FAIL`, `TIMEOUT`, signals, and `TRY n` retries in the same shape.
/// The last line for a test wins, which is what a retry means.
fn parse_nextest(stderr: &str, tests: &[String]) -> Vec<Outcome> {
    let mut verdicts: HashMap<&str, bool> = HashMap::new();
    let plain = strip_ansi(stderr);
    for line in plain.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(status) = tokens.windows(2).find_map(|w| match w {
            [status, next] if next.starts_with('[') => verdict_of(status),
            _ => None,
        }) else {
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
    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        // The same bytes are parsed for verdicts, and a coloured `PASS` is
        // not the word `PASS`: cargo, nextest and pytest all honour these.
        .env("CARGO_TERM_COLOR", "never")
        .env("NO_COLOR", "1")
        .env("PY_COLORS", "0")
        // A runner grid drives is not an interactive one; a child that asks
        // the terminal a question would hang a hook forever.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    // Both pipes drained at once, and to stderr as the bytes arrive: a
    // twelve-second suite with nothing on screen looks hung, and a child
    // that fills one pipe while grid reads the other *is* hung.
    let draining = [child.stdout.take().map(tee), child.stderr.take().map(tee)];
    let status = child.wait().map_err(|e| format!("{program}: {e}"))?;
    let [stdout, stderr] =
        draining.map(|d| d.map_or_else(String::new, |t| t.join().unwrap_or_default()));
    if stdout.is_empty() && stderr.is_empty() && !status.success() {
        return Err(format!("{program} {} exited {}", args.join(" "), status));
    }
    Ok(format!("{stdout}\n{stderr}"))
}

/// Drain one of the child's pipes on its own thread: every chunk to stderr as
/// it arrives, and the same bytes kept for the verdict parse. A read error
/// ends the drain with what was read — a pipe that stops answering is the end
/// of that stream, and the exit status is what says whether the run was one.
fn tee(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut kept: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(read @ 1..) = pipe.read(&mut buf) {
            let chunk = buf.get(..read).unwrap_or_default();
            // The runner's output is the user's; grid's records stay on
            // stdout, and a write nobody is listening to is not an error.
            let _unwritable = std::io::stderr().write_all(chunk);
            kept.extend_from_slice(chunk);
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::{context_pattern, parse_nextest};

    /// A coloured verdict line is still a verdict: `CARGO_TERM_COLOR=always`
    /// must not turn every selected test into "not reported by the runner".
    #[test]
    fn a_coloured_nextest_verdict_still_parses() {
        let tests = ["grid-wire::conformance a_test".to_owned()];
        let stderr = "\x1b[32;1m        PASS\x1b[0m [   0.011s] (1/1) \
                      \x1b[36mgrid-wire::conformance\x1b[0m \x1b[1ma_test\x1b[0m\n";
        let parsed = parse_nextest(stderr, &tests);
        assert_eq!(parsed.len(), 1, "the coloured line is parsed: {parsed:?}");
        assert!(parsed[0].pass);
        let failed = parse_nextest(
            "\x1b[31;1m        FAIL\x1b[0m [   0.011s] (1/1) grid-wire::conformance a_test\n",
            &tests,
        );
        assert_eq!(failed.len(), 1);
        assert!(!failed[0].pass);
    }

    /// The per-test profiles the nextest wrapper template leaves behind are
    /// counted where they are, and an absent directory is none of them —
    /// the template is a template, so the ordinary tree has no such dir.
    #[test]
    fn per_test_profiles_are_counted_where_the_wrapper_writes_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(super::per_test_profiles(&dir.path().join("absent")), 0);
        assert_eq!(super::per_test_profiles(dir.path()), 0);
        // The wrapper's own naming: the test id, percent-encoded.
        for name in ["grid%20drift%3A%3Atests%3A%3At.profraw", "x%20y.profraw"] {
            std::fs::write(dir.path().join(name), []).expect("write");
        }
        std::fs::write(dir.path().join("coverage.lcov"), []).expect("write");
        assert_eq!(super::per_test_profiles(dir.path()), 2);
    }

    /// Both of the child's streams are drained while it runs, and the parse
    /// still gets all of both. The child writes past a pipe buffer on stdout
    /// before saying anything on stderr, so a reader that waits for the exit
    /// — or reads one pipe to its end before starting the other — wedges here
    /// rather than returning short. That is the failure this proves absent,
    /// and it is why there are two threads.
    ///
    /// `ponytail:` `66_000` bytes because `65_536` is the largest pipe buffer
    /// this runs on; the number is a platform bound, not a chosen size.
    #[test]
    fn both_streams_are_drained_while_the_child_runs() {
        let script = "head -c 66000 /dev/zero | tr '\\0' o; echo drained >&2";
        let out = super::passthrough(
            "/bin/sh",
            std::path::Path::new("."),
            &["-c".to_owned(), script.to_owned()],
        )
        .expect("the child ran");
        assert_eq!(out.matches('o').count(), 66_000, "stdout is complete");
        assert!(out.contains("drained"), "stderr is complete");
    }

    /// The coverage.py context is a regex: the id is escaped and anchored,
    /// so `test_a` cannot select `test_ab` and `test_z[1-2]` selects itself.
    #[test]
    fn a_pytest_id_is_escaped_and_anchored_as_a_context_regex() {
        assert_eq!(
            context_pattern("tests/test_x.py::test_a"),
            "^tests\\/test_x\\.py\\:\\:test_a\\|"
        );
        assert_eq!(
            context_pattern("tests/t.py::test_z[1-2]"),
            "^tests\\/t\\.py\\:\\:test_z\\[1\\-2\\]\\|"
        );
    }
}
