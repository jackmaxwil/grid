//! Snapshot membership: identity stored once, snapshots as reference sets.
//!
//! The `HyperDiff` steal (`docs/research/ANALYSIS.md` §2). Grid already has the
//! identity half — the datum *is* the dedup key — and what was missing is the
//! access structure: a global table of every datum ever retained, plus one
//! sorted list of references into it per retained snapshot. With that,
//! "changed between S1 and S2" is a merge-diff of two sorted lists and
//! "unchanged" is id-equality, never a load and never a diff. Cross-snapshot
//! questions stop being patch replay.
//!
//! This is derived data. It is rebuildable from the retained chain and safe
//! to delete, like every other cache under `.grid/`. It is also its own
//! image, with its own magic and its own epoch: it is not part of a
//! per-snapshot segment, so it has no interaction with the segment format
//! epoch at all.
//!
//! Determinism is by construction: the datum table is the sorted union,
//! snapshots are in ascending id order, and each membership row is ascending.
//! [`build`] is the only writer, so an image extended one snapshot at a time
//! and one rebuilt from the whole chain are the same bytes — which is the
//! property the store's tests pin.
use crate::{checksum, ChartError, DatumId, HEADER_LEN, SECTION_ENTRY_LEN};

/// Bytes prefixing every membership image. Deliberately not [`crate::MAGIC`]:
/// a membership image is not a segment and must never be parsed as one.
pub const MEMBERSHIP_MAGIC: &[u8; 8] = b"GRIDMEM\x00";
/// Membership format epoch. Derived data, so a bump is a rebuild rather than
/// a refusal — but the loader still declines bytes it does not understand.
pub const MEMBERSHIP_EPOCH: u32 = 1;
/// Sections a v1 membership image declares.
pub const MEMBERSHIP_SECTIONS: usize = 6;

/// One definition as membership records it: identity plus the two cheap
/// numbers the survey already computed at hash time.
///
/// The metadata is `HyperDiff`'s height/size filtering, miniaturised: it is
/// what lets a shape-join rank candidates without loading a definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Member {
    /// Content identity.
    pub datum: DatumId,
    /// Cyclomatic complexity, or 0 when the snapshot did not record one.
    pub complexity: u16,
    /// Byte length of the definition's span, saturating — a definition
    /// longer than 64KiB is rare and its exact size is not load-bearing.
    pub size: u16,
}

/// What a merge-diff of two snapshots' rows found for one datum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// In the later snapshot only.
    Added(DatumId),
    /// In the earlier snapshot only.
    Removed(DatumId),
}

/// Why a membership question could not be answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipError {
    /// The image is not a membership image, or not one this version reads.
    Image(ChartError),
    /// A snapshot the caller named is not in the retained chain. A typed
    /// refusal, never an empty diff: "no changes" and "I have never heard of
    /// that snapshot" are opposite answers.
    UnknownSnapshot(String),
}

impl std::fmt::Display for MembershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(e) => write!(f, "membership: {e}"),
            Self::UnknownSnapshot(id) => {
                write!(f, "snapshot {id} is not in the retained chain")
            }
        }
    }
}

impl std::error::Error for MembershipError {}

impl From<ChartError> for MembershipError {
    fn from(e: ChartError) -> Self {
        Self::Image(e)
    }
}

