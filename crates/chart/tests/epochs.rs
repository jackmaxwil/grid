//! Every epoch ever shipped parses forever. The fixtures are real segments
//! published by the builder of their own epoch over this repository; they are
//! never regenerated — regenerating one would test the current builder against
//! itself.
use std::error::Error;

use grid_chart::{Ordinal, Segment};

fn fixture(name: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    Ok(std::fs::read(path)?)
}

#[test]
fn an_epoch1_image_parses_and_its_new_columns_answer_none() -> Result<(), Box<dyn Error>> {
    let bytes = fixture("epoch1.seg")?;
    let seg = Segment::parse(bytes.as_slice())?;
    assert!(seg.def_count() > 0);
    let ord = Ordinal(0);
    // The old columns answer; the epoch-2 columns decline, honestly.
    assert!(seg.datum(ord).is_some());
    assert!(seg.callsign(ord).is_some());
    assert_eq!(seg.location(ord), None);
    assert_eq!(seg.shape(ord), None);
    assert_eq!(seg.complexity(ord), None);
    assert_eq!(seg.sig(ord), None);
    assert_eq!(seg.cfg(ord), None);
    Ok(())
}

#[test]
fn a_freshly_built_image_carries_every_column() -> Result<(), Box<dyn Error>> {
    let defs = [grid_chart::Def {
        datum: grid_chart::DatumId([7; 32]),
        callsign: "m.f".into(),
        lang: Some(grid_wire::Lang::Rs),
        facts: Some(grid_chart::DefFacts {
            path: "src/lib.rs".into(),
            line: 1,
            line_end: 2,
            span: (0, 9),
            shape: [8; 32],
            complexity: 3,
            sig: [9; 32],
            cfg: "all(test,unix)".into(),
        }),
    }];
    let declined = std::collections::HashMap::from([(
        grid_chart::DatumId([7; 32]),
        grid_chart::Declined {
            external: 3,
            method: 5,
            internal: 1,
            refused: 2,
        },
    )]);
    let seg = Segment::parse(grid_chart::build_declined(&defs, &[], &declined)?)?;
    assert_eq!(seg.sig(Ordinal(0)), Some([9; 32]));
    assert_eq!(seg.cfg(Ordinal(0)), Some("all(test,unix)"));
    // The epoch-5 column round-trips all four counters.
    assert_eq!(
        seg.declined(Ordinal(0)),
        Some(grid_chart::Declined {
            external: 3,
            method: 5,
            internal: 1,
            refused: 2,
        })
    );
    // A definition the map does not name declined nothing — a real zero on
    // a fresh image, as distinct from an older image's `None`.
    let seg = Segment::parse(grid_chart::build(&defs, &[])?)?;
    assert_eq!(
        seg.declined(Ordinal(0)),
        Some(grid_chart::Declined::default())
    );
    assert_eq!(seg.declined(Ordinal(1)), None, "past the end");
    Ok(())
}

/// Conservatism (mutation: default the missing column to zeros): a
/// pre-epoch-5 image answers `declined` with `None` — unknown — never with
/// a fabricated "nothing declined" that a consumer would sum into a clean
/// zero for a closure the chart never counted.
#[test]
fn a_pre_epoch5_image_answers_unknown_never_zero() -> Result<(), Box<dyn Error>> {
    for name in ["epoch1.seg", "epoch2.seg", "epoch3.seg", "epoch4.seg"] {
        let bytes = fixture(name)?;
        let seg = Segment::parse(bytes.as_slice())?;
        assert!(seg.def_count() > 0);
        for i in 0..seg.def_count().min(64) {
            assert_eq!(seg.declined(Ordinal(i)), None, "{name} ordinal {i}");
        }
    }
    Ok(())
}

#[test]
fn an_unconditioned_definition_answers_the_empty_cfg_not_none() -> Result<(), Box<dyn Error>> {
    // `Some("")` is "nothing conditions it"; `None` is "this image cannot
    // say". A fresh image always says.
    let defs = [grid_chart::Def {
        datum: grid_chart::DatumId([7; 32]),
        callsign: "m.f".into(),
        lang: Some(grid_wire::Lang::Py),
        facts: None,
    }];
    let seg = Segment::parse(grid_chart::build(&defs, &[])?)?;
    assert_eq!(seg.cfg(Ordinal(0)), Some(""));
    assert_eq!(seg.cfg(Ordinal(1)), None);
    Ok(())
}

#[test]
fn an_epoch2_image_parses_and_answers_its_own_columns() -> Result<(), Box<dyn Error>> {
    let bytes = fixture("epoch2.seg")?;
    let seg = Segment::parse(bytes.as_slice())?;
    assert!(seg.def_count() > 0);
    let ord = Ordinal(0);
    assert!(seg.datum(ord).is_some());
    assert!(seg.callsign(ord).is_some());
    // Epoch 2's own columns are present, unlike epoch 1's.
    assert!(seg.location(ord).is_some());
    assert!(seg.shape(ord).is_some());
    assert!(seg.complexity(ord).is_some());
    // The epoch-3 column declines, honestly: unknown, never a fabricated
    // interface hash that would answer "unchanged" for a whole snapshot.
    assert_eq!(seg.sig(ord), None);
    assert_eq!(seg.cfg(ord), None);
    Ok(())
}

#[test]
fn an_epoch3_image_parses_and_answers_its_own_columns() -> Result<(), Box<dyn Error>> {
    let bytes = fixture("epoch3.seg")?;
    let seg = Segment::parse(bytes.as_slice())?;
    assert!(seg.def_count() > 0);
    let ord = Ordinal(0);
    assert!(seg.datum(ord).is_some());
    assert!(seg.callsign(ord).is_some());
    assert!(seg.location(ord).is_some());
    assert!(seg.shape(ord).is_some());
    assert!(seg.complexity(ord).is_some());
    // Epoch 3's own column is present, unlike epoch 2's.
    assert!(seg.sig(ord).is_some());
    // The epoch-4 column declines: unknown, never "unconditioned" — which
    // would call every `#[cfg(test)]` caller in the snapshot compiled.
    assert_eq!(seg.cfg(ord), None);
    Ok(())
}

#[test]
fn an_epoch4_image_parses_and_answers_its_own_columns() -> Result<(), Box<dyn Error>> {
    let bytes = fixture("epoch4.seg")?;
    let seg = Segment::parse(bytes.as_slice())?;
    assert!(seg.def_count() > 0);
    let ord = Ordinal(0);
    assert!(seg.datum(ord).is_some());
    assert!(seg.callsign(ord).is_some());
    assert!(seg.location(ord).is_some());
    assert!(seg.shape(ord).is_some());
    assert!(seg.complexity(ord).is_some());
    assert!(seg.sig(ord).is_some());
    // Epoch 4's own column is present, unlike epoch 3's: `Some("")` for a
    // definition nothing conditions, never `None`.
    assert!(seg.cfg(ord).is_some());
    // The epoch-5 column declines: unknown, never "nothing declined".
    assert_eq!(seg.declined(ord), None);
    Ok(())
}
