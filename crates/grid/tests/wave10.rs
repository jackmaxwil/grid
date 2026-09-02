//! Wave 10 regression evidence: the fail-closed CLI boundary.
//!
//! This module exercises only refusal against a repo with no retained
//! history. The seeded-spectra ranking and seeded-chain bisect proofs live in
//! `#[cfg(test)]` modules inside `src/suspects.rs` and `src/bisect.rs`, since
//! `grid` is a bin-only crate; those fixtures are algorithm fixtures and are
//! not evidence of accumulated production CI history.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

fn grid(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(dir)
        .env("GRID_HINTS", "1")
        .output()
        .expect("run grid")
}

/// C23: absence of retained production spectra refuses rather than returning
/// an empty ranking that could be mistaken for evidence.
#[test]
fn suspects_refuses_without_retained_history() {
    let repo = tempfile::tempdir().expect("temp repo");
    let out = grid(repo.path(), &["suspects", "seeded-regression"]);

    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "refusal must not emit success records"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("insufficient history"),
        "stderr must explain the fail-closed boundary: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
