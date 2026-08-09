//! Claim C2 evidence for the chart: the same graph is the same bytes.
//!
//! A segment is built from *sets*, so anything that only changes the order or
//! the multiplicity of the input must not change one byte of the image — and
//! therefore must not change the snapshot id derived from it (D5).
use grid_chart::{build, DatumId, Def, Edge, Manifest, Segment, SegmentEntry};
use grid_wire::{Lang, Relation, Zoc};
use proptest::prelude::*;

fn datum(seed: u8) -> DatumId {
    DatumId([seed; 32])
}

fn def(seed: u8) -> Def {
    Def {
        datum: datum(seed),
        callsign: format!("pkg.mod.f{seed}"),
        lang: Some(Lang::Py),
    }
}

fn arb_defs() -> impl Strategy<Value = Vec<Def>> {
    prop::collection::vec(any::<u8>(), 1..24).prop_map(|seeds| seeds.into_iter().map(def).collect())
}

fn arb_edges(n: usize) -> impl Strategy<Value = Vec<Edge>> {
    prop::collection::vec((any::<u8>(), any::<u8>(), 0u8..12, 0u8..4), 0..n).prop_map(|raw| {
        raw.into_iter()
            .map(|(s, d, r, z)| Edge {
                src: datum(s),
                dst: datum(d),
                relation: relation(r),
                zoc: zoc(z),
            })
            .collect()
    })
}

fn relation(tag: u8) -> Relation {
    [
        Relation::Calls,
        Relation::Constructs,
        Relation::ReadsField,
        Relation::WritesField,
        Relation::Implements,
        Relation::DispatchesTo,
        Relation::FlowsInto,
        Relation::SerializesTo,
        Relation::DeserializesFrom,
        Relation::RegistersUnder,
        Relation::SendsOver,
        Relation::LinksAgainst,
    ][tag as usize % 12]
}

fn zoc(tag: u8) -> Zoc {
    [Zoc::Proven, Zoc::Observed, Zoc::Modeled, Zoc::Heuristic][tag as usize % 4]
}

proptest! {
    /// Input order is not information: shuffling defs and edges is a no-op.
    #[test]
    fn build_is_order_independent(defs in arb_defs(), edges in arb_edges(40), rot in 0usize..16) {
        let a = build(&defs, &edges).unwrap();

        let mut defs2 = defs.clone();
        let n = defs2.len();
        defs2.rotate_left(rot % n);
        defs2.reverse();
        let mut edges2 = edges.clone();
        let m = edges2.len();
        if m > 0 {
            edges2.rotate_left(rot % m);
            edges2.reverse();
        }
        let b = build(&defs2, &edges2).unwrap();
        prop_assert_eq!(a, b);
    }

    /// Nor is multiplicity: a graph is a set of defs and a set of edges.
    #[test]
    fn build_ignores_duplicates(defs in arb_defs(), edges in arb_edges(24)) {
        let a = build(&defs, &edges).unwrap();
        let doubled_defs: Vec<Def> = defs.iter().chain(defs.iter()).cloned().collect();
        let doubled_edges: Vec<Edge> = edges.iter().chain(edges.iter()).cloned().collect();
        prop_assert_eq!(a, build(&doubled_defs, &doubled_edges).unwrap());
    }

    /// Everything built parses, and parsing recovers exactly what went in.
    #[test]
    fn round_trips(defs in arb_defs(), edges in arb_edges(40)) {
        let image = build(&defs, &edges).unwrap();
        let seg = Segment::parse(image).unwrap();

        let mut wanted: Vec<&Def> = defs.iter().collect();
        wanted.sort_by(|a, b| a.datum.cmp(&b.datum).then_with(|| a.callsign.cmp(&b.callsign)));
        wanted.dedup_by(|a, b| a.datum == b.datum);
        prop_assert_eq!(seg.def_count() as usize, wanted.len());

        for (i, d) in wanted.iter().enumerate() {
            let ord = grid_chart::Ordinal(u32::try_from(i).unwrap());
            prop_assert_eq!(seg.datum(ord), Some(d.datum));
            prop_assert_eq!(seg.callsign(ord), Some(d.callsign.as_str()));
            prop_assert_eq!(seg.lang(ord), d.lang);
            // Interning is a bijection: the ordinal is a function of content.
            prop_assert_eq!(seg.ordinal(&d.datum), Some(ord));
        }
    }

    /// Both CSR directions describe the same edge set, from either end.
    #[test]
    fn csr_directions_agree(defs in arb_defs(), edges in arb_edges(40)) {
        let seg = Segment::parse(build(&defs, &edges).unwrap()).unwrap();
        let mut out = Vec::new();
        let mut inc = Vec::new();
        for i in 0..seg.def_count() {
            let ord = grid_chart::Ordinal(i);
            for n in seg.out_edges(ord) {
                out.push((i, n.other.0, n.relation, n.zoc));
            }
            for n in seg.in_edges(ord) {
                inc.push((n.other.0, i, n.relation, n.zoc));
            }
        }
        out.sort_by_key(|e| (e.0, e.1));
        inc.sort_by_key(|e| (e.0, e.1));
        prop_assert_eq!(out, inc);
    }

    /// A snapshot id is a pure function of the manifest's content (D5), so
    /// segment order and repetition cannot change it.
    #[test]
    fn snapshot_id_is_content(rev in "[0-9a-f]{40}", names in prop::collection::vec(any::<u8>(), 1..8)) {
        let segments: Vec<SegmentEntry> = names
            .iter()
            .map(|&n| SegmentEntry { name: format!("{n}.seg"), checksum: [n; 32] })
            .collect();
        let a = Manifest { git_rev: rev.clone(), worktree_diff_hash: None, segments: segments.clone() };
        let mut shuffled = segments.clone();
        shuffled.reverse();
        shuffled.extend(segments);
        let b = Manifest { git_rev: rev, worktree_diff_hash: None, segments: shuffled };
        prop_assert_eq!(a.snapshot_id(), b.snapshot_id());
        prop_assert_eq!(Manifest::parse(&a.canonical_bytes()).unwrap().snapshot_id(), a.snapshot_id());
    }

    /// A dirty tree is a different snapshot than the clean rev it sits on.
    #[test]
    fn dirty_tree_is_a_different_snapshot(rev in "[0-9a-f]{40}", h in any::<u8>()) {
        let clean = Manifest { git_rev: rev.clone(), worktree_diff_hash: None, segments: Vec::new() };
        let dirty = Manifest { git_rev: rev, worktree_diff_hash: Some([h; 32]), segments: Vec::new() };
        prop_assert_ne!(clean.snapshot_id(), dirty.snapshot_id());
    }
}
