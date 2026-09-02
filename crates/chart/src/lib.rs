//! grid-chart — graph + definition store: immutable segments, CSR indexes.
//!
//! A *segment* is a self-describing, immutable byte image: interned datum
//! ordinals, callsigns, and edges in CSR form in both directions. It is built
//! by [`build`] and read back by [`Segment::parse`], which validates every
//! structural claim the image makes about itself *before* any accessor can
//! observe it — a parsed [`Segment`] is a proof that all its indexes are in
//! bounds and monotone.
//!
//! Nothing here is unsafe and nothing here does I/O: `chart-io` maps the bytes,
//! this crate decides what they mean. The split is what keeps the C1 blast
//! radius to one crate — an adversarial image can only reach validation code.
//!
//! Determinism (claim C2): [`build`] sorts and dedups its inputs, so the same
//! (defs, edges) set is the same bytes on every machine, and a snapshot id is
//! therefore a pure function of content.
mod build;
mod delta;
mod manifest;
mod membership;

pub use build::{build, build_declined, Def, DefFacts, Edge};
pub use delta::diff;
pub use manifest::{Manifest, SegmentEntry, SnapshotId};
pub use membership::{
    build as build_membership, Change, Member, Membership, MembershipError, MEMBERSHIP_EPOCH,
    MEMBERSHIP_MAGIC, MEMBERSHIP_SECTIONS,
};

/// Bytes prefixing every segment image.
pub const MAGIC: &[u8; 8] = b"GRIDSEG\x00";
/// Segment format epoch. A change here is a declared re-survey, never a silent
/// reinterpretation of bytes on disk (see ASSURANCE.md, normalization
/// stability). Every epoch ever shipped parses forever; a section an older
/// image does not declare answers `None`, never a fabricated value.
pub const FORMAT_EPOCH: u32 = 5;
/// Sections an epoch-1 image declares — the first ten of [`Section`].
pub const V1_SECTION_COUNT: usize = 10;
/// Sections an epoch-2 image declares — the first fifteen of [`Section`].
pub const V2_SECTION_COUNT: usize = 15;
/// Sections an epoch-3 image declares — the first sixteen of [`Section`].
pub const V3_SECTION_COUNT: usize = 16;
/// Sections an epoch-4 image declares — the first nineteen of [`Section`].
pub const V4_SECTION_COUNT: usize = 19;
/// Fixed-size header: magic, counts, and the checksum of everything after it.
pub const HEADER_LEN: usize = 64;
/// Number of sections a v5 image declares, in the order of [`Section`].
pub const SECTION_COUNT: usize = 20;
/// Bytes per section-table entry: `(offset: u64, len: u64)`.
pub const SECTION_ENTRY_LEN: usize = 16;

