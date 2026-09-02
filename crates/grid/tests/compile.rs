//! `grid check --compile` end to end: a real `cargo check` over a real,
//! tiny crate, and the proven callers it did not compile.
//!
//! The fixtures are one-crate repositories with no dependencies so the
//! compile inside them stays fast; cargo is spawned exactly as the verb
//! spawns it, because a fake compile would leave the one claim this verb
//! makes — "the compiler did not see this" — untested.
use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
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

fn write(dir: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(full, text).expect("write");
    }
}

const MANIFEST: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
                        [features]\ndefault = []\ntui = []\n\n[workspace]\n";

/// A crate with `changed` and its callers at `before`, then `changed`'s
/// signature moved at `after` while every caller still compiles under it.
fn repo(before: &str, after: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[("Cargo.toml", MANIFEST), ("src/lib.rs", before)],
    );
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "before"]);
    write(dir.path(), &[("src/lib.rs", after)]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "after"]);
    dir
}

fn grid(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(dir)
        // The test runner's own cargo environment must not leak into the
        // fixture's compile: a target dir or a feature set of *this*
        // workspace would make the fixture's compile a different question.
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("RUSTFLAGS")
        .output()
        .expect("run grid")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout is utf-8")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// `(dependent, reason)` of every `drift` record.
fn drifts(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&"drift"))
        .filter_map(|c| Some((c.get(2)?.to_string(), c.get(6)?.to_string())))
        .collect()
}

/// `(subject, message)` of every `warn` record.
fn warns(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("warn")?.trim_start();
            let (subject, message) = rest.split_once("  ")?;
            Some((subject.to_owned(), message.to_owned()))
        })
        .collect()
}

fn cargo_present() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

const BEFORE_TEST: &str = "pub fn changed(x: u32) -> u32 { x }\n\
                           pub fn compiled() -> u32 { changed(1) }\n\
                           #[cfg(test)]\nmod tests {\n    use super::*;\n    pub fn t() -> u32 { changed(1) }\n}\n";
const AFTER_TEST: &str = "pub fn changed(x: u64) -> u64 { x }\n\
                          pub fn compiled() -> u64 { changed(1) }\n\
                          #[cfg(test)]\nmod tests {\n    use super::*;\n    pub fn t() -> u64 { changed(1) }\n}\n";

/// Ground truth, `cfg(test)`: the test-module caller of a moved signature
/// is a `drift` record after a plain compile, and none after `-- --tests`.
#[test]
fn a_test_module_caller_is_uncompiled_until_tests_are_compiled() {
    if !cargo_present() {
        eprintln!("skipping: cargo is not on PATH");
        return;
    }
    let dir = repo(BEFORE_TEST, AFTER_TEST);
    let out = grid(dir.path(), &["check", "--compile"]);
    assert_eq!(
        code(&out),
        3,
        "an uncompiled proven caller diverges:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        drifts(&out),
        [("app.tests.t".to_owned(), "uncompiled:test".to_owned())]
    );
    assert!(
        dir.path().join(".grid/compile.json").exists(),
        "the record was written"
    );

    // The base was surveyed on demand and retained: the worktree is still
    // what every read verb answers about.
    let live: serde_json::Value =
        serde_json::from_str(&stdout(&grid(dir.path(), &["version", "--json"]))).expect("json");
    let worktree: serde_json::Value =
        serde_json::from_str(&stdout(&grid(dir.path(), &["survey", "--json"]))).expect("json");
    assert_eq!(
        live["snapshot"], worktree["snapshot_id"],
        "a bare check --compile leaves the worktree live, not HEAD~1"
    );

    let out = grid(dir.path(), &["check", "--compile", "--", "--tests"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "", "compiled under --tests: nothing to say");
}

const BEFORE_FEATURE: &str = "pub fn changed(x: u32) -> u32 { x }\n\
                              #[cfg(feature = \"tui\")]\npub fn ui() -> u32 { changed(1) }\n";
const AFTER_FEATURE: &str = "pub fn changed(x: u64) -> u64 { x }\n\
                             #[cfg(feature = \"tui\")]\npub fn ui() -> u64 { changed(1) }\n";

/// Ground truth, features: a caller behind a feature is a record without
/// the feature, none with `--features tui`, none with `--all-features`.
#[test]
fn a_feature_gated_caller_is_uncompiled_until_the_feature_is_given() {
    if !cargo_present() {
        eprintln!("skipping: cargo is not on PATH");
        return;
    }
    let dir = repo(BEFORE_FEATURE, AFTER_FEATURE);
    let out = grid(dir.path(), &["check", "--compile"]);
    assert_eq!(code(&out), 3, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        drifts(&out),
        [("app.ui".to_owned(), "uncompiled:feature=\"tui\"".to_owned())]
    );
    for args in [
        &["check", "--compile", "--", "--features", "tui"][..],
        &["check", "--compile", "--", "--all-features"][..],
    ] {
        let out = grid(dir.path(), args);
        assert_eq!(
            code(&out),
            0,
            "{args:?}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(stdout(&out), "", "{args:?}");
    }
    // The record says what the compile was asked for.
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join(".grid/compile.json")).expect("record"),
    )
    .expect("record is JSON");
    assert_eq!(record["all_features"], true);
    assert_eq!(record["tests"], false);
    assert!(record["snapshot"]
        .as_str()
        .is_some_and(|s| s.starts_with("snap:")));
}

