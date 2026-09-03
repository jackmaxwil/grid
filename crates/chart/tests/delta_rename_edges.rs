//! A rename must not manufacture edge changes.
//!
//! Edges are compared in callsign space but reported in datum space, so a
//! renamed endpoint moves the comparison key while the reported record holds:
//! `grid gate` shows the identical `(src, relation, dst, grade)` record as
//! both `added` and `removed`, both counted `edge surplus`, and holds on a
//! change that touched no relationship.
use std::error::Error;

use grid_chart::{build, diff, DatumId, Def, Edge, Segment};
use grid_wire::{EdgeChangeAction, Grade, Relation, SnapshotId};

/// One edge record, reduced to what a contradiction is visible in: the two
/// endpoints by their datum fill byte, and the grade.
type Row = (EdgeChangeAction, u8, u8, Grade);

fn def(callsign: &str, datum_fill: u8, shape_fill: u8) -> Def {
    Def {
        datum: DatumId([datum_fill; 32]),
        callsign: callsign.to_owned(),
        lang: Some(grid_wire::Lang::Rs),
        facts: Some(grid_chart::DefFacts {
            path: "src/lib.rs".into(),
            line: 1,
            line_end: 1,
            span: (0, 1),
            shape: [shape_fill; 32],
            complexity: 1,
            sig: [shape_fill; 32],
            cfg: String::new(),
        }),
    }
}

fn calls(src: u8, dst: u8, grade: Grade) -> Edge {
    Edge {
        src: DatumId([src; 32]),
        dst: DatumId([dst; 32]),
        relation: Relation::Calls,
        grade,
    }
}

fn edges(base: (&[Def], &[Edge]), head: (&[Def], &[Edge])) -> Result<Vec<Row>, Box<dyn Error>> {
    let (b, h) = (build(base.0, base.1)?, build(head.0, head.1)?);
    let (b, h) = (Segment::parse(b.as_slice())?, Segment::parse(h.as_slice())?);
    let mg = diff(
        SnapshotId("snap:b3:base".into()),
        &[b],
        SnapshotId("snap:b3:head".into()),
        &[h],
    );
    // The datum fill byte identifies the endpoint; two records agreeing on
    // all four fields are the contradiction this file is about.
    let byte = |d: &grid_wire::Datum| -> u8 {
        u8::from_str_radix(&d.0[d.0.len() - 2..], 16).unwrap_or(0xff)
    };
    Ok(mg
        .edges
        .into_iter()
        .map(|e| (e.action, byte(&e.edge.src), byte(&e.edge.dst), e.edge.grade))
        .collect())
}

/// A rename of a containing scope rebinds the callsigns of the definitions
/// inside it and touches neither body, so both endpoint datums hold. The
/// relationship between them did not change; the diff must say nothing.
#[test]
fn a_pure_rename_manufactures_no_edge_change() -> Result<(), Box<dyn Error>> {
    let base = [def("Reaper.sweep", 1, 10), def("Reaper.validate", 2, 20)];
    let head = [def("Grim.sweep", 1, 10), def("Grim.validate", 2, 20)];
    let edge = [calls(1, 2, Grade::Proven)];
    assert_eq!(edges((&base, &edge), (&head, &edge))?, vec![]);
    Ok(())
}

/// The other direction: a rename that also changed what the edge reaches is
/// a real change and stays reported — with the grade each side actually
/// carries, so the rename join can never upgrade one.
#[test]
fn a_rename_that_moves_an_edge_is_still_reported() -> Result<(), Box<dyn Error>> {
    let base = [def("Reaper.sweep", 1, 10), def("Reaper.validate", 2, 20)];
    let head = [
        def("Grim.sweep", 3, 10),
        def("Grim.validate", 2, 20),
        def("Grim.other", 4, 30),
    ];
    let rows = edges(
        (&base, &[calls(1, 2, Grade::Proven)]),
        (&head, &[calls(3, 4, Grade::Heuristic)]),
    )?;
    assert!(
        rows.contains(&(EdgeChangeAction::Removed, 1, 2, Grade::Proven)),
        "the edge the rename left behind is gone: {rows:?}"
    );
    assert!(
        rows.contains(&(EdgeChangeAction::Added, 3, 4, Grade::Heuristic)),
        "and the one it built arrived, at its own grade: {rows:?}"
    );
    Ok(())
}
