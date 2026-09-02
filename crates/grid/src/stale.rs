//! `grid drift --tests` — which recorded verdicts no longer apply.
//!
//! The cache already knows a test's closure changed, but only when asked
//! about a specific key, and nobody asks. Stale verdicts need *enumeration*:
//! pushed, not pulled.
//!
//! The registry exists already and implicitly: every `grid trace ingest --test
//! <name>` names a test, and a retained trace carries that name. So the set of
//! known tests is the set of names the trace store holds, and a verdict is
//! attributed to a test through the trace id the cache line already records —
//! no second copy of the name anywhere. A run that produced no trace has one
//! more thing to say and says it in the same column: [`TEST_ATTRIBUTION`].
//!
//! For each known test, its closure key is recomputed at head and looked up:
//!
//! - present -> fresh, and nothing is printed;
//! - absent, with an older verdict on record -> **verdict-stale**;
//! - the closure walk rejected (fuel, or a datum the chart does not hold) ->
//!   *also* reported, because a rejection is a miss and the cache's
//!   one-directional soundness says so.
//!
//! Staleness here is proven-grade: a closure key is identity math, not a
//! guess. So it gates.
use std::collections::{BTreeMap, BTreeSet};

use grid_wire::{DriftWarning, StaleTest, StaleTestReason};

/// The namespace `grid test` files an untraced verdict under: `test:` and the
/// test's callsign. No trace file is written and none is implied — the prefix
/// is an *attribution*, the one thing the run knows about a verdict it
/// recorded itself and which a coverage report would otherwise have to carry.
pub const TEST_ATTRIBUTION: &str = "test:";

/// The test a recorded verdict belongs to: the retained trace's command, or
/// the `test:` attribution `grid test` wrote when the run produced no trace.
///
/// One-directional, like everything else on this path. An id that names
/// neither is `None`, and `None` is counted unattributed rather than guessed
/// at; an empty `test:` suffix names no test and so is no attribution.
pub fn attributed<'a>(test_of: &BTreeMap<&'a str, &'a str>, trace_id: &'a str) -> Option<&'a str> {
    test_of.get(trace_id).copied().or_else(|| {
        trace_id
            .strip_prefix(TEST_ATTRIBUTION)
            .filter(|name| !name.is_empty())
    })
}

/// What the enumeration found.
pub struct Report {
    /// Tests whose recorded verdict no longer applies, sorted by name.
    pub stale: Vec<StaleTest>,
    /// Recorded verdicts that name no retained trace, so no test owns them.
    /// Counted rather than ignored: an unattributed verdict is one this
    /// enumeration silently did not check.
    pub unattributed: u64,
    /// Tests the head chart cannot resolve to exactly one definition. Their
    /// verdicts cannot be re-keyed at all, which is worth saying and is not
    /// a divergence to gate on.
    pub warnings: Vec<DriftWarning>,
}

