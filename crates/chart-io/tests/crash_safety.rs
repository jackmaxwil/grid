//! Crash safety, by execution rather than by argument: kill the writer with
//! SIGKILL at a random instant and reopen the store. Publication is a temp
//! write plus an atomic rename, so every possible interruption point must
//! leave either the old live set or the new one — never a manifest naming a
//! segment that is not fully there.
//!
//! Iterations default low enough for PR time; the soak job sets
//! `GRID_FAULT_ITERS=10000` (plan Wave 4 accept).
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn kill_nine_always_leaves_a_valid_store() {
    let iters: u32 = std::env::var("GRID_FAULT_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    let dir = tempfile::tempdir().expect("tempdir");
    let mut last_snapshot = None;
    let mut publishes_seen = 0u32;

    for i in 0..iters {
        let mut child = Command::new(env!("CARGO_BIN_EXE_crash-writer"))
            .arg(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn writer");
        // Sweep the delay so the kill lands at many different points of the
        // write/rename sequence rather than always the same one.
        let micros = 200 + u64::from(i % 37) * 900;
        std::thread::sleep(Duration::from_micros(micros));
        child.kill().expect("kill -9");
        child.wait().expect("reap");

        // The only assertion that matters: it reopens, every time.
        let store = grid_chart_io::Store::open(dir.path())
            .unwrap_or_else(|e| panic!("store invalid after kill at iteration {i}: {e}"));
        let epoch = store.load();
        for seg in epoch.segments() {
            // Accessors on a reopened segment are still total.
            for ord in 0..seg.def_count() {
                let ord = grid_chart::Ordinal(ord);
                assert!(seg.datum(ord).is_some());
                assert!(seg.callsign(ord).is_some());
            }
        }
        if epoch.manifest().git_rev.is_empty() {
            continue;
        }
        publishes_seen += 1;
        let id = epoch.snapshot_id().clone();
        assert!(id.as_str().starts_with("snap:b3:"));
        last_snapshot = Some(id);
    }

    // A loop that never got far enough to publish would pass vacuously; this
    // is the same "never silently no-op" discipline as the Wave 2 smoke.
    assert!(
        publishes_seen > 0,
        "fault injection never observed a published epoch — the harness proved nothing"
    );
    assert!(last_snapshot.is_some());
}
