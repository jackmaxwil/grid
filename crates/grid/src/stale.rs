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
//! no second copy of the name anywhere.
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
    let traces = grid_wake::Store::open(crate::rpc::trace_dir(root))
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
        let attributed = entry
            .trace_id
            .as_deref()
            .and_then(|id| test_of.get(id))
            .zip(entry.env_hash.as_deref());
        match attributed {
            Some((test, env)) => {
                envs.entry(test).or_default().insert(env);
            }
            // Either the verdict named no trace, or its trace is not retained,
            // or it predates the environment column. All three mean the same
            // thing: nobody can say which test it belongs to.
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
            observations: Vec::new(),
        };
        let store = grid_wake::Store::open(crate::rpc::trace_dir(dir.path())).expect("trace store");
        let empty: [Segment<Vec<u8>>; 0] = [];
        store
            .put(&trace, &grid_wake::ingest(&empty, &trace))
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
}