/// Sections of a v1 image, in table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Section {
    /// 32-byte blake3 digests, ascending and unique; index = ordinal.
    Datums = 0,
    /// `def_count + 1` u32 CSR row starts into [`Section::OutTargets`].
    OutOffsets = 1,
    /// One u32 destination ordinal per out-edge.
    OutTargets = 2,
    /// One u16 `(relation << 8) | grade` per out-edge.
    OutAttrs = 3,
    /// `def_count + 1` u32 CSR row starts into [`Section::InTargets`].
    InOffsets = 4,
    /// One u32 source ordinal per in-edge.
    InTargets = 5,
    /// One u16 `(relation << 8) | grade` per in-edge.
    InAttrs = 6,
    /// `def_count + 1` u32 byte starts into [`Section::CallsignBytes`].
    CallsignOffsets = 7,
    /// UTF-8 callsigns, concatenated.
    CallsignBytes = 8,
    /// One u8 language tag per def.
    Langs = 9,
    /// `def_count + 1` u32 byte starts into [`Section::PathBytes`] (epoch 2).
    PathOffsets = 10,
    /// UTF-8 repo-relative paths, one per def, concatenated (epoch 2).
    PathBytes = 11,
    /// Per def: `(line: u32, line_end: u32, byte_start: u32, byte_end: u32)`
    /// (epoch 2).
    DefLocs = 12,
    /// Per def: 32-byte shape — blake3 of the canonical bytes with the
    /// definition's own name erased. The rename-join key; all-zero = absent
    /// (epoch 2).
    DefShapes = 13,
    /// Per def: `(complexity: u16, reserved: u16)` (epoch 2).
    DefMetrics = 14,
    /// Per def: 32-byte interface identity — blake3 of the frontend's
    /// canonical sig bytes, everything a caller can observe without the
    /// body. All-zero = absent, which is what an epoch-1 or epoch-2 image
    /// answers (epoch 3).
    DefSigs = 15,
    /// Per def: u32 index into the cfg table ([`Section::CfgOffsets`] /
    /// [`Section::CfgBytes`]) — the conjunction of every `#[cfg]` predicate
    /// that conditions the definition, as written. Index 0 is the empty
    /// predicate: nothing conditions it (epoch 4).
    DefCfgs = 16,
    /// `cfg_count + 1` u32 byte starts into [`Section::CfgBytes`]; entry 0
    /// is always the empty string (epoch 4).
    CfgOffsets = 17,
    /// UTF-8 cfg predicates, deduplicated and concatenated (epoch 4).
    CfgBytes = 18,
    /// Per def: `(external: u16, method: u16, internal: u16, refused: u16)`
    /// — the references the survey declined inside this definition, split
    /// as the repo-wide count is, saturating. What lets a consumer say how
    /// much of *this* neighbourhood the chart could not follow (epoch 5).
    DefDeclined = 19,
}

/// The references the survey declined inside one definition, by why. The
/// per-definition form of the survey's own split: `external` + `method` +
/// `internal` is the definition's declined total, `refused` a sub-count of
/// `method`. Saturating at `u16::MAX`, which the survey reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Declined {
    pub external: u16,
    pub method: u16,
    pub internal: u16,
    pub refused: u16,
}

/// Why an image was rejected. Every variant is a refusal, never a panic: the
/// loader's whole contract is that hostile bytes produce one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartError {
    /// Shorter than the header, or a section runs past the end.
    Truncated,
    /// Missing or wrong magic.
    BadMagic,
    /// Written by a different segment format epoch.
    BadEpoch(u32),
    /// Section table length disagrees with this format version.
    BadSectionCount(u32),
    /// A section's length is not a whole number of its elements, or does not
    /// match the count the header declares.
    BadSectionSize(Section),
    /// Sections overlap, so one byte would have two meanings.
    OverlappingSections,
    /// Content checksum does not cover the bytes present.
    ChecksumMismatch,
    /// A CSR row start went backwards or past the end of its target array.
    BadCsr(Section),
    /// Datum digests are not strictly ascending, so ordinals are not a
    /// function of content and lookup would be undefined.
    UnsortedDatums,
    /// An edge names an ordinal no definition has.
    OrdinalOutOfRange,
    /// A callsign or a cfg predicate is not UTF-8.
    BadUtf8,
    /// A definition's cfg index names a table entry the image does not hold.
    BadCfgIndex,
    /// A relation or grade byte is not one this version knows.
    BadAttrs,
    /// More defs or edges than the format's u32 ordinals can address.
    TooLarge,
}

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => f.write_str("segment truncated"),
            Self::BadMagic => f.write_str("not a grid segment"),
            Self::BadEpoch(e) => write!(f, "segment format epoch {e} != {FORMAT_EPOCH}"),
            Self::BadSectionCount(n) => write!(f, "section count {n} != {SECTION_COUNT}"),
            Self::BadSectionSize(s) => write!(f, "section {s:?} has an impossible length"),
            Self::OverlappingSections => f.write_str("sections overlap"),
            Self::ChecksumMismatch => f.write_str("checksum mismatch"),
            Self::BadCsr(s) => write!(f, "section {s:?} is not a monotone CSR index"),
            Self::UnsortedDatums => f.write_str("datum digests are not strictly ascending"),
            Self::OrdinalOutOfRange => f.write_str("edge names an unknown ordinal"),
            Self::BadUtf8 => f.write_str("callsign or cfg predicate is not UTF-8"),
            Self::BadCfgIndex => f.write_str("cfg index names no table entry"),
            Self::BadAttrs => f.write_str("unknown relation or grade"),
            Self::TooLarge => f.write_str("more defs or edges than u32 ordinals address"),
        }
    }
}

