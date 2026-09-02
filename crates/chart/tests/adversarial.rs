//! Claim C1 evidence: no input to the loader causes UB — or a panic.
//!
//! The fuzz target (`fuzz/fuzz_targets/segment.rs`) covers the unbounded case;
//! these are the named attacks, kept as regressions so a refactor cannot
//! quietly re-open one. Every case must produce a typed `ChartError`, never a
//! panic and never a `Segment` whose accessors could read out of bounds.
use grid_chart::{build, ChartError, DatumId, Def, Edge, Ordinal, Section, Segment, HEADER_LEN};
use grid_wire::{Grade, Lang, Relation};

fn sample() -> Vec<u8> {
    let defs: Vec<Def> = (0u8..6)
        .map(|i| Def {
            datum: DatumId([i; 32]),
            callsign: format!("pkg.mod.f{i}"),
            lang: Some(Lang::Py),
            facts: None,
        })
        .collect();
    let edges: Vec<Edge> = (0u8..5)
        .map(|i| Edge {
            src: DatumId([i; 32]),
            dst: DatumId([i + 1; 32]),
            relation: Relation::Calls,
            grade: Grade::Proven,
        })
        .collect();
    build(&defs, &edges).unwrap()
}

/// Overwrite the u64 at `at` and re-checksum, so the test attacks the field
/// under test and not the integrity check that would otherwise mask it.
fn poke_u64(image: &mut [u8], at: usize, v: u64) {
    image[at..at + 8].copy_from_slice(&v.to_le_bytes());
    reseal(image);
}

fn poke_u32(image: &mut [u8], at: usize, v: u32) {
    image[at..at + 4].copy_from_slice(&v.to_le_bytes());
    reseal(image);
}

fn reseal(image: &mut [u8]) {
    let sum = grid_chart::checksum(image);
    image[32..64].copy_from_slice(&sum);
}

fn section_offset_field(s: Section) -> usize {
    HEADER_LEN + s as usize * 16
}

fn err(image: Vec<u8>) -> ChartError {
    Segment::parse(image).expect_err("hostile image must be refused")
}

#[test]
fn valid_sample_parses() {
    let seg = Segment::parse(sample()).unwrap();
    assert_eq!(seg.def_count(), 6);
    assert_eq!(seg.edge_count(), 5);
}

#[test]
fn empty_and_short_inputs() {
    assert_eq!(err(Vec::new()), ChartError::Truncated);
    assert_eq!(err(vec![0u8; 8]), ChartError::Truncated);
    assert_eq!(err(vec![0u8; HEADER_LEN]), ChartError::BadMagic);
    assert_eq!(err(vec![0xff; 4096]), ChartError::BadMagic);
}

#[test]
fn truncation_at_every_length_is_refused() {
    let full = sample();
    for cut in 0..full.len() {
        // Truncation must be an error at every single prefix length; a
        // truncated image that parsed would be a mapping over bytes that
        // are not there.
        let _ = err(full[..cut].to_vec());
    }
}

#[test]
fn foreign_epoch_is_refused() {
    let mut image = sample();
    poke_u32(&mut image, 8, 99);
    assert_eq!(err(image), ChartError::BadEpoch(99));
}

#[test]
fn wrong_section_count_is_refused() {
    let mut image = sample();
    poke_u32(&mut image, 12, 3);
    assert_eq!(err(image), ChartError::BadSectionCount(3));
}

#[test]
fn tampering_without_resealing_is_caught() {
    let mut image = sample();
    let last = image.len() - 1;
    image[last] ^= 0xff;
    assert_eq!(err(image), ChartError::ChecksumMismatch);
}

#[test]
fn section_running_past_the_end_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::Datums);
    poke_u64(&mut image, at + 8, u64::MAX);
    assert_eq!(err(image), ChartError::Truncated);
}

#[test]
fn section_offset_overflow_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::OutTargets);
    poke_u64(&mut image, at, u64::MAX);
    assert_eq!(err(image), ChartError::Truncated);
}

#[test]
fn section_pointing_into_the_header_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::Langs);
    poke_u64(&mut image, at, 0);
    assert_eq!(err(image), ChartError::Truncated);
}

#[test]
fn overlapping_sections_are_refused() {
    let mut image = sample();
    let langs = section_offset_field(Section::Langs);
    let datums = section_offset_field(Section::Datums);
    let datums_off = u64::from_le_bytes(image[datums..datums + 8].try_into().unwrap());
    poke_u64(&mut image, langs, datums_off);
    assert_eq!(err(image), ChartError::OverlappingSections);
}

#[test]
fn declared_counts_must_match_section_sizes() {
    let mut image = sample();
    poke_u32(&mut image, 16, 5000);
    assert!(matches!(
        err(image),
        ChartError::BadSectionSize(_) | ChartError::Truncated
    ));
}

#[test]
fn edge_count_lie_is_refused() {
    let mut image = sample();
    poke_u32(&mut image, 20, 4);
    assert!(matches!(err(image), ChartError::BadSectionSize(_)));
}

#[test]
fn out_of_range_ordinal_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::OutTargets);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    poke_u32(&mut image, off, 9999);
    assert_eq!(err(image), ChartError::OrdinalOutOfRange);
}

#[test]
fn non_monotone_csr_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::OutOffsets);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    // Row 1 starting past the end of the target array is the classic OOB read.
    poke_u32(&mut image, off + 4, 4_000_000);
    assert_eq!(err(image), ChartError::BadCsr(Section::OutOffsets));
}

#[test]
fn csr_row_going_backwards_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::InOffsets);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    poke_u32(&mut image, off, 3);
    assert_eq!(err(image), ChartError::BadCsr(Section::InOffsets));
}