const BEFORE_CUSTOM: &str = "pub fn changed(x: u32) -> u32 { x }\n\
                             #[cfg(kani)]\npub fn proof() -> u32 { changed(1) }\n\
                             #[cfg_attr(kani, cfg(never))]\npub fn maybe() -> u32 { changed(2) }\n\
                             #[cfg_attr(kani, allow(dead_code))]\npub fn plain() -> u32 { changed(3) }\n";
const AFTER_CUSTOM: &str = "pub fn changed(x: u64) -> u64 { x }\n\
                            #[cfg(kani)]\npub fn proof() -> u64 { changed(1) }\n\
                            #[cfg_attr(kani, cfg(never))]\npub fn maybe() -> u64 { changed(2) }\n\
                            #[cfg_attr(kani, allow(dead_code))]\npub fn plain() -> u64 { changed(3) }\n";

/// Conservatism: a caller under a custom `cfg(kani)` or a `cfg_attr(…)`
/// that can expand to a `cfg` is a `warn`, never a `drift` record, and the
/// exit stays 0. An unproven exclusion does not gate — an evaluator that
/// read unknown as false would hold every repo with a custom cfg. A
/// `cfg_attr` that expands to an `allow` conditions nothing: its caller was
/// compiled, and it is neither a record nor a warning.
#[test]
fn an_undecidable_cfg_warns_and_never_gates() {
    if !cargo_present() {
        eprintln!("skipping: cargo is not on PATH");
        return;
    }
    let dir = repo(BEFORE_CUSTOM, AFTER_CUSTOM);
    let out = grid(dir.path(), &["check", "--compile"]);
    assert_eq!(drifts(&out), [], "an unknown cfg is not a proven exclusion");
    let mut found = warns(&out);
    found.sort();
    assert_eq!(
        found,
        [
            (
                "app.maybe".to_owned(),
                "cfg unknown: cfg_attr(kani,cfg(never))".to_owned()
            ),
            ("app.proof".to_owned(), "cfg unknown: kani".to_owned()),
        ],
        "and `plain`, under a cfg_attr that conditions nothing, is neither"
    );
    assert_eq!(
        code(&out),
        0,
        "a warning never moves the exit:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Property: with every feature and every test target compiled, a repo
/// with no custom cfg has nothing left uncompiled and nothing unknown —
/// the report is empty, in porcelain and on the wire. That includes a
/// `#[cfg(not(test))]` caller: `--tests` compiles the lib both ways, so
/// the compiler saw it, and calling it uncompiled was a false gate.
#[test]
fn everything_compiled_emits_nothing() {
    if !cargo_present() {
        eprintln!("skipping: cargo is not on PATH");
        return;
    }
    let before = format!(
        "{BEFORE_TEST}#[cfg(feature = \"tui\")]\npub fn ui() -> u32 {{ changed(1) }}\n\
         #[cfg(all(unix, not(windows)))]\npub fn host() -> u32 {{ changed(3) }}\n\
         #[cfg(not(test))]\npub fn prod() -> u32 {{ changed(4) }}\n"
    );
    let after = format!(
        "{AFTER_TEST}#[cfg(feature = \"tui\")]\npub fn ui() -> u64 {{ changed(1) }}\n\
         #[cfg(all(unix, not(windows)))]\npub fn host() -> u64 {{ changed(3) }}\n\
         #[cfg(not(test))]\npub fn prod() -> u64 {{ changed(4) }}\n"
    );
    let dir = repo(&before, &after);
    let out = grid(
        dir.path(),
        &["check", "--compile", "--", "--all-features", "--tests"],
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stdout(&out), "");
    let out = grid(
        dir.path(),
        &[
            "check",
            "--compile",
            "--json",
            "--",
            "--all-features",
            "--tests",
        ],
    );
    let report: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("--json");
    assert_eq!(report["uncompiled"], serde_json::json!([]));
    assert_eq!(report["unknown"], serde_json::json!([]));
}

/// A failing compile passes cargo's exit through and grid says nothing on
/// stdout: the compiler's diagnostics are the answer, and no record is
/// written for a compile that did not happen.
#[test]
fn a_failed_compile_passes_through_and_records_nothing() {
    if !cargo_present() {
        eprintln!("skipping: cargo is not on PATH");
        return;
    }
    let dir = repo(BEFORE_TEST, "pub fn changed(x: u64) -> u64 { x + }\n");
    let out = grid(dir.path(), &["check", "--compile"]);
    assert_eq!(code(&out), 101, "cargo's own exit code");
    assert_eq!(stdout(&out), "");
    assert!(!dir.path().join(".grid/compile.json").exists());
}

/// Python: there is no compile to wrap, and grid says so rather than
/// pretending one ran.
#[test]
fn a_python_tree_has_no_compile_to_wrap() {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "def a():\n    pass\n"),
        ],
    );
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "one"]);
    let out = grid(dir.path(), &["check", "--compile"]);
    assert_eq!(code(&out), 0);
    assert_eq!(stdout(&out), "");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no compile to wrap"),
        "said, not silent"
    );
    assert!(!dir.path().join(".grid/compile.json").exists());
}

