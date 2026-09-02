//! Concurrent publishers into one store: the two advertised journeys — a
//! `PostToolUse` hook on every edit, and parallel agents in one tree — both
//! write, and the daemon writes beside the CLI. Six surveys of different
//! revisions at once used to leave a segment holding another writer's bytes
//! under its own content hash, which every verb then refused forever.
//!
//! The reproduction is the test: N writers publish distinct images into a
//! shared store at once, then the store opens and every snapshot reopens.
//! Threads rather than processes, because the temp-name collision and the
//! membership read-modify-write are per store directory, not per process —
//! and the lock that closes them is an `flock`, which excludes separate
//! opens of one file within a process exactly as it does across processes.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::sync::Barrier;

use grid_chart::{build, DatumId, Def, Edge};
use grid_chart_io::Store;
use grid_wire::{Grade, Lang, Relation};

/// A distinct image per (writer, round): different bytes, different name.
fn image(writer: u8, round: u8) -> Vec<u8> {
    let fill = |i: u8| {
        DatumId([
            writer, round, i, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0,
        ])
    };
    let defs: Vec<Def> = (0..4)
        .map(|i| Def {
            datum: fill(i),
            callsign: format!("w{writer}.r{round}.f{i}"),
            lang: Some(Lang::Rs),
            facts: None,
        })
        .collect();
    let edges: Vec<Edge> = (0..3)
        .map(|i| Edge {
            src: fill(i),
            dst: fill(i + 1),
            relation: Relation::Calls,
            grade: Grade::Proven,
        })
        .collect();
    build(&defs, &edges).unwrap()
}

#[test]
fn concurrent_publishers_leave_a_store_that_opens_and_answers() {
    const WRITERS: u8 = 6;
    const ROUNDS: u8 = 12;
    let dir = tempfile::tempdir().unwrap();
    let start = Barrier::new(usize::from(WRITERS));
    let ids = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..WRITERS)
            .map(|writer| {
                let (dir, start) = (dir.path(), &start);
                scope.spawn(move || {
                    let mut store = Store::open(dir).unwrap();
                    start.wait();
                    (0..ROUNDS)
                        .map(|round| {
                            store
                                .publish(
                                    &format!("rev-{writer}-{round}"),
                                    None,
                                    &[image(writer, round)],
                                )
                                .unwrap_or_else(|e| {
                                    panic!("writer {writer} round {round}: publish failed: {e}")
                                })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });

    // The store opens, live, and every snapshot ever published reopens.
    let store = Store::open(dir.path())
        .unwrap_or_else(|e| panic!("store did not open after concurrent publishes: {e}"));
    assert!(!store.load().segments().is_empty());
    for id in &ids {
        let epoch = store
            .epoch_of(id)
            .unwrap_or_else(|e| panic!("snapshot {id} did not reopen: {e}"));
        assert_eq!(epoch.snapshot_id(), id);
    }
    // And the derived membership image saw every one of them: a lost
    // update there would make "unchanged since <snapshot>" an unknown.
    let membership = store.membership().unwrap().expect("membership image");
    let held: Vec<String> = membership.chain().into_iter().map(|(id, _)| id).collect();
    for id in &ids {
        assert!(
            held.contains(&id.as_str().to_owned()),
            "membership lost snapshot {id}"
        );
    }
    assert_eq!(ids.len(), usize::from(WRITERS) * usize::from(ROUNDS));
}
