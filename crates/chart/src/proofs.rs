//! Kani harnesses for claim C3's neighbour of C1: CSR indexing is total.
//!
//! Run with `just verify` (`cargo kani -p grid-chart`). The harnesses drive
//! [`crate::Segment`] accessors with unconstrained ordinals over a symbolic
//! image, proving there is no in-bounds path an adversary can miss: every
//! accessor either answers or returns `None`, for *any* bytes that parsed.
//!
//! The heavy lifting lives in `Segment::parse`; these prove the postcondition
//! the rest of the engine relies on, rather than re-testing the parser.
use crate::{Ordinal, Segment};

/// A parsed segment answers every ordinal query without panicking.
#[kani::proof]
#[kani::unwind(8)]
fn accessors_are_total() {
    const N: usize = 160;
    let bytes: [u8; N] = kani::any();
    let Ok(seg) = Segment::parse(bytes.to_vec()) else {
        return;
    };
    let ord = Ordinal(kani::any());
    let _ = seg.datum(ord);
    let _ = seg.callsign(ord);
    let _ = seg.lang(ord);
    for n in seg.out_edges(ord) {
        assert!(n.other.0 < seg.def_count());
    }
    for n in seg.in_edges(ord) {
        assert!(n.other.0 < seg.def_count());
    }
}

/// Ordinal lookup is a partial inverse of `datum`: it never reports an ordinal
/// the segment does not hold, and round-trips the ones it does.
#[kani::proof]
#[kani::unwind(8)]
fn ordinal_round_trips() {
    const N: usize = 160;
    let bytes: [u8; N] = kani::any();
    let Ok(seg) = Segment::parse(bytes.to_vec()) else {
        return;
    };
    let ord = Ordinal(kani::any());
    if let Some(d) = seg.datum(ord) {
        assert_eq!(seg.ordinal(&d), Some(ord));
    }
}