/// Build a membership image from every retained snapshot.
///
/// Input is treated as a set: snapshots sort by id, members sort by datum,
/// and duplicates collapse. The same chain is therefore the same bytes on
/// every machine and in any input order.
///
/// # Errors
/// [`ChartError::TooLarge`] if the chain exceeds what u32 ordinals address.
pub fn build(snapshots: &[(String, Vec<Member>)]) -> Result<Vec<u8>, ChartError> {
    let mut rows: Vec<(&str, Vec<Member>)> = snapshots
        .iter()
        .map(|(id, members)| {
            let mut members = members.clone();
            members.sort_unstable();
            members.dedup_by_key(|m| m.datum);
            (id.as_str(), members)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(b.0));
    rows.dedup_by(|a, b| a.0 == b.0);

    // The global table: every datum the chain ever held, once.
    let mut table: Vec<Member> = rows.iter().flat_map(|(_, m)| m.iter().copied()).collect();
    table.sort_unstable();
    table.dedup_by_key(|m| m.datum);
    let datum_count = u32::try_from(table.len()).map_err(|_| ChartError::TooLarge)?;
    let snapshot_count = u32::try_from(rows.len()).map_err(|_| ChartError::TooLarge)?;

    let ordinal = |d: DatumId| -> Option<u32> {
        table
            .binary_search_by(|probe| probe.datum.cmp(&d))
            .ok()
            .and_then(|i| u32::try_from(i).ok())
    };

    let mut member_ids: Vec<u8> = Vec::new();
    let mut member_offsets: Vec<u8> = Vec::new();
    let mut acc = 0u32;
    for (_, members) in &rows {
        member_offsets.extend_from_slice(&acc.to_le_bytes());
        for m in members {
            // Every member came from the table above, so this cannot miss;
            // `get`-shaped anyway, because a silent zero would be a wrong
            // answer rather than a loud one.
            let Some(ord) = ordinal(m.datum) else {
                continue;
            };
            member_ids.extend_from_slice(&ord.to_le_bytes());
            acc = acc.checked_add(1).ok_or(ChartError::TooLarge)?;
        }
    }
    member_offsets.extend_from_slice(&acc.to_le_bytes());

    let mut id_bytes: Vec<u8> = Vec::new();
    let mut id_offsets: Vec<u8> = Vec::new();
    let mut at = 0u32;
    for (id, _) in &rows {
        id_offsets.extend_from_slice(&at.to_le_bytes());
        id_bytes.extend_from_slice(id.as_bytes());
        at = at
            .checked_add(u32::try_from(id.len()).map_err(|_| ChartError::TooLarge)?)
            .ok_or(ChartError::TooLarge)?;
    }
    id_offsets.extend_from_slice(&at.to_le_bytes());

    let sections: [Vec<u8>; MEMBERSHIP_SECTIONS] = [
        table.iter().flat_map(|m| m.datum.0).collect(),
        table
            .iter()
            .flat_map(|m| {
                let mut out = [0u8; 4];
                out[..2].copy_from_slice(&m.complexity.to_le_bytes());
                out[2..].copy_from_slice(&m.size.to_le_bytes());
                out
            })
            .collect(),
        id_offsets,
        id_bytes,
        member_offsets,
        member_ids,
    ];

    let table_len = MEMBERSHIP_SECTIONS * SECTION_ENTRY_LEN;
    let mut image = Vec::with_capacity(HEADER_LEN + table_len);
    image.extend_from_slice(MEMBERSHIP_MAGIC);
    image.extend_from_slice(&MEMBERSHIP_EPOCH.to_le_bytes());
    let count = u32::try_from(MEMBERSHIP_SECTIONS).map_err(|_| ChartError::TooLarge)?;
    image.extend_from_slice(&count.to_le_bytes());
    image.extend_from_slice(&datum_count.to_le_bytes());
    image.extend_from_slice(&snapshot_count.to_le_bytes());
    image.resize(HEADER_LEN, 0);

    let mut offset = HEADER_LEN + table_len;
    for s in &sections {
        let at = u64::try_from(offset).map_err(|_| ChartError::TooLarge)?;
        let len = u64::try_from(s.len()).map_err(|_| ChartError::TooLarge)?;
        image.extend_from_slice(&at.to_le_bytes());
        image.extend_from_slice(&len.to_le_bytes());
        offset = offset.checked_add(s.len()).ok_or(ChartError::TooLarge)?;
    }
    for s in &sections {
        image.extend_from_slice(s);
    }
    let sum = checksum(&image);
    image
        .get_mut(32..64)
        .ok_or(ChartError::Truncated)?
        .copy_from_slice(&sum);
    Ok(image)
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let raw: [u8; 8] = bytes.get(at..at.checked_add(8)?)?.try_into().ok()?;
    Some(u64::from_le_bytes(raw))
}

/// A validated membership image over owned or mapped bytes.
///
/// Like [`crate::Segment`], parsing borrows nothing, so a mapped image needs
/// no unsafe to keep around, and every accessor below is total because
/// [`Membership::parse`] refused everything that would not be.
#[derive(Debug)]
pub struct Membership<B> {
    bytes: B,
    spans: [(usize, usize); MEMBERSHIP_SECTIONS],
    datum_count: u32,
    snapshot_count: u32,
}

/// Section indexes, named where they are used.
mod section {
    pub const DATUMS: usize = 0;
    pub const META: usize = 1;
    pub const ID_OFFSETS: usize = 2;
    pub const ID_BYTES: usize = 3;
    pub const MEMBER_OFFSETS: usize = 4;
    pub const MEMBER_IDS: usize = 5;
}

impl<B: AsRef<[u8]>> Membership<B> {
    /// Validate an image and take ownership of its bytes.
    ///
    /// # Errors
    /// [`ChartError`] describing the first structural claim the bytes failed.
    pub fn parse(bytes: B) -> Result<Self, ChartError> {
        let b = bytes.as_ref();
        let header = b.get(..HEADER_LEN).ok_or(ChartError::Truncated)?;
        if header.get(..8) != Some(&MEMBERSHIP_MAGIC[..]) {
            return Err(ChartError::BadMagic);
        }
        let epoch = read_u32(header, 8).ok_or(ChartError::Truncated)?;
        if epoch != MEMBERSHIP_EPOCH {
            return Err(ChartError::BadEpoch(epoch));
        }
        let sections = read_u32(header, 12).ok_or(ChartError::Truncated)?;
        if sections as usize != MEMBERSHIP_SECTIONS {
            return Err(ChartError::BadSectionCount(sections));
        }
        let datum_count = read_u32(header, 16).ok_or(ChartError::Truncated)?;
        let snapshot_count = read_u32(header, 20).ok_or(ChartError::Truncated)?;
        let stored: [u8; 32] = header
            .get(32..64)
            .and_then(|s| s.try_into().ok())
            .ok_or(ChartError::Truncated)?;
        if checksum(b) != stored {
            return Err(ChartError::ChecksumMismatch);
        }

        let table_end = HEADER_LEN
            .checked_add(
                MEMBERSHIP_SECTIONS
                    .checked_mul(SECTION_ENTRY_LEN)
                    .ok_or(ChartError::Truncated)?,
            )
            .ok_or(ChartError::Truncated)?;
        if b.len() < table_end {
            return Err(ChartError::Truncated);
        }
        let mut spans = [(0usize, 0usize); MEMBERSHIP_SECTIONS];
        for (i, span) in spans.iter_mut().enumerate() {
            let at = HEADER_LEN + i * SECTION_ENTRY_LEN;
            let off = read_u64(b, at).ok_or(ChartError::Truncated)?;
            let len = read_u64(b, at + 8).ok_or(ChartError::Truncated)?;
            let start = usize::try_from(off).map_err(|_| ChartError::Truncated)?;
            let size = usize::try_from(len).map_err(|_| ChartError::Truncated)?;
            let end = start.checked_add(size).ok_or(ChartError::Truncated)?;
            if start < table_end || end > b.len() {
                return Err(ChartError::Truncated);
            }
            *span = (start, end);
        }

        let me = Self {
            bytes,
            spans,
            datum_count,
            snapshot_count,
        };
        me.validate()?;
        Ok(me)
    }

    fn section(&self, i: usize) -> &[u8] {
        let (start, end) = self.spans[i];
        self.bytes.as_ref().get(start..end).unwrap_or(&[])
    }

    fn validate(&self) -> Result<(), ChartError> {
        let datums = self.datum_count as usize;
        let snaps = self.snapshot_count as usize;
        let want = |i: usize, n: usize| {
            if self.section(i).len() == n {
                Ok(())
            } else {
                Err(ChartError::BadSectionSize(crate::Section::Datums))
            }
        };
        want(
            section::DATUMS,
            datums.checked_mul(32).ok_or(ChartError::TooLarge)?,
        )?;
        want(
            section::META,
            datums.checked_mul(4).ok_or(ChartError::TooLarge)?,
        )?;
        let rows = snaps.checked_add(1).ok_or(ChartError::TooLarge)?;
        let row_bytes = rows.checked_mul(4).ok_or(ChartError::TooLarge)?;
        want(section::ID_OFFSETS, row_bytes)?;
        want(section::MEMBER_OFFSETS, row_bytes)?;

        // Ordinals are a binary search over the digests, which is only a
        // function if they are strictly ascending.
        let digests = self.section(section::DATUMS);
        for w in digests.chunks_exact(32).collect::<Vec<_>>().windows(2) {
            if w[0] >= w[1] {
                return Err(ChartError::UnsortedDatums);
            }
        }
        self.check_csr(
            section::ID_OFFSETS,
            u32::try_from(self.section(section::ID_BYTES).len())
                .map_err(|_| ChartError::TooLarge)?,
        )?;
        std::str::from_utf8(self.section(section::ID_BYTES)).map_err(|_| ChartError::BadUtf8)?;
        let members = u32::try_from(self.section(section::MEMBER_IDS).len() / 4)
            .map_err(|_| ChartError::TooLarge)?;
        if !self.section(section::MEMBER_IDS).len().is_multiple_of(4) {
            return Err(ChartError::BadSectionSize(crate::Section::OutTargets));
        }
        self.check_csr(section::MEMBER_OFFSETS, members)?;
        for raw in self.section(section::MEMBER_IDS).chunks_exact(4) {
            let ord = u32::from_le_bytes(raw.try_into().unwrap_or_default());
            if ord >= self.datum_count {
                return Err(ChartError::OrdinalOutOfRange);
            }
        }
        // Snapshot ids are binary-searched, so they must ascend too.
        let mut prev: Option<&str> = None;
        for i in 0..self.snapshot_count {
            let Some(id) = self.snapshot_id(i) else {
                return Err(ChartError::Truncated);
            };
            if prev.is_some_and(|p| p >= id) {
                return Err(ChartError::UnsortedDatums);
            }
            prev = Some(id);
        }
        Ok(())
    }

    fn check_csr(&self, offsets: usize, total: u32) -> Result<(), ChartError> {
        let raw = self.section(offsets);
        let mut prev = 0u32;
        for (i, chunk) in raw.chunks_exact(4).enumerate() {
            let v = u32::from_le_bytes(chunk.try_into().unwrap_or_default());
            if v < prev || v > total {
                return Err(ChartError::BadCsr(crate::Section::OutOffsets));
            }
            if i + 1 == raw.len() / 4 && v != total {
                return Err(ChartError::BadCsr(crate::Section::OutOffsets));
            }
            prev = v;
        }
        Ok(())
    }

    /// How many distinct datums the whole retained chain holds.
    #[must_use]
    pub const fn datum_count(&self) -> u32 {
        self.datum_count
    }

    /// How many snapshots the chain holds.
    #[must_use]
    pub const fn snapshot_count(&self) -> u32 {
        self.snapshot_count
    }

    /// The raw image, for checksumming and republication.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// The datum at a global ordinal.
    #[must_use]
    pub fn datum(&self, ord: u32) -> Option<DatumId> {
        let at = (ord as usize).checked_mul(32)?;
        let raw: [u8; 32] = self
            .section(section::DATUMS)
            .get(at..at.checked_add(32)?)?
            .try_into()
            .ok()?;
        Some(DatumId(raw))
    }

    /// The cheap metadata beside a datum: `(complexity, size)`.
    #[must_use]
    pub fn meta(&self, ord: u32) -> Option<(u16, u16)> {
        let at = (ord as usize).checked_mul(4)?;
        let raw = self.section(section::META).get(at..at.checked_add(4)?)?;
        let pair = |i: usize| -> Option<u16> {
            Some(u16::from_le_bytes(raw.get(i..i + 2)?.try_into().ok()?))
        };
        Some((pair(0)?, pair(2)?))
    }

    /// The global ordinal of a datum, by binary search.
    #[must_use]
    pub fn ordinal(&self, datum: &DatumId) -> Option<u32> {
        let digests = self.section(section::DATUMS);
        let (mut lo, mut hi) = (0usize, digests.len() / 32);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let probe = digests.get(mid * 32..mid * 32 + 32)?;
            match probe.cmp(&datum.0[..]) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return u32::try_from(mid).ok(),
            }
        }
        None
    }

    /// The snapshot id at index `i`.
    #[must_use]
    pub fn snapshot_id(&self, i: u32) -> Option<&str> {
        let (start, end) = self.row(section::ID_OFFSETS, i)?;
        let raw = self
            .section(section::ID_BYTES)
            .get(start as usize..end as usize)?;
        std::str::from_utf8(raw).ok()
    }

    /// The index of a snapshot id, by binary search over ascending ids.
    #[must_use]
    pub fn index_of(&self, id: &str) -> Option<u32> {
        let (mut lo, mut hi) = (0u32, self.snapshot_count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.snapshot_id(mid)?.cmp(id) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    fn row(&self, offsets: usize, i: u32) -> Option<(u32, u32)> {
        if i >= self.snapshot_count {
            return None;
        }
        let raw = self.section(offsets);
        let at = (i as usize).checked_mul(4)?;
        let start = read_u32(raw, at)?;
        let end = read_u32(raw, at.checked_add(4)?)?;
        (end >= start).then_some((start, end))
    }

    /// The datum ordinals a snapshot holds, ascending.
    pub fn members(&self, i: u32) -> impl Iterator<Item = u32> + '_ {
        let (start, end) = self.row(section::MEMBER_OFFSETS, i).unwrap_or((0, 0));
        let ids = self.section(section::MEMBER_IDS);
        (start as usize..end as usize).filter_map(move |k| read_u32(ids, k.checked_mul(4)?))
    }

    /// Is `datum` in snapshot `i`? Membership by binary search over the
    /// snapshot's ascending row — never a load, which is the whole point.
    #[must_use]
    pub fn holds(&self, i: u32, datum: &DatumId) -> bool {
        let Some(want) = self.ordinal(datum) else {
            return false;
        };
        let Some((start, end)) = self.row(section::MEMBER_OFFSETS, i) else {
            return false;
        };
        let ids = self.section(section::MEMBER_IDS);
        let (mut lo, mut hi) = (start, end);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let Some(probe) = read_u32(ids, (mid as usize).saturating_mul(4)) else {
                return false;
            };
            match probe.cmp(&want) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return true,
            }
        }
        false
    }

    /// What changed between two snapshots, as a merge-diff of their sorted
    /// rows. No definition is loaded and nothing is compared but ordinals.
    ///
    /// # Errors
    /// [`MembershipError::UnknownSnapshot`] when either id is not in the
    /// retained chain. A snapshot this image has never heard of is a refusal,
    /// never an empty diff — those are opposite answers.
    pub fn changed_between(&self, base: &str, head: &str) -> Result<Vec<Change>, MembershipError> {
        let a = self
            .index_of(base)
            .ok_or_else(|| MembershipError::UnknownSnapshot(base.to_owned()))?;
        let b = self
            .index_of(head)
            .ok_or_else(|| MembershipError::UnknownSnapshot(head.to_owned()))?;
        let (mut left, mut right) = (self.members(a).peekable(), self.members(b).peekable());
        let mut out = Vec::new();
        loop {
            match (left.peek().copied(), right.peek().copied()) {
                (None, None) => break,
                (Some(l), None) => {
                    left.next();
                    if let Some(d) = self.datum(l) {
                        out.push(Change::Removed(d));
                    }
                }
                (None, Some(r)) => {
                    right.next();
                    if let Some(d) = self.datum(r) {
                        out.push(Change::Added(d));
                    }
                }
                (Some(l), Some(r)) => match l.cmp(&r) {
                    std::cmp::Ordering::Equal => {
                        left.next();
                        right.next();
                    }
                    std::cmp::Ordering::Less => {
                        left.next();
                        if let Some(d) = self.datum(l) {
                            out.push(Change::Removed(d));
                        }
                    }
                    std::cmp::Ordering::Greater => {
                        right.next();
                        if let Some(d) = self.datum(r) {
                            out.push(Change::Added(d));
                        }
                    }
                },
            }
        }
        Ok(out)
    }

    /// The chain as [`build`] would take it back: every snapshot with its
    /// members. This is what makes extending an image cheap — the snapshots
    /// already in it need not be read out of their segments again.
    #[must_use]
    pub fn chain(&self) -> Vec<(String, Vec<Member>)> {
        (0..self.snapshot_count)
            .filter_map(|i| {
                let id = self.snapshot_id(i)?.to_owned();
                let members = self
                    .members(i)
                    .filter_map(|ord| {
                        let (complexity, size) = self.meta(ord)?;
                        Some(Member {
                            datum: self.datum(ord)?,
                            complexity,
                            size,
                        })
                    })
                    .collect();
                Some((id, members))
            })
            .collect()
    }
}
