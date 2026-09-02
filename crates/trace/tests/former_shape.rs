//! A trace retained under wire epoch 0 — `wake` and `trace` keys, a
//! `wake_id` — still reads after the naming plan's rename. The directory
//! rename moved every file whole; a file that then stopped parsing would be
//! a retained trace dropped, whatever the rename said.
use grid_chart::DatumId;
use grid_trace::Store;

fn epoch_zero_record(wake_id: &str, a: &DatumId, b: &DatumId) -> String {
    format!(
        concat!(
            "{{\"wake\":{{\"wake_id\":\"{id}\",\"git_rev\":\"abc123\",",
            "\"command\":\"tests::t\",",
            "\"observations\":[{{\"caller\":{{\"datum\":\"{a}\"}},",
            "\"callee\":{{\"datum\":\"{b}\"}},\"count\":1}}]}},",
            "\"trace\":[[\"{a}\",\"{b}\"]]}}"
        ),
        id = wake_id,
        a = a,
        b = b,
    )
}

#[test]
fn an_epoch_zero_record_is_read_not_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path()).expect("store");
    let (a, b) = (DatumId([1; 32]), DatumId([2; 32]));
    let wake_id = "wake:epoch-0";
    // Filed exactly where an epoch-0 build filed it: blake3 of the id.
    let path = dir.path().join(format!(
        "{}.json",
        blake3::hash(wake_id.as_bytes()).to_hex()
    ));
    std::fs::write(&path, epoch_zero_record(wake_id, &a, &b)).expect("write");

    let all = store.retained().expect("an epoch-0 record is not corrupt");
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].trace.trace_id, wake_id,
        "the id survives the key rename"
    );
    assert_eq!(all[0].trace.command.as_deref(), Some("tests::t"));
    assert_eq!(all[0].hops, vec![(a, b)]);

    let one = store
        .get(wake_id)
        .expect("read")
        .expect("filed under its id");
    assert_eq!(one.trace.observations.len(), 1);
}

/// The fallback reads the old shape only: bytes that are neither shape stay
/// corrupt, with the *current* shape's error — the one a file written today
/// is held to.
#[test]
fn bytes_that_are_neither_shape_are_still_corrupt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path()).expect("store");
    std::fs::write(dir.path().join("junk.json"), b"{\"neither\":1}").expect("write");
    let err = store.retained().expect_err("neither shape parses");
    assert!(err.to_string().contains("corrupt record"), "{err}");
}
