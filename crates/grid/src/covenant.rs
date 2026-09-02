//! Covenants: coupling a human declares because no language establishes it.
//!
//! The serde boundary, the JSON contract, the Python dict a Rust struct
//! deserializes — grid can prove none of these, and until now had nowhere to
//! write them down. A covenant is a symmetric pair of callsign patterns and
//! the boundary they meet across.
//!
//! Storage follows the standing-invariants precedent exactly
//! ([`crate::standing`]): `.grid/covenants.json`, a JSON array, absent means
//! an empty set, and a file that exists but does not parse is a loud error at
//! the caller — a broken declaration file must never be mistaken for an
//! absence of declarations.
//!
//! Covenants fire on **datum** change, not on `sig`: the coupling is
//! semantic, and a normalizer's *body* is the contract. A covenant suspect is
//! `Grade::Modeled` and it **gates**, which is the one exception to "modeled
//! never gates" — and it is exception-by-declaration, so the honesty rule is
//! holding rather than breaking. A human said these two move together.

use std::path::Path;

use grid_wire::{Covenant, DriftWarning};

/// Where covenants live, relative to the repo root. A declaration, not a
/// cache: it travels with the repo, like `.grid/invariants.json`.
pub const FILE: &str = ".grid/covenants.json";

/// Read the repo's covenants.
///
/// # Errors
/// A human-readable message when the file exists but is not a valid array of
/// covenants. Absent is an empty set, which is not an error.
pub fn load(root: &Path) -> Result<Vec<Covenant>, String> {
    let path = root.join(FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a valid covenant list: {e}", path.display()))
}

/// Write the covenant set back, pretty-printed so a human can edit it — which
/// is how one is removed, exactly as a plan is edited.
///
/// # Errors
/// A human-readable message when the file cannot be written.
pub fn save(root: &Path, covenants: &[Covenant]) -> Result<(), String> {
    let path = root.join(FILE);
    let Some(dir) = path.parent() else {
        return Err(format!("{} has no parent directory", path.display()));
    };
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let body = serde_json::to_string_pretty(covenants)
        .map_err(|e| format!("cannot serialize covenants: {e}"))?;
    // Published by rename, never truncated in place: this file is a
    // declaration a human authored, and a crash between the truncate and the
    // write would leave the repo with no rules and every gate refusing.
    grid_chart_io::write_atomic(dir, &path, (body + "\n").as_bytes(), "covenants.json.tmp")
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// One side of a covenant, resolved against a snapshot: the callsigns it
/// matches, split by whether each changed.
struct Side {
    matched: Vec<String>,
    changed: bool,
}

fn side<'a>(pattern: &str, callsigns: impl Iterator<Item = &'a str>, touched: &[&str]) -> Side {
    let matched: Vec<String> = callsigns
        .filter(|c| grid_gate::pattern::matches(pattern, c))
        .map(str::to_owned)
        .collect();
    let changed = matched.iter().any(|c| touched.contains(&c.as_str()));
    Side { matched, changed }
}

/// One covenant suspect: a definition that held while its declared partner
/// moved.
pub struct Fired {
    /// The callsign that did not change and is now suspect.
    pub held: String,
    /// A callsign on the other side that did change — the cause to report.
    pub moved: String,
    /// `covenant:<via>`, the reason the record carries.
    pub reason: String,
}

/// Fire every covenant against a delta.
///
/// For a covenant `(a, b)`: if any def matching `a` changed while every def
/// matching `b` held, every held `b` def is a suspect — and symmetrically.
/// Both sides changing is a covenant *kept*, which is the whole point of
/// declaring it, so it fires nothing.
///
/// The join runs over `base` — the snapshot the coupling described — while
/// the *vacuity* check runs over `head`, which is drift time: a pattern that
/// matches nothing now is a declaration that has stopped being checked, and
/// a rule that quietly stops being checked is worse than no rule. It is
/// returned as a warning, never as silent satisfaction.
#[must_use]
pub fn fire(
    covenants: &[Covenant],
    base: &[&str],
    head: &[&str],
    touched: &[&str],
) -> (Vec<Fired>, Vec<DriftWarning>) {
    let mut fired = Vec::new();
    let mut warnings = Vec::new();
    for covenant in covenants {
        let a = side(&covenant.a.0, base.iter().copied(), touched);
        let b = side(&covenant.b.0, base.iter().copied(), touched);
        for pattern in [&covenant.a.0, &covenant.b.0] {
            if !head.iter().any(|c| grid_gate::pattern::matches(pattern, c)) {
                warnings.push(DriftWarning {
                    subject: pattern.clone(),
                    message: format!(
                        "covenant:{} matches nothing — the declaration is not being checked",
                        covenant.via
                    ),
                });
            }
        }
        let reason = format!("covenant:{}", covenant.via);
        // Symmetric by construction: the same join, run both ways.
        for (moved, held) in [(&a, &b), (&b, &a)] {
            if !moved.changed || held.changed {
                continue;
            }
            let Some(cause) = moved
                .matched
                .iter()
                .find(|c| touched.contains(&c.as_str()))
                .cloned()
            else {
                continue;
            };
            for name in &held.matched {
                fired.push(Fired {
                    held: name.clone(),
                    moved: cause.clone(),
                    reason: reason.clone(),
                });
            }
        }
    }
    // Deterministic: the file's order is an author's choice, not an answer.
    fired.sort_by(|x, y| (&x.held, &x.moved).cmp(&(&y.held, &y.moved)));
    fired.dedup_by(|x, y| x.held == y.held && x.moved == y.moved);
    warnings.sort_by(|x, y| (&x.subject, &x.message).cmp(&(&y.subject, &y.message)));
    warnings.dedup_by(|x, y| x.subject == y.subject && x.message == y.message);
    (fired, warnings)
}
