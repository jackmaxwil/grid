//! Green-but-stale: tests that proved nothing about the change they covered.
//!
//! A test that covered a definition, kept passing while that definition's
//! *body* changed, and was not itself touched, demonstrated nothing about the
//! change. Extreme-mutation research finds these in every studied project;
//! nobody pays mutation-testing prices to look for them. Grid holds all four
//! facts already, so naming them is free.
//!
//! The rule, over a patch `P` from base to head:
//!
//! - a definition `d` whose datum changed across `P`;
//! - a retained trace showing test `t` covering `d`, and still describing code
//!   base holds — a *decayed* trace produces no finding at all;
//! - `t`'s own datum unchanged across `P`;
//! - a recorded `Pass` for `t` at base's closure key **and** at head's.
//!
//! Then `t` is weak for `d`. `Grade::Heuristic` — a ranking, never a gate, and
//! the exit code does not move for it.
//!
//! The join runs over step 6's membership image: "datum changed" is a
//! merge-diff of two sorted rows, "test unchanged" is `holds`, an
//! id-comparison. Neither loads a definition, and no per-snapshot definition
//! set is materialised — the one un-lazified stage was 26.6% of `HyperDiff`'s
//! runtime, and that is the recorded lesson.
use std::collections::{BTreeMap, BTreeSet};

use grid_chart::{Change, DatumId, Membership, Segment};
use grid_wire::{Callsign, Datum, DefRef, WeakTest};

/// What the weak-test join found.
pub struct Found {
    /// One row per `(test, definition)` pair, sorted.
    pub weak: Vec<WeakTest>,
    /// Coverage facts declined because the trace that carried them had
    /// decayed. A heuristic built on a demoted heuristic is noise, so grid
    /// declines it — and counts, so the declining is visible.
    pub skipped: u64,
}

/// The pieces the join needs from the caller, so this module opens no store
/// of its own and the two snapshots are named once.
pub struct Inputs<'a, B: AsRef<[u8]>> {
    /// The membership image over the retained chain.
    pub membership: &'a Membership<B>,
    /// The snapshot the patch starts at.
    pub base: &'a str,
    /// The snapshot it ends at.
    pub head: &'a str,
    /// Base's segments — the closure key at base, and the chart a retained
    /// trace is graded against.
    pub base_segments: &'a [Segment<B>],
    /// Head's segments — the closure key at head.
    pub head_segments: &'a [Segment<B>],
}