impl std::error::Error for ChartError {}

/// A 32-byte blake3 digest: the binary form of a wire `Datum` (`b3:<hex>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DatumId(pub [u8; 32]);

impl DatumId {
    /// Parse the wire form `b3:<64 lowercase hex>`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix("b3:")?;
        if hex.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            let pair = hex.get(i * 2..i * 2 + 2)?;
            *byte = u8::from_str_radix(pair, 16).ok()?;
            if pair.bytes().any(|c| c.is_ascii_uppercase()) {
                return None;
            }
        }
        Some(Self(out))
    }
}

impl std::fmt::Display for DatumId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("b3:")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Ordinal of a definition within one segment. Only meaningful with the
/// segment that minted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ordinal(pub u32);

/// One neighbour, as stored: the other endpoint plus its evidence grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Neighbour {
    /// The other endpoint's ordinal (destination for out-edges, source for in).
    pub other: Ordinal,
    /// Typed edge kind.
    pub relation: grid_wire::Relation,
    /// Evidence grade.
    pub grade: grid_wire::Grade,
}

/// Wire `Relation` as its on-disk tag, and back. The on-disk order is frozen:
/// adding a relation is a format-epoch bump, not an enum edit.
#[must_use]
pub fn relation_tag(r: grid_wire::Relation) -> u8 {
    use grid_wire::Relation as R;
    match r {
        R::Calls => 0,
        R::Constructs => 1,
        R::ReadsField => 2,
        R::WritesField => 3,
        R::Implements => 4,
        R::DispatchesTo => 5,
        R::FlowsInto => 6,
        R::SerializesTo => 7,
        R::DeserializesFrom => 8,
        R::RegistersUnder => 9,
        R::SendsOver => 10,
        R::LinksAgainst => 11,
    }
}

fn relation_of_tag(tag: u8) -> Option<grid_wire::Relation> {
    use grid_wire::Relation as R;
    Some(match tag {
        0 => R::Calls,
        1 => R::Constructs,
        2 => R::ReadsField,
        3 => R::WritesField,
        4 => R::Implements,
        5 => R::DispatchesTo,
        6 => R::FlowsInto,
        7 => R::SerializesTo,
        8 => R::DeserializesFrom,
        9 => R::RegistersUnder,
        10 => R::SendsOver,
        11 => R::LinksAgainst,
        _ => return None,
    })
}

/// Wire `Grade` as its on-disk tag. Strongest evidence is 0, so the byte order
/// is the `at_or_above` filter order.
#[must_use]
pub fn grade_tag(z: grid_wire::Grade) -> u8 {
    use grid_wire::Grade as Z;
    match z {
        Z::Proven => 0,
        Z::Observed => 1,
        Z::Modeled => 2,
        Z::Heuristic => 3,
    }
}

fn grade_of_tag(tag: u8) -> Option<grid_wire::Grade> {
    use grid_wire::Grade as Z;
    Some(match tag {
        0 => Z::Proven,
        1 => Z::Observed,
        2 => Z::Modeled,
        3 => Z::Heuristic,
        _ => return None,
    })
}

/// Wire `Lang` as its on-disk tag. 255 means "the survey did not say".
#[must_use]
pub fn lang_tag(l: Option<grid_wire::Lang>) -> u8 {
    use grid_wire::Lang as L;
    match l {
        Some(L::Ts) => 0,
        Some(L::Js) => 1,
        Some(L::Py) => 2,
        Some(L::Rs) => 3,
        None => 255,
    }
}

/// The lang tag a segment stores, decoded. `None` means either "unknown to
/// this version" or "the survey did not say" — [`lang_tag_known`] separates
/// them, so validation can refuse the first without refusing the second.
fn lang_of_tag(tag: u8) -> Option<grid_wire::Lang> {
    use grid_wire::Lang as L;
    Some(match tag {
        0 => L::Ts,
        1 => L::Js,
        2 => L::Py,
        3 => L::Rs,
        _ => return None,
    })
}

