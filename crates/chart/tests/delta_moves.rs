//! Rename detection in the delta: same datum under a new callsign is one
//! `moved` row, never an add/remove pair — and an ambiguous pairing refuses
//! back into add/remove rather than guessing which body went where.
use std::error::Error;

use grid_chart::{build, made_good, DatumId, Def, Segment};
use grid_wire::DefChangeAction;
use grid_wire::SnapshotId;

type Row = (DefChangeAction, String, Option<String>);

fn def(name: &str, fill: u8) -> Def {
    Def {
        datum: DatumId([fill; 32]),
        callsign: name.to_owned(),
        lang: Some(grid_wire::Lang::Rs),
    }
}

fn chart(defs: &[Def]) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(build(defs, &[])?)
}

fn diff(base: &[Def], head: &[Def]) -> Result<Vec<Row>, Box<dyn Error>> {
    let (b, h) = (chart(base)?, chart(head)?);
    let (b, h) = (Segment::parse(b.as_slice())?, Segment::parse(h.as_slice())?);
    let mg = made_good(
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
    let rows = diff(&[def("old_name", 7)], &[def("new_name", 7)])?;
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
        chart(&[def("a", 7)])?,
        chart(&[def("b", 7)])?,
        chart(&[def("c", 7)])?,
    );
    let (b1, b2, h) = (
        Segment::parse(b1.as_slice())?,
        Segment::parse(b2.as_slice())?,
        Segment::parse(h.as_slice())?,
    );
    let mg = made_good(
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
    let rows = diff(&[def("same_name", 7)], &[def("same_name", 9)])?;
    assert_eq!(rows[0].0, DefChangeAction::Modified);
    assert_eq!(rows[0].2, None);
    Ok(())
}
