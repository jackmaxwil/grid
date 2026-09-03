//! The delta between two charts: what a `Diff` actually says.
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
    Callsign, Datum, DefChange, DefChangeAction, Diff, Edge, EdgeChange, EdgeChangeAction,
    EdgeChangeMovedFrom, Grade, Relation, SnapshotId,
};

use crate::{relation_tag, DatumId, Ordinal, Segment};

/// Definitions of a chart, by callsign, with the shape as move evidence and
/// the sig as interface evidence.
///
/// A callsign two segments both define is a collision the survey already
/// refused to resolve; here the lowest datum wins, so the comparison is a
/// function of content rather than of segment order.
type Defs = BTreeMap<String, DefFacts>;

/// What one side of the comparison knows about a definition. `shape` and
/// `sig` are `None` on images that predate their column — unknown, never a
/// fabricated hash.
type DefFacts = (DatumId, Option<[u8; 32]>, Option<[u8; 32]>);

/// Did the interface move between two sides?
///
/// `None` when either side predates segment epoch 3: the honest answer is
/// *unknown*, and every consumer treats unknown as changed (drift's changed
/// set, per the plan). Answering `Some(false)` for an image that cannot
/// carry a sig would be a fabricated all-clear.
fn sig_changed(before: Option<[u8; 32]>, after: Option<[u8; 32]>) -> Option<bool> {
    Some(before? != after?)
}

/// Edges keyed by `(src callsign, relation tag, dst callsign)` — the
/// name-space identity of a relationship — carrying the datums that prove it.
type Edges = BTreeMap<(String, u8, String), EdgeFacts>;

#[derive(Debug, Clone, Copy)]
struct EdgeFacts {
    src: DatumId,
    dst: DatumId,
    relation: Relation,
    grade: Grade,
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
            let facts = (datum, segment.shape(ord), segment.sig(ord));
            out.entry(callsign.to_owned())
                .and_modify(|d| *d = (*d).min(facts))
                .or_insert(facts);
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
                        grade: n.grade,
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
        grade: facts.grade,
        via: None,
    }
}

/// Compare two live sets into a wire `Diff`.
///
/// Output order is deterministic (both maps are sorted), which the wire
/// contract requires and every downstream hash depends on.
#[must_use]
pub fn diff<A: AsRef<[u8]>, B: AsRef<[u8]>>(
    base_id: SnapshotId,
    base: &[Segment<A>],
    head_id: SnapshotId,
    head: &[Segment<B>],
) -> Diff {
    let (base_defs, head_defs) = (defs_of(base), defs_of(head));
    let mut defs = Vec::new();
    let mut added: Vec<(&String, DefFacts)> = Vec::new();
    let mut removed: Vec<(&String, DefFacts)> = Vec::new();
    for (callsign, head_facts) in &head_defs {
        let (after, _, after_sig) = *head_facts;
        match base_defs.get(callsign) {
            None => added.push((callsign, *head_facts)),
            // Same callsign, same datum: the definition did not change. Not a
            // record — a `Diff` reports what changed.
            Some((before, _, _)) if *before == after => {}
            Some((before, _, before_sig)) => defs.push(DefChange {
                action: DefChangeAction::Modified,
                callsign: Callsign(callsign.clone()),
                before: Some(datum(*before)),
                after: Some(datum(after)),
                moved_from: None,
                sig_changed: sig_changed(*before_sig, after_sig),
            }),
        }
    }
    for (callsign, base_facts) in &base_defs {
        if !head_defs.contains_key(callsign) {
            removed.push((callsign, *base_facts));
        }
    }
    defs.extend(detect_def_moves(&added, &removed));
    defs.sort_by(|a, b| a.callsign.0.cmp(&b.callsign.0));

    let (base_edges, head_edges) = (rebind(edges_of(base), &defs), edges_of(head));
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

    Diff {
        base: base_id,
        head: head_id,
        defs,
        edges,
    }
}

