//! C4: no cache hit without an established closure.
//!
//! The claim is one-directional, so the test is too. It constructs arbitrary
//! charts and arbitrary mutations of them, and asserts the *only* thing that
//! makes soundness: if a key hits after a mutation, nothing the key's closure
//! covers can have changed. Misses are never asserted to be misses — a cache
//! that never hits is sound, just useless, and the useful direction is covered
//! separately by the hit tests below.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use grid_cache::{Cache, Chart, Entry, Kind, Reject, Verdict, FUEL};
use grid_chart::{build, DatumId, Def, Edge, Segment};
use grid_wire::{Grade, Lang, Relation};
use proptest::prelude::*;

/// Digest `n`: distinct datums that are cheap to talk about in a failure.
fn datum(n: u8) -> DatumId {
    let mut bytes = [0u8; 32];
    bytes[0] = n;
    DatumId(bytes)
}

fn def(n: u8, callsign: &str) -> Def {
    Def {
        datum: datum(n),
        callsign: callsign.to_string(),
        lang: Some(Lang::Py),
        facts: None,
    }
}

fn edge(src: u8, dst: u8, grade: Grade) -> Edge {
    Edge {
        src: datum(src),
        dst: datum(dst),
        relation: Relation::Calls,
        grade,
    }
}

fn segment(defs: &[Def], edges: &[Edge]) -> Segment<Vec<u8>> {
    Segment::parse(build(defs, edges).expect("build")).expect("parse")
}

const ENV: &str = "env:b3:cafecafe";

#[test]
fn lint_closure_is_the_datum_alone() {
    // A lint verdict must not be invalidated by a callee change: lint reads
    // one span. This is the claim that makes per-kind keys worth having.
    let defs = vec![def(1, "a"), def(2, "b")];
    let before = segment(&defs, &[edge(1, 2, Grade::Proven)]);
    let after = segment(
        &[def(1, "a"), def(3, "b_changed")],
        &[edge(1, 3, Grade::Proven)],
    );

    let key = |seg: &Segment<Vec<u8>>| {
        Chart::new(std::slice::from_ref(seg))
            .key(datum(1), Kind::Lint, ENV)
            .expect("key")
    };
    assert_eq!(key(&before), key(&after));
}

#[test]
fn typecheck_follows_proven_edges_and_stops_at_weaker_ones() {
    let defs = vec![def(1, "a"), def(2, "proven_callee"), def(3, "guessed")];
    let chart_defs = defs.clone();
    let seg = segment(
        &chart_defs,
        &[edge(1, 2, Grade::Proven), edge(1, 3, Grade::Heuristic)],
    );
    let chart = Chart::new(std::slice::from_ref(&seg));

    let closure = chart.closure(datum(1), Kind::Typecheck).expect("closure");
    assert_eq!(
        closure,
        BTreeSet::from([datum(1), datum(2)]),
        "typecheck trusts proven edges only"
    );

    let full = chart.closure(datum(1), Kind::Test).expect("closure");
    assert_eq!(
        full,
        BTreeSet::from([datum(1), datum(2), datum(3)]),
        "a test executes whatever it reaches, whatever grade the edge has"
    );
}

#[test]
fn a_callee_change_invalidates_its_callers() {
    // The wave-7 acceptance scenario, in miniature: b changes, a's test key
    // must move even though a itself is byte-identical.
    let seg = segment(
        &[def(1, "a"), def(2, "b")],
        &[edge(1, 2, Grade::Proven), edge(2, 4, Grade::Proven)],
    );
    let mutated = segment(
        &[def(1, "a"), def(5, "b")],
        &[edge(1, 5, Grade::Proven), edge(5, 4, Grade::Proven)],
    );

    for kind in [Kind::Typecheck, Kind::Test] {
        let before = Chart::new(std::slice::from_ref(&seg))
            .key(datum(1), kind, ENV)
            .expect("key");
        let after = Chart::new(std::slice::from_ref(&mutated))
            .key(datum(1), kind, ENV)
            .expect("key");
        assert_ne!(before, after, "{kind:?} key must move when a callee moves");
    }
}