fn lang_tag_known(tag: u8) -> bool {
    tag == 255 || lang_of_tag(tag).is_some()
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    let raw: [u8; 8] = bytes.get(at..at + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(raw))
}

/// Where a section lives in the image. Validated at parse time: `start..end`
/// is always a real range of the backing bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Span {
    start: usize,
    end: usize,
}

/// A validated, immutable segment over owned or mapped bytes.
///
/// `B` is whatever holds the bytes — `Vec<u8>` in tests, a `Mmap` in
/// `chart-io`. Parsing borrows nothing, so a mapped segment is not a
/// self-referential struct and needs no unsafe to keep around.
#[derive(Debug)]
pub struct Segment<B> {
    bytes: B,
    spans: [Span; SECTION_COUNT],
    def_count: u32,
    edge_count: u32,
    epoch: u32,
}

/// One definition's location, read back off an epoch-2 segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefLoc<'a> {
    /// Repo-relative path, slash-separated.
    pub path: &'a str,
    /// 1-based line of the definition's first byte at survey time.
    pub line: u32,
    /// 1-based line of the definition's last byte — containment's other end.
    pub line_end: u32,
    /// Byte range of the definition in its file.
    pub span: std::ops::Range<u32>,
}

impl<B: AsRef<[u8]>> Segment<B> {
    /// Validate an image and take ownership of its bytes.
    ///
    /// This is the whole trust boundary for claim C1. Every later accessor is
    /// total because this function refused everything that would not be.
    ///
    /// # Errors
    /// [`ChartError`] describing the first structural claim the bytes failed.
    pub fn parse(bytes: B) -> Result<Self, ChartError> {
        let b = bytes.as_ref();
        let header = b.get(..HEADER_LEN).ok_or(ChartError::Truncated)?;
        if header.get(..8) != Some(&MAGIC[..]) {
            return Err(ChartError::BadMagic);
        }
        let epoch = read_u32(header, 8).ok_or(ChartError::Truncated)?;
        let expected_sections = match epoch {
            1 => V1_SECTION_COUNT,
            2 => V2_SECTION_COUNT,
            3 => V3_SECTION_COUNT,
            4 => V4_SECTION_COUNT,
            5 => SECTION_COUNT,
            other => return Err(ChartError::BadEpoch(other)),
        };
        let sections = read_u32(header, 12).ok_or(ChartError::Truncated)?;
        if sections as usize != expected_sections {
            return Err(ChartError::BadSectionCount(sections));
        }
        let def_count = read_u32(header, 16).ok_or(ChartError::Truncated)?;
        let edge_count = read_u32(header, 20).ok_or(ChartError::Truncated)?;

        // The checksum covers the header with its own field zeroed plus every
        // byte after the header: truncation and tampering are the same event.
        let stored: [u8; 32] = header
            .get(32..64)
            .and_then(|s| s.try_into().ok())
            .ok_or(ChartError::Truncated)?;
        if checksum(b) != stored {
            return Err(ChartError::ChecksumMismatch);
        }

        let table_len = expected_sections
            .checked_mul(SECTION_ENTRY_LEN)
            .ok_or(ChartError::Truncated)?;
        let table_end = HEADER_LEN
            .checked_add(table_len)
            .ok_or(ChartError::Truncated)?;
        if b.len() < table_end {
            return Err(ChartError::Truncated);
        }
        let mut spans = [Span::default(); SECTION_COUNT];
        for (i, span) in spans.iter_mut().enumerate().take(expected_sections) {
            let at = HEADER_LEN + i * SECTION_ENTRY_LEN;
            let off = read_u64(b, at).ok_or(ChartError::Truncated)?;
            let len = read_u64(b, at + 8).ok_or(ChartError::Truncated)?;
            let start = usize::try_from(off).map_err(|_| ChartError::Truncated)?;
            let size = usize::try_from(len).map_err(|_| ChartError::Truncated)?;
            let end = start.checked_add(size).ok_or(ChartError::Truncated)?;
            if start < table_end || end > b.len() {
                return Err(ChartError::Truncated);
            }
            *span = Span { start, end };
        }
        // Two sections claiming one byte would make an accessor's answer
        // depend on which one you asked. Refuse rather than pick. Absent
        // epoch-2 sections are zero-length and cannot overlap anything.
        let mut order: Vec<Span> = spans.iter().copied().filter(|s| s.end > s.start).collect();
        order.sort_unstable_by_key(|s| s.start);
        for pair in order.windows(2) {
            if pair[0].end > pair[1].start {
                return Err(ChartError::OverlappingSections);
            }
        }

        let seg = Self {
            bytes,
            spans,
            def_count,
            edge_count,
            epoch,
        };
        seg.validate()?;
        Ok(seg)
    }

