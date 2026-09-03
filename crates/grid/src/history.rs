//! The patch log: every edit the repo has sailed, and the test outcomes
//! recorded against datums along the way.
//!
//! Two append-only JSONL files under `.grid/`. Appends are a single
//! `write` + fsync of one line; a torn tail (power loss mid-append) is
//! tolerated by skipping any line that does not parse — the log loses at
//! most the append that was in flight, never what preceded it. Wire `Edit`
//! rows, so the log is schema-stable by the same additive rules as the
//! socket.
//!
//! `grid.anchor.carry` answers from here too: carrying an anchor onto a
//! newer chart *is* following this log, so the method sits beside the
//! chain walk it is built on rather than in a module of its own.
use std::io::Write as _;
use std::path::{Path, PathBuf};

use grid_wire::Edit;

fn log_path(root: &Path) -> PathBuf {
    root.join(".grid/log")
}

fn outcomes_path(root: &Path) -> PathBuf {
    root.join(".grid/outcomes")
}

/// One recorded test outcome against one datum version.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub datum: String,
    pub test: String,
    pub fails: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

fn append_lines<T: serde::Serialize>(path: &Path, rows: &[T]) -> std::io::Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = Vec::new();
    for row in rows {
        let line = serde_json::to_string(row).map_err(std::io::Error::other)?;
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&buf)?;
    file.sync_all()
}

fn read_lines<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    // A line that does not parse is a torn append or foreign junk: skipped,
    // and everything before it still counts.
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Record edits, in the order they happened.
///
/// # Errors
/// The underlying I/O error, verbatim.
pub fn append(root: &Path, edits: &[Edit]) -> std::io::Result<()> {
    append_lines(&log_path(root), edits)
}

/// Every recorded edit, oldest first.
#[must_use]
pub fn read(root: &Path) -> Vec<Edit> {
    read_lines(&log_path(root))
}

/// Record test outcomes against datum versions.
///
/// # Errors
/// The underlying I/O error, verbatim.
pub fn append_outcomes(root: &Path, rows: &[Outcome]) -> std::io::Result<()> {
    append_lines(&outcomes_path(root), rows)
}

/// Every recorded outcome, oldest first.
#[must_use]
pub fn outcomes(root: &Path) -> Vec<Outcome> {
    read_lines(&outcomes_path(root))
}