#[test]
fn an_unresolvable_closure_is_rejected_not_stored() {
    let seg = segment(&[def(1, "a")], &[]);
    let chart = Chart::new(std::slice::from_ref(&seg));
    assert_eq!(
        chart.key(datum(9), Kind::Test, ENV),
        Err(Reject::UnknownDatum),
        "a datum outside the snapshot has no closure, so it gets no key"
    );
}

#[test]
fn a_closure_past_fuel_is_rejected() {
    // A chain longer than the budget: the walk must refuse rather than return
    // the prefix it managed, because a prefix key is a forged key.
    let n = FUEL + 2;
    let defs: Vec<Def> = (0..n)
        .map(|i| Def {
            datum: chain_datum(i),
            callsign: format!("n{i}"),
            lang: None,
            facts: None,
        })
        .collect();
    let edges: Vec<Edge> = (0..n - 1)
        .map(|i| Edge {
            src: chain_datum(i),
            dst: chain_datum(i + 1),
            relation: Relation::Calls,
            grade: Grade::Proven,
        })
        .collect();
    let seg = segment(&defs, &edges);
    let chart = Chart::new(std::slice::from_ref(&seg));
    assert_eq!(
        chart.key(chain_datum(0), Kind::Test, ENV),
        Err(Reject::FuelExhausted)
    );
    // Fuel bounds the walk, it does not bound the graph: a small closure in a
    // huge chart still resolves.
    assert!(chart.key(chain_datum(n - 1), Kind::Test, ENV).is_ok());
}

fn chain_datum(i: u32) -> DatumId {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&i.to_le_bytes());
    bytes[31] = 0xcc;
    DatumId(bytes)
}

#[test]
fn an_environment_change_invalidates_every_kind() {
    let seg = segment(&[def(1, "a")], &[]);
    let chart = Chart::new(std::slice::from_ref(&seg));
    for kind in [Kind::Lint, Kind::Typecheck, Kind::Test] {
        assert_ne!(
            chart.key(datum(1), kind, "env:b3:aaaa").expect("key"),
            chart.key(datum(1), kind, "env:b3:bbbb").expect("key"),
            "{kind:?} key must pin the environment"
        );
    }
}

#[test]
fn kinds_do_not_share_keys() {
    let seg = segment(&[def(1, "a")], &[]);
    let chart = Chart::new(std::slice::from_ref(&seg));
    // Even where the closures coincide (an isolated def), a lint pass must not
    // answer a test check.
    let keys: BTreeSet<_> = [Kind::Lint, Kind::Typecheck, Kind::Test]
        .into_iter()
        .map(|k| chart.key(datum(1), k, ENV).expect("key"))
        .collect();
    assert_eq!(keys.len(), 3);
}

#[test]
fn a_recorded_verdict_survives_reopening() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("verdicts/log");
    let seg = segment(&[def(1, "a")], &[]);
    let key = Chart::new(std::slice::from_ref(&seg))
        .key(datum(1), Kind::Lint, ENV)
        .expect("key");

    let mut cache = Cache::open(&path).expect("open");
    assert!(cache.is_empty());
    cache
        .put(
            key,
            Entry {
                verdict: Verdict::Fail,
                recorded_at: "2026-08-07T18:05:40Z".into(),
                trace_id: None,
                env_hash: Some("env".into()),
            },
        )
        .expect("put");

    let reopened = Cache::open(&path).expect("reopen");
    let entry = reopened.get(key).expect("hit after reopen");
    assert_eq!(entry.verdict, Verdict::Fail, "a failure caches too");
    assert_eq!(entry.recorded_at, "2026-08-07T18:05:40Z");
}

#[test]
fn a_torn_tail_line_is_dropped_not_guessed_at() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("log");
    let seg = segment(&[def(1, "a")], &[]);
    let key = Chart::new(std::slice::from_ref(&seg))
        .key(datum(1), Kind::Lint, ENV)
        .expect("key");
    let mut cache = Cache::open(&path).expect("open");
    cache
        .put(
            key,
            Entry {
                verdict: Verdict::Pass,
                recorded_at: "2026-08-07T18:05:40Z".into(),
                trace_id: Some("trace:1".into()),
                env_hash: Some("env".into()),
            },
        )
        .expect("put");

    // A crash mid-append leaves a partial line. It must cost the tail, not the
    // log.
    let mut text = std::fs::read_to_string(&path).expect("read");
    text.push_str("deadbeef pass 2026-0");
    std::fs::write(&path, text).expect("write");

    let reopened = Cache::open(&path).expect("reopen");
    assert_eq!(reopened.len(), 1);
    assert!(reopened.get(key).is_some());
}

