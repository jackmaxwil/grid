//! Store behaviour: epoch swap, D5 idempotence, and refusal to open a store
//! whose bytes no longer match its manifest.
use grid_chart::{build, DatumId, Def, Edge};
use grid_chart_io::{Store, StoreError, MANIFEST};
use grid_wire::{Grade, Lang, Relation};

fn image(n: u8) -> Vec<u8> {
    let defs: Vec<Def> = (0..=n)
        .map(|i| Def {
            datum: DatumId([i; 32]),
            callsign: format!("pkg.f{i}"),
            lang: Some(Lang::Rs),
            facts: None,
        })
        .collect();
    let edges: Vec<Edge> = (0..n)
        .map(|i| Edge {
            src: DatumId([i; 32]),
            dst: DatumId([i + 1; 32]),
            relation: Relation::Calls,
            grade: Grade::Proven,
        })
        .collect();
    build(&defs, &edges).unwrap()
}

#[test]
fn fresh_directory_is_an_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("nested")).unwrap();
    assert!(store.load().segments().is_empty());
}

#[test]
fn publish_swaps_the_epoch_and_readers_keep_their_own() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    store.publish("rev1", None, &[image(3)]).unwrap();
    // A reader that took a snapshot before the swap still sees its own epoch:
    // that is the whole point of the arc-swap, so assert it rather than trust it.
    let held = store.load();
    let before = held.snapshot_id().clone();
    store.publish("rev2", None, &[image(9)]).unwrap();
    assert_eq!(held.snapshot_id(), &before);
    assert_ne!(store.load().snapshot_id(), &before);
    assert_eq!(store.load().segments()[0].def_count(), 10);
}

#[test]
fn republishing_the_same_state_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let a = store.publish("rev1", None, &[image(4)]).unwrap();
    let b = store.publish("rev1", None, &[image(4)]).unwrap();
    // D5: identical states dedupe to one snapshot, no minted ids.
    assert_eq!(a, b);
    let segs: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".seg"))
        .collect();
    assert_eq!(
        segs.len(),
        1,
        "identical content must not write a second file"
    );
}

#[test]
fn a_dirty_tree_publishes_a_different_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let clean = store.publish("rev1", None, &[image(2)]).unwrap();
    let dirty = store.publish("rev1", Some([7; 32]), &[image(2)]).unwrap();
    assert_ne!(clean, dirty);
}

#[test]
fn reopening_recovers_the_same_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let mut store = Store::open(dir.path()).unwrap();
        store.publish("rev1", None, &[image(5), image(2)]).unwrap()
    };
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.load().snapshot_id(), &id);
    assert_eq!(reopened.load().segments().len(), 2);
}

#[test]
fn an_invalid_image_never_enters_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let err = store.publish("rev1", None, &[vec![0u8; 100]]).unwrap_err();
    assert!(matches!(err, StoreError::Chart(_)));
    assert!(store.load().segments().is_empty());
    assert!(!dir.path().join(MANIFEST).exists());
}

#[test]
fn a_segment_rewritten_behind_the_manifest_is_caught() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = Store::open(dir.path()).unwrap();
        store.publish("rev1", None, &[image(3)]).unwrap();
    }
    let seg = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .find(|e| e.file_name().to_string_lossy().ends_with(".seg"))
        .unwrap()
        .path();
    // Grid never does this; an operator or a bit-flip might.
    let mut bytes = std::fs::read(&seg).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&seg, &bytes).unwrap();
    assert!(matches!(
        Store::open(dir.path()),
        Err(StoreError::ChecksumMismatch { .. })
    ));
}

