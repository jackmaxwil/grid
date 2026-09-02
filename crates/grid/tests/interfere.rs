//! `grid interfere` end to end, over two real `git worktree` checkouts of one
//! fixture repo.
//!
//! The worktrees are real because the thing under test is exactly what two
//! parallel sessions do to each other: git merges text, and the finding this
//! verb exists for is one where the two sides share no file at all.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "grid")
        .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
        .env("GIT_COMMITTER_NAME", "grid")
        .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(full, text).expect("write");
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

const CALLER: &str = "crates/app/src/caller.rs";
const TARGET: &str = "crates/app/src/target.rs";

const BASE_TARGET: &str = "pub fn target(x: u32) -> u32 { x }\n";
const BASE_CALLER: &str = "use crate::target::target;\npub fn caller() -> u32 { target(1) }\n";

/// Two worktrees, `a` and `b`, branched off one committed base, each with its
/// own store. Returns their roots and the base rev.
///
/// The `mod` declarations live in `lib.rs` so the two files are one crate and
/// the call across them is a proven edge.
struct Pair {
    /// Held, not ignored: dropping it deletes the repository the two
    /// worktrees are checkouts of, so every test binds it.
    repo: tempfile::TempDir,
    a: PathBuf,
    b: PathBuf,
    base: String,
}

fn pair() -> Pair {
    let repo = tempfile::tempdir().expect("tempdir");
    let root = repo.path();
    git(root, &["init", "-q", "-b", "main"]);
    write(
        root,
        "crates/app/src/lib.rs",
        "pub mod caller;\npub mod target;\n",
    );
    write(root, TARGET, BASE_TARGET);
    write(root, CALLER, BASE_CALLER);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "base"]);
    let base = git(root, &["rev-parse", "--short", "HEAD"]);

    let (a, b) = (root.join("wt-a"), root.join("wt-b"));
    git(
        root,
        &["worktree", "add", "-q", "-b", "wt-a", "wt-a", "HEAD"],
    );
    git(
        root,
        &["worktree", "add", "-q", "-b", "wt-b", "wt-b", "HEAD"],
    );
    Pair { repo, a, b, base }
}

/// Commit one worktree's edits and chart both coordinates `grid interfere`
/// needs: the shared base, and this side's head.
///
/// Committing is what the two sides are about to do to each other, and it is
/// also what keeps the two snapshots distinguishable — a dirty survey is
/// labelled with the rev it was checked out from, so an uncommitted worktree
/// sitting on the base commit charts *as* the base.
///
/// Each side surveys its own store: `grid interfere` never surveys the other
/// root, which is the whole point of opening it read-only.
fn settle(dir: &Path, base: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--allow-empty", "-m", "edit"]);
    for args in [vec!["survey", base], vec!["survey"]] {
        let out = grid(dir, &args);
        assert_eq!(code(&out), 0, "{args:?}: {}", stderr(&out));
    }
}

/// The `(dependent, changed, reason)` of every `drift` record.
fn suspects(out: &Output) -> Vec<(String, String)> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&"drift"))
        .filter_map(|c| Some((c.get(2)?.to_string(), c.get(4)?.to_string())))
        .collect()
}

fn both_touched(out: &Output) -> Vec<String> {
    stdout(out)
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|c| c.first() == Some(&"warn") && c.get(2) == Some(&"both-touched"))
        .filter_map(|c| c.get(1).map(ToString::to_string))
        .collect()
}

