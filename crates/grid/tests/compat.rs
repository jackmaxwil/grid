//! `grid drift --compat` end to end: the Deserialize boundary the compiler
//! cannot see, because the bytes on the other side of it are old.
//!
//! Each ground-truth fixture is the smallest struct or enum that trips
//! exactly one rule, surveyed by the real Rust frontend at both ends. None of
//! them has a caller, so drift itself finds no suspects and the exit code
//! under test is the compat pass's alone.
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

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(full, text).expect("write");
}

const FILE: &str = "crates/app/src/lib.rs";

/// A repo whose one file held `before` at `HEAD~1` and holds `after` in the
/// worktree now — which is exactly the pair `grid drift` compares by default.
fn repo(before: &str, after: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(dir.path(), &["init", "-q", "-b", "main"]);
    write(dir.path(), FILE, before);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "before"]);
    write(dir.path(), FILE, after);
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &["commit", "-q", "--allow-empty", "-m", "after"],
    );
    dir
}

fn grid(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run grid")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("stderr is utf-8")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// The `(callsign, rule)` of every undeclared compat finding.
fn warnings(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("warn")?.trim_start();
            let (subject, message) = rest.split_once("  ")?;
            let rule = message.trim().strip_prefix("compat: ")?;
            Some((subject.trim().to_owned(), rule.to_owned()))
        })
        .collect()
}

/// One fixture, run: the compat findings and the exit code.
fn findings(before: &str, after: &str) -> (Vec<(String, String)>, i32, String) {
    let dir = repo(before, after);
    let out = grid(dir.path(), &["drift", "--compat"]);
    (warnings(&out), code(&out), stdout(&out))
}

fn assert_trips(rule: &str, before: &str, after: &str) {
    let (found, exit, raw) = findings(before, after);
    assert_eq!(
        found,
        vec![("app.C".to_owned(), rule.to_owned())],
        "exactly one rule, and it is `{rule}`:\n{raw}"
    );
    assert_eq!(
        exit, 0,
        "an undeclared finding is heuristic and never moves the exit code:\n{raw}"
    );
}

#[test]
fn rule_one_a_required_field_added() {
    assert_trips(
        "required field added",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n    pub b: u32,\n}\n",
    );
}

#[test]
fn rule_two_a_field_renamed_without_an_alias() {
    assert_trips(
        "field renamed without alias",
        "#[derive(Deserialize)]\npub struct C {\n    pub old: String,\n}\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub new: String,\n}\n",
    );
}

#[test]
fn rule_three_a_field_type_changed() {
    assert_trips(
        "type changed",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u64,\n}\n",
    );
}

#[test]
fn rule_four_a_variant_removed() {
    let (found, exit, raw) = findings(
        "#[derive(Deserialize)]\npub enum C {\n    A,\n    B,\n}\n",
        "#[derive(Deserialize)]\npub enum C {\n    A,\n}\n",
    );
    assert_eq!(
        found,
        vec![("app.C".to_owned(), "variant removed".to_owned())],
        "{raw}"
    );
    assert_eq!(exit, 0, "{raw}");
}

#[test]
fn rule_five_a_field_removed_under_deny_unknown_fields() {
    assert_trips(
        "field removed under deny_unknown_fields",
        "#[derive(Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct C {\n    pub a: u32,\n    pub b: String,\n}\n",
        "#[derive(Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct C {\n    pub a: u32,\n}\n",
    );
}

#[test]
fn rule_six_the_representation_changed() {
    assert_trips(
        "representation changed",
        "#[derive(Deserialize)]\n#[serde(rename_all = \"snake_case\")]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Deserialize)]\n#[serde(rename_all = \"camelCase\")]\npub struct C {\n    pub a: u32,\n}\n",
    );
}

/// Property: nothing is reported for a type whose interface held — the
/// changed set is definitions a caller can see move, and the whole check
/// hangs off it. Adding a derive moves the datum and holds the sig.
#[test]
fn a_type_whose_sig_held_produces_nothing() {
    let (found, exit, raw) = findings(
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Debug, Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
    );
    assert!(found.is_empty(), "the interface never moved:\n{raw}");
    assert_eq!(exit, 0, "{raw}");
}

