//! Determinism against a real tree (claim C2), not a fixture.
//!
//! Fixtures prove the logic; a real repo proves the assumptions. This suite
//! runs against `GRID_CORPUS` — the repo itself in CI — because the failure
//! modes that matter here (a name shape nobody anticipated, a file that only
//! exists on one machine, an ordering that only shows up at scale) do not
//! occur in a three-file fixture.
//!
//! It never silently no-ops: with no corpus it falls back to this very repo,
//! and it asserts the corpus actually contained something to survey.
use std::path::PathBuf;

use grid_survey::{run, Cache};

/// The tree to survey: `GRID_CORPUS`, else the grid repo itself.
fn corpus() -> PathBuf {
    std::env::var_os("GRID_CORPUS").map_or_else(
        || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")),
        PathBuf::from,
    )
}

#[test]
fn a_real_tree_surveys_identically_twice_and_warms_completely() {
    let root = corpus();
    let mut cache = Cache::new();
    let cold = run(&root, &mut cache).expect("cold survey");

    // Non-vacuity: a corpus with no definitions would make every assertion
    // below trivially true.
    assert!(
        cold.defs > 0,
        "corpus at {} surveyed to nothing — this test would prove nothing",
        root.display()
    );
    assert_eq!(cold.reread, 0, "a cold survey reuses nothing");

    let warm = run(&root, &mut cache).expect("warm survey");
    assert_eq!(
        cold.image, warm.image,
        "the same tree must survey to the same bytes"
    );
    assert!(
        warm.reread > 0,
        "an unchanged tree must come back from the cache"
    );
    // Counts are read off the built segment, so agreeing means the graph
    // itself agreed, not just the file list.
    assert_eq!((cold.defs, cold.edges), (warm.defs, warm.edges));
}

#[test]
fn a_cold_survey_from_a_fresh_cache_equals_the_warm_one() {
    // The incrementality oracle at corpus scale: whatever the cache did, the
    // answer is the one a machine that had never seen this tree would get.
    let root = corpus();
    let warm = {
        let mut cache = Cache::new();
        run(&root, &mut cache).expect("cold");
        run(&root, &mut cache).expect("warm")
    };
    let fresh = run(&root, &mut Cache::new()).expect("fresh");
    assert_eq!(warm.image, fresh.image);
}
