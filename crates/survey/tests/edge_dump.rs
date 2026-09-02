//! The differential harness: dump a tree's chart as text so two binaries'
//! charts can be diffed. Ignored by default — it is run by hand before and
//! after a resolver change, on this repo and on a second corpus, and the
//! diff of its two outputs is what a commit message quotes.
//!
//! `GRID_CORPUS` names the tree (this repo when unset); `GRID_EDGES_OUT`
//! names the file to write. One line per edge, `src\tdst\trelation\tgrade`,
//! sorted, plus a trailing `# declined` line with the survey's split.
use std::fmt::Write as _;
use std::path::PathBuf;

use grid_chart::{Ordinal, Segment};
use grid_survey::{run, Cache};

#[test]
#[ignore = "run by hand: GRID_EDGES_OUT=<file> cargo test -p grid-survey --test edge_dump -- --ignored"]
fn dump_edges() {
    let root = std::env::var_os("GRID_CORPUS").map_or_else(
        || PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")),
        PathBuf::from,
    );
    let Some(out) = std::env::var_os("GRID_EDGES_OUT") else {
        return;
    };
    let survey = run(&root, &mut Cache::new()).expect("survey");
    let seg = Segment::parse(survey.image.as_slice()).expect("parse");
    let mut lines = Vec::new();
    for i in 0..seg.def_count() {
        let src = seg.callsign(Ordinal(i)).unwrap_or("?");
        for n in seg.out_edges(Ordinal(i)) {
            let dst = seg.callsign(n.other).unwrap_or("?");
            lines.push(format!("{src}\t{dst}\t{:?}\t{:?}", n.relation, n.grade));
        }
    }
    lines.sort();
    let mut text = lines.join("\n");
    let _ = write!(
        text,
        "\n# declined {} external {} method {} internal {}\n",
        survey.declined_refs,
        survey.declined_external,
        survey.declined_method,
        survey.declined_internal
    );
    std::fs::write(out, text).expect("write");
}