/// Property: `#[serde(default)]` is exactly what makes a new field readable
/// from old bytes, so adding it silences the first rule and nothing else.
#[test]
fn a_serde_default_silences_the_required_field_rule() {
    let base = "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n";
    let (tripped, _, _) = findings(
        base,
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n    pub b: u32,\n}\n",
    );
    assert_eq!(tripped.len(), 1, "{tripped:?}");

    let (silenced, exit, raw) = findings(
        base,
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n    #[serde(default)]\n    pub b: u32,\n}\n",
    );
    assert!(
        silenced.is_empty(),
        "a defaulted field is readable from bytes that lack it:\n{raw}"
    );
    assert_eq!(exit, 0, "{raw}");
}

/// A `persisted` invariant is the human fact that old bytes exist. It
/// promotes the same finding to modeled evidence: an `inv violated` record
/// and exit 3.
#[test]
fn a_persisted_declaration_turns_the_same_finding_into_a_gate() {
    let dir = repo(
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n    pub retries: u32,\n}\n",
    );
    write(
        dir.path(),
        ".grid/invariants.json",
        "[{\"id\": \"config-on-disk\", \"rule\": {\"persisted\": \"app.C\"}}]",
    );

    let out = grid(dir.path(), &["drift", "--compat"]);
    let text = stdout(&out);
    assert_eq!(
        code(&out),
        3,
        "a declared boundary gates at modeled:\n{text}{}",
        stderr(&out)
    );
    assert!(
        text.lines().any(|line| {
            line.starts_with("inv")
                && line.contains("violated")
                && line.contains("persisted:app.C")
                && line.contains("required field added")
        }),
        "the record names the declaration and the rule:\n{text}"
    );
    assert!(
        warnings(&out).is_empty(),
        "a declared finding is not also a warning: {:?}",
        warnings(&out)
    );

    // The new rule shape does not disturb the verbs that read the same file
    // for edges: `grid scope` filters standing invariants by the callsigns in
    // scope, and a `persisted` rule names one pattern rather than two.
    let scoped = grid(dir.path(), &["scope", "app.C", "--json"]);
    assert_eq!(code(&scoped), 0, "{}", stderr(&scoped));
    let pack: serde_json::Value = serde_json::from_str(&stdout(&scoped)).expect("ScopePack");
    assert_eq!(
        pack["invariants"][0]["id"], "config-on-disk",
        "a persisted rule is in scope for the type it names: {pack}"
    );

    // And the same tree with no declaration is the same finding, ranked.
    std::fs::remove_file(dir.path().join(".grid/invariants.json")).expect("rm");
    let undeclared = grid(dir.path(), &["drift", "--compat"]);
    assert_eq!(code(&undeclared), 0, "{}", stdout(&undeclared));
    assert_eq!(warnings(&undeclared).len(), 1);
}

/// Conservatism: a derive the source does not spell out is skipped and
/// counted. "I cannot see the derive" and "these bytes still read" are
/// opposite answers, and only one of them is available here.
#[test]
fn a_derive_behind_a_cfg_is_counted_never_cleared() {
    let dir = repo(
        "#[cfg_attr(feature = \"serde\", derive(Deserialize))]\npub struct C {\n    pub a: u32,\n}\n",
        "#[cfg_attr(feature = \"serde\", derive(Deserialize))]\npub struct C {\n    pub a: u32,\n    pub b: u32,\n}\n",
    );
    let out = grid(dir.path(), &["drift", "--compat"]);
    let text = stdout(&out);
    assert_eq!(code(&out), 0, "{text}{}", stderr(&out));
    assert!(
        warnings(&out).is_empty(),
        "an unreadable derive yields no rule verdict: {:?}",
        warnings(&out)
    );
    assert!(
        text.lines().any(|line| line.starts_with("miss")
            && line.contains("compat unchecked (derive not visible)")),
        "and it is counted, not silently cleared:\n{text}"
    );

    let report: serde_json::Value =
        serde_json::from_str(&stdout(&grid(dir.path(), &["drift", "--compat", "--json"])))
            .expect("DriftReport");
    assert_eq!(report["unchecked"]["compat_unchecked"], 1, "{report}");
    assert_eq!(
        report["compat"].as_array().map(Vec::len),
        Some(0),
        "the pass ran and found nothing it could decide: {report}"
    );
}