/// Run the join.
///
/// # Errors
/// A human-readable message when a named snapshot is not in the retained
/// chain, or when the trace store or verdict log cannot be read. None of
/// those is an empty answer.
pub fn join<B: AsRef<[u8]>>(
    root: &std::path::Path,
    inputs: &Inputs<'_, B>,
) -> Result<Found, String> {
    // "Datum changed across P" — one merge-diff over two sorted rows. The
    // base-side halves are the definitions whose bodies moved or vanished.
    let moved: BTreeSet<DatumId> = inputs
        .membership
        .changed_between(inputs.base, inputs.head)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|c| match c {
            Change::Removed(d) => Some(d),
            Change::Added(_) => None,
        })
        .collect();
    if moved.is_empty() {
        return Ok(Found {
            weak: Vec::new(),
            skipped: 0,
        });
    }
    let head_index = inputs
        .membership
        .index_of(inputs.head)
        .ok_or_else(|| format!("snapshot {} is not in the retained chain", inputs.head))?;

    let traces = crate::rpc::trace_dir(root)
        .and_then(grid_trace::Store::open)
        .and_then(|store| store.retained())
        .map_err(|e| e.to_string())?;
    let cache = grid_cache::Cache::open(crate::rpc::cache_path(root)).map_err(|e| e.to_string())?;

    // The environments each test's verdicts were recorded in — the same
    // attribution `grid drift --tests` uses, through the trace id the cache
    // line already carries.
    let mut envs: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let test_of: BTreeMap<&str, &str> = traces
        .iter()
        .filter_map(|w| Some((w.trace.trace_id.as_str(), w.trace.command.as_deref()?)))
        .collect();
    for (_, entry) in cache.entries() {
        if let Some((test, env)) = entry
            .trace_id
            .as_deref()
            .and_then(|id| crate::stale::attributed(&test_of, id))
            .zip(entry.env_hash.as_deref())
        {
            envs.entry(test).or_default().insert(env);
        }
    }

    let base_chart = grid_cache::Chart::new(inputs.base_segments);
    let head_chart = grid_cache::Chart::new(inputs.head_segments);
    let mut weak: BTreeMap<(String, DatumId), WeakTest> = BTreeMap::new();
    let mut skipped = 0u64;

    for stored in &traces {
        let Some(test) = stored.trace.command.as_deref() else {
            continue;
        };
        for edge in stored.edges() {
            if !moved.contains(&edge.callee) {
                continue;
            }
            // Conservatism: coverage carried by a decayed trace is a
            // heuristic on top of a demoted heuristic, which is noise. Grid
            // declines it and counts the declining.
            if grid_trace::grade(inputs.base_segments, edge).grade != grid_wire::Grade::Observed {
                skipped = skipped.saturating_add(1);
                continue;
            }
            // The test itself must not have been touched: id-equality
            // against head's row, never a load.
            //
            // This is the cheap half of a condition the verdict check below
            // also enforces — an edited test has a new datum, so head holds
            // no closure key for the old one. Kept because it costs one
            // binary search and saves two closure walks, and because it is
            // the condition the rule is stated in.
            if !inputs.membership.holds(head_index, &edge.caller) {
                continue;
            }
            if !passed_on_both_sides(
                &cache,
                &base_chart,
                &head_chart,
                edge.caller,
                envs.get(test).map(|e| e.iter().copied()),
            ) {
                continue;
            }
            weak.entry((test.to_owned(), edge.callee))
                .or_insert_with(|| WeakTest {
                    test: test.to_owned(),
                    def: def_ref(inputs.base_segments, edge.callee),
                });
        }
    }
    Ok(Found {
        weak: weak.into_values().collect(),
        skipped,
    })
}

/// Did this test record a `Pass` under the same environment at *both*
/// endpoints?
///
/// Both sides matter and neither is enough. A pass only at base says the test
/// was not rerun; a pass only at head says nothing about what it used to
/// prove. The finding is precisely "it passed before, it passes now, and the
/// thing it covers is different".
fn passed_on_both_sides<'a, B: AsRef<[u8]>>(
    cache: &grid_cache::Cache,
    base: &grid_cache::Chart<'_, B>,
    head: &grid_cache::Chart<'_, B>,
    test: DatumId,
    environments: Option<impl Iterator<Item = &'a str>>,
) -> bool {
    let Some(environments) = environments else {
        return false;
    };
    environments.into_iter().any(|env| {
        let passed = |chart: &grid_cache::Chart<'_, B>| {
            chart
                .key(test, grid_cache::Kind::Test, env)
                .ok()
                .and_then(|key| cache.get(key))
                .is_some_and(|entry| entry.verdict == grid_cache::Verdict::Pass)
        };
        passed(base) && passed(head)
    })
}

/// The definition, named where base can name it. A `DefRef` rather than a
/// bare callsign so `--json` carries the identity the finding is about.
fn def_ref<B: AsRef<[u8]>>(segments: &[Segment<B>], datum: DatumId) -> DefRef {
    let named = segments.iter().find_map(|s| {
        let ord = s.ordinal(&datum)?;
        Some((s.callsign(ord)?.to_owned(), s.location(ord)))
    });
    DefRef {
        datum: Datum(datum.to_string()),
        callsign: named.as_ref().map(|(c, _)| Callsign(c.clone())),
        lang: None,
        signature: None,
        cfg: None,
        materialization: named.and_then(|(_, loc)| loc).map(|loc| {
            grid_wire::DefRefMaterialization {
                file: loc.path.to_owned(),
                line: u64::from(loc.line),
            }
        }),
    }
}