    fn section(&self, s: Section) -> &[u8] {
        let span = self.spans[s as usize];
        // Parse proved this range; `get` keeps the proof local anyway.
        self.bytes.as_ref().get(span.start..span.end).unwrap_or(&[])
    }

    fn validate(&self) -> Result<(), ChartError> {
        let defs = self.def_count as usize;
        let edges = self.edge_count as usize;
        let sized = |s: Section, want: usize| {
            if self.section(s).len() == want {
                Ok(())
            } else {
                Err(ChartError::BadSectionSize(s))
            }
        };
        let rows = defs.checked_add(1).ok_or(ChartError::TooLarge)?;
        let row_bytes = rows.checked_mul(4).ok_or(ChartError::TooLarge)?;
        let edge_bytes = edges.checked_mul(4).ok_or(ChartError::TooLarge)?;
        let attr_bytes = edges.checked_mul(2).ok_or(ChartError::TooLarge)?;
        sized(
            Section::Datums,
            defs.checked_mul(32).ok_or(ChartError::TooLarge)?,
        )?;
        sized(Section::OutOffsets, row_bytes)?;
        sized(Section::OutTargets, edge_bytes)?;
        sized(Section::OutAttrs, attr_bytes)?;
        sized(Section::InOffsets, row_bytes)?;
        sized(Section::InTargets, edge_bytes)?;
        sized(Section::InAttrs, attr_bytes)?;
        sized(Section::CallsignOffsets, row_bytes)?;
        sized(Section::Langs, defs)?;
        if self.epoch >= 2 {
            sized(Section::PathOffsets, row_bytes)?;
            sized(
                Section::DefLocs,
                defs.checked_mul(16).ok_or(ChartError::TooLarge)?,
            )?;
            sized(
                Section::DefShapes,
                defs.checked_mul(32).ok_or(ChartError::TooLarge)?,
            )?;
            sized(
                Section::DefMetrics,
                defs.checked_mul(4).ok_or(ChartError::TooLarge)?,
            )?;
            let paths_len = u32::try_from(self.section(Section::PathBytes).len())
                .map_err(|_| ChartError::TooLarge)?;
            self.check_csr(Section::PathOffsets, Section::PathBytes, paths_len)?;
            std::str::from_utf8(self.section(Section::PathBytes))
                .map_err(|_| ChartError::BadUtf8)?;
        }
        if self.epoch >= 3 {
            sized(
                Section::DefSigs,
                defs.checked_mul(32).ok_or(ChartError::TooLarge)?,
            )?;
        }
        if self.epoch >= 4 {
            self.validate_cfgs(defs)?;
        }
        if self.epoch >= 5 {
            sized(
                Section::DefDeclined,
                defs.checked_mul(8).ok_or(ChartError::TooLarge)?,
            )?;
        }

        // Ordinals are a binary search over the digests, which is only a
        // function if they are strictly ascending.
        let datums = self.section(Section::Datums);
        for w in datums.chunks_exact(32).collect::<Vec<_>>().windows(2) {
            if w[0] >= w[1] {
                return Err(ChartError::UnsortedDatums);
            }
        }

        for (offsets, targets) in [
            (Section::OutOffsets, Section::OutTargets),
            (Section::InOffsets, Section::InTargets),
        ] {
            self.check_csr(offsets, targets, self.edge_count)?;
            for raw in self.section(targets).chunks_exact(4) {
                let ord = u32::from_le_bytes(raw.try_into().unwrap_or_default());
                if ord >= self.def_count {
                    return Err(ChartError::OrdinalOutOfRange);
                }
            }
        }
        for attrs in [Section::OutAttrs, Section::InAttrs] {
            for raw in self.section(attrs).chunks_exact(2) {
                let packed = u16::from_le_bytes(raw.try_into().unwrap_or_default());
                let rel = u8::try_from(packed >> 8).unwrap_or(u8::MAX);
                let grade = u8::try_from(packed & 0xff).unwrap_or(u8::MAX);
                if relation_of_tag(rel).is_none() || grade_of_tag(grade).is_none() {
                    return Err(ChartError::BadAttrs);
                }
            }
        }

        let names_len = u32::try_from(self.section(Section::CallsignBytes).len())
            .map_err(|_| ChartError::TooLarge)?;
        self.check_csr(Section::CallsignOffsets, Section::CallsignBytes, names_len)?;
        // Validate UTF-8 once, here, so `callsign` can be infallible.
        std::str::from_utf8(self.section(Section::CallsignBytes))
            .map_err(|_| ChartError::BadUtf8)?;
        for &tag in self.section(Section::Langs) {
            if !lang_tag_known(tag) {
                return Err(ChartError::BadAttrs);
            }
        }
        Ok(())
    }

