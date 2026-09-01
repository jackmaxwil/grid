//! grid-chart-io — THE ONLY unsafe crate: mmap + segment loading, fully validating.
//!
//! Everything unsafe in grid is one `unsafe` block in this file, wrapping
//! `memmap2::Mmap::map`. The safety obligation it carries — that nobody
//! mutates or truncates the file while the mapping lives — is discharged
//! *structurally*, not by convention: segments are written to a temp name and
//! published by atomic rename, and a published segment is never opened for
//! writing again. Compaction writes new files; it never edits old ones.
//!
//! Byte interpretation lives in `grid-chart`, which is `forbid(unsafe_code)`.
//! So an adversarial image reaches only safe validation code (claim C1), and
//! this crate stays small enough to read in one sitting.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arc_swap::ArcSwap;
use grid_chart::{ChartError, Manifest, Member, Membership, Segment, SegmentEntry, SnapshotId};
use memmap2::Mmap;

/// Bytes of a segment, however they are held.
///
/// `Mapped` is the hot path; `Owned` exists because a mapping of a zero-length
/// file is not portable and small stores are common. Both are immutable for
/// their whole life, which is the invariant the one unsafe block needs.
#[derive(Debug)]
pub enum Bytes {
    /// A read-only memory mapping of a published segment file.
    Mapped(Mmap),
    /// A heap copy, for tiny or in-memory segments.
    Owned(Vec<u8>),
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Mapped(m) => m,
            Self::Owned(v) => v,
        }
    }
}

/// Which of [`Bytes`]'s two representations a segment ended up with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Read-only mmap — the hot path, and the one the unsafe block serves.
    Mapped,
    /// Heap copy, used only for empty files.
    Owned,
}

/// Where a segment's bytes came from. Exists so tests can assert the mmap path
/// is the one being exercised rather than assuming it.
#[must_use]
pub fn bytes_source(seg: &Segment<Bytes>) -> Source {
    match seg.holder() {
        Bytes::Mapped(_) => Source::Mapped,
        Bytes::Owned(_) => Source::Owned,
    }
}

/// Anything that can go wrong opening a store: I/O, or bytes that lied.
#[derive(Debug)]
pub enum StoreError {
    /// The filesystem said no.
    Io(std::io::Error),
    /// The bytes were there and were not a valid segment or manifest.
    Chart(ChartError),
    /// A manifest names a segment whose image hashes to something else — the
    /// file was replaced or corrupted after publication.
    ChecksumMismatch {
        /// Segment file name from the manifest.
        name: String,
    },
    /// No snapshot with this id was ever published to this store.
    UnknownSnapshot(SnapshotId),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Chart(e) => write!(f, "{e}"),
            Self::ChecksumMismatch { name } => write!(f, "segment {name} does not match manifest"),
            Self::UnknownSnapshot(id) => write!(f, "unknown snapshot {id}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<ChartError> for StoreError {
    fn from(e: ChartError) -> Self {
        Self::Chart(e)
    }
}

/// File name of the live manifest. Publication is `rename(MANIFEST_TMP, MANIFEST)`.
pub const MANIFEST: &str = "manifest";
/// The membership image: identity once, snapshots as reference sets. Derived
/// data — deletable, and rebuilt from the retained chain by the next publish.
pub const MEMBERSHIP: &str = "membership";
const MANIFEST_TMP: &str = "manifest.tmp";

/// One immutable epoch of the chart: a manifest and the segments it names.
///
/// Readers hold an `Arc<Epoch>` and are unaffected by publication — the swap
/// replaces the pointer, and the old epoch drops when its last reader lets go.
#[derive(Debug)]
pub struct Epoch {
    manifest: Manifest,
    snapshot_id: SnapshotId,
    segments: Vec<Segment<Bytes>>,
}

impl Epoch {
    /// The manifest this epoch was published from.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Snapshot identity (D5): the hash of the canonical manifest.
    #[must_use]
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    /// The live segments, in manifest order.
    #[must_use]
    pub fn segments(&self) -> &[Segment<Bytes>] {
        &self.segments
    }
}

/// A segment store on disk: append-only segment files plus a manifest naming
/// the live set.
///
/// Crash safety is structural. A segment file is fsynced and renamed into
/// place before any manifest mentions it, and the manifest itself is published
/// by atomic rename, so `kill -9` at any instant leaves either the old live
/// set or the new one — never a manifest pointing at a half-written segment,
/// and never a torn manifest. Orphaned temp files are garbage, not state.
#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    epoch: ArcSwap<Epoch>,
    /// Set when `open` dropped a manifest written by a past format epoch.
    superseded: Option<u32>,
}

