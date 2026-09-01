//! Membership in the store: identity once, snapshots as reference sets.
//!
//! Three things are pinned here, and they are the three the plan names:
//! a membership diff agrees with the delta path it is meant to replace; an
//! image extended one snapshot at a time is byte-identical to one rebuilt
//! from the whole chain; and a snapshot the chain has never held is a typed
//! refusal, never an empty diff.
use grid_chart::{build, Change, DatumId, Def, DefFacts, Membership, MembershipError};
use grid_chart_io::Store;

fn def(callsign: &str, fill: u8) -> Def {
    Def {
        datum: DatumId([fill; 32]),
        callsign: callsign.to_owned(),
        lang: Some(grid_wire::Lang::Rs),
        facts: Some(DefFacts {
            path: "src/lib.rs".into(),
            line: 1,
            line_end: 1,
            span: (0, u32::from(fill)),
            shape: [fill; 32],
            complexity: u16::from(fill),
            sig: [fill; 32],
        }),
    }
}

fn image(defs: &[Def]) -> Vec<u8> {
    build(defs, &[]).expect("image")
}

/// The plan's ground truth: a membership merge-diff finds exactly the
/// definitions `made_good` reports as added and removed. The two paths share
/// no code — one is set algebra over ordinals, the other loads and compares
/// definitions — so agreement is evidence.
#[test]
fn a_membership_diff_agrees_with_the_delta_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::open(dir.path()).expect("store");
    let base = store
        .publish(
            "rev1",
            None,
            &[image(&[def("m.kept", 1), def("m.gone", 2)])],
        )
        .expect("publish base");
    let head = store
        .publish("rev2", None, &[image(&[def("m.kept", 1), def("m.new", 3)])])
        .expect("publish head");

    let membership = store.membership().expect("read").expect("built on publish");
    let mut mine = membership
        .changed_between(base.as_str(), head.as_str())
        .expect("both snapshots are retained");
    mine.sort_by_key(|c| match c {
        Change::Added(d) | Change::Removed(d) => d.0,
    });

    let delta = grid_chart::made_good(
        grid_wire::SnapshotId(base.as_str().to_owned()),
        store.epoch_of(&base).expect("base").segments(),
        grid_wire::SnapshotId(head.as_str().to_owned()),
        store.epoch_of(&head).expect("head").segments(),
    );
    let mut theirs: Vec<Change> = delta
        .defs
        .iter()
        .flat_map(|d| {
            [
                d.before
                    .as_ref()
                    .and_then(|b| DatumId::parse(&b.0))
                    .map(Change::Removed),
                d.after
                    .as_ref()
                    .and_then(|a| DatumId::parse(&a.0))
                    .map(Change::Added),
            ]
        })
        .flatten()
        .collect();
    theirs.sort_by_key(|c| match c {
        Change::Added(d) | Change::Removed(d) => d.0,
    });
    assert_eq!(mine, theirs);
    assert!(!mine.is_empty(), "a vacuous agreement proves nothing");
}

/// The determinism property, in the form the plan states it: extending an
/// image one snapshot at a time and rebuilding it from the whole chain are
/// the same bytes. If they were not, the cheap path would be a second,
/// diverging implementation of history.
#[test]
fn incremental_extension_and_a_full_rebuild_are_byte_identical() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::open(dir.path()).expect("store");
    store
        .publish("rev1", None, &[image(&[def("m.a", 1)])])
        .expect("publish");
    store
        .publish("rev2", None, &[image(&[def("m.a", 1), def("m.b", 2)])])
        .expect("publish");
    store
        .publish("rev3", None, &[image(&[def("m.b", 2), def("m.c", 3)])])
        .expect("publish");

    let incremental = store
        .membership()
        .expect("read")
        .expect("built")
        .bytes()
        .to_vec();
    store.rebuild_membership().expect("rebuild");
    let rebuilt = store
        .membership()
        .expect("read")
        .expect("built")
        .bytes()
        .to_vec();
    assert_eq!(incremental, rebuilt);
}

/// Conservatism: a snapshot the chain has never held is a typed refusal.
/// "Nothing changed" and "I have never heard of that snapshot" are opposite
/// answers, and returning the first for the second is the exact failure the
/// whole report shape exists to prevent.
#[test]
fn an_unknown_snapshot_is_a_refusal_not_an_empty_diff() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::open(dir.path()).expect("store");
    let real = store
        .publish("rev1", None, &[image(&[def("m.a", 1)])])
        .expect("publish");
    let membership = store.membership().expect("read").expect("built");

    let bogus = "snap:b3:0000000000000000000000000000000000000000000000000000000000000000";
    assert_eq!(
        membership.changed_between(real.as_str(), bogus),
        Err(MembershipError::UnknownSnapshot(bogus.to_owned()))
    );
    assert_eq!(
        membership.changed_between(bogus, real.as_str()),
        Err(MembershipError::UnknownSnapshot(bogus.to_owned()))
    );
    // And the real pair still answers, so the refusal is about the snapshot
    // rather than about the image being unusable.
    assert_eq!(
        membership.changed_between(real.as_str(), real.as_str()),
        Ok(Vec::new())
    );
}

/// "Unchanged" is id-equality — never a load, which is the fast path behind
/// the whole structure — and the cheap metadata rides along with it.
#[test]
fn membership_answers_holds_and_metadata_without_loading_a_definition() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::open(dir.path()).expect("store");
    let snap = store
        .publish("rev1", None, &[image(&[def("m.a", 7)])])
        .expect("publish");
    let membership = store.membership().expect("read").expect("built");
    let i = membership
        .index_of(snap.as_str())
        .expect("a known snapshot");

    assert!(membership.holds(i, &DatumId([7; 32])));
    assert!(!membership.holds(i, &DatumId([8; 32])));
    let ord = membership.ordinal(&DatumId([7; 32])).expect("interned");
    assert_eq!(membership.meta(ord), Some((7, 7)), "complexity and size");
}

/// The image is validated like a segment: hostile bytes reach validation and
/// nothing else.
#[test]
fn a_tampered_membership_image_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::open(dir.path()).expect("store");
    store
        .publish("rev1", None, &[image(&[def("m.a", 1)])])
        .expect("publish");
    let mut bytes = store
        .membership()
        .expect("read")
        .expect("built")
        .bytes()
        .to_vec();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    assert!(Membership::parse(bytes).is_err(), "the checksum covers it");

    // And a segment is not a membership image, however well-formed it is.
    assert!(Membership::parse(image(&[def("m.a", 1)])).is_err());
}
