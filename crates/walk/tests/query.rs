//! The wire-level surface: a `WalkQuery` in, a `WalkResult` out.
//!
//! `walk.rs` covers the traversal; this covers the translation — locator
//! resolution, the sweep/path-finding fork, and the promise that a rendered
//! path names real edges in their real direction.

use grid_chart::{build, DatumId, Def, Edge, Segment};
use grid_walk::{query, resolve, Error};
use grid_wire::{
    Callsign, Datum, Fuel, Grade, Handle, Lang, Locator, Relation, SnapshotId, WalkQuery,
    WalkQueryDirection, WalkResultStatus,
};

fn datum(seed: u8) -> DatumId {
    DatumId([seed; 32])
}

fn def(seed: u8, callsign: &str) -> Def {
    Def {
        datum: datum(seed),
        callsign: callsign.to_string(),
        lang: Some(Lang::Py),
        facts: None,
    }
}

fn edge(s: u8, d: u8) -> Edge {
    Edge {
        src: datum(s),
        dst: datum(d),
        relation: Relation::Calls,
        grade: Grade::Proven,
    }
}

/// `api.fetchCard -> models.CardModel -> db.row`, plus an unrelated leaf.
fn chain() -> Segment<Vec<u8>> {
    let defs = vec![
        def(0, "webapp.api.fetchCard"),
        def(1, "cortana.models.CardModel"),
        def(2, "cortana.db.row"),
        def(3, "webapp.api.other"),
    ];
    Segment::parse(build(&defs, &[edge(0, 1), edge(1, 2)]).unwrap()).unwrap()
}

fn snapshot() -> SnapshotId {
    SnapshotId("snap:b3:00".into())
}

fn callsign(name: &str) -> Locator {
    Locator::Callsign {
        callsign: Callsign(name.into()),
    }
}

fn sweep(from: Vec<Locator>, dir: WalkQueryDirection) -> WalkQuery {
    WalkQuery {
        from,
        to: None,
        direction: Some(dir),
        relations: None,
        at_or_above: None,
        fuel: None,
        snapshot: None,
    }
}

#[test]
fn a_sweep_answers_who_reaches_a_definition() {
    let seg = chain();
    let q = sweep(
        vec![callsign("cortana.models.CardModel")],
        WalkQueryDirection::In,
    );
    let r = query(&seg, &q, snapshot()).unwrap();
    assert_eq!(r.status, WalkResultStatus::Complete);
    let reached = r.reached.expect("a sweep answers `reached`");
    // The caller asked who reaches CardModel: fetchCard, and not the root.
    let names: Vec<&str> = reached
        .iter()
        .map(|d| d.callsign.as_ref().unwrap().0.as_str())
        .collect();
    assert_eq!(names, vec!["webapp.api.fetchCard"]);
    assert_eq!(reached[0].lang, Some(Lang::Py));
    // A sweep never fills the path-finding field.
    assert!(r.paths.is_none());
    assert!(r.frontier.is_none());
}

#[test]
fn a_query_with_to_answers_paths() {
    let seg = chain();
    let q = WalkQuery {
        to: Some(vec![callsign("cortana.db.row")]),
        ..sweep(
            vec![callsign("webapp.api.fetchCard")],
            WalkQueryDirection::Out,
        )
    };
    let r = query(&seg, &q, snapshot()).unwrap();
    let paths = r.paths.expect("path-finding answers `paths`");
    assert_eq!(paths.len(), 1);
    let p = &paths[0];
    // edges[i] joins nodes[i] to nodes[i+1] (contract, `Path`).
    assert_eq!(p.nodes.len(), 3);
    assert_eq!(p.edges.len(), 2);
    for (i, e) in p.edges.iter().enumerate() {
        assert_eq!(e.src, p.nodes[i]);
        assert_eq!(e.dst, p.nodes[i + 1]);
    }
    assert!(r.reached.is_none());
}

#[test]
fn an_unreached_target_is_an_empty_path_list_not_an_error() {
    let seg = chain();
    let q = WalkQuery {
        to: Some(vec![callsign("webapp.api.other")]),
        ..sweep(
            vec![callsign("webapp.api.fetchCard")],
            WalkQueryDirection::Out,
        )
    };
    let r = query(&seg, &q, snapshot()).unwrap();
    assert_eq!(r.paths.unwrap(), Vec::new());
}

#[test]
fn a_backwards_hop_still_names_the_real_source_and_destination() {
    let seg = chain();
    // Walking `both` from db.row crosses 1->2 against its direction; the
    // rendered edge must still say src=CardModel, dst=db.row.
    let q = WalkQuery {
        to: Some(vec![callsign("webapp.api.fetchCard")]),
        ..sweep(vec![callsign("cortana.db.row")], WalkQueryDirection::Both)
    };
    let r = query(&seg, &q, snapshot()).unwrap();
    let p = &r.paths.unwrap()[0];
    let d = |seed: u8| Datum(datum(seed).to_string());
    assert_eq!(p.edges[0].src, d(1));
    assert_eq!(p.edges[0].dst, d(2));
    assert_eq!(p.edges[1].src, d(0));
    assert_eq!(p.edges[1].dst, d(1));
}

