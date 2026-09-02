//! Rename detection in the delta: same datum under a new callsign is one
//! `moved` row, never an add/remove pair — and an ambiguous pairing refuses
//! back into add/remove rather than guessing which body went where.
use std::error::Error;

use grid_chart::{build, diff, DatumId, Def, Segment};
use grid_wire::DefChangeAction;
use grid_wire::SnapshotId;

type Row = (DefChangeAction, String, Option<String>);

fn def(name: &str, fill: u8) -> Def {
    Def {
        datum: DatumId([fill; 32]),
        callsign: name.to_owned(),
        lang: Some(grid_wire::Lang::Rs),
        facts: None,
    }
}

/// datum and shape filled independently: a rename holds the shape while the
/// datum (which contains the name) moves.
fn shaped(name: &str, datum_fill: u8, shape_fill: u8) -> Def {
    Def {
        datum: DatumId([datum_fill; 32]),
        callsign: name.to_owned(),
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

fn chart(defs: &[Def]) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(build(defs, &[])?)
}

fn rows(base: &[Def], head: &[Def]) -> Result<Vec<Row>, Box<dyn Error>> {
    let (b, h) = (chart(base)?, chart(head)?);
    let (b, h) = (Segment::parse(b.as_slice())?, Segment::parse(h.as_slice())?);
    let mg = diff(
        SnapshotId("snap:b3:base".into()),
        &[b],
        SnapshotId("snap:b3:head".into()),
        &[h],
    );
    Ok(mg
        .defs
        .into_iter()
        .map(|d| (d.action, d.callsign.0, d.moved_from.map(|c| c.0)))
        .collect())
}

#[test]
fn a_rename_is_one_moved_row() -> Result<(), Box<dyn Error>> {
    // Different datums (the name is in them), same shape: a rename.
    let rows = rows(&[shaped("old_name", 7, 42)], &[shaped("new_name", 8, 42)])?;
    assert_eq!(
        rows,
        vec![(
            DefChangeAction::Moved,
            "new_name".into(),
            Some("old_name".into())
        )]
    );
    Ok(())
}

#[test]
fn an_ambiguous_pairing_refuses_back_to_add_and_remove() -> Result<(), Box<dyn Error>> {
    // Two identical bodies removed, one added: which one moved is a guess,
    // and a guess about history is worse than a coarse answer. One segment
    // interns a datum once, so the duplicate arrives via a second segment —
    // exactly how identical bodies in two files reach a real live set.
    let (b1, b2, h) = (
        chart(&[shaped("a", 1, 42)])?,
        chart(&[shaped("b", 2, 42)])?,
        chart(&[shaped("c", 3, 42)])?,
    );
    let (b1, b2, h) = (
        Segment::parse(b1.as_slice())?,
        Segment::parse(b2.as_slice())?,
        Segment::parse(h.as_slice())?,
    );
    let mg = diff(
        SnapshotId("snap:b3:base".into()),
        &[b1, b2],
        SnapshotId("snap:b3:head".into()),
        &[h],
    );
    assert!(mg.defs.iter().all(|d| d.action != DefChangeAction::Moved));
    assert_eq!(mg.defs.len(), 3);
    Ok(())
}

#[test]
fn a_body_edit_is_still_modified_not_moved() -> Result<(), Box<dyn Error>> {
    let rows = rows(&[shaped("same_name", 7, 40)], &[shaped("same_name", 9, 41)])?;
    assert_eq!(rows[0].0, DefChangeAction::Modified);
    assert_eq!(rows[0].2, None);
    Ok(())
}

#[test]
fn an_epoch1_side_detects_no_moves_and_still_diffs() -> Result<(), Box<dyn Error>> {
    // Shape-less defs (facts: None) are what an epoch-1 image answers.
    let rows = rows(&[def("old_name", 7)], &[def("new_name", 8)])?;
    assert!(rows.iter().all(|(a, _, _)| *a != DefChangeAction::Moved));
    assert_eq!(rows.len(), 2);
    Ok(())
}
