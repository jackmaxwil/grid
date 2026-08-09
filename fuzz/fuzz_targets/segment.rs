#![no_main]
//! The segment loader must never panic and never read out of bounds, whatever
//! the bytes are. Wave 4 evidence for claim C1 — this is the crate the mmap
//! points at, so "hostile file on disk" is exactly this input.
//!
//! Anything that parses is then exercised through every accessor with
//! unconstrained ordinals: refusing bad images is only half the claim, the
//! other half is that an accepted image cannot be made to read out of bounds.
use grid_chart::{Ordinal, Segment};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(seg) = Segment::parse(data.to_vec()) else {
        return;
    };
    // Parsing succeeded, so every accessor must now be total.
    for ord in [0, 1, seg.def_count(), seg.def_count().wrapping_add(1), u32::MAX] {
        let ord = Ordinal(ord);
        let _ = seg.datum(ord);
        let _ = seg.callsign(ord);
        let _ = seg.lang(ord);
        for n in seg.out_edges(ord) {
            assert!(n.other.0 < seg.def_count(), "out-edge escapes the segment");
        }
        for n in seg.in_edges(ord) {
            assert!(n.other.0 < seg.def_count(), "in-edge escapes the segment");
        }
    }
    // Interning is a bijection on everything the segment actually holds.
    for i in 0..seg.def_count().min(64) {
        let ord = Ordinal(i);
        if let Some(d) = seg.datum(ord) {
            assert_eq!(seg.ordinal(&d), Some(ord), "ordinal is not a function of content");
        }
    }
    // A validated image re-checksums to what it claims: no accepted image is
    // outside the integrity claim.
    assert_eq!(grid_chart::checksum(seg.bytes()), seg.bytes()[32..64]);
});