    /// The epoch-4 cfg column: one index per def into a table that always
    /// holds entry 0 (the empty predicate), so index 0 is a real row and
    /// never an out-of-range read; the table's offsets are a monotone CSR;
    /// the bytes are UTF-8.
    fn validate_cfgs(&self, defs: usize) -> Result<(), ChartError> {
        if self.section(Section::DefCfgs).len()
            != defs.checked_mul(4).ok_or(ChartError::TooLarge)?
        {
            return Err(ChartError::BadSectionSize(Section::DefCfgs));
        }
        let offsets = self.section(Section::CfgOffsets);
        if offsets.len() < 8 || !offsets.len().is_multiple_of(4) {
            return Err(ChartError::BadSectionSize(Section::CfgOffsets));
        }
        let table_len = u32::try_from(self.section(Section::CfgBytes).len())
            .map_err(|_| ChartError::TooLarge)?;
        self.check_csr(Section::CfgOffsets, Section::CfgBytes, table_len)?;
        std::str::from_utf8(self.section(Section::CfgBytes)).map_err(|_| ChartError::BadUtf8)?;
        let cfg_count = offsets.len() / 4 - 1;
        for raw in self.section(Section::DefCfgs).chunks_exact(4) {
            let idx = u32::from_le_bytes(raw.try_into().unwrap_or_default());
            if idx as usize >= cfg_count {
                return Err(ChartError::BadCfgIndex);
            }
        }
        Ok(())
    }

    fn check_csr(&self, offsets: Section, target: Section, total: u32) -> Result<(), ChartError> {
        let mut prev = 0u32;
        let raw = self.section(offsets);
        for (i, chunk) in raw.chunks_exact(4).enumerate() {
            let v = u32::from_le_bytes(chunk.try_into().unwrap_or_default());
            if v < prev || v > total {
                return Err(ChartError::BadCsr(offsets));
            }
            if i + 1 == raw.len() / 4 && v != total {
                return Err(ChartError::BadCsr(offsets));
            }
            prev = v;
        }
        let _ = target;
        Ok(())
    }

    /// How many definitions this segment holds.
    #[must_use]
    pub fn def_count(&self) -> u32 {
        self.def_count
    }

    /// How many edges this segment holds (each stored twice, once per direction).
    #[must_use]
    pub fn edge_count(&self) -> u32 {
        self.edge_count
    }

    /// The raw image, for checksumming and republication.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// The byte holder itself, so the layer that chose it can ask what it is.
    #[must_use]
    pub fn holder(&self) -> &B {
        &self.bytes
    }

    /// Digest of the definition at `ord`, or `None` if no such definition.
    #[must_use]
    pub fn datum(&self, ord: Ordinal) -> Option<DatumId> {
        let at = (ord.0 as usize).checked_mul(32)?;
        let raw: [u8; 32] = self
            .section(Section::Datums)
            .get(at..at + 32)?
            .try_into()
            .ok()?;
        Some(DatumId(raw))
    }