/// Ground truth: A moves `target`'s signature, B gives `caller` a second call
/// to it. No file is in common, so git merges both cleanly — and `caller` is
/// left calling a signature that no longer exists.
#[test]
fn a_moved_signature_here_and_a_new_call_there_is_one_record() {
    let Pair { repo, a, b, base } = pair();
    let _repo = repo;
    write(
        &a,
        TARGET,
        "pub fn target(x: u32, y: u32) -> u32 { x + y }\n",
    );
    write(
        &b,
        CALLER,
        "use crate::target::target;\npub fn caller() -> u32 { target(1) + target(2) }\n",
    );
    settle(&a, &base);
    settle(&b, &base);

    let out = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(
        code(&out),
        3,
        "semantic overlap diverges:\n{}{}",
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(
        suspects(&out),
        vec![(
            "app.caller.caller".to_owned(),
            "app.target.target".to_owned()
        )],
        "one record, not a list:\n{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains("interfere:"),
        "and it names the worktree it came from:\n{}",
        stdout(&out)
    );

    // Property: the same pair from the other side is the same finding, up to
    // the mine/theirs labels.
    let mirrored = grid(&b, &["interfere", a.to_str().expect("path")]);
    assert_eq!(code(&mirrored), 3, "{}", stderr(&mirrored));
    assert_eq!(
        suspects(&mirrored),
        suspects(&out),
        "interference is symmetric in the two roots"
    );

    let mine: serde_json::Value = serde_json::from_str(&stdout(&grid(
        &a,
        &["interfere", b.to_str().expect("path"), "--json"],
    )))
    .expect("InterfereReport");
    let theirs: serde_json::Value = serde_json::from_str(&stdout(&grid(
        &b,
        &["interfere", a.to_str().expect("path"), "--json"],
    )))
    .expect("InterfereReport");
    assert_eq!(mine["base"], theirs["base"], "one base, seen from both");
    assert_eq!(
        mine["mine"], theirs["theirs"],
        "and the labels are the only thing that swaps"
    );
    assert_eq!(mine["theirs"], theirs["mine"]);
    assert_eq!(mine["both_touched"], theirs["both_touched"]);
}

/// Two worktrees editing definitions that have nothing to do with each other
/// produce no records and exit 0. "No interference" is a real answer, and
/// this is the only situation that earns it.
#[test]
fn edits_that_do_not_meet_produce_nothing() {
    let Pair { repo, a, b, base } = pair();
    let _repo = repo;
    write(&a, TARGET, "pub fn target(x: u32) -> u32 { x + 1 }\n");
    write(
        &b,
        "crates/app/src/apart.rs",
        "pub fn apart() -> u32 { 7 }\n",
    );
    write(
        &b,
        "crates/app/src/lib.rs",
        "pub mod apart;\npub mod caller;\npub mod target;\n",
    );
    settle(&a, &base);
    settle(&b, &base);

    let out = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(code(&out), 0, "{}{}", stdout(&out), stderr(&out));
    assert!(suspects(&out).is_empty(), "{}", stdout(&out));
    assert!(both_touched(&out).is_empty(), "{}", stdout(&out));
}

/// Both worktrees editing the same definition is listed for completeness —
/// git's merge finds this one too — and does not gate.
#[test]
fn a_definition_both_sides_moved_is_listed_and_does_not_gate() {
    let Pair { repo, a, b, base } = pair();
    let _repo = repo;
    write(&a, TARGET, "pub fn target(x: u32) -> u32 { x + 1 }\n");
    write(&b, TARGET, "pub fn target(x: u32) -> u32 { x + 2 }\n");
    settle(&a, &base);
    settle(&b, &base);

    let out = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(
        both_touched(&out),
        vec!["app.target.target".to_owned()],
        "{}",
        stdout(&out)
    );
    assert_eq!(
        code(&out),
        0,
        "a textual conflict git will also find does not gate here:\n{}",
        stdout(&out)
    );
}

/// Conservatism: a store grid cannot read is a refusal, never "no
/// interference". A superseded chart opens *empty*, which is exactly the
/// state that would otherwise be reported as a clean answer.
#[test]
fn a_superseded_store_on_the_other_side_refuses() {
    let Pair { repo, a, b, base } = pair();
    let _repo = repo;
    write(
        &a,
        TARGET,
        "pub fn target(x: u32, y: u32) -> u32 { x + y }\n",
    );
    write(
        &b,
        CALLER,
        "use crate::target::target;\npub fn caller() -> u32 { target(1) + target(2) }\n",
    );
    settle(&a, &base);
    settle(&b, &base);

    // The finding is there before the store is damaged.
    let before = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(code(&before), 3, "{}", stderr(&before));

    // A manifest from a past format epoch: the epoch is the four bytes after
    // the 17-byte magic (`grid-manifest/v1\n`), so writing 3 there is exactly
    // the store a previous release left behind. It opens empty and says so.
    let manifest = b.join(".grid/chart/manifest");
    let mut bytes = std::fs::read(&manifest).expect("read manifest");
    bytes[17..21].copy_from_slice(&3u32.to_le_bytes());
    std::fs::write(&manifest, bytes).expect("write manifest");

    let out = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(
        code(&out),
        2,
        "a chart grid could not read is bad input:\n{}{}",
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(
        stdout(&out),
        "",
        "and it writes no records at all: {:?}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("superseded"),
        "the refusal says why:\n{}",
        stderr(&out)
    );
}

/// A worktree grid has never surveyed refuses with the command that fixes it,
/// and grid does not create a store there on the way past.
#[test]
fn an_unsurveyed_other_root_refuses_without_writing_to_it() {
    let Pair { repo, a, b, base } = pair();
    let _repo = repo;
    write(&a, TARGET, "pub fn target(x: u32) -> u32 { x + 1 }\n");
    settle(&a, &base);

    let out = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(code(&out), 2, "{}{}", stdout(&out), stderr(&out));
    assert!(
        stderr(&out).contains("grid survey"),
        "the refusal names the command:\n{}",
        stderr(&out)
    );
    assert!(
        !b.join(".grid/chart").exists(),
        "and the other worktree is left exactly as it was"
    );
}

/// Conservatism, the plan's own use case 6: two dirty worktrees on the base
/// commit, each surveyed by the hook and nothing committed. A dirty survey is
/// labelled with the base rev but is not the base, so the other side refuses
/// with the commands that would chart it — never "no interference" — and
/// once they have run, the finding is the same one record. This side's base
/// is surveyed on demand, retained, and its worktree stays live.
#[test]
fn a_dirty_worktree_on_the_base_commit_is_never_taken_for_the_base() {
    let Pair { repo, a, b, base } = pair();
    let _repo = repo;
    write(
        &a,
        TARGET,
        "pub fn target(x: u32, y: u32) -> u32 { x + y }\n",
    );
    write(
        &b,
        CALLER,
        "use crate::target::target;\npub fn caller() -> u32 { target(1) + target(2) }\n",
    );
    for dir in [&a, &b] {
        let out = grid(dir, &["survey"]);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
    }

    let out = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(
        code(&out),
        2,
        "a store holding only a dirty survey at the base does not hold the base:\n{}{}",
        stdout(&out),
        stderr(&out)
    );
    assert_eq!(stdout(&out), "", "and no records: {:?}", stdout(&out));
    assert!(
        stderr(&out).contains(&format!("grid survey {base}")),
        "the refusal names the command:\n{}",
        stderr(&out)
    );

    // The other side charts the base, then its worktree again.
    for args in [vec!["survey", base.as_str()], vec!["survey"]] {
        let out = grid(&b, &args);
        assert_eq!(code(&out), 0, "{args:?}: {}", stderr(&out));
    }
    let live_before = stdout(&grid(&a, &["resolve", "app.target.target", "--json"]));
    let out = grid(&a, &["interfere", b.to_str().expect("path")]);
    assert_eq!(code(&out), 3, "{}{}", stdout(&out), stderr(&out));
    assert_eq!(
        suspects(&out),
        vec![(
            "app.caller.caller".to_owned(),
            "app.target.target".to_owned()
        )],
        "{}",
        stdout(&out)
    );
    let live_after = stdout(&grid(&a, &["resolve", "app.target.target", "--json"]));
    assert_eq!(
        live_before, live_after,
        "surveying the base on demand left this worktree live"
    );
}

/// T-10, conservatism: a refusal about the *other* worktree names the other
/// worktree. "Run `grid survey`" with no tree behind it is the version a
/// reader follows in the wrong directory — it succeeds there, changes
/// nothing about the store that refused, and the next run refuses again.
/// Every one of these refusals therefore carries the command and the path.
#[test]
fn a_refusal_about_the_other_worktree_names_that_worktree() {
    let Pair { repo, a, b, base } = pair();
    let _repo = repo;
    write(&a, TARGET, "pub fn target(x: u32) -> u32 { x + 1 }\n");
    settle(&a, &base);
    let theirs = b.to_str().expect("path");
    // The verb canonicalizes the other root before it opens anything, and the
    // hint names what it opened.
    let named = b.canonicalize().expect("canonicalize");
    let named = named.display().to_string();

    // Never surveyed there at all.
    let unsurveyed = grid(&a, &["interfere", theirs]);
    assert_eq!(code(&unsurveyed), 2, "{}", stderr(&unsurveyed));
    assert!(
        stderr(&unsurveyed).contains(&format!("hint: run `grid survey` in {named}")),
        "the hint names the other worktree, not this one:\n{}",
        stderr(&unsurveyed)
    );
    assert!(
        !stderr(&unsurveyed).contains("hint: run `grid survey` in .\n"),
        "and never this one:\n{}",
        stderr(&unsurveyed)
    );

    // Surveyed there, then superseded: the same fix, a different reason, and
    // the reason is said too.
    settle(&b, &base);
    let manifest = b.join(".grid/chart/manifest");
    let mut bytes = std::fs::read(&manifest).expect("read manifest");
    bytes[17..21].copy_from_slice(&3u32.to_le_bytes());
    std::fs::write(&manifest, bytes).expect("write manifest");
    let superseded = grid(&a, &["interfere", theirs]);
    assert_eq!(code(&superseded), 2, "{}", stderr(&superseded));
    assert!(
        stderr(&superseded).contains("superseded")
            && stderr(&superseded).contains(&format!("hint: run `grid survey` in {named}")),
        "a superseded store says why and where:\n{}",
        stderr(&superseded)
    );
}
