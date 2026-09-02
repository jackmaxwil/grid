//! T-13: retained traces travel. `.grid/traces` is per machine, so the
//! traces CI refreshes never reach the store local selection reads unless
//! someone carries them — as a build artifact, which is a directory.
//!
//! Export is content-addressed and import is keyed by trace id, and the
//! bytes are copied rather than re-serialized, which is what makes a round
//! trip byte-identical and an epoch-0 record still an epoch-0 record on the
//! far side.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use grid_chart::{build, DatumId, Def, DefFacts, Segment};
use grid_trace::{grade, ingest, Store};
use grid_wire::{Grade, Trace, TraceEndpoint, TraceObservation};

fn def(callsign: &str, fill: u8) -> Def {
    Def {
        datum: DatumId([fill; 32]),
        callsign: callsign.to_owned(),
        lang: Some(grid_wire::Lang::Py),
        facts: Some(DefFacts {
            path: "m.py".into(),
            line: 1,
            line_end: 1,
            span: (0, 1),
            shape: [fill; 32],
            complexity: 1,
            sig: [fill; 32],
            cfg: String::new(),
        }),
    }
}

fn chart(defs: &[Def]) -> Segment<Vec<u8>> {
    Segment::parse(build(defs, &[]).expect("image")).expect("parse")
}

fn trace(id: &str, from: &DatumId, to: &DatumId) -> Trace {
    Trace {
        trace_id: id.to_owned(),
        git_rev: grid_wire::GitRev("abc123".into()),
        command: Some("tests::t".into()),
        started_at: None,
        snapshot: None,
        observations: vec![TraceObservation {
            caller: TraceEndpoint::Datum {
                datum: grid_wire::Datum(from.to_string()),
            },
            callee: TraceEndpoint::Datum {
                datum: grid_wire::Datum(to.to_string()),
            },
            count: Some(1),
        }],
    }
}

/// Every retained file, by name and bytes — the thing a round trip has to
/// leave alone.
fn files(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .expect("read dir")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).expect("read"),
            )
        })
        .collect();
    out.sort();
    out
}

/// A store holding two traces over a two-definition chart.
fn seeded(dir: &std::path::Path) -> Store {
    let (a, b) = (DatumId([1; 32]), DatumId([2; 32]));
    let segments = [chart(&[def("m.a", 1), def("m.b", 2)])];
    let store = Store::open(dir).expect("store");
    for id in ["trace:one", "trace:two"] {
        let w = trace(id, &a, &b);
        store.put(&w, &ingest(&segments, &w)).expect("retain");
    }
    store
}

/// Ground truth: a store exported to a directory and imported into an empty
/// one holds the same files, byte for byte. A re-serializing transport would
/// pass a round trip of its *meaning* and lose a record shape it does not
/// know how to write.
#[test]
fn a_round_trip_is_byte_identical() {
    let home = tempfile::tempdir().expect("tempdir");
    let away = tempfile::tempdir().expect("tempdir");
    let far = tempfile::tempdir().expect("tempdir");
    let source = seeded(&home.path().join("traces"));

    let moved = source.export(&away.path().join("out")).expect("export");
    assert_eq!(moved.written, ["trace:one", "trace:two"]);
    assert!(moved.unchanged.is_empty() && moved.conflicts.is_empty());

    let destination = Store::open(far.path().join("traces")).expect("store");
    let moved = destination
        .import(&away.path().join("out"))
        .expect("import");
    assert_eq!(moved.written, ["trace:one", "trace:two"]);
    assert_eq!(
        files(&home.path().join("traces")),
        files(&far.path().join("traces")),
        "the same names and the same bytes"
    );

    // And both directions are idempotent: run either again and nothing is
    // written, because content-addressing and id-keying both already answer.
    let again = source.export(&away.path().join("out")).expect("export");
    assert!(again.written.is_empty(), "{again:?}");
    assert_eq!(again.unchanged, ["trace:one", "trace:two"]);
    let again = destination
        .import(&away.path().join("out"))
        .expect("import");
    assert!(again.written.is_empty(), "{again:?}");
    assert_eq!(again.unchanged, ["trace:one", "trace:two"]);
}