impl Store {
    /// Open a store, creating the directory and an empty manifest if absent.
    ///
    /// # Errors
    /// [`StoreError`] if the directory cannot be read, or if the live manifest
    /// names a segment that is missing, corrupt, or checksum-mismatched.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let mut superseded = None;
        let manifest = match fs::read(dir.join(MANIFEST)) {
            Ok(bytes) => match Manifest::parse(&bytes) {
                Ok(manifest) => manifest,
                // A manifest from a past format epoch is refused, exactly as
                // ASSURANCE.md declares — but the recovery it declares is a
                // re-survey, and a hard error here would take `grid survey`
                // down with everything else. So: the empty store, plus the
                // epoch that was dropped, so the caller can say so rather
                // than pretend the store was always empty.
                Err(grid_chart::ChartError::BadEpoch(e)) => {
                    superseded = Some(e);
                    Manifest::default()
                }
                Err(e) => return Err(e.into()),
            },
            // No manifest is the empty store, not an error: this is what a
            // fresh checkout and a crash before the first publish look like.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Manifest::default(),
            Err(e) => return Err(e.into()),
        };
        let epoch = load_epoch(&dir, manifest)?;
        Ok(Self {
            dir,
            epoch: ArcSwap::from(Arc::new(epoch)),
            superseded,
        })
    }

    /// The format epoch of a manifest this store refused to load, if it
    /// refused one. A store that answers `Some` is empty because its contents
    /// predate the current format, and the recovery is a re-survey.
    #[must_use]
    pub const fn superseded(&self) -> Option<u32> {
        self.superseded
    }

    /// The current epoch. Lock-free: readers never block a writer, and a
    /// publication in flight cannot be observed half-applied.
    #[must_use]
    pub fn load(&self) -> Arc<Epoch> {
        self.epoch.load_full()
    }

    /// The store's directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write `images` as segment files and publish them as the new live set.
    ///
    /// Each image is validated before it is written: an invalid segment cannot
    /// enter the store even from inside the process. Returns the new snapshot
    /// id (D5: a pure function of the manifest, so republishing an identical
    /// state is idempotent).
    ///
    /// The daemon serializes all calls to this (D6, two writers total); a
    /// `&mut self` receiver is that rule expressed in the type system.
    ///
    /// # Errors
    /// [`StoreError`] on I/O failure or an image that does not validate.
    pub fn publish(
        &mut self,
        git_rev: &str,
        worktree_diff_hash: Option<[u8; 32]>,
        images: &[Vec<u8>],
    ) -> Result<SnapshotId, StoreError> {
        let mut segments = Vec::with_capacity(images.len());
        for (i, image) in images.iter().enumerate() {
            // Validate first: the store never contains bytes we would refuse
            // to load back.
            Segment::parse(image.as_slice())?;
            let checksum = grid_chart::checksum(image);
            let name = format!("{}.seg", hex(&checksum));
            let path = self.dir.join(&name);
            // Content-addressed name: an identical segment is already there,
            // byte for byte, so rewriting it would be pure churn.
            if !path.exists() {
                write_atomic(&self.dir, &path, image, &format!("seg.{i}.tmp"))?;
            }
            segments.push(SegmentEntry { name, checksum });
        }
        let manifest = Manifest {
            git_rev: git_rev.to_owned(),
            worktree_diff_hash,
            segments,
        };
        let bytes = manifest.canonical_bytes();
        // Retain the manifest under its own snapshot id before publishing it.
        // A snapshot id is the hash of these bytes (D5), so this file is
        // content-addressed too: republishing an identical state rewrites
        // nothing, and a gate can still reach a base snapshot after head has
        // moved on. Written first, so the live manifest never names a snapshot
        // that cannot be reopened.
        let retained = self.dir.join(retained_name(&manifest.snapshot_id()));
        if !retained.exists() {
            write_atomic(&self.dir, &retained, &bytes, "snap.tmp")?;
        }
        // The live manifest is content too: republishing an identical state
        // must rewrite nothing (D5), and the unconditional rewrite here was
        // measured as the whole fsync cost of an unchanged `grid survey`.
        let live = self.dir.join(MANIFEST);
        if !fs::read(&live).is_ok_and(|current| current == bytes) {
            write_atomic(&self.dir, &live, &bytes, MANIFEST_TMP)?;
        }
        let epoch = load_epoch(&self.dir, manifest)?;
        let id = epoch.snapshot_id().clone();
        // Membership is derived: extending it is best effort, because losing
        // it costs a rebuild and never costs the snapshot that just landed.
        let _derived = self.extend_membership(&epoch);
        self.epoch.store(Arc::new(epoch));
        Ok(id)
    }

    /// The membership image, if one has been built.
    ///
    /// Mapped like a segment, and validated the same way: a caller only ever
    /// sees bytes that passed [`grid_chart::Membership::parse`].
    ///
    /// # Errors
    /// [`StoreError`] if the file exists but does not map or does not parse.
    /// A missing file is `Ok(None)` — derived data that has not been built
    /// yet is an absence, not a fault.
    pub fn membership(&self) -> Result<Option<Membership<Bytes>>, StoreError> {
        let path = self.dir.join(MEMBERSHIP);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Membership::parse(map_file(&path)?)?))
    }

    /// Fold one epoch into the membership image, writing the result.
    ///
    /// Cheap by construction: the snapshots already in the image are read
    /// back out of it rather than out of their segments, so the only segment
    /// work is this epoch's. The bytes are produced by the same builder a
    /// full rebuild uses, so the two cannot disagree.
    ///
    /// ponytail: rewrites the whole image, O(chain), because a global sorted
    /// datum table makes a new datum shift every later ordinal — a true
    /// append needs a table that grows without renumbering (an id assigned at
    /// first sight, with the sort kept as a separate index). Worth doing when
    /// a chain is long enough for the rewrite to show up in a survey.
    ///
    /// # Errors
    /// [`StoreError`] on I/O failure or an image that does not build.
    pub fn extend_membership(&self, epoch: &Epoch) -> Result<(), StoreError> {
        let mut chain = match self.membership()? {
            Some(existing) => existing.chain(),
            None => Vec::new(),
        };
        let id = epoch.snapshot_id().as_str().to_owned();
        chain.retain(|(known, _)| known != &id);
        chain.push((id, members_of(epoch)));
        self.write_membership(&chain)
    }

    /// Rebuild the membership image from every retained snapshot.
    ///
    /// The recovery path, and the reference the incremental one is checked
    /// against: same builder, same inputs, therefore the same bytes.
    ///
    /// # Errors
    /// [`StoreError`] if the store cannot be read or the image cannot build.
    pub fn rebuild_membership(&self) -> Result<(), StoreError> {
        let mut chain = Vec::new();
        for id in self.retained_snapshots()? {
            let epoch = self.epoch_of(&id)?;
            chain.push((id.as_str().to_owned(), members_of(&epoch)));
        }
        self.write_membership(&chain)
    }

    fn write_membership(&self, chain: &[(String, Vec<Member>)]) -> Result<(), StoreError> {
        let image = grid_chart::build_membership(chain)?;
        write_atomic(
            &self.dir,
            &self.dir.join(MEMBERSHIP),
            &image,
            "membership.tmp",
        )?;
        Ok(())
    }

    /// Every snapshot id the store retains, ascending — the chain, as a set.
    ///
    /// # Errors
    /// [`StoreError`] if the directory cannot be read.
    pub fn retained_snapshots(&self) -> Result<Vec<SnapshotId>, StoreError> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(body) = name
                .strip_prefix("snap-")
                .and_then(|n| n.strip_suffix(".manifest"))
            else {
                continue;
            };
            // The retained file name is the snapshot id with its separators
            // flattened; the manifest inside is the authority, so read it
            // rather than un-flattening a name.
            let _ = body;
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(manifest) = Manifest::parse(&bytes) else {
                continue;
            };
            out.push(manifest.snapshot_id());
        }
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        out.dedup_by(|a, b| a.as_str() == b.as_str());
        Ok(out)
    }

    /// Reopen a previously published snapshot by id.
    ///
    /// This is what makes a gate between two snapshots possible: `head` is
    /// usually the live epoch, but `base` is a state the store has moved past.
    /// Segments are content-addressed and never deleted in place, so a
    /// retained manifest still names files that are there.
    ///
    /// # Errors
    /// [`StoreError::UnknownSnapshot`] if no such snapshot was published here.
    pub fn epoch_of(&self, id: &SnapshotId) -> Result<Arc<Epoch>, StoreError> {
        // The live epoch is the common case and is already in memory.
        let live = self.load();
        if live.snapshot_id() == id {
            return Ok(live);
        }
        let bytes = match fs::read(self.dir.join(retained_name(id))) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::UnknownSnapshot(id.clone()))
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Arc::new(load_epoch(&self.dir, Manifest::parse(&bytes)?)?))
    }

    /// The most recent retained snapshot surveyed at `rev`, if any.
    ///
    /// CI holds a rev, not a snapshot id, so this is the lookup the gate
    /// actually performs. Retained manifests record the rev they were
    /// surveyed at; scanning them is bounded by the number of published
    /// snapshots, and the live epoch is checked by the caller first.
    ///
    /// # Errors
    /// [`StoreError`] if the store directory cannot be read.
    pub fn epoch_for_rev(&self, rev: &str) -> Result<Option<Arc<Epoch>>, StoreError> {
        let mut best: Option<(std::time::SystemTime, Manifest)> = None;
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("snap-") || !name.ends_with(".manifest") {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            // A manifest that no longer parses is not a candidate; it is also
            // not a reason to fail a lookup for some other rev.
            let Ok(manifest) = Manifest::parse(&bytes) else {
                continue;
            };
            if manifest.git_rev != rev {
                continue;
            }
            let at = entry.metadata()?.modified()?;
            if best.as_ref().is_none_or(|(prior, _)| at > *prior) {
                best = Some((at, manifest));
            }
        }
        best.map(|(_, m)| load_epoch(&self.dir, m).map(Arc::new))
            .transpose()
    }
}