const MANIFEST_DEFAULT_TUI: &str =
    "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
     [features]\ndefault = [\"tui\"]\ntui = []\n\n[workspace]\n";

/// Conservatism: a feature closure grid cannot read is a refusal, never a
/// shrunken closure. With `cargo metadata` failing, `default = ["tui"]` no
/// longer enables `tui` and every `tui` caller would be a false gate; the
/// verb refuses (exit 1), writes no record, and says why.
#[test]
fn an_unreadable_feature_closure_refuses_rather_than_gating() {
    if !cargo_present() {
        eprintln!("skipping: cargo is not on PATH");
        return;
    }
    let Ok(real_cargo) = std::env::var("CARGO") else {
        eprintln!("skipping: $CARGO is not set by the harness");
        return;
    };
    // `repo`, with a manifest whose default enables `tui` from the start.
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(
        dir.path(),
        &[
            ("Cargo.toml", MANIFEST_DEFAULT_TUI),
            ("src/lib.rs", BEFORE_FEATURE),
        ],
    );
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "before"]);
    write(dir.path(), &[("src/lib.rs", AFTER_FEATURE)]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "after"]);
    // The closure holds when cargo answers.
    let out = grid(dir.path(), &["check", "--compile"]);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));

    // A cargo whose `metadata` fails, everything else passed through.
    let shim = tempfile::tempdir().expect("tempdir");
    let script = shim.path().join("cargo");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = metadata ]; then exit 1; fi\nexec \"{real_cargo}\" \"$@\"\n"
        ),
    )
    .expect("write shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let _ = std::fs::remove_file(dir.path().join(".grid/compile.json"));
    let path = format!(
        "{}:{}",
        shim.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(["check", "--compile"])
        .current_dir(dir.path())
        .env("PATH", path)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("RUSTFLAGS")
        .output()
        .expect("run grid");
    assert_eq!(
        code(&out),
        1,
        "a closure grid cannot read is refused, never gated on:\n{}{}",
        stdout(&out),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout(&out), "", "and nothing is reported");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cargo metadata"),
        "the refusal says why:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !dir.path().join(".grid/compile.json").exists(),
        "no record was written"
    );
}
