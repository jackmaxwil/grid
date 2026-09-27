//! `grid plan new --from-diff <base>` end to end: a real repository, a real
//! edit, and the one property that makes the on-ramp an on-ramp — the draft
//! `grid plan new` writes is a plan `grid plan lint` accepts unchanged.
//!
//! Authoring a plan by hand meant writing callsign patterns and promise kinds
//! for a change already made. The diff knows all of it.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::path::Path;
use std::process::Command;

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

fn grid(root: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(root)
        .output()
        .expect("spawn grid");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(full, text).expect("write");
}

#[test]
fn a_draft_from_the_working_diff_lints_clean_and_names_what_changed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    write(root, "pkg/__init__.py", "");
    write(
        root,
        "pkg/core.py",
        "def a():\n    return 1\n\n\ndef b():\n    return 2\n",
    );
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);

    // A body edit, a signature edit, a new definition that calls one of them,
    // and a removal — every def promise kind in one change.
    write(
        root,
        "pkg/core.py",
        "def a():\n    return 7\n\n\ndef c(x):\n    return a() + x\n",
    );

    let plan = root.join("drafted.json");
    let (code, stdout, stderr) = grid(
        root,
        &[
            "plan",
            "new",
            "--from-diff",
            "HEAD",
            "--out",
            &plan.display().to_string(),
        ],
    );
    assert_eq!(code, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.starts_with("todo"), "{stdout}");

    let text = std::fs::read_to_string(&plan).expect("the draft was written");
    let drafted: serde_json::Value = serde_json::from_str(&text).expect("the draft is JSON");
    // Base is the snapshot the diff was taken from, pinned, not a rev label.
    let base = drafted["base"]["snapshot"]
        .as_str()
        .expect("base is a snapshot");
    assert!(base.starts_with("snap:b3:"), "{base}");

    let promised: Vec<(&str, &str)> = drafted["delta"]["defs"]
        .as_array()
        .expect("def promises")
        .iter()
        .map(|p| {
            (
                p["action"].as_str().expect("action"),
                p["callsign"].as_str().expect("callsign"),
            )
        })
        .collect();
    assert!(
        promised.contains(&("modify", "pkg.core.a")),
        "the body edit is promised: {promised:?}"
    );
    assert!(
        promised.contains(&("remove", "pkg.core.b")),
        "the removal is promised: {promised:?}"
    );
    assert!(
        promised
            .iter()
            .any(|(a, c)| *a == "add" && *c == "pkg.core.c"),
        "the new definition is promised: {promised:?}"
    );
    // `a`'s body moved and its signature did not, and the draft says so.
    let a = drafted["delta"]["defs"]
        .as_array()
        .expect("defs")
        .iter()
        .find(|p| p["callsign"] == "pkg.core.a")
        .expect("a is promised");
    assert_eq!(a["interface"], serde_json::json!(false));
    // An add carries no interface claim: `plan lint` refuses one there.
    let c = drafted["delta"]["defs"]
        .as_array()
        .expect("defs")
        .iter()
        .find(|p| p["callsign"] == "pkg.core.c")
        .expect("c is promised");
    assert!(c.get("interface").is_none(), "{c}");
    // The edge the new definition added, in callsign space.
    let edges: Vec<(&str, &str, &str)> = drafted["delta"]["edges"]
        .as_array()
        .expect("edge promises")
        .iter()
        .map(|p| {
            (
                p["action"].as_str().expect("action"),
                p["src"].as_str().expect("src"),
                p["dst"].as_str().expect("dst"),
            )
        })
        .collect();
    assert!(
        edges.contains(&("add", "pkg.core.c", "pkg.core.a")),
        "{edges:?}"
    );

    // The property the on-ramp rests on: unchanged, it lints clean.
    let (code, stdout, stderr) = grid(root, &["plan", "lint", &plan.display().to_string()]);
    assert_eq!(
        code, 0,
        "the draft is not gateable:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // And without `--out` the plan itself is stdout, byte for byte the same.
    let (code, piped, _) = grid(root, &["plan", "new", "--from-diff", "HEAD"]);
    assert_eq!(code, 0);
    let piped: serde_json::Value = serde_json::from_str(&piped).expect("stdout is the plan");
    assert_eq!(piped["delta"], drafted["delta"]);
}

/// A rev survey reads blobs through `git cat-file --batch`. Asked with `-z`,
/// which git before 2.43 rejects, the child exited on its first request and
/// the survey read "git cat-file: answers ran out" — every rev survey on
/// Ubuntu 22.04's git 2.34, CI's included. A `git` that refuses `-z` there
/// stands in for the old one.
#[cfg(unix)]
#[test]
fn a_rev_survey_works_with_a_git_that_has_no_batch_z() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).expect("mkdir");
    git(&root, &["init", "-q", "-b", "main"]);
    write(&root, "pkg/core.py", "def a():\n    return 1\n");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    write(&root, "pkg/core.py", "def a():\n    return 2\n");

    let real = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("find git");
    let real = String::from_utf8_lossy(&real.stdout).trim().to_owned();
    let shims = dir.path().join("shims");
    std::fs::create_dir_all(&shims).expect("mkdir");
    let shim = shims.join("git");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\ncase \" $* \" in *\" cat-file \"*\" -z \"*) echo \"error: unknown switch \\`z'\" >&2; exit 129;; esac\nexec '{real}' \"$@\"\n"
        ),
    )
    .expect("write shim");
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let path = format!(
        "{}:{}",
        shims.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(["plan", "new", "--from-diff", "HEAD"])
        .current_dir(&root)
        .env("PATH", path)
        .output()
        .expect("spawn grid");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