/// Pair a removed definition with an added one that carries the same
/// *shape* — the name-erased identity, the ontology's definition of a
/// rename (callsign rebound, body held). The datum cannot be the join key:
/// it deliberately contains the name. The pairing must be unambiguous — a
/// shape appearing more than once on either side stays -/+ rather than
/// guessing which body went where. Refusal over invention, the same rule
/// the index applies to name collisions. Epoch-1 sides carry no shapes and
/// detect nothing.
fn detect_def_moves(
    added: &[(&String, DefFacts)],
    removed: &[(&String, DefFacts)],
) -> Vec<DefChange> {
    use std::collections::HashMap;
    let mut add_count: HashMap<[u8; 32], usize> = HashMap::new();
    let mut rem_at: HashMap<[u8; 32], (&String, DefFacts, usize)> = HashMap::new();
    for (_, (_, shape, _)) in added {
        if let Some(shape) = shape {
            *add_count.entry(*shape).or_insert(0) += 1;
        }
    }
    for (name, facts) in removed {
        if let Some(shape) = facts.1 {
            let entry = rem_at.entry(shape).or_insert((name, *facts, 0));
            entry.2 += 1;
        }
    }
    let unique = |shape: &Option<[u8; 32]>| {
        shape.as_ref().is_some_and(|s| {
            add_count.get(s) == Some(&1) && rem_at.get(s).is_some_and(|(_, _, n)| *n == 1)
        })
    };
    let mut out = Vec::new();
    for (callsign, facts) in added {
        let (d, shape, after_sig) = *facts;
        if unique(&shape) {
            let Some((old, (before, _, before_sig), _)) = shape.and_then(|s| rem_at.get(&s)) else {
                continue;
            };
            out.push(DefChange {
                action: DefChangeAction::Moved,
                callsign: Callsign((*callsign).clone()),
                before: Some(datum(*before)),
                after: Some(datum(d)),
                moved_from: Some(Callsign((*old).clone())),
                // A move has both sides in hand, so the interface question is
                // answerable: a rename that also changed the signature is a
                // different event from one that only rebound the name.
                sig_changed: sig_changed(*before_sig, after_sig),
            });
        } else {
            out.push(DefChange {
                action: DefChangeAction::Added,
                callsign: Callsign((*callsign).clone()),
                before: None,
                after: Some(datum(d)),
                moved_from: None,
                // Nothing to compare against: an addition has no old
                // interface, so the field stays unknown rather than false.
                sig_changed: None,
            });
        }
    }
    for (callsign, facts) in removed {
        if !unique(&facts.1) {
            out.push(DefChange {
                action: DefChangeAction::Removed,
                callsign: Callsign((*callsign).clone()),
                before: Some(datum(facts.0)),
                after: None,
                moved_from: None,
                sig_changed: None,
            });
        }
    }
    out
}

/// Rewrite the base edge keys through the renames the def comparison just
/// found, so a relationship is compared under one name on both sides.
///
/// Incident: comparison is in callsign space and the record is in datum
/// space, so a rename moved the key while the record held: renaming a scope
/// rebinds the callsigns inside it without touching a body, and the identical
/// `(src, relation, dst, grade)` record came out as both `added` and
/// `removed` — a gate held on hundreds of `edge surplus` findings that
/// described nothing that happened. The shape join is where the identity
/// lives, so it is where the rewrite belongs; a rename the join refused as
/// ambiguous rewrites nothing and the edge stays -/+.
fn rebind(edges: Edges, defs: &[DefChange]) -> Edges {
    let renamed: BTreeMap<&str, &str> = defs
        .iter()
        .filter_map(|d| Some((d.moved_from.as_ref()?.0.as_str(), d.callsign.0.as_str())))
        .collect();
    if renamed.is_empty() {
        return edges;
    }
    let to = |name: String| renamed.get(name.as_str()).map_or(name, |s| (*s).to_owned());
    edges
        .into_iter()
        .map(|((src, tag, dst), facts)| ((to(src), tag, to(dst)), facts))
        .collect()
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