#[test]
fn a_manifest_naming_a_missing_segment_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = Store::open(dir.path()).unwrap();
        store.publish("rev1", None, &[image(3)]).unwrap();
    }
    for e in std::fs::read_dir(dir.path()).unwrap().flatten() {
        if e.file_name().to_string_lossy().ends_with(".seg") {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
    assert!(matches!(Store::open(dir.path()), Err(StoreError::Io(_))));
}

#[test]
fn a_garbage_manifest_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(MANIFEST), b"not a manifest at all").unwrap();
    assert!(matches!(Store::open(dir.path()), Err(StoreError::Chart(_))));
    for cut in 0..40 {
        // Every truncation of a real manifest, too: the crash window.
        let real = grid_chart::Manifest {
            git_rev: "rev1".into(),
            worktree_diff_hash: None,
            segments: Vec::new(),
        }
        .canonical_bytes();
        let n = cut.min(real.len());
        std::fs::write(dir.path().join(MANIFEST), &real[..n]).unwrap();
        let _ = Store::open(dir.path());
    }
}

#[test]
fn segments_are_actually_mapped() {
    // Without this the sanitizer job could pass over the `Owned` path and
    // prove nothing about the one unsafe block it exists to cover.
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    store.publish("rev1", None, &[image(3)]).unwrap();
    let epoch = store.load();
    assert!(
        grid_chart_io::bytes_source(&epoch.segments()[0]) == grid_chart_io::Source::Mapped,
        "segment was not mmap'd — the unsafe path is untested"
    );
    // Touch every byte through the mapping so ASan/MIRI see the real reads.
    let seg = &epoch.segments()[0];
    for i in 0..seg.def_count() {
        let ord = grid_chart::Ordinal(i);
        assert!(seg.datum(ord).is_some());
        assert!(seg.callsign(ord).is_some());
        assert!(seg.out_edges(ord).count() + seg.in_edges(ord).count() > 0);
    }
}

#[test]
fn two_stores_over_the_same_tree_agree() {
    // The Wave 4 accept criterion in miniature: two surveys of the same tree
    // produce the same manifest hash.
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut sa = Store::open(a.path()).unwrap();
    let mut sb = Store::open(b.path()).unwrap();
    let images = [image(6), image(1)];
    let ida = sa.publish("deadbeef", None, &images).unwrap();
    // Same content, opposite order: the manifest sorts, so the id is equal.
    let flipped = [images[1].clone(), images[0].clone()];
    let idb = sb.publish("deadbeef", None, &flipped).unwrap();
    assert_eq!(ida, idb);
}

#[test]
fn a_superseded_snapshot_can_still_be_reopened() {
    // The gate compares base to head, and base is by definition a state the
    // store has moved past. Without retention there is nothing to compare to.
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let base = store.publish("rev1", None, &[image(2)]).unwrap();
    let head = store.publish("rev2", None, &[image(5)]).unwrap();
    assert_ne!(base, head);

    let reopened = store.epoch_of(&base).unwrap();
    assert_eq!(reopened.snapshot_id(), &base);
    assert_eq!(reopened.manifest().git_rev, "rev1");
    // And the live epoch is still head, unaffected by the lookup.
    assert_eq!(store.load().snapshot_id(), &head);
}

#[test]
fn reopening_a_snapshot_survives_a_reopened_store() {
    // Retention is on disk, not in memory: a daemon restart between base and
    // head must not lose the base.
    let dir = tempfile::tempdir().unwrap();
    let base = {
        let mut store = Store::open(dir.path()).unwrap();
        let base = store.publish("rev1", None, &[image(2)]).unwrap();
        store.publish("rev2", None, &[image(5)]).unwrap();
        base
    };
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.epoch_of(&base).unwrap().manifest().git_rev, "rev1");
}

#[test]
fn an_unpublished_snapshot_id_is_an_error_not_a_wrong_answer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let bogus = grid_chart::Manifest {
        git_rev: "never-published".into(),
        worktree_diff_hash: None,
        segments: Vec::new(),
    }
    .snapshot_id();
    assert!(matches!(
        store.epoch_of(&bogus),
        Err(StoreError::UnknownSnapshot(_))
    ));
}

