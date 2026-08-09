//! Standing invariants: the rules that hold for the whole repo, not for one
//! track.
//!
//! `Invariant` is described in the wire contract as "a standing query checked
//! at every gate", and the `track.lint` example expects a track with no
//! invariants of its own to be judged against `boards-skip-persistence`. So
//! the repo owns a set, a track may add to it, and both lint and gate see the
//! union.
//!
//! The file is `.grid/invariants.json`: a JSON array of `Invariant`. Absent or
//! unreadable means an empty set — a repo that has declared no rules is not an
//! error, and a broken file must not silently pass a gate, so it is reported
//! by the caller rather than swallowed here.

use std::path::Path;

use grid_wire::{Invariant, Track};

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

/// A track with the repo's standing invariants folded into its own. Order is
/// standing-first so a track's additions read as additions.
#[must_use]
pub fn fold(track: &Track, standing: Vec<Invariant>) -> Track {
    if standing.is_empty() {
        return track.clone();
    }
    let mut invariants = standing;
    invariants.extend(track.invariants.iter().flatten().cloned());
    Track {
        invariants: Some(invariants),
        ..track.clone()
    }
}