#[test]
fn running_out_of_fuel_is_aground_with_a_frontier() {
    let seg = chain();
    let q = WalkQuery {
        fuel: Some(Fuel {
            max_hops: Some(1),
            max_nodes: Some(10),
            max_ms: None,
        }),
        ..sweep(
            vec![callsign("webapp.api.fetchCard")],
            WalkQueryDirection::Out,
        )
    };
    let r = query(&seg, &q, snapshot()).unwrap();
    assert_eq!(r.status, WalkResultStatus::Aground);
    // Aground shows its edge: the caller can raise the budget and ask again.
    assert_eq!(r.frontier.unwrap(), vec![Datum(datum(1).to_string())]);
}

#[test]
fn a_unique_name_suffix_resolves_but_an_ambiguous_one_does_not() {
    let seg = chain();
    assert_eq!(resolve(&seg, &callsign("fetchCard")).unwrap().0, 0);
    // `api.other` is unique; bare `api` is a prefix of nothing whole.
    assert_eq!(resolve(&seg, &callsign("api.other")).unwrap().0, 3);
    // A suffix must break on a dot: `Card` is not a suffix of `fetchCard`.
    assert!(matches!(
        resolve(&seg, &callsign("Card")),
        Err(Error::Unresolved(_))
    ));
    let defs = vec![def(0, "a.dup"), def(1, "b.dup")];
    let two = Segment::parse(build(&defs, &[]).unwrap()).unwrap();
    assert!(matches!(
        resolve(&two, &callsign("dup")),
        Err(Error::Unresolved(_))
    ));
}

#[test]
fn a_handle_resolves_by_its_name_half() {
    let seg = chain();
    let h = Locator::Handle {
        handle: Handle("fetchCard@b3:0000".into()),
    };
    assert_eq!(resolve(&seg, &h).unwrap().0, 0);
}

#[test]
fn a_datum_locator_resolves_by_identity() {
    let seg = chain();
    let l = Locator::Datum {
        datum: Datum(datum(2).to_string()),
    };
    assert_eq!(resolve(&seg, &l).unwrap().0, 2);
}

#[test]
fn coordinate_locators_say_they_need_an_index_rather_than_guessing() {
    let seg = chain();
    let l = Locator::FileLine {
        file: "a.py".into(),
        line: 3,
    };
    assert!(matches!(resolve(&seg, &l), Err(Error::Unsupported(_))));
}

#[test]
fn an_unknown_root_is_an_error_but_an_unknown_target_is_not() {
    let seg = chain();
    let q = sweep(vec![callsign("nope.nothing")], WalkQueryDirection::Out);
    assert!(matches!(
        query(&seg, &q, snapshot()),
        Err(Error::Unresolved(_))
    ));

    let q = WalkQuery {
        to: Some(vec![callsign("nope.nothing")]),
        ..sweep(
            vec![callsign("webapp.api.fetchCard")],
            WalkQueryDirection::Out,
        )
    };
    // An unknown target is simply not reachable: the empty answer is honest.
    assert_eq!(
        query(&seg, &q, snapshot()).unwrap().paths.unwrap(),
        Vec::new()
    );
}

#[test]
fn a_relation_filter_restricts_which_edges_are_followed() {
    let defs = vec![def(0, "a.f"), def(1, "b.g")];
    let constructs = Edge {
        relation: Relation::Constructs,
        ..edge(0, 1)
    };
    let seg = Segment::parse(build(&defs, &[constructs]).unwrap()).unwrap();
    let q = WalkQuery {
        relations: Some(vec![Relation::Calls]),
        ..sweep(vec![callsign("a.f")], WalkQueryDirection::Out)
    };
    assert_eq!(
        query(&seg, &q, snapshot()).unwrap().reached.unwrap(),
        Vec::new()
    );

    let q = WalkQuery {
        relations: Some(vec![Relation::Constructs]),
        ..sweep(vec![callsign("a.f")], WalkQueryDirection::Out)
    };
    assert_eq!(
        query(&seg, &q, snapshot()).unwrap().reached.unwrap().len(),
        1
    );
}

/// Claim C2 reaching through to queries: ordinal order is content order, so
/// the answer does not depend on `HashMap` iteration order.
#[test]
fn the_same_query_answers_the_same_bytes_every_time() {
    let seg = chain();
    let q = sweep(
        vec![callsign("webapp.api.fetchCard")],
        WalkQueryDirection::Out,
    );
    let once = serde_json::to_string(&query(&seg, &q, snapshot()).unwrap().reached).unwrap();
    for _ in 0..32 {
        let again = serde_json::to_string(&query(&seg, &q, snapshot()).unwrap().reached).unwrap();
        assert_eq!(once, again);
    }
}
