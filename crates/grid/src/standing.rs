//! Standing invariants: the rules that hold for the whole repo, not for one
//! plan.
//!
//! `Invariant` is described in the wire contract as "a standing query checked
//! at every gate", and the `plan.lint` example expects a plan with no
//! invariants of its own to be judged against `boards-skip-persistence`. So
//! the repo owns a set, a plan may add to it, and both lint and gate see the
//! union.
//!
//! The file is `.grid/invariants.json`: a JSON array of `Invariant`. Absent or
//! unreadable means an empty set — a repo that has declared no rules is not an
//! error, and a broken file must not silently pass a gate, so it is reported
//! by the caller rather than swallowed here.

use std::path::Path;

use grid_wire::{Invariant, Plan};

/// Where standing invariants live, relative to the repo root.
pub const FILE: &str = ".grid/invariants.json";

/// Read the repo's standing invariants.
///
/// # Errors
/// Returns a human-readable message if the file exists but is not a valid
/// array of invariants.
pub fn load(root: &Path) -> Result<Vec<Invariant>, String> {
    let path = root.join(FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a valid invariant list: {e}", path.display()))
}

/// Write the invariant set back, pretty-printed so a human can edit it —
/// which is how one is removed, and how the two rule kinds `grid invariant`
/// does not write (`forbid-edge`, `require-edge`) are authored at all.
///
/// # Errors
/// A human-readable message when the file cannot be written.
pub fn save(root: &Path, invariants: &[Invariant]) -> Result<(), String> {
    let path = root.join(FILE);
    let Some(dir) = path.parent() else {
        return Err(format!("{} has no parent directory", path.display()));
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let body = serde_json::to_string_pretty(invariants)
        .map_err(|e| format!("cannot serialize invariants: {e}"))?;
    // Published by rename, never truncated in place: this file is a
    // declaration a human authored, and a crash between the truncate and the
    // write would leave the repo with no rules and every gate refusing.
    grid_chart_io::write_atomic(dir, &path, (body + "\n").as_bytes(), "invariants.json.tmp")
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// A plan with the repo's standing invariants folded into its own. Order is
/// standing-first so a plan's additions read as additions.
#[must_use]
pub fn fold(plan: &Plan, standing: Vec<Invariant>) -> Plan {
    if standing.is_empty() {
        return plan.clone();
    }
    let mut invariants = standing;
    invariants.extend(plan.invariants.iter().flatten().cloned());
    Plan {
        invariants: Some(invariants),
        ..plan.clone()
    }
}