/// Without `--compat` the report is byte-identical to what it was before the
/// check existed: no findings array, no count, no records.
#[test]
fn the_check_is_silent_unless_it_is_asked_for() {
    let dir = repo(
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n    pub b: u32,\n}\n",
    );
    let out = grid(dir.path(), &["drift", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("DriftReport");
    assert!(report.get("compat").is_none(), "{report}");
    assert!(
        report["unchecked"].get("compat_unchecked").is_none(),
        "{report}"
    );
    assert!(warnings(&grid(dir.path(), &["drift"])).is_empty());
}

/// The `inv` records a listing prints: `(status, id)`.
fn invs(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .filter(|l| l.starts_with("inv"))
        .filter_map(|l| {
            let mut c = l.split_whitespace();
            c.next()?;
            Some((c.next()?.to_owned(), c.next()?.to_owned()))
        })
        .collect()
}

/// T-12, ground truth: `grid invariant persisted <pattern>` declares the
/// human fact by hand no longer. The rule it writes is the rule the gate
/// reads, and `--list` says what the repo has declared.
#[test]
fn the_invariant_verb_declares_a_persisted_rule_the_gate_then_fires_on() {
    let dir = repo(
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n    pub retries: u32,\n}\n",
    );
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);

    // Undeclared, the same change ranks and does not gate.
    let before = grid(dir.path(), &["drift", "--compat"]);
    assert_eq!(code(&before), 0, "{}", stdout(&before));

    let declared = grid(
        dir.path(),
        &["invariant", "persisted", "app.C", "--note", "on disk"],
    );
    assert_eq!(code(&declared), 0, "{}", stderr(&declared));
    assert_eq!(
        invs(&declared),
        [("standing".to_owned(), "persisted:app.C".to_owned())],
        "the write echoes the rule set: {}",
        stdout(&declared)
    );
    assert_eq!(
        invs(&grid(dir.path(), &["invariant", "--list"])),
        invs(&declared),
        "--list is the same set"
    );

    // The gate reads it: the same finding is now modeled evidence, exit 3.
    let after = grid(dir.path(), &["drift", "--compat"]);
    assert_eq!(
        code(&after),
        3,
        "a declared boundary gates:\n{}{}",
        stdout(&after),
        stderr(&after)
    );
    assert!(
        stdout(&after).contains("persisted:app.C") && stdout(&after).contains("required field"),
        "the record names the declaration this verb wrote:\n{}",
        stdout(&after)
    );

    // Declaring the same pattern twice is one rule, not two.
    assert_eq!(
        code(&grid(dir.path(), &["invariant", "persisted", "app.C"])),
        0
    );
    assert_eq!(invs(&grid(dir.path(), &["invariant", "--list"])).len(), 1);

    // The note rides as the rule's description, and `--json` is the record
    // form.
    let listed: Vec<serde_json::Value> = serde_json::from_str(&stdout(&grid(
        dir.path(),
        &["invariant", "--list", "--json"],
    )))
    .expect("--json is an Invariant list");
    assert_eq!(listed[0]["rule"]["persisted"], "app.C", "{listed:?}");
}

/// T-12, conservatism: a pattern that matches nothing is refused and nothing
/// is written. A declared rule that can never fire reads as protection and
/// is not — it is the one state worse than no rule, because a reader sees a
/// declaration and stops looking. The hand-authored rule kinds this verb
/// does not write survive it untouched.
#[test]
fn an_invariant_over_nothing_is_refused_and_hand_authored_rules_survive() {
    let dir = repo(
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n    pub retries: u32,\n}\n",
    );
    // A rule kind `grid invariant` never writes, authored by hand.
    let hand = "[{\"id\": \"no-app-to-db\", \"rule\": {\"forbid_edge\": \
                {\"src\": \"app.**\", \"dst\": \"db.**\"}}}]";
    write(dir.path(), ".grid/invariants.json", hand);
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);

    let out = grid(dir.path(), &["invariant", "persisted", "app.NoSuchType"]);
    assert_eq!(
        code(&out),
        2,
        "a rule about code that does not exist is bad input: {}",
        stdout(&out)
    );
    assert!(
        stdout(&out).is_empty(),
        "a refusal writes no records: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("nothing is called `app.NoSuchType`"),
        "and says so: {}",
        stderr(&out)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".grid/invariants.json")).expect("read"),
        hand,
        "nothing was written, and the hand-authored rule is byte-identical"
    );

    // A rule this verb *does* write leaves the hand-authored one alone.
    assert_eq!(
        code(&grid(dir.path(), &["invariant", "persisted", "app.C"])),
        0
    );
    let ids: Vec<String> = invs(&grid(dir.path(), &["invariant", "--list"]))
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    assert_eq!(ids, ["no-app-to-db", "persisted:app.C"], "both rules stand");
}

