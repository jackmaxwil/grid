//! The patch log: every leg the repo has sailed, and the test outcomes
//! recorded against datums along the way.
//!
//! Two append-only JSONL files under `.grid/`. Appends are a single
//! `write` + fsync of one line; a torn tail (power loss mid-append) is
//! tolerated by skipping any line that does not parse — the log loses at
//! most the append that was in flight, never what preceded it. Wire `Leg`
//! rows, so the log is schema-stable by the same additive rules as the
//! socket.
use std::io::Write as _;
use std::path::{Path, PathBuf};

use grid_wire::Leg;

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

/// Record legs, in the order they happened.
///
/// # Errors
/// The underlying I/O error, verbatim.
pub fn append(root: &Path, legs: &[Leg]) -> std::io::Result<()> {
    append_lines(&log_path(root), legs)
}

/// Every recorded leg, oldest first.
#[must_use]
pub fn read(root: &Path) -> Vec<Leg> {
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

/// The legs that touch `datum` — as either end — newest first: a datum's
/// history. The chain follows identity, so it survives renames for free.
#[must_use]
pub fn history(root: &Path, datum: &str) -> Vec<Leg> {
    let mut lineage: std::collections::BTreeSet<String> = [datum.to_owned()].into();
    let all = read(root);
    // Two passes: walking backwards, a leg into the lineage adds its `from`;
    // repeated until closed so an A->B->C chain is found from C.
    loop {
        let mut grew = false;
        for leg in &all {
            if lineage.contains(&leg.to.0) && lineage.insert(leg.from.0.clone()) {
                grew = true;
            }
            if lineage.contains(&leg.from.0) && lineage.insert(leg.to.0.clone()) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let mut hits: Vec<Leg> = all
        .into_iter()
        .filter(|leg| lineage.contains(&leg.from.0) || lineage.contains(&leg.to.0))
        .collect();
    hits.reverse();
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_torn_tail_loses_only_the_append_in_flight() {
        let dir = tempfile::tempdir().expect("tempdir");
        let leg = |from: &str, to: &str| Leg {
            from: grid_wire::Datum(from.into()),
            to: grid_wire::Datum(to.into()),
            git_rev: None,
            at: None,
        };
        append(dir.path(), &[leg("b3:aa", "b3:bb"), leg("b3:bb", "b3:cc")]).expect("append");
        // Tear the tail mid-line, as a crash during the third append would.
        let path = log_path(dir.path());
        let mut bytes = std::fs::read(&path).expect("read");
        bytes.extend_from_slice(b"{\"from\":\"b3:cc\",\"to\":");
        std::fs::write(&path, bytes).expect("write");
        let legs = read(dir.path());
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[1].to.0, "b3:cc");
    }

    #[test]
    fn history_chains_identity_across_legs_newest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let leg = |from: &str, to: &str| Leg {
            from: grid_wire::Datum(from.into()),
            to: grid_wire::Datum(to.into()),
            git_rev: None,
            at: None,
        };
        append(
            dir.path(),
            &[
                leg("b3:aa", "b3:bb"),
                leg("b3:zz", "b3:yy"), // an unrelated def's leg
                leg("b3:bb", "b3:cc"),
            ],
        )
        .expect("append");
        let hits = history(dir.path(), "b3:cc");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].to.0, "b3:cc");
        assert_eq!(hits[1].to.0, "b3:bb");
    }
}
