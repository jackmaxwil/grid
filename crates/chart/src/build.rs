//! Building a segment image. Deterministic by construction (claim C2): the
//! input is treated as a *set*, sorted by content, so two surveys that found
//! the same graph write byte-identical bytes regardless of discovery order.
use crate::{
    checksum, lang_tag, relation_tag, zoc_tag, ChartError, DatumId, Section, FORMAT_EPOCH,
    HEADER_LEN, MAGIC, SECTION_COUNT, SECTION_ENTRY_LEN,
};

/// A definition to intern: identity, name, and the adapter that found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Def {
    /// Content identity — the ordinal is derived from this and nothing else.
    pub datum: DatumId,
    /// Fully qualified human name at survey time (mutable; not identity).
    pub callsign: String,
    /// Language adapter, if the survey named one.
    pub lang: Option<grid_wire::Lang>,
    /// Epoch-2 facts: location, shape, complexity. `None` writes the absent
    /// sentinels, which read back as `None`.
    pub facts: Option<DefFacts>,
}

/// The epoch-2 per-definition columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefFacts {
    /// Repo-relative path, slash-separated.
    pub path: String,
    /// 1-based line of the definition's first byte at survey time.
    pub line: u32,
    /// 1-based line of its last byte.
    pub line_end: u32,
    /// Byte range in the file.
    pub span: (u32, u32),
    /// Name-erased canonical hash — the rename-join key.
    pub shape: [u8; 32],
    /// Cyclomatic complexity.
    pub complexity: u16,
    /// Epoch-3 interface identity — the frontend's canonical sig hash.
    /// All-zero writes the absent sentinel, which reads back as `None`.
    pub sig: [u8; 32],
}

/// An edge to store. Edges naming an unknown datum are dropped by [`build`]:
/// a segment is closed, so a dangling ordinal cannot be represented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// Source definition.
    pub src: DatumId,
    /// Destination definition.
    pub dst: DatumId,
    /// Typed edge kind.
    pub relation: grid_wire::Relation,
    /// Evidence grade.
    pub zoc: grid_wire::Zoc,
}