#[test]
fn unsorted_datums_are_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::Datums);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    // Duplicate digests make `ordinal` a relation, not a function.
    image.copy_within(off..off + 32, off + 32);
    reseal(&mut image);
    assert_eq!(err(image), ChartError::UnsortedDatums);
}

#[test]
fn unknown_relation_or_zoc_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::OutAttrs);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    image[off + 1] = 200;
    reseal(&mut image);
    assert_eq!(err(image), ChartError::BadAttrs);
}

#[test]
fn unknown_lang_tag_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::Langs);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    image[off] = 7;
    reseal(&mut image);
    assert_eq!(err(image), ChartError::BadAttrs);
}

#[test]
fn invalid_utf8_callsign_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::CallsignBytes);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    image[off] = 0xff;
    reseal(&mut image);
    assert_eq!(err(image), ChartError::BadUtf8);
}

/// The epoch-4 cfg column: a def's index must name a table entry, the
/// table's offsets must be a monotone CSR, and the bytes must be UTF-8.
#[test]
fn cfg_index_past_the_table_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::DefCfgs);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    // The sample's table holds only entry 0 (no def is conditioned), so
    // index 1 is one past the end — the classic off-by-one read.
    poke_u32(&mut image, off, 1);
    assert_eq!(err(image), ChartError::BadCfgIndex);
}

#[test]
fn cfg_table_running_backwards_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::CfgOffsets);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    poke_u32(&mut image, off, 7);
    assert_eq!(err(image), ChartError::BadCsr(Section::CfgOffsets));
}

#[test]
fn cfg_table_with_no_empty_entry_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::CfgOffsets);
    // Zero-length offsets: no entry 0, so every def's index 0 would dangle.
    poke_u64(&mut image, at + 8, 0);
    assert!(matches!(
        err(image),
        ChartError::BadSectionSize(Section::CfgOffsets) | ChartError::OverlappingSections
    ));
}

#[test]
fn invalid_utf8_cfg_is_refused() {
    let defs = vec![Def {
        datum: DatumId([1; 32]),
        callsign: "a".into(),
        lang: None,
        facts: Some(grid_chart::DefFacts {
            path: "a.rs".into(),
            line: 1,
            line_end: 1,
            span: (0, 1),
            shape: [0; 32],
            complexity: 1,
            sig: [0; 32],
            cfg: "test".into(),
        }),
    }];
    let mut image = build(&defs, &[]).unwrap();
    let at = section_offset_field(Section::CfgBytes);
    let off = usize::try_from(u64::from_le_bytes(image[at..at + 8].try_into().unwrap())).unwrap();
    image[off] = 0xff;
    reseal(&mut image);
    assert_eq!(err(image), ChartError::BadUtf8);
}

/// The epoch-5 declined column: exactly eight bytes per definition. A
/// length that is not — one row short, one byte over — is refused before
/// any accessor can read a half row.
#[test]
fn declined_column_of_the_wrong_length_is_refused() {
    let mut image = sample();
    let at = section_offset_field(Section::DefDeclined);
    let len = u64::from_le_bytes(image[at + 8..at + 16].try_into().unwrap());
    assert_eq!(len, 6 * 8, "six defs, eight bytes each");
    poke_u64(&mut image, at + 8, len - 8);
    assert_eq!(err(image), ChartError::BadSectionSize(Section::DefDeclined));
    let mut image = sample();
    poke_u64(&mut image, at + 8, len - 1);
    assert_eq!(err(image), ChartError::BadSectionSize(Section::DefDeclined));
}

#[test]
fn accessors_are_total_on_wild_ordinals() {
    let seg = Segment::parse(sample()).unwrap();
    // Past the end, at the boundary, and at u32::MAX: all answer, none panic.
    for ord in [0, 5, 6, 7, 1000, u32::MAX] {
        let ord = Ordinal(ord);
        let _ = seg.datum(ord);
        let _ = seg.callsign(ord);
        let _ = seg.lang(ord);
        let _ = seg.cfg(ord);
        let _ = seg.declined(ord);
        assert!(seg.out_edges(ord).all(|n| n.other.0 < seg.def_count()));
        assert!(seg.in_edges(ord).all(|n| n.other.0 < seg.def_count()));
    }
}

#[test]
fn every_single_byte_flip_is_caught_or_survived() {
    let full = sample();
    // Not a fuzz replacement — a cheap guarantee that the checksum covers the
    // whole image, so no byte is silently outside the integrity claim.
    for i in 0..full.len() {
        let mut image = full.clone();
        image[i] ^= 0x01;
        if (32..64).contains(&i) {
            continue; // the checksum field itself is excluded by construction
        }
        assert!(Segment::parse(image).is_err(), "flip at {i} went unnoticed");
    }
}

#[test]
fn empty_graph_is_valid() {
    let seg = Segment::parse(build(&[], &[]).unwrap()).unwrap();
    assert_eq!(seg.def_count(), 0);
    assert_eq!(seg.edge_count(), 0);
    assert_eq!(seg.datum(Ordinal(0)), None);
}

#[test]
fn edges_to_unknown_defs_are_dropped_not_dangling() {
    let defs = vec![Def {
        datum: DatumId([1; 32]),
        callsign: "a".into(),
        lang: None,
        facts: None,
    }];
    let edges = vec![Edge {
        src: DatumId([1; 32]),
        dst: DatumId([2; 32]), // never surveyed
        relation: Relation::Calls,
        grade: Grade::Proven,
    }];
    let seg = Segment::parse(build(&defs, &edges).unwrap()).unwrap();
    assert_eq!(seg.edge_count(), 0);
}