/// Enumerate the known tests and check each one's recorded verdicts against
/// the head chart.
///
/// # Errors
/// A human-readable message when the trace store or the verdict log cannot be
/// read. Neither is an empty set: a store that fails to open has unknown
/// contents, and reporting "no stale tests" for it would be exactly the
/// dishonest silence this verb exists to end.
pub fn tests<B: AsRef<[u8]>>(
    root: &std::path::Path,
    head: &[grid_chart::Segment<B>],
    resolve: impl Fn(&str) -> Option<grid_chart::DatumId>,
) -> Result<Report, String> {
    let traces = crate::rpc::trace_dir(root)
        .and_then(grid_trace::Store::open)
        .and_then(|store| store.retained())
        .map_err(|e| e.to_string())?;
    // trace id -> the test that produced it. A trace with no command names no
    // test, and its verdicts are unattributed rather than guessed at.
    let test_of: BTreeMap<&str, &str> = traces
        .iter()
        .filter_map(|w| Some((w.trace.trace_id.as_str(), w.trace.command.as_deref()?)))
        .collect();

    let cache = grid_cache::Cache::open(crate::rpc::cache_path(root)).map_err(|e| e.to_string())?;
    let chart = grid_cache::Chart::new(head);

    // test -> the environments its verdicts were recorded in. A verdict is
    // only re-keyable in the environment it was taken in, so this is the
    // pairing the recomputation needs.
    let mut envs: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut unattributed = 0u64;
    for (_, entry) in cache.entries() {
        let owner = entry
            .trace_id
            .as_deref()
            .and_then(|id| attributed(&test_of, id))
            .zip(entry.env_hash.as_deref());
        match owner {
            Some((test, env)) => {
                envs.entry(test).or_default().insert(env);
            }
            // Either the verdict named no trace and no test, or its trace is
            // not retained, or it predates the environment column. All three
            // mean the same thing: nobody can say which test it belongs to.
            None => unattributed = unattributed.saturating_add(1),
        }
    }

    let mut stale = Vec::new();
    let mut warnings = Vec::new();
    for (test, environments) in envs {
        let Some(datum) = resolve(test) else {
            warnings.push(DriftWarning {
                subject: test.to_owned(),
                message: "no longer resolves to one definition — its recorded verdicts \
                          cannot be re-keyed"
                    .to_owned(),
            });
            continue;
        };
        for env in environments {
            match chart.key(datum, grid_cache::Kind::Test, env) {
                // The verdict recorded for this environment still keys to
                // something on record: the closure is unchanged, so the
                // verdict still applies. Silence is the right output.
                Ok(key) if cache.get(key).is_some() => {}
                Ok(_) => stale.push(StaleTest {
                    test: test.to_owned(),
                    reason: StaleTestReason::ClosureChanged,
                }),
                // A rejection is a miss, which is the cache crate's
                // one-directional claim restated here: it is never a hit, so
                // it is never fresh.
                Err(_) => stale.push(StaleTest {
                    test: test.to_owned(),
                    reason: StaleTestReason::ClosureRejected,
                }),
            }
        }
    }
    // One row per test: two environments disagreeing is still one test to
    // rerun, and the stronger reason is the one that describes the work.
    stale.sort_by(|a, b| (&a.test, reason_rank(a.reason)).cmp(&(&b.test, reason_rank(b.reason))));
    stale.dedup_by(|a, b| a.test == b.test);
    Ok(Report {
        stale,
        unattributed,
        warnings,
    })
}

/// `closure-rejected` sorts first, so a test that is both rejected somewhere
/// and merely changed elsewhere reports the refusal — the stronger statement.
const fn reason_rank(reason: StaleTestReason) -> u8 {
    match reason {
        StaleTestReason::ClosureRejected => 0,
        StaleTestReason::ClosureChanged => 1,
    }
}

#[cfg(test)]
mod tests {
    //! The refusal arm, tested where it can be made to happen: a closure the
    //! chart cannot establish is the same `Err` the cache's fuel bound
    //! produces, and both must read as *stale*, never as fresh.
    use super::{tests as enumerate, StaleTestReason};
    use grid_chart::{DatumId, Segment};

    fn seeded(trace_id: &str, command: Option<&str>, env: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let trace = grid_wire::Trace {
            trace_id: trace_id.to_owned(),
            git_rev: grid_wire::GitRev("rev".into()),
            command: command.map(str::to_owned),
            started_at: None,
            snapshot: None,
            observations: Vec::new(),
        };
        let store = crate::rpc::trace_dir(dir.path())
            .and_then(grid_trace::Store::open)
            .expect("trace store");
        let empty: [Segment<Vec<u8>>; 0] = [];
        store
            .put(&trace, &grid_trace::ingest(&empty, &trace))
            .expect("retain");

        let mut cache = grid_cache::Cache::open(crate::rpc::cache_path(dir.path())).expect("cache");
        // The key's value is irrelevant: what matters is that an entry
        // exists and says which trace and which environment produced it.
        cache
            .put(
                seed_key(),
                grid_cache::Entry {
                    verdict: grid_cache::Verdict::Pass,
                    recorded_at: "1970-01-01T00:00:00Z".into(),
                    trace_id: Some(trace_id.to_owned()),
                    env_hash: env.map(str::to_owned),
                },
            )
            .expect("record");
        dir
    }

    /// A verdict on record with no retained trace behind it at all — the
    /// shape `grid test` writes when the run produced no coverage report.
    fn seeded_bare(trace_id: Option<&str>, env: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cache = grid_cache::Cache::open(crate::rpc::cache_path(dir.path())).expect("cache");
        cache
            .put(
                seed_key(),
                grid_cache::Entry {
                    verdict: grid_cache::Verdict::Pass,
                    recorded_at: "1970-01-01T00:00:00Z".into(),
                    trace_id: trace_id.map(str::to_owned),
                    env_hash: env.map(str::to_owned),
                },
            )
            .expect("record");
        dir
    }

