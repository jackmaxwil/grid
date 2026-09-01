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
    Ok(())
}
