//! The provisional delta provider: file-level, git-backed.
//!
//! Wave 3 needs a `MadeGood` before the chart exists (Wave 4) or the survey
//! that fills it (Wave 5). This provider answers from `git diff --numstat`:
//! it sees which *files* changed, so it reports def changes at file
//! granularity and no edges at all. That is the honest resolution of a
//! file-level view, and it is enough to gate def promises and test
//! expectations end to end.
//!
//! It implements the same [`DeltaProvider`] trait the chart will, so Wave 5
//! swaps the implementation and nothing above this line changes.

use std::process::Command;

use grid_wire::{Callsign, Datum, DefChange, DefChangeAction, MadeGood, SnapshotId, SnapshotOrRev};
use grid_xte::DeltaProvider;

/// Why a delta could not be produced.
#[derive(Debug)]
pub enum Error {
    /// git could not be run, or answered with a failure.
    Git(String),
    /// A snapshot id was given, but no snapshot store exists yet (Wave 4).
    NoSnapshotStore(SnapshotId),
    /// Repo configuration under `.grid` is present but unreadable.
    Config(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git(msg) => write!(f, "git: {msg}"),
            Self::NoSnapshotStore(id) => write!(
                f,
                "snapshot {} is unknown: no snapshot store until the survey lands (wave 5); \
                 gate against a git rev meanwhile",
                id.0
            ),
            Self::Config(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for Error {}

/// A file-level delta provider rooted at a working tree.
pub struct FileDelta {
    root: std::path::PathBuf,
}

impl FileDelta {
    /// A provider over the tree at `root`.
    #[must_use]
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn git(&self, args: &[&str]) -> Result<String, Error> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .map_err(|e| Error::Git(e.to_string()))?;
        if !out.status.success() {
            return Err(Error::Git(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        String::from_utf8(out.stdout).map_err(|e| Error::Git(e.to_string()))
    }

    /// The rev a `SnapshotOrRev` names. Snapshot ids have no store to resolve
    /// against until Wave 4, so they are a typed refusal, not a silent guess.
    fn rev(target: &SnapshotOrRev) -> Result<String, Error> {
        match target {
            SnapshotOrRev::GitRev { git_rev } => Ok(git_rev.0.clone()),
            SnapshotOrRev::Snapshot { snapshot } => Err(Error::NoSnapshotStore(snapshot.clone())),
        }
    }

    /// A provisional snapshot id for a rev: the rev, in snapshot clothing.
    /// Wave 5 replaces this with the manifest hash (D5); until then a caller
    /// can still tell two gate runs apart.
    fn snapshot_of(rev: &str) -> SnapshotId {
        SnapshotId(format!("rev:{rev}"))
    }

    /// Datum stand-in for a file at a rev: `blake3` is Wave 2's job on real
    /// definitions; here identity is the coordinate itself, so the porcelain
    /// and the wire both stay honest about what is known.
    fn file_datum(path: &str, rev: &str) -> Datum {
        Datum(format!("file:{rev}:{path}"))
    }

    /// A file path read as a callsign: the dotted module path, extension
    /// dropped. `packages/cortana/reaper.py` -> `packages.cortana.reaper`.
    fn callsign_of(path: &str) -> String {
        let stem = path.rsplit_once('.').map_or(path, |(head, _)| head);
        stem.replace(['/', '\\'], ".")
    }
}

impl DeltaProvider for FileDelta {
    type Error = Error;

    fn made_good(
        &self,
        base: &SnapshotOrRev,
        head: &SnapshotOrRev,
    ) -> Result<MadeGood, Self::Error> {
        let (base_rev, head_rev) = (Self::rev(base)?, Self::rev(head)?);
        let status = self.git(&["diff", "--name-status", "-z", &base_rev, &head_rev])?;

        // -z output: NUL-separated fields, status then path (then a second
        // path for renames). Parsing the machine format keeps paths with
        // spaces or newlines from becoming two files.
        let mut fields = status.split('\0').filter(|s| !s.is_empty());
        let mut defs = Vec::new();
        while let Some(code) = fields.next() {
            let Some(path) = fields.next() else { break };
            let (action, path) = match code.as_bytes().first() {
                Some(b'A') => (DefChangeAction::Added, path),
                Some(b'D') => (DefChangeAction::Removed, path),
                // A rename carries a second path; the new one is the def.
                Some(b'R') => match fields.next() {
                    Some(to) => (DefChangeAction::Modified, to),
                    None => break,
                },
                _ => (DefChangeAction::Modified, path),
            };
            defs.push(DefChange {
                action,
                callsign: Callsign(Self::callsign_of(path)),
                before: (action != DefChangeAction::Added)
                    .then(|| Self::file_datum(path, &base_rev)),
                after: (action != DefChangeAction::Removed)
                    .then(|| Self::file_datum(path, &head_rev)),
            });
        }
        // The wire contract requires deterministic order: defs by callsign.
        defs.sort_by(|a, b| a.callsign.0.cmp(&b.callsign.0));

        Ok(MadeGood {
            base: Self::snapshot_of(&base_rev),
            head: Self::snapshot_of(&head_rev),
            defs,
            // A file-level view proves no edges. Promised edges therefore come
            // back `missing` rather than falsely matched: the gate holds until
            // the chart can see them, which is the safe direction.
            edges: Vec::new(),
        })
    }

    /// A file-level view names files, not definitions, and the def changes it
    /// reports already carry their callsigns. No datum here needs naming, so
    /// none is named — an honest `None` beats a guess that would silently
    /// match a promise pattern.
    fn callsign(&self, _datum: &Datum) -> Option<Callsign> {
        None
    }

    // `test_outcome` keeps the trait default: with no runner report, every
    // expected test reads `missing` and holds the gate. Real outcomes arrive
    // with wake ingestion (Wave 8).
}

#[cfg(test)]
mod tests {
    use super::FileDelta;

    #[test]
    fn path_reads_as_a_dotted_callsign() {
        assert_eq!(
            FileDelta::callsign_of("packages/cortana/reaper.py"),
            "packages.cortana.reaper"
        );
        assert_eq!(FileDelta::callsign_of("justfile"), "justfile");
    }
}