    /// Any key at all; the enumeration never looks this one up by value.
    fn seed_key() -> grid_cache::Key {
        let seg = Segment::parse(
            grid_chart::build(
                &[grid_chart::Def {
                    datum: DatumId([1; 32]),
                    callsign: "m.t".into(),
                    lang: None,
                    facts: None,
                }],
                &[],
            )
            .expect("image"),
        )
        .expect("parse");
        grid_cache::Chart::new(std::slice::from_ref(&seg))
            .key(DatumId([1; 32]), grid_cache::Kind::Test, "env-1")
            .expect("a closure over one def")
    }

    /// Conservatism: a closure the chart cannot establish is a *miss*, and a
    /// miss is never a hit. Reporting the test as fresh would be the cache's
    /// one-directional soundness read backwards.
    #[test]
    fn a_closure_that_cannot_be_established_is_stale_never_fresh() {
        let dir = seeded("trace:t", Some("m.t"), Some("env-1"));
        let empty: [Segment<Vec<u8>>; 0] = [];
        let found =
            enumerate(dir.path(), &empty, |_| Some(DatumId([1; 32]))).expect("enumeration answers");
        assert_eq!(found.stale.len(), 1, "the refusal is reported");
        assert_eq!(found.stale[0].test, "m.t");
        assert_eq!(found.stale[0].reason, StaleTestReason::ClosureRejected);
        assert_eq!(found.unattributed, 0);
    }

    /// A verdict whose trace names no test cannot be attributed, and is
    /// counted rather than checked-and-passed.
    #[test]
    fn a_verdict_from_a_nameless_wake_is_unattributed() {
        let dir = seeded("trace:t", None, Some("env-1"));
        let empty: [Segment<Vec<u8>>; 0] = [];
        let found =
            enumerate(dir.path(), &empty, |_| Some(DatumId([1; 32]))).expect("enumeration answers");
        assert!(found.stale.is_empty());
        assert_eq!(found.unattributed, 1);
    }

    /// And one from before the environment column: same answer, because the
    /// key it was filed under cannot be recomputed without it.
    #[test]
    fn a_verdict_with_no_recorded_environment_is_unattributed() {
        let dir = seeded("trace:t", Some("m.t"), None);
        let empty: [Segment<Vec<u8>>; 0] = [];
        let found =
            enumerate(dir.path(), &empty, |_| Some(DatumId([1; 32]))).expect("enumeration answers");
        assert!(found.stale.is_empty());
        assert_eq!(found.unattributed, 1);
    }

    /// A verdict `grid test` recorded for a test that ran untraced names its
    /// test in the `test:` namespace, so it is enumerated and re-keyed like
    /// any other — not counted, forever, as one nobody can attribute.
    #[test]
    fn a_synthetic_test_attribution_names_its_test() {
        let dir = seeded_bare(Some("test:m.t"), Some("env-1"));
        let empty: [Segment<Vec<u8>>; 0] = [];
        let found =
            enumerate(dir.path(), &empty, |_| Some(DatumId([1; 32]))).expect("enumeration answers");
        assert_eq!(found.unattributed, 0, "the verdict names its own test");
        assert_eq!(found.stale.len(), 1);
        assert_eq!(found.stale[0].test, "m.t");
    }

    /// Conservatism: an entry with no attribution at all is still counted
    /// unattributed. The `test:` namespace attributes what `grid test` knew
    /// it was recording; it never licenses attributing everything else, and
    /// a bare `test:` names no test either.
    #[test]
    fn a_verdict_with_no_attribution_at_all_is_still_unattributed() {
        let empty: [Segment<Vec<u8>>; 0] = [];
        for id in [None, Some("test:"), Some("trace:not-retained")] {
            let dir = seeded_bare(id, Some("env-1"));
            let found = enumerate(dir.path(), &empty, |_| Some(DatumId([1; 32])))
                .expect("enumeration answers");
            assert!(found.stale.is_empty(), "{id:?} named a test");
            assert_eq!(found.unattributed, 1, "{id:?} was attributed to something");
        }
    }
}