/// Conservatism: an import that would put different content under an id the
/// store already holds is refused, whole, naming the id. A trace id names a
/// run; two records under one id are two claims about one run, and choosing
/// between them is not something an import gets to do.
#[test]
fn an_import_never_overwrites_a_retained_trace_with_different_content() {
    let home = tempfile::tempdir().expect("tempdir");
    let away = tempfile::tempdir().expect("tempdir");
    let store = seeded(&home.path().join("traces"));
    let before = files(&home.path().join("traces"));

    // The far side's `trace:one` watched a different hop, and its `trace:new`
    // is one this store has never seen.
    let (a, b, c) = (DatumId([1; 32]), DatumId([2; 32]), DatumId([3; 32]));
    let segments = [chart(&[def("m.a", 1), def("m.b", 2), def("m.c", 3)])];
    let elsewhere = Store::open(away.path().join("other")).expect("store");
    for (id, to) in [("trace:one", &c), ("trace:new", &b)] {
        let w = trace(id, &a, to);
        elsewhere.put(&w, &ingest(&segments, &w)).expect("retain");
    }
    elsewhere
        .export(&away.path().join("out"))
        .expect("export the far side");

    let moved = store.import(&away.path().join("out")).expect("import");
    assert_eq!(moved.conflicts, ["trace:one"], "the id is named");
    assert!(
        moved.written.is_empty(),
        "the refusal is whole: {moved:?} — `trace:new` was not written either"
    );
    assert_eq!(
        files(&home.path().join("traces")),
        before,
        "nothing on disk moved"
    );
}

/// A record written under wire epoch 0 travels as itself: the transport
/// copies bytes, so the far side reads it through the same fallback reader
/// that reads it at home rather than refusing a shape it cannot write.
#[test]
fn an_older_record_shape_travels_and_still_reads() {
    let home = tempfile::tempdir().expect("tempdir");
    let away = tempfile::tempdir().expect("tempdir");
    let far = tempfile::tempdir().expect("tempdir");
    let (a, b) = (DatumId([1; 32]), DatumId([2; 32]));
    let source = Store::open(home.path().join("traces")).expect("store");
    let record = format!(
        concat!(
            "{{\"wake\":{{\"wake_id\":\"wake:epoch-0\",\"git_rev\":\"abc123\",",
            "\"command\":\"tests::t\",",
            "\"observations\":[{{\"caller\":{{\"datum\":\"{a}\"}},",
            "\"callee\":{{\"datum\":\"{b}\"}},\"count\":1}}]}},",
            "\"trace\":[[\"{a}\",\"{b}\"]]}}"
        ),
        a = a,
        b = b,
    );
    let filed = home
        .path()
        .join("traces")
        .join(format!("{}.json", blake3::hash(b"wake:epoch-0").to_hex()));
    std::fs::write(&filed, &record).expect("write");

    source.export(&away.path().join("out")).expect("export");
    let destination = Store::open(far.path().join("traces")).expect("store");
    let moved = destination
        .import(&away.path().join("out"))
        .expect("import");
    assert_eq!(moved.written, ["wake:epoch-0"]);

    let there = destination.retained().expect("read");
    assert_eq!(there.len(), 1);
    assert_eq!(there[0].trace.trace_id, "wake:epoch-0");
    assert_eq!(there[0].hops, vec![(a, b)]);
}

/// An imported trace is graded exactly like a local one: it decays against
/// the chart being asked about, not against the chart it was taken on. A
/// trace carried in from another machine is evidence about the code it
/// watched, and against a snapshot that no longer holds those definitions it
/// is heuristic with `stale-trace` — the same answer, from the same one
/// place, as a trace this machine produced.
#[test]
fn an_imported_trace_decays_against_the_chart_it_is_asked_about() {
    let home = tempfile::tempdir().expect("tempdir");
    let away = tempfile::tempdir().expect("tempdir");
    let far = tempfile::tempdir().expect("tempdir");
    seeded(&home.path().join("traces"))
        .export(&away.path().join("out"))
        .expect("export");
    let destination = Store::open(far.path().join("traces")).expect("store");
    destination
        .import(&away.path().join("out"))
        .expect("import");

    let stored = destination.retained().expect("read");
    let edges: Vec<_> = stored.iter().flat_map(grid_trace::Stored::edges).collect();
    assert!(!edges.is_empty());

    // The snapshot the trace was taken against: both endpoints interned.
    let here = [chart(&[def("m.a", 1), def("m.b", 2)])];
    for edge in &edges {
        let graded = grade(&here, *edge);
        assert_eq!(graded.grade, Grade::Observed);
        assert_eq!(graded.reason, None);
    }
    // A snapshot where the callee has since been edited: demoted, with the
    // reason, and the retained file is untouched by the asking.
    let later = [chart(&[def("m.a", 1), def("m.b", 9)])];
    for edge in &edges {
        let graded = grade(&later, *edge);
        assert_eq!(graded.grade, Grade::Heuristic);
        assert_eq!(graded.reason, Some(grid_trace::STALE_TRACE));
    }
}