/// File name a snapshot's manifest is retained under. The id is
/// `snap:b3:<hex>`, and `:` is not a portable path character.
/// One epoch's definitions, as membership records them: identity plus the
/// cheap metadata the segment already carries.
fn members_of(epoch: &Epoch) -> Vec<Member> {
    let mut out = Vec::new();
    for segment in epoch.segments() {
        for ord in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(ord);
            let Some(datum) = segment.datum(ord) else {
                continue;
            };
            let size = segment.location(ord).map_or(0, |loc| {
                u16::try_from(loc.span.end.saturating_sub(loc.span.start)).unwrap_or(u16::MAX)
            });
            out.push(Member {
                datum,
                complexity: segment.complexity(ord).unwrap_or(0),
                size,
            });
        }
    }
    out
}

fn retained_name(id: &SnapshotId) -> String {
    format!("{}.manifest", id.as_str().replace(':', "-"))
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Durable, all-or-nothing file write: temp file, fsync, atomic rename, fsync
/// the directory so the rename itself survives power loss.
fn write_atomic(dir: &Path, path: &Path, bytes: &[u8], tmp_name: &str) -> std::io::Result<()> {
    let tmp = dir.join(tmp_name);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // Without this, the rename can be lost while the data survives — the one
    // ordering that would leave a manifest naming a segment that is not there.
    fs::File::open(dir)?.sync_all()
}

fn load_epoch(dir: &Path, manifest: Manifest) -> Result<Epoch, StoreError> {
    let mut segments = Vec::with_capacity(manifest.segments.len());
    for entry in &manifest.segments {
        let bytes = map_file(&dir.join(&entry.name))?;
        // The manifest is the authority on what the live set is; the checksum
        // is how we notice the file underneath it changed.
        if grid_chart::checksum(bytes.as_ref()) != entry.checksum {
            return Err(StoreError::ChecksumMismatch {
                name: entry.name.clone(),
            });
        }
        segments.push(Segment::parse(bytes)?);
    }
    let snapshot_id = manifest.snapshot_id();
    Ok(Epoch {
        manifest,
        snapshot_id,
        segments,
    })
}

/// Map a published segment file read-only.
///
/// This is the only unsafe in grid. `Mmap::map` is unsafe because the mapping
/// aliases a file other processes could mutate or truncate under us, which
/// would be UB (a torn read is the least of it; SIGBUS on truncation is the
/// real hazard).
///
/// Why it is sound here:
/// - Segment files are written to a temp name and published by atomic rename.
///   A path that exists in a manifest was complete before it had that name.
/// - Names are content-addressed (`<blake3>.seg`), so "rewriting" a segment
///   is a byte-identical no-op and in practice does not happen at all.
/// - Nothing in grid ever opens a published segment for writing or truncates
///   one. Compaction publishes new files and drops old names from the
///   manifest; deletion of an unreferenced file cannot affect a live mapping
///   because unlink does not invalidate an existing mapping on POSIX.
/// - Therefore, for the life of the returned `Mmap`, the mapped bytes are
///   immutable. External tampering is outside grid's model, and is caught
///   before use anyway by the manifest checksum in `load_epoch`.
///
/// Empty files are read instead of mapped: a zero-length mapping is not
/// portable, and a copy of zero bytes is free.
fn map_file(path: &Path) -> std::io::Result<Bytes> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() == 0 {
        return Ok(Bytes::Owned(Vec::new()));
    }
    // The crate-level lint is `deny`, not `forbid`, precisely so that each
    // unsafe block must opt in by name at its own call site.
    #[allow(unsafe_code, reason = "the sole mmap in grid; contract above")]
    // SAFETY: see the contract above — published segment files are immutable
    // for the lifetime of the mapping, enforced by write-once-then-rename and
    // content-addressed names, so no aliasing mutation or truncation occurs.
    let map = unsafe { Mmap::map(&file)? };
    Ok(Bytes::Mapped(map))
}