/// The edits that touch `datum` — as either end — newest first: a datum's
/// history. The chain follows identity, so it survives renames for free.
#[must_use]
pub fn chain(root: &Path, datum: &str) -> Vec<Edit> {
    let mut lineage: std::collections::BTreeSet<String> = [datum.to_owned()].into();
    let all = read(root);
    // Two passes: walking backwards, a edit into the lineage adds its `from`;
    // repeated until closed so an A->B->C chain is found from C.
    loop {
        let mut grew = false;
        for edit in &all {
            if lineage.contains(&edit.to.0) && lineage.insert(edit.from.0.clone()) {
                grew = true;
            }
            if lineage.contains(&edit.from.0) && lineage.insert(edit.to.0.clone()) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let mut hits: Vec<Edit> = all
        .into_iter()
        .filter(|edit| lineage.contains(&edit.from.0) || lineage.contains(&edit.to.0))
        .collect();
    hits.reverse();
    hits
}

/// `grid.anchor.carry` — follow a consumer's anchors onto the live chart.
///
/// Grid stores no anchors: they arrive as parameters, owned by whoever made
/// them (a review annotation, a spec reference), and this answers where each
/// one now points. That is why the method needs no anchor store to exist and
/// why implementing it closes C10's gap rather than adding a feature.
///
/// Three answers, and the confidence says which of them grid actually knows:
///
/// - `exact` — the chart still holds the anchor's own datum, so nothing moved
///   under it.
/// - `carried` — the datum is gone and the recorded patch DAG
///   ([`crate::history::chain`]) reaches a successor the chart does hold. An
///   edit edge is a fact grid wrote down, not a guess, so identity carries at
///   confidence 1. An anchor with an interior [`grid_wire::AstPath`] is the
///   exception: the body changed, grid does not re-verify that the path still
///   addresses the same node, and it says so with confidence 0.5 rather than
///   claiming a precision it did not check.
/// - `orphaned` — no successor in the chart. `position` is absent, because a
///   position that points nowhere is worse than none.
///
/// # Errors
/// [`crate::chart_provider::Error::NotLive`] when `to` is not the epoch the engine
/// holds — the same refusal every other verb makes about a snapshot it cannot
/// open — and [`crate::chart_provider::Error::NoSnapshot`] when nothing is published.
pub fn carry(
    params: &grid_wire::methods::AnchorCarryParams,
    root: &std::path::Path,
) -> Result<grid_wire::methods::AnchorCarryResult, crate::chart_provider::Error> {
    let chart = crate::chart_provider::ChartDelta::live(root)?;
    let live = chart.current().ok_or_else(|| chart.no_snapshot())?;
    if live != params.to {
        return Err(crate::chart_provider::Error::NotLive(params.to.0.clone()));
    }

    let held = |datum: &grid_wire::Datum| {
        !chart
            .resolve(&grid_wire::Locator::Datum {
                datum: datum.clone(),
            })
            .is_empty()
    };

    let mut out = Vec::with_capacity(params.anchors.len());
    for anchor in &params.anchors {
        let start = &anchor.position.datum;
        let interior = anchor.position.path.is_some();
        if held(start) {
            out.push(grid_wire::CarriedAnchor {
                anchor_id: anchor.anchor_id.clone(),
                status: grid_wire::CarriedAnchorStatus::Exact,
                position: Some(anchor.position.clone()),
                confidence: 1.0,
                via: Vec::new(),
            });
            continue;
        }
        let (position, via) = follow(root, start, &held);
        out.push(match position {
            Some(datum) => grid_wire::CarriedAnchor {
                anchor_id: anchor.anchor_id.clone(),
                status: grid_wire::CarriedAnchorStatus::Carried,
                position: Some(grid_wire::Position {
                    datum,
                    path: anchor.position.path.clone(),
                }),
                // Invariant: identity carried through recorded edits is a
                // fact; an interior path across a changed body is not.
                confidence: if interior { 0.5 } else { 1.0 },
                via,
            },
            None => grid_wire::CarriedAnchor {
                anchor_id: anchor.anchor_id.clone(),
                status: grid_wire::CarriedAnchorStatus::Orphaned,
                position: None,
                confidence: 0.0,
                via: Vec::new(),
            },
        });
    }
    Ok(out)
}

/// Walk the recorded patch DAG forward from `start` to the first datum the
/// chart still holds, returning it with the edits crossed, in order.
///
/// Bounded by the number of edits in the lineage, and every datum is visited
/// at most once, so a cycle in a hand-edited log terminates rather than
/// spinning.
fn follow(
    root: &std::path::Path,
    start: &grid_wire::Datum,
    held: &dyn Fn(&grid_wire::Datum) -> bool,
) -> (Option<grid_wire::Datum>, Vec<grid_wire::Edit>) {
    let lineage = crate::history::chain(root, &start.0);
    let mut seen: std::collections::BTreeSet<String> = [start.0.clone()].into();
    let mut cursor = start.clone();
    let mut via = Vec::new();
    loop {
        let Some(step) = lineage
            .iter()
            .find(|edit| edit.from == cursor && seen.insert(edit.to.0.clone()))
        else {
            return (None, Vec::new());
        };
        via.push(step.clone());
        cursor = step.to.clone();
        if held(&cursor) {
            return (Some(cursor), via);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_torn_tail_loses_only_the_append_in_flight() {
        let dir = tempfile::tempdir().expect("tempdir");
        let edit = |from: &str, to: &str| Edit {
            from: grid_wire::Datum(from.into()),
            to: grid_wire::Datum(to.into()),
            git_rev: None,
            at: None,
        };
        append(
            dir.path(),
            &[edit("b3:aa", "b3:bb"), edit("b3:bb", "b3:cc")],
        )
        .expect("append");
        // Tear the tail mid-line, as a crash during the third append would.
        let path = log_path(dir.path());
        let mut bytes = std::fs::read(&path).expect("read");
        bytes.extend_from_slice(b"{\"from\":\"b3:cc\",\"to\":");
        std::fs::write(&path, bytes).expect("write");
        let edits = read(dir.path());
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[1].to.0, "b3:cc");
    }

    #[test]
    fn history_chains_identity_across_legs_newest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let edit = |from: &str, to: &str| Edit {
            from: grid_wire::Datum(from.into()),
            to: grid_wire::Datum(to.into()),
            git_rev: None,
            at: None,
        };
        append(
            dir.path(),
            &[
                edit("b3:aa", "b3:bb"),
                edit("b3:zz", "b3:yy"), // an unrelated def's edit
                edit("b3:bb", "b3:cc"),
            ],
        )
        .expect("append");
        let hits = chain(dir.path(), "b3:cc");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].to.0, "b3:cc");
        assert_eq!(hits[1].to.0, "b3:bb");
    }
}