/// Serialize defs and edges into a validated segment image.
///
/// Duplicate defs collapse by datum (first callsign wins after sorting, which
/// is content order, so the choice is deterministic); duplicate edges collapse
/// entirely. Edges whose endpoints are not both present are dropped.
///
/// # Errors
/// [`ChartError::TooLarge`] if the graph exceeds what u32 ordinals address.
pub fn build(defs: &[Def], edges: &[Edge]) -> Result<Vec<u8>, ChartError> {
    let mut defs: Vec<&Def> = defs.iter().collect();
    defs.sort_unstable_by(|a, b| {
        a.datum
            .cmp(&b.datum)
            .then_with(|| a.callsign.cmp(&b.callsign))
    });
    defs.dedup_by(|a, b| a.datum == b.datum);
    let def_count = u32::try_from(defs.len()).map_err(|_| ChartError::TooLarge)?;

    let ordinal = |d: &DatumId| -> Option<u32> {
        defs.binary_search_by(|probe| probe.datum.cmp(d))
            .ok()
            .and_then(|i| u32::try_from(i).ok())
    };

    // (src, dst, relation, zoc) as ordinals, sorted so CSR rows are contiguous
    // and the byte image is a function of the edge *set*.
    let mut rows: Vec<(u32, u32, u16)> = edges
        .iter()
        .filter_map(|e| {
            let attrs = (u16::from(relation_tag(e.relation)) << 8) | u16::from(zoc_tag(e.zoc));
            Some((ordinal(&e.src)?, ordinal(&e.dst)?, attrs))
        })
        .collect();
    rows.sort_unstable();
    rows.dedup();
    let edge_count = u32::try_from(rows.len()).map_err(|_| ChartError::TooLarge)?;

    let out = &rows;
    let mut inc: Vec<(u32, u32, u16)> = rows.iter().map(|&(s, d, a)| (d, s, a)).collect();
    inc.sort_unstable();

    let sections = sections(&defs, out, &inc, def_count)?;

    let table_len = SECTION_COUNT * SECTION_ENTRY_LEN;
    let mut image = Vec::with_capacity(HEADER_LEN + table_len);
    image.extend_from_slice(MAGIC);
    image.extend_from_slice(&FORMAT_EPOCH.to_le_bytes());
    let count = u32::try_from(SECTION_COUNT).map_err(|_| ChartError::TooLarge)?;
    image.extend_from_slice(&count.to_le_bytes());
    image.extend_from_slice(&def_count.to_le_bytes());
    image.extend_from_slice(&edge_count.to_le_bytes());
    image.resize(HEADER_LEN, 0); // reserved + checksum, filled in below

    let mut offset = HEADER_LEN + table_len;
    for s in &sections {
        let at = u64::try_from(offset).map_err(|_| ChartError::TooLarge)?;
        let len = u64::try_from(s.len()).map_err(|_| ChartError::TooLarge)?;
        image.extend_from_slice(&at.to_le_bytes());
        image.extend_from_slice(&len.to_le_bytes());
        offset += s.len();
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

/// The section payloads, in table order. Split out of [`build`] so the one
/// place that decides what a section *contains* is one screen long: adding a
/// column is an entry here and an arm in [`Section`], nothing else.
fn sections(
    defs: &[&Def],
    out: &[(u32, u32, u16)],
    inc: &[(u32, u32, u16)],
    def_count: u32,
) -> Result<[Vec<u8>; SECTION_COUNT], ChartError> {
    fn path_of(d: &Def) -> &str {
        d.facts.as_ref().map_or("", |f| f.path.as_str())
    }
    Ok([
        defs.iter().flat_map(|d| d.datum.0).collect(),
        csr_offsets(def_count, out.iter().map(|r| r.0)),
        out.iter().flat_map(|r| r.1.to_le_bytes()).collect(),
        out.iter().flat_map(|r| r.2.to_le_bytes()).collect(),
        csr_offsets(def_count, inc.iter().map(|r| r.0)),
        inc.iter().flat_map(|r| r.1.to_le_bytes()).collect(),
        inc.iter().flat_map(|r| r.2.to_le_bytes()).collect(),
        csr_byte_offsets(defs.iter().map(|d| d.callsign.len()))?,
        defs.iter()
            .flat_map(|d| d.callsign.as_bytes().iter().copied())
            .collect(),
        defs.iter().map(|d| lang_tag(d.lang)).collect(),
        csr_byte_offsets(defs.iter().map(|d| path_of(d).len()))?,
        defs.iter()
            .flat_map(|d| path_of(d).as_bytes().iter().copied())
            .collect(),
        defs.iter()
            .flat_map(|d| {
                let (line, line_end, s, e) = d
                    .facts
                    .as_ref()
                    .map_or((0, 0, 0, 0), |f| (f.line, f.line_end, f.span.0, f.span.1));
                [
                    line.to_le_bytes(),
                    line_end.to_le_bytes(),
                    s.to_le_bytes(),
                    e.to_le_bytes(),
                ]
            })
            .flatten()
            .collect(),
        defs.iter()
            .flat_map(|d| d.facts.as_ref().map_or([0u8; 32], |f| f.shape))
            .collect(),
        defs.iter()
            .flat_map(|d| {
                let cx = d.facts.as_ref().map_or(0u16, |f| f.complexity);
                let mut m = [0u8; 4];
                m[..2].copy_from_slice(&cx.to_le_bytes());
                m
            })
            .collect(),
        defs.iter()
            .flat_map(|d| d.facts.as_ref().map_or([0u8; 32], |f| f.sig))
            .collect(),
    ])
}

/// Row starts for `count` rows over an ascending stream of row indexes.
fn csr_offsets(count: u32, rows: impl Iterator<Item = u32>) -> Vec<u8> {
    let mut counts = vec![0u32; count as usize];
    for r in rows {
        if let Some(c) = counts.get_mut(r as usize) {
            *c += 1;
        }
    }
    let mut out = Vec::with_capacity((count as usize + 1) * 4);
    let mut acc = 0u32;
    for c in counts {
        out.extend_from_slice(&acc.to_le_bytes());
        acc += c;
    }
    out.extend_from_slice(&acc.to_le_bytes());
    out
}

/// Row starts for a concatenated byte arena.
fn csr_byte_offsets(lens: impl Iterator<Item = usize>) -> Result<Vec<u8>, ChartError> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    for len in lens {
        out.extend_from_slice(&acc.to_le_bytes());
        acc = acc
            .checked_add(u32::try_from(len).map_err(|_| ChartError::TooLarge)?)
            .ok_or(ChartError::TooLarge)?;
    }
    out.extend_from_slice(&acc.to_le_bytes());
    Ok(out)
}

impl Section {
    /// Section name, for diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Datums => "datums",
            Self::OutOffsets => "out_offsets",
            Self::OutTargets => "out_targets",
            Self::OutAttrs => "out_attrs",
            Self::InOffsets => "in_offsets",
            Self::InTargets => "in_targets",
            Self::InAttrs => "in_attrs",
            Self::CallsignOffsets => "callsign_offsets",
            Self::CallsignBytes => "callsign_bytes",
            Self::Langs => "langs",
            Self::PathOffsets => "path_offsets",
            Self::PathBytes => "path_bytes",
            Self::DefLocs => "def_locs",
            Self::DefShapes => "def_shapes",
            Self::DefMetrics => "def_metrics",
            Self::DefSigs => "def_sigs",
        }
    }
}
