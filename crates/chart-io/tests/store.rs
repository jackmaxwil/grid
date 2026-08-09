//! Store behaviour: epoch swap, D5 idempotence, and refusal to open a store
//! whose bytes no longer match its manifest.
use grid_chart::{build, DatumId, Def, Edge};
use grid_chart_io::{Store, StoreError, MANIFEST};
use grid_wire::{Lang, Relation, Zoc};

fn image(n: u8) -> Vec<u8> {
    let defs: Vec<Def> = (0..=n)
        .map(|i| Def {
            datum: DatumId([i; 32]),
            callsign: format!("pkg.f{i}"),
            lang: Some(Lang::Rs),
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