/// An arbitrary small chart: defs 0..n, and edges among them.
fn chart_strategy() -> impl Strategy<Value = (Vec<Def>, Vec<Edge>)> {
    (1u8..8).prop_flat_map(|n| {
        let defs: Vec<Def> = (0..n).map(|i| def(i, &format!("d{i}"))).collect();
        let edges = proptest::collection::vec(
            (
                0..n,
                0..n,
                prop_oneof![
                    Just(Grade::Proven),
                    Just(Grade::Observed),
                    Just(Grade::Modeled),
                    Just(Grade::Heuristic)
                ],
            )
                .prop_map(|(s, d, z)| edge(s, d, z)),
            0..12,
        );
        (Just(defs), edges)
    })
}

proptest! {
    /// C4, stated as the property: a key that still hits after an arbitrary
    /// mutation covers a closure whose members all kept their content.
    ///
    /// Since a datum *is* its content, "kept their content" means "is still
    /// present". Any member vanishing while the key stayed equal would be a
    /// hit without an established closure.
    #[test]
    fn no_hit_survives_a_change_inside_its_closure(
        (defs, edges) in chart_strategy(),
        drop_ord in 0usize..8,
        kind_ix in 0usize..3,
    ) {
        let kind = [Kind::Lint, Kind::Typecheck, Kind::Test][kind_ix];
        let before = segment(&defs, &edges);
        let chart = Chart::new(std::slice::from_ref(&before));

        let subject = defs[0].datum;
        let Ok(key_before) = chart.key(subject, kind, ENV) else { return Ok(()); };
        let closure = chart.closure(subject, kind).expect("closure");

        // Mutate: drop one def (and every edge touching it).
        let victim = defs[drop_ord % defs.len()].datum;
        let kept: Vec<Def> = defs.iter().filter(|d| d.datum != victim).cloned().collect();
        if kept.is_empty() {
            return Ok(());
        }
        let kept_edges: Vec<Edge> = edges
            .iter()
            .filter(|e| e.src != victim && e.dst != victim)
            .cloned()
            .collect();
        let after = segment(&kept, &kept_edges);
        let after_chart = Chart::new(std::slice::from_ref(&after));

        match after_chart.key(subject, kind, ENV) {
            // The one forbidden outcome: the key still hits, but something the
            // verdict depended on is gone.
            Ok(key_after) if key_after == key_before => {
                prop_assert!(
                    !closure.contains(&victim),
                    "{kind:?} key survived the removal of {victim:?}, which its closure covers"
                );
            }
            // A moved key or a rejection are both sound: the cache misses.
            _ => {}
        }
    }

    /// A key is a pure function of (kind, env, closure): recomputing it must
    /// not drift, or every hit is a coincidence.
    #[test]
    fn keys_are_deterministic((defs, edges) in chart_strategy(), kind_ix in 0usize..3) {
        let kind = [Kind::Lint, Kind::Typecheck, Kind::Test][kind_ix];
        let seg = segment(&defs, &edges);
        let chart = Chart::new(std::slice::from_ref(&seg));
        for d in &defs {
            prop_assert_eq!(
                chart.key(d.datum, kind, ENV),
                chart.key(d.datum, kind, ENV)
            );
        }
    }

    /// Edge discovery order is not identity: the same graph, shuffled, keys the
    /// same. This is C2's shape, restated where the cache can feel it.
    #[test]
    fn key_ignores_edge_order((defs, edges) in chart_strategy(), kind_ix in 0usize..3) {
        let kind = [Kind::Lint, Kind::Typecheck, Kind::Test][kind_ix];
        let straight = segment(&defs, &edges);
        let mut reversed = edges.clone();
        reversed.reverse();
        let shuffled = segment(&defs, &reversed);
        prop_assert_eq!(
            Chart::new(std::slice::from_ref(&straight)).key(defs[0].datum, kind, ENV),
            Chart::new(std::slice::from_ref(&shuffled)).key(defs[0].datum, kind, ENV)
        );
    }
}
