//! The manifest: which segments are live, and the snapshot id derived from
//! them (decision D5 — a snapshot id is *never* minted, it is the hash of the
//! canonical manifest, so re-surveying an identical state dedupes by itself).
use crate::ChartError;

/// One live segment: its file name and the checksum of its image.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SegmentEntry {
    /// File name within the store directory, e.g. `0001.seg`.
    pub name: String,
    /// blake3 of the segment image (`crate::checksum`).
    pub checksum: [u8; 32],
}

/// The live set at one instant, plus the provenance the wire `Snapshot`
/// reports.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    /// Git revision the survey ran against.
    pub git_rev: String,
    /// blake3 of the canonical worktree diff, iff the tree was dirty.
    pub worktree_diff_hash: Option<[u8; 32]>,
    /// Live segments. Order is irrelevant: canonicalization sorts them.
    pub segments: Vec<SegmentEntry>,
}

/// `snap:b3:<64 hex>` — content identity of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotId(String);

impl SnapshotId {
    /// The wire form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Manifest {
    /// Canonical bytes: format epoch, rev, dirty hash, then segments sorted by
    /// name with explicit lengths. Length-prefixing every field is what stops
    /// two different manifests from sharing a serialization.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"grid-manifest/v1\n");
        out.extend_from_slice(&crate::FORMAT_EPOCH.to_le_bytes());
        push_str(&mut out, &self.git_rev);
        match self.worktree_diff_hash {
            Some(h) => {
                out.push(1);
                out.extend_from_slice(&h);
            }
            None => out.push(0),
        }
        let mut segments = self.segments.clone();
        segments.sort_unstable();
        segments.dedup();
        out.extend_from_slice(&u64_of(segments.len()).to_le_bytes());
        for s in &segments {
            push_str(&mut out, &s.name);
            out.extend_from_slice(&s.checksum);
        }
        out
    }

    /// Snapshot identity: `snap:b3:` + blake3 of [`Manifest::canonical_bytes`].
    #[must_use]
    pub fn snapshot_id(&self) -> SnapshotId {
        SnapshotId(format!(
            "snap:b3:{}",
            blake3::hash(&self.canonical_bytes()).to_hex()
        ))
    }

    /// Read a manifest back from its canonical bytes.
    ///
    /// # Errors
    /// [`ChartError::Truncated`] if the bytes end mid-field,
    /// [`ChartError::BadMagic`] / [`ChartError::BadEpoch`] on a foreign
    /// manifest, [`ChartError::BadUtf8`] on a non-UTF-8 name.
    pub fn parse(bytes: &[u8]) -> Result<Self, ChartError> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(17)? != b"grid-manifest/v1\n" {
            return Err(ChartError::BadMagic);
        }
        let epoch = u32::from_le_bytes(r.take(4)?.try_into().map_err(|_| ChartError::Truncated)?);
        if epoch != crate::FORMAT_EPOCH {
            return Err(ChartError::BadEpoch(epoch));
        }
        let git_rev = r.string()?;
        let worktree_diff_hash = match r.take(1)?[0] {
            0 => None,
            1 => Some(r.take(32)?.try_into().map_err(|_| ChartError::Truncated)?),
            _ => return Err(ChartError::BadAttrs),
        };
        let n = u64::from_le_bytes(r.take(8)?.try_into().map_err(|_| ChartError::Truncated)?);
        let n = usize::try_from(n).map_err(|_| ChartError::Truncated)?;
        let mut segments = Vec::new();
        for _ in 0..n {
            let name = r.string()?;
            let checksum = r.take(32)?.try_into().map_err(|_| ChartError::Truncated)?;
            segments.push(SegmentEntry { name, checksum });
        }
        Ok(Self {
            git_rev,
            worktree_diff_hash,
            segments,
        })
    }
}

fn u64_of(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&u64_of(s.len()).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ChartError> {
        let end = self.at.checked_add(n).ok_or(ChartError::Truncated)?;
        let out = self.bytes.get(self.at..end).ok_or(ChartError::Truncated)?;
        self.at = end;
        Ok(out)
    }

    fn string(&mut self) -> Result<String, ChartError> {
        let len = u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| ChartError::Truncated)?,
        );
        let len = usize::try_from(len).map_err(|_| ChartError::Truncated)?;
        std::str::from_utf8(self.take(len)?)
            .map(ToOwned::to_owned)
            .map_err(|_| ChartError::BadUtf8)
    }
}
