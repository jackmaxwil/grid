//! `DefChange::sig_changed`: the interface half of a delta, and the one
//! answer it must never fabricate.
//!
//! The conservatism case is an image from before epoch 3. It carries no sig
//! column, so the honest answer is *unknown* — `None`, which drift's changed
//! set reads as changed. `Some(false)` there would be an all-clear invented
//! out of an absent column, and would silently exclude every dependent of
//! every definition in that snapshot.
use std::error::Error;

use grid_chart::{build, diff, DatumId, Def, DefFacts, Ordinal, Segment};
use grid_wire::{DefChangeAction, SnapshotId};

fn def(callsign: &str, datum: u8, sig: u8) -> Def {
    Def {
        datum: DatumId([datum; 32]),
        callsign: callsign.to_owned(),
        lang: Some(grid_wire::Lang::Rs),
        facts: Some(DefFacts {
            path: "src/lib.rs".into(),
            line: 1,
            line_end: 1,
            span: (0, 1),
            shape: [datum; 32],
            complexity: 1,
            sig: [sig; 32],
        }),
    }
}

fn segment(defs: &[Def]) -> Result<Segment<Vec<u8>>, Box<dyn Error>> {
    Ok(Segment::parse(build(defs, &[])?)?)
}

fn fixture(name: &str) -> Result<Segment<Vec<u8>>, Box<dyn Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    Ok(Segment::parse(std::fs::read(path)?)?)
}

fn changes<B: AsRef<[u8]>>(
    base: &[Segment<B>],
    head: &[Segment<B>],
) -> Vec<(DefChangeAction, String, Option<bool>)> {
    diff(
        SnapshotId("snap:b3:base".into()),
        base,
        SnapshotId("snap:b3:head".into()),
        head,
    )
    .defs
    .into_iter()
    .map(|d| (d.action, d.callsign.0, d.sig_changed))
    .collect()
}

#[test]
fn a_body_only_change_reports_the_interface_as_held() -> Result<(), Box<dyn Error>> {
    let base = segment(&[def("m.f", 1, 9)])?;
    let head = segment(&[def("m.f", 2, 9)])?;
    assert_eq!(
        changes(&[base], &[head]),
        vec![(DefChangeAction::Modified, "m.f".to_owned(), Some(false))]
    );
    Ok(())
}

#[test]
fn an_interface_change_reports_it() -> Result<(), Box<dyn Error>> {
    let base = segment(&[def("m.f", 1, 9)])?;
    let head = segment(&[def("m.f", 2, 8)])?;
    assert_eq!(
        changes(&[base], &[head]),
        vec![(DefChangeAction::Modified, "m.f".to_owned(), Some(true))]
    );
    Ok(())
}

#[test]
fn an_addition_or_removal_has_no_pair_to_compare() -> Result<(), Box<dyn Error>> {
    let base = segment(&[def("m.f", 1, 9)])?;
    let head = segment(&[def("m.g", 2, 8)])?;
    let mut rows = changes(&[base], &[head]);
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    assert_eq!(
        rows,
        vec![
            (DefChangeAction::Removed, "m.f".to_owned(), None),
            (DefChangeAction::Added, "m.g".to_owned(), None),
        ]
    );
    Ok(())
}

#[test]
fn a_move_answers_the_interface_question_too() -> Result<(), Box<dyn Error>> {
    // Same shape on both sides is a rename; the datum moved with the name, so
    // the sig moved with it, and the row says so rather than staying silent.
    let base = segment(&[def("m.f", 1, 9)])?;
    let head = segment(&[def("m.g", 1, 8)])?;
    let rows = changes(&[base], &[head]);
    assert_eq!(
        rows,
        vec![(DefChangeAction::Moved, "m.g".to_owned(), Some(true))]
    );
    Ok(())
}

/// Conservatism. An epoch-2 image predates the sig column, so every
/// comparison against it answers `None` — never `Some(false)`.
#[test]
fn a_pre_epoch3_side_answers_unknown_not_unchanged() -> Result<(), Box<dyn Error>> {
    let old = fixture("epoch2.seg")?;
    // A real callsign out of the fixture, re-declared at a different datum:
    // the delta must call it modified, and must decline the sig question.
    let ord = Ordinal(0);
    let callsign = old
        .callsign(ord)
        .expect("fixture names its defs")
        .to_owned();
    assert_eq!(old.sig(ord), None, "epoch 2 carries no sig column");

    let head = segment(&[def(&callsign, 0xAB, 0xCD)])?;
    let rows: Vec<_> = diff(
        SnapshotId("snap:b3:base".into()),
        &[old],
        SnapshotId("snap:b3:head".into()),
        &[head],
    )
    .defs
    .into_iter()
    .filter(|d| d.callsign.0 == callsign)
    .map(|d| (d.action, d.sig_changed))
    .collect();
    assert_eq!(rows, vec![(DefChangeAction::Modified, None)]);
    Ok(())
}
