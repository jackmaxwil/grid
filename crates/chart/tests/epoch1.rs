//! An epoch-1 image parses forever. The fixture is a real segment published
//! by the epoch-1 builder over this repository; it is never regenerated —
//! regenerating it would test the current builder against itself.
use std::error::Error;

use grid_chart::{Ordinal, Segment};

#[test]
fn an_epoch1_image_parses_and_its_new_columns_answer_none() -> Result<(), Box<dyn Error>> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/epoch1.seg"
    ))?;
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