#[test]
fn republishing_an_identical_state_is_idempotent_including_retention() {
    // D5: the id is the content, so this is the same snapshot, not a second
    // one that happens to look alike.
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let first = store.publish("rev1", None, &[image(3)]).unwrap();
    let again = store.publish("rev1", None, &[image(3)]).unwrap();
    assert_eq!(first, again);
    assert_eq!(store.epoch_of(&first).unwrap().snapshot_id(), &first);
}

#[test]
fn a_manifest_from_a_past_epoch_is_dropped_and_named_not_reinterpreted() {
    // ASSURANCE.md's declared epoch discipline: old bytes are refused and the
    // recovery is a re-survey. The refusal must not take the re-survey down
    // with it, so the store opens empty — and says which epoch it dropped, so
    // the emptiness is explained rather than mysterious.
    let dir = tempfile::tempdir().unwrap();
    let mut stale = b"grid-manifest/v1\n".to_vec();
    stale.extend_from_slice(&2u32.to_le_bytes());
    std::fs::write(dir.path().join("manifest"), stale).unwrap();

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.superseded(), Some(2));
    assert!(store.load().segments().is_empty());

    // A fresh store over a current manifest supersedes nothing.
    let mut store = Store::open(dir.path()).unwrap();
    store.publish("rev1", None, &[image(3)]).unwrap();
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.superseded(), None);
    assert_eq!(reopened.load().segments().len(), 1);
}

#[test]
fn a_corrupt_manifest_is_still_an_error() {
    // Only a *past epoch* is recoverable. Garbage is the store being wrong
    // about itself, and that must not read as "empty".
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("manifest"), b"not a manifest at all").unwrap();
    assert!(Store::open(dir.path()).is_err());
}

/// A dirty worktree survey is labelled with the rev it was checked out from,
/// and that label is provenance, not identity: a rev lookup answers with the
/// tree at the rev, never with the worktree — and a store holding only the
/// dirty survey holds nothing for the rev at all.
#[test]
fn a_dirty_worktree_snapshot_never_answers_for_its_rev() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let clean = store.publish("rev1", None, &[image(1)]).unwrap();
    // Newer by mtime, which is the tie-break the lookup uses.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let dirty = store.publish("rev1", Some([7; 32]), &[image(2)]).unwrap();
    assert_ne!(clean, dirty);
    assert_eq!(
        store.snapshot_for_rev("rev1").unwrap(),
        Some(clean.clone()),
        "the rev names the tree at the rev, never the dirty worktree labelled with it"
    );
    assert_eq!(
        store
            .epoch_for_rev("rev1")
            .unwrap()
            .map(|e| e.snapshot_id().clone()),
        Some(clean)
    );

    let only_dirty = tempfile::tempdir().unwrap();
    let mut store = Store::open(only_dirty.path()).unwrap();
    store.publish("rev1", Some([7; 32]), &[image(2)]).unwrap();
    assert_eq!(
        store.snapshot_for_rev("rev1").unwrap(),
        None,
        "a store holding only a dirty survey at rev1 does not hold rev1"
    );
}

/// A retained publish holds the snapshot — reopenable by id, findable by
/// rev, folded into membership — and changes nothing about what is live.
#[test]
fn publish_retained_holds_a_snapshot_without_making_it_live() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let live = store.publish("rev1", None, &[image(1)]).unwrap();
    let held = store.publish_retained("rev0", None, &[image(2)]).unwrap();
    assert_ne!(live, held);
    assert_eq!(store.load().snapshot_id(), &live, "live is untouched");
    assert_eq!(store.epoch_of(&held).unwrap().snapshot_id(), &held);
    assert_eq!(store.snapshot_for_rev("rev0").unwrap(), Some(held.clone()));
    assert!(store
        .membership()
        .unwrap()
        .expect("membership image")
        .chain()
        .iter()
        .any(|(id, _)| id == held.as_str()));
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.load().snapshot_id(), &live, "and on disk too");
}