/// T-12, conservatism: a repository declaration file is *published*, never
/// truncated in place.
///
/// `.grid/invariants.json` and `.grid/covenants.json` are not engine state —
/// they hold hand-authored rules the next survey cannot rebuild. Written with
/// a plain `fs::write`, the file is truncated before it is filled, so a
/// crash, a signal or ENOSPC in that window leaves it empty or half-written;
/// `load` then returns `Err` and `gate`, `todo`, `check --plan` and `drift
/// --compat` all refuse. `chart-io`'s `write_atomic` closes the window, and
/// the observable difference is the inode: a file published by rename is a
/// new one, a file truncated in place is the same one. So a reader holding
/// the old file always sees complete old content, and no reader ever sees a
/// half-written declaration.
#[cfg(unix)]
#[test]
fn a_declaration_file_is_published_by_rename_never_truncated_in_place() {
    use std::os::unix::fs::MetadataExt as _;

    let dir = repo(
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n\
         pub fn one() -> u32 { 1 }\npub fn two() -> u32 { 2 }\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n\
         pub fn one() -> u32 { 1 }\npub fn two() -> u32 { 2 }\n",
    );
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);

    let inode = |name: &str| {
        std::fs::metadata(dir.path().join(name))
            .expect("the declaration exists")
            .ino()
    };
    for (file, args) in [
        (
            ".grid/invariants.json",
            vec!["invariant", "persisted", "app.C"],
        ),
        (
            ".grid/covenants.json",
            vec!["covenant", "app.one", "app.two", "--via", "schema"],
        ),
    ] {
        let first = grid(dir.path(), &args);
        assert_eq!(code(&first), 0, "{}", stderr(&first));
        let before = inode(file);
        // A second, different declaration into the same file: the rewrite
        // path, which is the one that truncates.
        let mut second = args.clone();
        if second[0] == "invariant" {
            second[2] = "app.one";
        } else {
            second[1] = "app.two";
            second[2] = "app.one";
        }
        let out = grid(dir.path(), &second);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        assert_ne!(
            inode(file),
            before,
            "{file} was rewritten in place — the truncate window is open"
        );
        // And no temp file is left beside it.
        let strays: Vec<String> = std::fs::read_dir(dir.path().join(".grid"))
            .expect("read .grid")
            .filter_map(|e| {
                let name = e.ok()?.file_name();
                (Path::new(&name).extension()? == "tmp")
                    .then(|| name.to_string_lossy().into_owned())
            })
            .collect();
        assert_eq!(strays, Vec::<String>::new(), "the publish left {strays:?}");
    }
}

/// T-10 + T-12, conservatism: on a store nobody has surveyed, a declaration
/// verb names the survey rather than reporting the code absent.
///
/// `rpc::matching` counts matches over the live epoch, and an unsurveyed
/// store holds none — so every pattern matched zero and was refused as
/// "nothing is called `<x>`", which is silence read as "does not exist", the
/// one conclusion grid never licenses. It is also the only missing-snapshot
/// refusal on the branch that carried no `Recovery` line, and the recovery
/// here is exactly `grid survey`. A *stale* store is the same defect: a rule
/// declared for a type added since the last survey was refused as
/// nonexistent.
#[test]
fn a_declaration_on_an_unsurveyed_store_names_the_survey() {
    let dir = repo(
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n\
         pub fn one() -> u32 { 1 }\npub fn two() -> u32 { 2 }\n",
        "#[derive(Deserialize)]\npub struct C {\n    pub a: u32,\n}\n\
         pub fn one() -> u32 { 1 }\npub fn two() -> u32 { 2 }\n",
    );
    // Committed, never surveyed. Both verbs share `matching`, so both must
    // answer the same way.
    for args in [
        vec!["invariant", "persisted", "app.C"],
        vec!["covenant", "app.one", "app.two", "--via", "schema"],
    ] {
        let out = grid(dir.path(), &args);
        assert_eq!(code(&out), 2, "{args:?}: {}", stdout(&out));
        assert!(
            !stderr(&out).contains("nothing is called"),
            "{args:?}: an empty store is not an empty repository: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("no snapshot published"),
            "{args:?}: it says what is actually missing: {}",
            stderr(&out)
        );
        assert!(
            stderr(&out).contains("hint: run `grid survey` in"),
            "{args:?}: and names the command and the tree, as T-10 requires: {}",
            stderr(&out)
        );
        assert!(
            !dir.path().join(".grid/invariants.json").exists()
                && !dir.path().join(".grid/covenants.json").exists(),
            "{args:?}: a refused declaration is not written"
        );
    }

    // And once surveyed, the same call answers.
    assert_eq!(code(&grid(dir.path(), &["survey"])), 0);
    let out = grid(dir.path(), &["invariant", "persisted", "app.C"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}
