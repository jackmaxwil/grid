//! Test-only fault-injection victim: publishes segments in a tight loop until
//! it is killed. `tests/crash_safety.rs` spawns it, `kill -9`s it at a random
//! instant, and reopens the store — which must always be valid (plan Wave 4:
//! crash-safety is structural, not recovered).
use std::path::PathBuf;

use grid_chart::{build, DatumId, Def, Edge};
use grid_wire::{Lang, Relation, Zoc};

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: crash-writer <dir>"));
    let mut store = grid_chart_io::Store::open(&dir).expect("open");
    // Deterministic per-process sequence; the *timing* of the kill is the
    // random variable, which is the one that matters.
    let mut n: u8 = 0;
    loop {
        n = n.wrapping_add(1);
        let defs: Vec<Def> = (0..=n)
            .map(|i| Def {
                datum: DatumId([i; 32]),
                callsign: format!("pkg.f{i}"),
                lang: Some(Lang::Py),
                facts: None,
            })
            .collect();
        let edges: Vec<Edge> = (0..n)
            .map(|i| Edge {
                src: DatumId([i; 32]),
                dst: DatumId([i + 1; 32]),
                relation: Relation::Calls,
                zoc: Zoc::Proven,
            })
            .collect();
        let image = build(&defs, &edges).expect("build");
        store
            .publish(&format!("rev{n}"), None, std::slice::from_ref(&image))
            .expect("publish");
    }
}
