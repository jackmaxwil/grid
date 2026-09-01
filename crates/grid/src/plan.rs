//! `grid plan` — the plan store and its verbs.
//!
//! A plan is a JSON file under `.grid/plans`, named by its id. It is the
//! promise half of the gate: authored by the planner, read by `gate.check`,
//! and never written by the engine (D6 — survey and `edit.apply` are the only
//! writers; a plan file is planner input, not engine state).

use std::path::{Path, PathBuf};

use grid_wire::Track;

use crate::{exit, hint, porcelain};

/// Where plans live, relative to the repo root.
const STORE: &str = ".grid/plans";

/// Load a plan from a path, or from the store by id.
///
/// A spec that looks like a path is read as one; anything else is resolved
/// against the store, with or without the id prefix.
///
/// # Errors
/// Returns a human-readable message when the plan cannot be read or parsed.
pub fn load(spec: &str) -> Result<Track, String> {
    let direct = PathBuf::from(spec);
    let path = if direct.is_file() {
        direct
    } else {
        let file = store_path(spec);
        if !file.is_file() {
            return Err(format!(
                "no plan `{spec}`: not a file, and not in {STORE}\nhint: `grid plan new {spec}` writes a starter plan"
            ));
        }
        file
    };
    parse(&path)
}

fn parse(path: &Path) -> Result<Track, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not a valid plan: {e}", path.display()))
}

fn store_path(spec: &str) -> PathBuf {
    let id = spec.strip_prefix("track:").unwrap_or(spec);
    Path::new(STORE).join(format!("{id}.json"))
}

/// `grid plan new|show|lint`.
pub fn main(args: &[&str], json: bool) -> u8 {
    match args.split_first() {
        Some((&"new", rest)) => new(rest),
        Some((&"show", rest)) => show(rest, json),
        Some((&"lint", rest)) => lint(rest, json),
        _ => {
            hint("usage: grid plan new <id> | grid plan show <id> | grid plan lint <id>");
            exit::BAD_INPUT
        }
    }
}

/// Write a starter plan: valid, empty, and lint-clean but for the warning
/// that it promises nothing — which is exactly the next thing to correct.
fn new(args: &[&str]) -> u8 {
    let Some(id) = args.first() else {
        hint("usage: grid plan new <id>");
        return exit::BAD_INPUT;
    };
    let track_id = if id.starts_with("track:") {
        (*id).to_string()
    } else {
        format!("track:{id}")
    };
    let path = store_path(&track_id);
    if path.exists() {
        hint(&format!(
            "grid: {} already exists\nhint: `grid plan show {track_id}` reads it",
            path.display()
        ));
        return exit::BAD_INPUT;
    }
    let track = Track {
        track_id: track_id.clone(),
        pr: None,
        plan_rev: None,
        base: grid_wire::SnapshotOrRev::GitRev {
            git_rev: grid_wire::GitRev("HEAD".into()),
        },
        delta: grid_wire::TrackDelta {
            defs: Vec::new(),
            edges: Vec::new(),
        },
        tests: None,
        invariants: None,
        tolerate: None,
    };
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            hint(&format!("grid: cannot create {}: {e}", dir.display()));
            return exit::INTERNAL;
        }
    }
    let Ok(body) = serde_json::to_string_pretty(&track) else {
        hint("grid: cannot serialize plan");
        return exit::INTERNAL;
    };
    if let Err(e) = std::fs::write(&path, body + "\n") {
        hint(&format!("grid: cannot write {}: {e}", path.display()));
        return exit::INTERNAL;
    }
    println!("todo     {track_id}  {}", path.display());
    hint("hint: add promises to delta.edges, then `grid plan lint` before you write code");
    exit::OK
}

fn show(args: &[&str], json: bool) -> u8 {
    let Some(spec) = args.first() else {
        hint("usage: grid plan show <id>");
        return exit::BAD_INPUT;
    };
    let track = match load(spec) {
        Ok(t) => t,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    if json {
        return crate::emit_json(&track);
    }
    print!("{}", porcelain::plan(&track));
    exit::OK
}

/// Lint exits 2 on errors — an unusable plan is bad input, not a divergence
/// verdict: no code has been written yet for reality to diverge from.
fn lint(args: &[&str], json: bool) -> u8 {
    let Some(spec) = args.first() else {
        hint("usage: grid plan lint <id>");
        return exit::BAD_INPUT;
    };
    let track = match load(spec) {
        Ok(t) => t,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    let result = match crate::rpc::track_lint(
        &grid_wire::methods::TrackLintParams { track },
        Path::new("."),
    ) {
        Ok(r) => r,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    let bad = !result.errors.is_empty();
    if json {
        let code = crate::emit_json(&result);
        return if code == exit::OK && bad {
            exit::BAD_INPUT
        } else {
            code
        };
    }
    print!("{}", porcelain::lint(&result));
    if bad {
        hint("grid: plan is not gateable until these are corrected");
        return exit::BAD_INPUT;
    }
    exit::OK
}