    /// Ordinal of `datum` in this segment: binary search over ascending digests.
    #[must_use]
    pub fn ordinal(&self, datum: &DatumId) -> Option<Ordinal> {
        let datums = self.section(Section::Datums);
        let (mut lo, mut hi) = (0usize, datums.len() / 32);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let probe = datums.get(mid * 32..mid * 32 + 32)?;
            match probe.cmp(&datum.0[..]) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(Ordinal(u32::try_from(mid).ok()?)),
            }
        }
        None
    }

    /// Human name of the definition at `ord`.
    #[must_use]
    pub fn callsign(&self, ord: Ordinal) -> Option<&str> {
        let (start, end) = self.row(Section::CallsignOffsets, ord)?;
        let raw = self
            .section(Section::CallsignBytes)
            .get(start as usize..end as usize)?;
        std::str::from_utf8(raw).ok()
    }

    /// Language adapter that produced the definition at `ord`, if it said.
    #[must_use]
    pub fn lang(&self, ord: Ordinal) -> Option<grid_wire::Lang> {
        let tag = *self.section(Section::Langs).get(ord.0 as usize)?;
        lang_of_tag(tag)
    }

    /// Where the definition at `ord` lives. `None` on an epoch-1 image or
    /// for a definition whose survey recorded no location.
    #[must_use]
    pub fn location(&self, ord: Ordinal) -> Option<DefLoc<'_>> {
        let (start, end) = self.row(Section::PathOffsets, ord)?;
        let raw = self
            .section(Section::PathBytes)
            .get(start as usize..end as usize)?;
        let path = std::str::from_utf8(raw).ok()?;
        if path.is_empty() {
            return None;
        }
        let at = (ord.0 as usize).checked_mul(16)?;
        let locs = self.section(Section::DefLocs);
        let field = |k: usize| {
            locs.get(at + k * 4..at + k * 4 + 4)
                .and_then(|b| b.try_into().ok())
                .map(u32::from_le_bytes)
        };
        Some(DefLoc {
            path,
            line: field(0)?,
            line_end: field(1)?,
            span: field(2)?..field(3)?,
        })
    }

    /// The name-erased shape of the definition at `ord` — the rename-join
    /// key. `None` on epoch-1 images and for the all-zero absent sentinel.
    #[must_use]
    pub fn shape(&self, ord: Ordinal) -> Option<[u8; 32]> {
        let at = (ord.0 as usize).checked_mul(32)?;
        let raw = self.section(Section::DefShapes).get(at..at + 32)?;
        let shape: [u8; 32] = raw.try_into().ok()?;
        (shape != [0u8; 32]).then_some(shape)
    }

    /// The interface identity of the definition at `ord` — what a caller can
    /// observe without the body. `None` on epoch-1 and epoch-2 images and
    /// for the all-zero absent sentinel: an image that predates the column
    /// answers *unknown*, never a fabricated hash.
    #[must_use]
    pub fn sig(&self, ord: Ordinal) -> Option<[u8; 32]> {
        let at = (ord.0 as usize).checked_mul(32)?;
        let raw = self.section(Section::DefSigs).get(at..at + 32)?;
        let sig: [u8; 32] = raw.try_into().ok()?;
        (sig != [0u8; 32]).then_some(sig)
    }

    /// The cfg predicate conditioning the definition at `ord`, as written —
    /// `all(...)`-joined over the item and its enclosing modules, empty when
    /// nothing conditions it. `None` on an image that predates epoch 4: an
    /// older snapshot answers *unknown*, never "unconditional", because the
    /// one consumer (`grid check --compile`) would otherwise call every
    /// `#[cfg(test)]` caller compiled.
    #[must_use]
    pub fn cfg(&self, ord: Ordinal) -> Option<&str> {
        let idx = read_u32(
            self.section(Section::DefCfgs),
            (ord.0 as usize).checked_mul(4)?,
        )? as usize;
        let offsets = self.section(Section::CfgOffsets);
        let start = read_u32(offsets, idx.checked_mul(4)?)?;
        let end = read_u32(offsets, idx.checked_add(1)?.checked_mul(4)?)?;
        let raw = self
            .section(Section::CfgBytes)
            .get(start as usize..end as usize)?;
        std::str::from_utf8(raw).ok()
    }

    /// The references the survey declined inside the definition at `ord`,
    /// split by why. `None` on an image that predates epoch 5: an older
    /// snapshot answers *unknown*, never "nothing declined" — a consumer
    /// summing a closure's holes would otherwise report a clean zero for a
    /// chart that never counted.
    #[must_use]
    pub fn declined(&self, ord: Ordinal) -> Option<Declined> {
        if self.epoch < 5 {
            return None;
        }
        let at = (ord.0 as usize).checked_mul(8)?;
        let raw = self
            .section(Section::DefDeclined)
            .get(at..at.checked_add(8)?)?;
        let column = |i: usize| {
            raw.get(i..i + 2)
                .and_then(|b| b.try_into().ok())
                .map(u16::from_le_bytes)
        };
        Some(Declined {
            external: column(0)?,
            method: column(2)?,
            internal: column(4)?,
            refused: column(6)?,
        })
    }

    /// Cyclomatic complexity of the definition at `ord`, if surveyed. A
    /// real complexity is at least 1, so zero is the absent sentinel.
    #[must_use]
    pub fn complexity(&self, ord: Ordinal) -> Option<u16> {
        let at = (ord.0 as usize).checked_mul(4)?;
        let raw = self.section(Section::DefMetrics).get(at..at + 2)?;
        let cx = u16::from_le_bytes(raw.try_into().ok()?);
        (cx > 0).then_some(cx)
    }

    fn row(&self, offsets: Section, ord: Ordinal) -> Option<(u32, u32)> {
        let i = ord.0 as usize;
        if i >= self.def_count as usize {
            return None;
        }
        let raw = self.section(offsets);
        let start = read_u32(raw, i.checked_mul(4)?)?;
        let end = read_u32(raw, i.checked_add(1)?.checked_mul(4)?)?;
        if end < start {
            return None;
        }
        Some((start, end))
    }

    fn neighbours(
        &self,
        offsets: Section,
        targets: Section,
        attrs: Section,
        ord: Ordinal,
    ) -> impl Iterator<Item = Neighbour> + '_ {
        let (start, end) = self.row(offsets, ord).unwrap_or((0, 0));
        let t = self.section(targets);
        let a = self.section(attrs);
        (start as usize..end as usize).filter_map(move |i| {
            let other = read_u32(t, i.checked_mul(4)?)?;
            let raw: [u8; 2] = a.get(i * 2..i * 2 + 2)?.try_into().ok()?;
            let packed = u16::from_le_bytes(raw);
            Some(Neighbour {
                other: Ordinal(other),
                relation: relation_of_tag(u8::try_from(packed >> 8).ok()?)?,
                grade: grade_of_tag(u8::try_from(packed & 0xff).ok()?)?,
            })
        })
    }

    /// Edges out of `ord`, in stored order: a contiguous CSR row scan.
    pub fn out_edges(&self, ord: Ordinal) -> impl Iterator<Item = Neighbour> + '_ {
        self.neighbours(
            Section::OutOffsets,
            Section::OutTargets,
            Section::OutAttrs,
            ord,
        )
    }

    /// Edges into `ord` — the same edges, indexed by destination.
    pub fn in_edges(&self, ord: Ordinal) -> impl Iterator<Item = Neighbour> + '_ {
        self.neighbours(
            Section::InOffsets,
            Section::InTargets,
            Section::InAttrs,
            ord,
        )
    }
}

/// Content checksum of an image: blake3 over the whole thing with the
/// checksum field itself read as zeros.
#[must_use]
pub fn checksum(image: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"grid-segment/v1");
    let head = image.get(..32).unwrap_or(image);
    h.update(head);
    h.update(&[0u8; 32]);
    h.update(image.get(HEADER_LEN..).unwrap_or(&[]));
    *h.finalize().as_bytes()
}

#[cfg(kani)]
mod proofs;
