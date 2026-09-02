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
