//! The delta between two charts: what a `MadeGood` actually says.
//!
//! Comparison happens in **callsign space, with datums as the evidence**, and
//! that choice is the whole design. Comparing edges by datum would be
//! literally correct and practically useless: a datum changes whenever its
//! body changes, so editing one line of a caller would report every edge it
//! has as removed-and-re-added. The relationship "`sweep` calls `validate`"
//! survives an edit to `sweep`; only the definitions' identities changed, and
//! that is exactly what a `def` record already says.
//!
//! So: definitions are compared by callsign and *report* their datum
//! transition, and edges are compared by the callsign triple.
use std::collections::BTreeMap;

use grid_wire::{
    Callsign, Datum, DefChange, DefChangeAction, Edge, EdgeChange, EdgeChangeAction,
    EdgeChangeMovedFrom, MadeGood, Relation, SnapshotId, Zoc,
};

use crate::{relation_tag, DatumId, Ordinal, Segment};

/// Definitions of a chart, by callsign.
///
/// A callsign two segments both define is a collision the survey already
/// refused to resolve; here the lowest datum wins, so the comparison is a
/// function of content rather than of segment order.
type Defs = BTreeMap<String, DatumId>;

/// Edges keyed by `(src callsign, relation tag, dst callsign)` — the
/// name-space identity of a relationship — carrying the datums that prove it.
type Edges = BTreeMap<(String, u8, String), EdgeFacts>;

#[derive(Debug, Clone, Copy)]
struct EdgeFacts {
    src: DatumId,
    dst: DatumId,
    relation: Relation,
    zoc: Zoc,
}

/// Read the definitions of a live set.
fn defs_of<B: AsRef<[u8]>>(segments: &[Segment<B>]) -> Defs {
    let mut out = Defs::new();
    for segment in segments {
        for ord in 0..segment.def_count() {
            let ord = Ordinal(ord);
            let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) else {
                continue;
            };
            out.entry(callsign.to_owned())
                .and_modify(|d| *d = (*d).min(datum))
                .or_insert(datum);
        }
    }
    out
}

/// Read the edges of a live set, in callsign space.
fn edges_of<B: AsRef<[u8]>>(segments: &[Segment<B>]) -> Edges {
    let mut out = Edges::new();
    for segment in segments {
        for ord in 0..segment.def_count() {
            let src_ord = Ordinal(ord);
            let (Some(src), Some(src_name)) = (segment.datum(src_ord), segment.callsign(src_ord))
            else {
                continue;
            };
            for n in segment.out_edges(src_ord) {
                let (Some(dst), Some(dst_name)) =
                    (segment.datum(n.other), segment.callsign(n.other))
                else {
                    continue;
                };
                out.insert(
                    (
                        src_name.to_owned(),
                        relation_tag(n.relation),
                        dst_name.to_owned(),
                    ),
                    EdgeFacts {
                        src,
                        dst,
                        relation: n.relation,
                        zoc: n.zoc,
                    },
                );
            }
        }
    }
    out
}

fn datum(id: DatumId) -> Datum {
    Datum(id.to_string())
}

fn wire_edge(facts: EdgeFacts) -> Edge {
    Edge {
        src: datum(facts.src),
        relation: facts.relation,
        dst: datum(facts.dst),
        zoc: facts.zoc,
        via: None,
    }
}

/// Compare two live sets into a wire `MadeGood`.
///
/// Output order is deterministic (both maps are sorted), which the wire
/// contract requires and every downstream hash depends on.
#[must_use]
pub fn made_good<B: AsRef<[u8]>>(
    base_id: SnapshotId,
    base: &[Segment<B>],
    head_id: SnapshotId,
    head: &[Segment<B>],
) -> MadeGood {
    let (base_defs, head_defs) = (defs_of(base), defs_of(head));
    let mut defs = Vec::new();
    for (callsign, after) in &head_defs {
        match base_defs.get(callsign) {
            None => defs.push(DefChange {
                action: DefChangeAction::Added,
                callsign: Callsign(callsign.clone()),
                before: None,
                after: Some(datum(*after)),
            }),
            // Same callsign, same datum: the definition did not change. Not a
            // record — a `MadeGood` reports what changed.
            Some(before) if before == after => {}
            Some(before) => defs.push(DefChange {
                action: DefChangeAction::Modified,
                callsign: Callsign(callsign.clone()),
                before: Some(datum(*before)),
                after: Some(datum(*after)),
            }),
        }
    }
    for (callsign, before) in &base_defs {
        if !head_defs.contains_key(callsign) {
            defs.push(DefChange {
                action: DefChangeAction::Removed,
                callsign: Callsign(callsign.clone()),
                before: Some(datum(*before)),
                after: None,
            });
        }
    }
    defs.sort_by(|a, b| a.callsign.0.cmp(&b.callsign.0));

    let (base_edges, head_edges) = (edges_of(base), edges_of(head));
    let added: Vec<_> = head_edges
        .iter()
        .filter(|(k, _)| !base_edges.contains_key(*k))
        .collect();
    let removed: Vec<_> = base_edges
        .iter()
        .filter(|(k, _)| !head_edges.contains_key(*k))
        .collect();
    let mut edges = detect_moves(&added, &removed);
    edges.sort_by(|a, b| {
        (&a.edge.src.0, relation_tag(a.edge.relation), &a.edge.dst.0).cmp(&(
            &b.edge.src.0,
            relation_tag(b.edge.relation),
            &b.edge.dst.0,
        ))
    });

    MadeGood {
        base: base_id,
        head: head_id,
        defs,
        edges,
    }
}

/// Pair a removed edge with an added one when the *same call to the same
/// unchanged callee* relocated to a different caller.
///
/// The conditions are deliberately narrow. Same relation, same callee
/// callsign, and — the load-bearing one — the callee's datum is byte-identical
/// on both sides, so the thing being called demonstrably did not change. A
/// looser rule would report coincidences as moves, and a move is a claim about
/// history that a reviewer will trust.
fn detect_moves(
    added: &[(&(String, u8, String), &EdgeFacts)],
    removed: &[(&(String, u8, String), &EdgeFacts)],
) -> Vec<EdgeChange> {
    let mut out = Vec::new();
    let mut taken = vec![false; removed.len()];
    for (key, facts) in added {
        // Same callee, same relation, callee unchanged, different caller.
        let matched = removed.iter().enumerate().position(|(i, (rkey, rfacts))| {
            !taken[i]
                && rkey.1 == key.1
                && rkey.2 == key.2
                && rfacts.dst == facts.dst
                && rkey.0 != key.0
        });
        if let Some(i) = matched {
            taken[i] = true;
            let (_, from) = removed[i];
            out.push(EdgeChange {
                action: EdgeChangeAction::Moved,
                edge: wire_edge(**facts),
                moved_from: Some(EdgeChangeMovedFrom {
                    src: datum(from.src),
                    dst: datum(from.dst),
                }),
            });
        } else {
            out.push(EdgeChange {
                action: EdgeChangeAction::Added,
                edge: wire_edge(**facts),
                moved_from: None,
            });
        }
    }
    for (i, (_, facts)) in removed.iter().enumerate() {
        if !taken[i] {
            out.push(EdgeChange {
                action: EdgeChangeAction::Removed,
                edge: wire_edge(**facts),
                moved_from: None,
            });
        }
    }
    out
}
