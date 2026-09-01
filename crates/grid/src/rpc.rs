//! Method layer shared by CLI and daemon.
use grid_wire::methods::{
    CacheCheckParams, CacheRecordParams, CacheRecordResult, CacheRecordResultRejectedItem,
    DeltaParams, DeltaResult, GateCheckParams, GateCheckResult, QueryWalkParams, QueryWalkResult,
    ResolveParams, ResolveResult, ScopeParams, ScopeResult, SnapshotCurrentResult, SurveyRunParams,
    SurveyRunResult, TodoParams, TodoResult, TrackLintParams, TrackLintResult, VersionResult,
    WakeDiffParams, WakeIngestParams, WakeIngestResult,
};
use grid_wire::{
    CacheCheckResult, CacheHit, CacheHitVerdict, CacheResultEntryVerdict, GitRev, LeadlineResult,
    LeadlineResultStats, LeadlineResultStatus, Snapshot, SnapshotId, SnapshotOrRev, SnapshotStats,
};

use grid_xte::DeltaProvider as _;

use crate::chart_provider::{self, ChartDelta};
use crate::{standing, ENGINE_VERSION, WIRE_VERSION};

/// `grid.version` — versions plus the current snapshot, if the repo has one.
///
/// A repo that has never been surveyed reports no snapshot rather than a
/// placeholder: the field means "what the chart currently holds".
#[must_use]
pub fn version(root: &std::path::Path) -> VersionResult {
    VersionResult {
        wire: WIRE_VERSION.to_string(),
        engine: ENGINE_VERSION.to_string(),
        porcelain: Some(crate::porcelain::VERSION.to_string()),
        snapshot: ChartDelta::open(root).ok().and_then(|c| c.current()),
    }
}

/// `grid.track.lint` — pure validation, no survey, no snapshot. Pure of the
/// chart, that is: the repo's standing invariants are part of the rules a
/// promise is judged against, and reading them touches no snapshot.
///
/// # Errors
/// Returns a message when the standing invariant file exists but is invalid;
/// a broken rule file must not be mistaken for an absence of rules.
pub fn track_lint(
    params: &TrackLintParams,
    root: &std::path::Path,
) -> Result<TrackLintResult, String> {
    let track = standing::fold(&params.track, standing::load(root)?);
    Ok(grid_xte::lint::lint(&track))
}

/// `grid.gate.check` — the track against the delta at head.
///
/// The provider is the chart now, not Wave 3's file-level stand-in, so a
/// promise about an edge is checked against a surveyed edge rather than
/// reported `missing` for want of one.
///
/// Both endpoints are surveyed on demand. A gate that failed because nobody
/// had run a survey would be a gate that reports the tool's state instead of
/// the code's, and CI would have to learn a second command to avoid it.
///
/// # Errors
/// Propagates the provider's error (unknown snapshot, failed survey).
pub fn gate_check(
    params: &GateCheckParams,
    root: &std::path::Path,
) -> Result<GateCheckResult, chart_provider::Error> {
    let standing = standing::load(root).map_err(chart_provider::Error::Config)?;
    let track = standing::fold(&params.track, standing);
    let mut chart = Chart::new(root);
    chart.ensure(&track.base)?;
    chart.ensure(&params.head)?;
    grid_xte::check(&track, &params.head, &ChartDelta::open(root)?)
}

/// What one survey *run* did, as opposed to what the snapshot *is*. Neither
/// field is a wire field: they describe this execution, which is why they are
/// returned beside the snapshot rather than inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ran {
    /// Files whose facts came from the cache rather than the parser.
    pub reread: u64,
    /// Format epoch of a store this run superseded, if it found one. A store
    /// written by an older epoch is dropped, not reinterpreted, and this is
    /// how the caller gets to say so instead of the re-survey looking
    /// unexplained.
    pub superseded: Option<u32>,
    /// References the resolver declined to bind, so no edge exists for them.
    /// Every consumer that reads the edge set as "everything that reaches
    /// this" is wrong by at most this much, and can now say so.
    pub declined_refs: u64,
}

/// The repo's segment store, plus the survey cache that lives beside it.
///
/// The cache is in-process and per-call for the CLI (one survey per process,
/// so nothing to reuse) and held by the daemon across calls, which is what
/// makes a warm survey warm.
pub struct Chart {
    root: std::path::PathBuf,
    cache: grid_survey::Cache,
}

impl Chart {
    /// A chart rooted at `root`.
    #[must_use]
    pub fn new(root: &std::path::Path) -> Self {
        Self {
            root: root.to_path_buf(),
            cache: grid_survey::Cache::new(),
        }
    }

    /// Publish a snapshot for `target` if the store has none.
    ///
    /// A snapshot id must already exist — it names content, so it cannot be
    /// conjured — but a rev can always be surveyed, and doing it here is what
    /// lets `grid gate` be the only command CI runs.
    ///
    /// # Errors
    /// [`chart_provider::Error`] if the rev cannot be surveyed or published.
    pub fn ensure(&mut self, target: &SnapshotOrRev) -> Result<(), chart_provider::Error> {
        let SnapshotOrRev::GitRev { git_rev } = target else {
            return Ok(());
        };
        // Already surveyed at this rev: surveying again would produce the same
        // bytes and the same id (D5), so it is pure cost.
        if ChartDelta::open(&self.root)?.has_rev(&git_rev.0)? {
            return Ok(());
        }
        self.survey_run(&SurveyRunParams {
            git_rev: Some(git_rev.clone()),
            worktree: None,
        })
        .map(|_| ())
    }

    /// `grid.survey.run` — survey the tree and publish a snapshot.
    ///
    /// Idempotent by construction (D5): an unchanged tree produces the same
    /// segment bytes, so the same manifest, so the same id. Surveying twice
    /// is not a second snapshot.
    ///
    /// # Errors
    /// [`chart_provider::Error`] if the tree cannot be surveyed or the store
    /// cannot be written.
    pub fn survey_run(
        &mut self,
        params: &SurveyRunParams,
    ) -> Result<(SurveyRunResult, Ran), chart_provider::Error> {
        // Surveying "at a rev" means the tree at that rev, read out of the
        // object store; with no rev it is the working tree, labelled with the
        // rev it was checked out from. Resolving the label is a git
        // subprocess that reads nothing the survey writes, so it runs
        // alongside the survey instead of after it.
        let named = params.git_rev.as_ref().map_or("HEAD", |rev| rev.0.as_str());
        let root = &self.root;
        let (survey, rev) = std::thread::scope(|scope| {
            let resolver = scope.spawn(move || chart_provider::resolve_rev(root, named));
            let survey = match params.git_rev.as_ref() {
                Some(rev) => grid_survey::run_at(root, &rev.0, &mut self.cache),
                None => grid_survey::run(root, &mut self.cache),
            };
            let rev = resolver
                .join()
                .unwrap_or_else(|_| chart_provider::resolve_rev(root, named));
            (survey, rev)
        });
        let survey = survey?;
        // The daemon is the single writer (D6); `publish` takes `&mut self`,
        // which is that rule in the type system.
        let mut store = grid_chart_io::Store::open(chart_provider::store_dir(&self.root))
            .map_err(chart_provider::Error::Store)?;
        let superseded = store.superseded();
        let prev = store.load();
        let id = store
            .publish(&rev, None, &[survey.image])
            .map_err(chart_provider::Error::Store)?;
        // Every modification between the previous live set and this one is a
        // leg: edits made outside `grid edit` are still captured, one leg
        // per survey interval. Best effort — an unwritable log costs
        // history, never the snapshot that already published.
        if !prev.segments().is_empty() && prev.snapshot_id() != &id {
            let next = store.load();
            let delta = grid_chart::made_good(
                grid_wire::SnapshotId(prev.snapshot_id().as_str().to_owned()),
                prev.segments(),
                grid_wire::SnapshotId(id.as_str().to_owned()),
                next.segments(),
            );
            let legs: Vec<grid_wire::Leg> = delta
                .defs
                .iter()
                .filter(|d| {
                    matches!(
                        d.action,
                        grid_wire::DefChangeAction::Modified | grid_wire::DefChangeAction::Moved
                    )
                })
                .filter_map(|d| {
                    Some(grid_wire::Leg {
                        from: d.before.clone()?,
                        to: d.after.clone()?,
                        git_rev: Some(GitRev(rev.clone())),
                        at: None,
                    })
                })
                .collect();
            let _unwritable = crate::legs::append(&self.root, &legs);
        }
        Ok((
            Snapshot {
                snapshot_id: SnapshotId(id.as_str().to_owned()),
                git_rev: GitRev(rev),
                created_at: String::new(),
                worktree_diff_hash: None,
                stats: SnapshotStats {
                    defs: survey.defs,
                    edges: survey.edges,
                    langs: Some(survey.langs),
                },
            },
            Ran {
                reread: survey.reread,
                superseded,
                declined_refs: survey.declined_refs,
            },
        ))
    }
}

/// `grid.drift` — dependents of changed interfaces.
///
/// Both endpoints are surveyed on demand, exactly as `grid gate` surveys
/// them, so CI needs no second command. Head is special in one way: a
/// literal `HEAD` surveys the *working tree*, because drift's everyday
/// question is "what did my uncommitted edits leave stale", and because the
/// survey it runs is also where the declined-reference count comes from. Any
/// other rev is surveyed at that rev; a snapshot id is taken as published,
/// and its declined count is then unknown rather than zero.
///
/// # Errors
/// [`crate::drift::Error::Chart`] when an endpoint does not resolve;
/// [`crate::drift::Error::BudgetRejected`] when an inbound walk ran past its
/// fuel — refused whole, never truncated.
pub fn drift(
    params: &grid_wire::methods::DriftParams,
    root: &std::path::Path,
) -> Result<grid_wire::methods::DriftResult, crate::drift::Error> {
    let mut chart = Chart::new(root);
    chart.ensure(&params.base)?;
    let declined = match &params.head {
        SnapshotOrRev::GitRev { git_rev } => {
            let at = (git_rev.0 != "HEAD").then(|| git_rev.clone());
            let (_, ran) = chart.survey_run(&SurveyRunParams {
                git_rev: at,
                worktree: None,
            })?;
            Some(ran.declined_refs)
        }
        // Nobody re-surveyed head in this run, so nobody can say what its
        // survey declined. Unknown, not zero.
        SnapshotOrRev::Snapshot { .. } => None,
    };
    // A declaration file that exists but does not parse is a loud error,
    // never an absence of declarations: a covenant nobody can read is not a
    // covenant nobody made.
    let covenants = crate::covenant::load(root).map_err(crate::drift::Error::Covenants)?;
    let mut report = crate::drift::compute(
        root,
        &crate::drift::Query {
            floor: params
                .floor
                .unwrap_or(crate::drift::Query::new(&params.base, &params.head).floor),
            track: params.track.as_ref(),
            covenants: &covenants,
            declined_refs: declined,
            ..crate::drift::Query::new(&params.base, &params.head)
        },
    )?;
    if params.tests == Some(true) {
        let chart = ChartDelta::open(root).map_err(crate::drift::Error::Chart)?;
        // Head, because the question is whether a recorded verdict still
        // applies *now* — the closure key is identity math over the current
        // chart, not over the one the verdict was taken against.
        let epoch = chart
            .epoch_of(&params.head)
            .map_err(crate::drift::Error::Chart)?;
        let base_epoch = chart
            .epoch_of(&params.base)
            .map_err(crate::drift::Error::Chart)?;
        let found = crate::stale::tests(root, epoch.segments(), |test| {
            // The `grid.resolve` rule, so the name a user can resolve is the
            // name a verdict can be attributed by. Ambiguity resolves to
            // nothing: two definitions is not one closure.
            let hits = chart.resolve(&grid_wire::Locator::Callsign {
                callsign: grid_wire::Callsign(test.to_owned()),
            });
            match hits.as_slice() {
                [one] => grid_chart::DatumId::parse(&one.datum.0),
                _ => None,
            }
        })
        .map_err(crate::drift::Error::Stale)?;
        report.unchecked.unattributed_entries = Some(found.unattributed);
        report.stale_tests = Some(found.stale);
        if !found.warnings.is_empty() {
            report
                .warnings
                .get_or_insert_with(Vec::new)
                .extend(found.warnings);
        }
        // Green-but-stale, over step 6's membership. Absent membership is an
        // absence of derived data, not a claim that nothing is weak — the
        // report simply does not answer a question it could not ask.
        let store = grid_chart_io::Store::open(chart_provider::store_dir(root))
            .map_err(|e| crate::drift::Error::Stale(e.to_string()))?;
        if let Some(membership) = store
            .membership()
            .map_err(|e| crate::drift::Error::Stale(e.to_string()))?
        {
            let weak = crate::weak::join(
                root,
                &crate::weak::Inputs {
                    membership: &membership,
                    base: base_epoch.snapshot_id().as_str(),
                    head: epoch.snapshot_id().as_str(),
                    base_segments: base_epoch.segments(),
                    head_segments: epoch.segments(),
                },
            )
            .map_err(crate::drift::Error::Stale)?;
            report.unchecked.weak_skipped = Some(weak.skipped);
            report.weak = Some(weak.weak);
        }
    }
    Ok(report)
}

/// How many charted callsigns a pattern matches.
///
/// Zero is the answer `grid bind` refuses on: a declaration about code that
/// does not exist is a rule that will never fire.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn matching(root: &std::path::Path, pattern: &str) -> Result<usize, chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    Ok(live_defs(&chart)
        .into_iter()
        .filter(|(_, callsign, _, _)| grid_xte::pattern::matches(pattern, callsign))
        .count())
}

/// The `k` charted callsigns nearest to `name`: ranked by edit distance on
/// the last dotted segment (the part a caller most likely typed), full-name
/// distance as the tiebreak. This is what turns a silent miss into a hint.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn nearest(
    root: &std::path::Path,
    name: &str,
    k: usize,
) -> Result<Vec<String>, chart_provider::Error> {
    let tail = |s: &str| s.rsplit('.').next().unwrap_or(s).to_owned();
    let wanted = tail(name);
    let chart = ChartDelta::open(root)?;
    let mut scored: Vec<(usize, usize, String)> = live_defs(&chart)
        .into_iter()
        .map(|(_, callsign, _, _)| {
            (
                edit_distance(&wanted, &tail(&callsign)),
                edit_distance(name, &callsign),
                callsign,
            )
        })
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.2 == b.2);
    Ok(scored.into_iter().take(k).map(|(_, _, c)| c).collect())
}

/// Levenshtein distance, plain DP — the inputs are callsigns, short enough
/// that anything cleverer is noise.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i.saturating_add(1)];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            let best = sub
                .min(prev[j + 1].saturating_add(1))
                .min(row[j].saturating_add(1));
            row.push(best);
        }
        prev = row;
    }
    prev.last().copied().unwrap_or(0)
}

/// `grid resolve <file>` — every definition the file contains, in line
/// order: the "what is in here" opener, half of `scope` for a whole file.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn defs_in_file(
    root: &std::path::Path,
    path: &str,
) -> Result<Vec<grid_wire::DefRef>, chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    let mut out = Vec::new();
    for segment in chart.head().segments() {
        for i in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(i);
            let Some(loc) = segment.location(ord) else {
                continue;
            };
            if loc.path != path {
                continue;
            }
            let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) else {
                continue;
            };
            out.push(grid_wire::DefRef {
                datum: grid_wire::Datum(datum.to_string()),
                callsign: Some(grid_wire::Callsign(callsign.to_owned())),
                lang: segment.lang(ord),
                signature: None,
                materialization: Some(grid_wire::DefRefMaterialization {
                    file: loc.path.to_owned(),
                    line: u64::from(loc.line),
                }),
            });
        }
    }
    out.sort_by_key(|d| d.materialization.as_ref().map_or(0, |m| m.line));
    Ok(out)
}

/// Does a def materialized at `path` fall under the `--under` prefix?
/// Boundary-aware: `--under crates` keeps `crates/x.rs`, not `crates2/`.
/// A def with no recorded location (an epoch-1 segment) matches nothing.
fn in_subtree(under: Option<&str>, path: Option<&str>) -> bool {
    let Some(prefix) = under else { return true };
    let prefix = prefix.trim_end_matches('/');
    path.is_some_and(|p| {
        p == prefix
            || p.strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// `grid.orphans` — every definition with no proven inbound edge, in
/// callsign order, optionally one language's or one subtree's.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn orphans(
    root: &std::path::Path,
    lang: Option<grid_wire::Lang>,
    under: Option<&str>,
) -> Result<Vec<grid_wire::DefRef>, chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    let mut out = Vec::new();
    for segment in chart.head().segments() {
        for i in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(i);
            if segment
                .in_edges(ord)
                .any(|n| n.zoc == grid_wire::Zoc::Proven)
            {
                continue;
            }
            if lang.is_some() && segment.lang(ord) != lang {
                continue;
            }
            if !in_subtree(under, segment.location(ord).map(|loc| loc.path)) {
                continue;
            }
            let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) else {
                continue;
            };
            out.push(grid_wire::DefRef {
                datum: grid_wire::Datum(datum.to_string()),
                callsign: Some(grid_wire::Callsign(callsign.to_owned())),
                lang: segment.lang(ord),
                signature: None,
                materialization: segment.location(ord).map(|loc| {
                    grid_wire::DefRefMaterialization {
                        file: loc.path.to_owned(),
                        line: u64::from(loc.line),
                    }
                }),
            });
        }
    }
    out.sort_by(|a, b| {
        let key = |d: &grid_wire::DefRef| d.callsign.as_ref().map(|c| c.0.clone());
        key(a).cmp(&key(b))
    });
    Ok(out)
}

/// One hotspot row: a definition scored by complexity times churn.
#[derive(Debug, serde::Serialize)]
pub struct Hotspot {
    pub callsign: String,
    pub datum: String,
    pub complexity: u16,
    pub legs: u64,
    pub score: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<grid_wire::DefRefMaterialization>,
}

/// Where the churn multiplier came from — named so the caller can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChurnSource {
    /// The grid patch log: identity-level, rename-proof.
    Legs,
    /// `git log --numstat`, file-level: every def in a file inherits its
    /// file's commit count. Coarser, but honest on a fresh checkout.
    GitFiles,
    /// Neither exists; the ranking is complexity alone.
    None,
}

/// `grid.hotspots` — complexity × churn, descending. Churn is the number of
/// legs in the datum's identity component of the patch log, so it survives
/// renames; with no log yet it falls back to file-level git history, so a
/// fresh checkout still ranks by both axes.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn hotspots(
    root: &std::path::Path,
    limit: usize,
    under: Option<&str>,
) -> Result<(Vec<Hotspot>, ChurnSource), chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    // Union legs into identity components by plain map-merging: the log is
    // small (one row per recorded modification), so O(legs · chain) is fine.
    // ponytail: replace with union-find when a real log makes this slow.
    let legs = crate::legs::read(root);
    let mut component: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut next = 0usize;
    for leg in &legs {
        let ids = (
            component.get(&leg.from.0).copied(),
            component.get(&leg.to.0).copied(),
        );
        let id = match ids {
            (Some(a), _) => a,
            (None, Some(b)) => b,
            (None, None) => {
                next += 1;
                next
            }
        };
        component.insert(leg.from.0.clone(), id);
        component.insert(leg.to.0.clone(), id);
        if let (Some(a), Some(b)) = ids {
            if a != b {
                for v in component.values_mut() {
                    if *v == b {
                        *v = a;
                    }
                }
            }
        }
    }
    let mut per_component: std::collections::HashMap<usize, u64> = std::collections::HashMap::new();
    for leg in &legs {
        if let Some(id) = component.get(&leg.to.0) {
            *per_component.entry(*id).or_insert(0) += 1;
        }
    }
    let file_churn = if legs.is_empty() {
        git_file_churn(root)
    } else {
        std::collections::HashMap::new()
    };
    let source = if !legs.is_empty() {
        ChurnSource::Legs
    } else if file_churn.is_empty() {
        ChurnSource::None
    } else {
        ChurnSource::GitFiles
    };

    let mut out = Vec::new();
    for segment in chart.head().segments() {
        for i in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(i);
            let (Some(datum), Some(callsign), Some(complexity)) = (
                segment.datum(ord),
                segment.callsign(ord),
                segment.complexity(ord),
            ) else {
                continue;
            };
            if !in_subtree(under, segment.location(ord).map(|loc| loc.path)) {
                continue;
            }
            let datum = datum.to_string();
            let churn = match source {
                ChurnSource::GitFiles => segment
                    .location(ord)
                    .and_then(|loc| file_churn.get(loc.path))
                    .copied()
                    .unwrap_or(0),
                _ => component
                    .get(&datum)
                    .and_then(|id| per_component.get(id))
                    .copied()
                    .unwrap_or(0),
            };
            out.push(Hotspot {
                callsign: callsign.to_owned(),
                datum,
                complexity,
                legs: churn,
                score: u64::from(complexity).saturating_mul(1 + churn),
                materialization: segment.location(ord).map(|loc| {
                    grid_wire::DefRefMaterialization {
                        file: loc.path.to_owned(),
                        line: u64::from(loc.line),
                    }
                }),
            });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.callsign.cmp(&b.callsign)));
    out.truncate(limit);
    Ok((out, source))
}

/// Commits touching each file, from `git log --numstat`.
/// ponytail: capped at the last 1000 commits — churn is a ranking input,
/// and deep history stops changing the order long before it stops costing.
fn git_file_churn(root: &std::path::Path) -> std::collections::HashMap<String, u64> {
    let out = std::process::Command::new("git")
        .args(["log", "--numstat", "--format=", "-n", "1000"])
        .current_dir(root)
        .output();
    let Ok(out) = out else {
        return std::collections::HashMap::new();
    };
    if !out.status.success() {
        return std::collections::HashMap::new();
    }
    let mut churn = std::collections::HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // "added\tdeleted\tpath"; a rename spells "{a => b}/c" and is left
        // uncounted — an omission, never a wrong attribution.
        let mut parts = line.split('\t');
        let (Some(_), Some(_), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if path.contains("=>") {
            continue;
        }
        *churn.entry(path.to_owned()).or_insert(0) += 1;
    }
    churn
}

/// `grid.snapshot.current` — the published snapshot, or `None` for a store
/// that has never been surveyed.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn snapshot_current(
    root: &std::path::Path,
) -> Result<Option<SnapshotCurrentResult>, chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    let Some(id) = chart.current() else {
        return Ok(None);
    };
    Ok(Some(Snapshot {
        snapshot_id: id,
        git_rev: GitRev(chart.git_rev().to_owned()),
        created_at: String::new(),
        worktree_diff_hash: None,
        stats: chart.stats(),
    }))
}

/// `grid.delta` — the actual delta between two snapshots or revs.
///
/// # Errors
/// [`chart_provider::Error`] if either endpoint does not resolve.
pub fn delta(
    params: &DeltaParams,
    root: &std::path::Path,
) -> Result<DeltaResult, chart_provider::Error> {
    let mut chart = Chart::new(root);
    chart.ensure(&params.base)?;
    chart.ensure(&params.head)?;
    ChartDelta::open(root)?.made_good(&params.base, &params.head)
}

/// `grid.resolve` — a locator to the definitions it names.
///
/// An unknown locator resolves to an empty list, not an error: "nothing is
/// called that" is an answer, and the wire contract says so.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn resolve(
    params: &ResolveParams,
    root: &std::path::Path,
) -> Result<ResolveResult, chart_provider::Error> {
    Ok(ChartDelta::open(root)?.resolve(&params.locator))
}

/// `grid.query.walk` — the leadline engine over the live chart.
///
/// The chart is a set of segments and [`grid_leadline::query`] walks one
/// graph, so the walk runs per segment and the answers are merged. A segment
/// that holds none of the roots answers `Unresolved`; that is an absence
/// here, not an error, the same reading `grid.review.pack` takes.
///
/// Merging keeps the engine's meaning: `aground` anywhere is `aground`
/// overall (some frontier went unexpanded), the frontier is the union of the
/// unexpanded edges, and `reached`/`paths` stay exclusive — a sweep answers
/// `reached`, a path query answers `paths`, exactly as the single-graph call
/// does.
///
/// # Errors
/// [`chart_provider::Error::NoSnapshot`] when the store has never been
/// surveyed — there is no graph to walk. Other variants as in [`open_for`].
pub fn query_walk(
    params: &QueryWalkParams,
    root: &std::path::Path,
) -> Result<QueryWalkResult, chart_provider::Error> {
    let named = params
        .query
        .snapshot
        .clone()
        .map(|snapshot| SnapshotOrRev::Snapshot { snapshot });
    let chart = open_for(named.as_ref(), root)?;
    let Some(snapshot) = chart.current() else {
        return Err(chart_provider::Error::NoSnapshot);
    };
    let sweep = params.query.to.is_none();
    let mut merged = LeadlineResult {
        status: LeadlineResultStatus::Complete,
        snapshot: snapshot.clone(),
        paths: (!sweep).then(Vec::new),
        reached: sweep.then(Vec::new),
        frontier: None,
        stats: LeadlineResultStats {
            visited: 0,
            hops: 0,
            ms: 0,
            // Merged below: an answer merged from several segments is
            // complete only to the shallowest radius any of them certified,
            // and to nothing at all if one of them certified nothing.
            complete_within: None,
        },
    };
    let mut frontier: Vec<grid_wire::Datum> = Vec::new();
    let mut certified: Option<u64> = None;
    let mut contributed = false;
    for segment in chart.head().segments() {
        let Ok(part) = grid_leadline::query(segment, &params.query, snapshot.clone()) else {
            continue;
        };
        if part.status == LeadlineResultStatus::Aground {
            merged.status = LeadlineResultStatus::Aground;
        }
        merged.stats.visited += part.stats.visited;
        merged.stats.hops = merged.stats.hops.max(part.stats.hops);
        merged.stats.ms += part.stats.ms;
        // The weakest certificate wins: the union of several answers is
        // complete only as far as every contributing walk finished.
        // The weakest certificate wins, and `None` from any contributor
        // poisons the merge: one walk that certified nothing means the union
        // certifies nothing.
        certified = match (contributed, certified, part.stats.complete_within) {
            (false, _, first) => first,
            (true, Some(a), Some(b)) => Some(a.min(b)),
            (true, _, _) => None,
        };
        contributed = true;
        frontier.extend(part.frontier.into_iter().flatten());
        if let Some(reached) = &mut merged.reached {
            reached.extend(part.reached.into_iter().flatten());
        }
        if let Some(paths) = &mut merged.paths {
            paths.extend(part.paths.into_iter().flatten());
        }
    }
    // Deterministic, and one datum named once however many segments saw it.
    frontier.sort_by(|a, b| a.0.cmp(&b.0));
    frontier.dedup();
    merged.frontier = (!frontier.is_empty()).then_some(frontier);
    merged.stats.complete_within = certified;
    if let Some(reached) = &mut merged.reached {
        reached.sort_by(|a, b| a.datum.0.cmp(&b.datum.0));
        reached.dedup_by(|a, b| a.datum == b.datum);
    }
    Ok(merged)
}

/// Where the cache log and the chart store live under a repo root. Both are
/// engine state, so both sit under `.grid/`.
pub(crate) fn cache_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".grid/cache/verdicts")
}

fn chart_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".grid/chart")
}

/// Open the live epoch. An empty store is not an error — it is a snapshot with
/// no defs, against which every cache check misses and every record is
/// rejected, which is the honest answer before a survey has run.
fn epoch(root: &std::path::Path) -> Result<std::sync::Arc<grid_chart_io::Epoch>, String> {
    grid_chart_io::Store::open(chart_dir(root))
        .map(|store| store.load())
        .map_err(|e| e.to_string())
}

/// `grid.cache.check` — look up frozen verdicts for a set of datums.
///
/// A datum is a hit only if its closure, recomputed against the live chart
/// *now*, keys to something recorded. Anything else is a miss: an unknown
/// datum, an unresolvable closure, and an absent key are all just "run it".
///
/// # Errors
/// Returns a message when the chart store or the cache log cannot be read. A
/// cache that cannot be read is an error, not a universal miss: silently
/// missing would hide a broken store behind a slow CI.
pub fn cache_check(
    params: &CacheCheckParams,
    root: &std::path::Path,
) -> Result<CacheCheckResult, String> {
    let epoch = epoch(root)?;
    let request = &params.request;
    // The request may name a snapshot; the engine holds exactly one live
    // epoch, so a request for any other snapshot cannot be answered from it.
    if let Some(want) = &request.snapshot {
        if want.0 != epoch.snapshot_id().as_str() {
            return Err(format!("snapshot {} is not the live epoch", want.0));
        }
    }
    let cache = grid_cache::Cache::open(cache_path(root)).map_err(|e| e.to_string())?;
    let chart = grid_cache::Chart::new(epoch.segments());
    let kind = grid_cache::Kind::from(request.kind);

    let mut hits = Vec::new();
    let mut misses = Vec::new();
    for datum in &request.datums {
        let hit = grid_chart::DatumId::parse(&datum.0)
            .and_then(|id| chart.key(id, kind, &request.env_hash).ok())
            .and_then(|key| cache.get(key).cloned());
        match hit {
            Some(entry) => hits.push(CacheHit {
                datum: datum.clone(),
                verdict: match entry.verdict {
                    grid_cache::Verdict::Pass => CacheHitVerdict::Pass,
                    grid_cache::Verdict::Fail => CacheHitVerdict::Fail,
                },
                recorded_at: entry.recorded_at,
                wake_id: entry.wake_id,
            }),
            None => misses.push(datum.clone()),
        }
    }
    Ok(CacheCheckResult {
        snapshot: grid_wire::SnapshotId(epoch.snapshot_id().as_str().to_string()),
        hits,
        misses,
    })
}

/// `grid.cache.record` — freeze keys and store verdicts.
///
/// Freezing happens here, at record time, against the chart the verdict was
/// produced on. A definition whose closure cannot be fully established is
/// rejected with a reason rather than stored under a partial key (claim C4):
/// a partial key is precisely how a stale verdict outlives the change it
/// depended on.
///
/// # Errors
/// Returns a message when the chart store or the cache log cannot be written.
pub fn cache_record(
    params: &CacheRecordParams,
    root: &std::path::Path,
) -> Result<CacheRecordResult, String> {
    let epoch = epoch(root)?;
    if let Some(want) = &params.snapshot {
        if want.0 != epoch.snapshot_id().as_str() {
            return Err(format!("snapshot {} is not the live epoch", want.0));
        }
    }
    let mut cache = grid_cache::Cache::open(cache_path(root)).map_err(|e| e.to_string())?;
    let chart = grid_cache::Chart::new(epoch.segments());
    let kind = grid_cache::Kind::from(params.kind);
    let recorded_at = grid_cache::now_rfc3339();

    // A verdict tied to a retained wake also lands in the datum-outcome
    // log: that pairing — this test, this datum version, this verdict — is
    // what bisect searches. Best effort, like every history writer.
    let outcome_test = params.wake_id.as_deref().and_then(|id| {
        grid_wake::Store::open(wake_dir(root))
            .ok()
            .and_then(|s| s.get(id).ok().flatten())
            .and_then(|stored| stored.wake.command)
    });
    let mut recorded = 0u64;
    let mut rejected = Vec::new();
    for entry in &params.results {
        let Some(id) = grid_chart::DatumId::parse(&entry.datum.0) else {
            rejected.push(CacheRecordResultRejectedItem {
                datum: entry.datum.clone(),
                reason: "not a b3 datum handle".to_string(),
            });
            continue;
        };
        match chart.key(id, kind, &params.env_hash) {
            Ok(key) => {
                cache
                    .put(
                        key,
                        grid_cache::Entry {
                            verdict: match entry.verdict {
                                CacheResultEntryVerdict::Pass => grid_cache::Verdict::Pass,
                                CacheResultEntryVerdict::Fail => grid_cache::Verdict::Fail,
                            },
                            recorded_at: recorded_at.clone(),
                            wake_id: params.wake_id.clone(),
                            // Stored so the key can be recomputed later:
                            // `grid drift --tests` asks whether this verdict
                            // still applies, and a key is a hash.
                            env_hash: Some(params.env_hash.clone()),
                        },
                    )
                    .map_err(|e| e.to_string())?;
                recorded += 1;
                if let Some(test) = &outcome_test {
                    let row = crate::legs::Outcome {
                        datum: entry.datum.0.clone(),
                        test: test.clone(),
                        fails: entry.verdict == CacheResultEntryVerdict::Fail,
                        at: Some(recorded_at.clone()),
                    };
                    let _unwritable = crate::legs::append_outcomes(root, &[row]);
                }
            }
            Err(reject) => rejected.push(CacheRecordResultRejectedItem {
                datum: entry.datum.clone(),
                reason: reject.to_string(),
            }),
        }
    }
    Ok(CacheRecordResult { recorded, rejected })
}

/// Open the chart against the epoch a request names.
///
/// `None` means the live epoch; a rev is surveyed on demand (which makes it
/// live); a snapshot id must *be* the live one — the engine holds exactly
/// one live epoch, the same rule `grid.cache.check` states.
fn open_for(
    snapshot: Option<&SnapshotOrRev>,
    root: &std::path::Path,
) -> Result<ChartDelta, chart_provider::Error> {
    if let Some(target @ SnapshotOrRev::GitRev { .. }) = snapshot {
        Chart::new(root).ensure(target)?;
    }
    let chart = ChartDelta::open(root)?;
    if let Some(SnapshotOrRev::Snapshot { snapshot }) = snapshot {
        if chart.current().as_ref() != Some(snapshot) {
            return Err(chart_provider::Error::NotLive(snapshot.0.clone()));
        }
    }
    Ok(chart)
}

/// `grid.scope` — the context pack around a set of callsigns: the matching
/// definitions, the chart edges touching them, and the standing invariants
/// whose patterns reach them. Signatures and bodies are deliberately absent
/// (wire contract, `ScopePack`): the segment does not carry them, and the
/// agent pulls bodies itself, at body cost, when it needs them.
///
/// Two fields answer empty at this wave and that is the honest answer, not a
/// gap: `anchors` because nothing stores anchors yet, and `churn` because
/// the leg log that would count edits does not exist until a later wave.
///
/// # Errors
/// [`chart_provider::Error::NoSnapshot`] when the store has never been
/// surveyed — a pack against nothing is not a pack. Other variants as in
/// [`open_for`] and the invariant load.
pub fn scope(
    params: &ScopeParams,
    root: &std::path::Path,
) -> Result<ScopeResult, chart_provider::Error> {
    let chart = open_for(params.snapshot.as_ref(), root)?;
    let Some(snapshot) = chart.current() else {
        return Err(chart_provider::Error::NoSnapshot);
    };
    let all = live_defs(&chart);
    let matched: Vec<String> = all
        .iter()
        .filter(|(_, callsign, _, _)| {
            params
                .callsigns
                .iter()
                .any(|p| grid_xte::pattern::matches(&p.0, callsign))
        })
        .map(|(datum, _, _, _)| datum.clone())
        .collect();
    let in_scope = expand(&chart, matched, params.depth.unwrap_or(0));

    let defs: Vec<grid_wire::DefRef> = all
        .iter()
        .filter(|(datum, _, _, _)| in_scope.contains(datum))
        .map(
            |(datum, callsign, lang, materialization)| grid_wire::DefRef {
                datum: grid_wire::Datum(datum.clone()),
                callsign: Some(grid_wire::Callsign(callsign.clone())),
                lang: *lang,
                signature: None,
                materialization: materialization.clone(),
            },
        )
        .collect();
    let (in_edges, out_edges) = incident_edges(&chart, &in_scope);

    // In-scope invariants: the standing rules whose patterns name a scoped
    // definition. Grounded in defs, not pattern algebra: a rule that matches
    // nothing in scope is not in scope, whatever the patterns claim.
    let callsigns: Vec<&str> = defs
        .iter()
        .filter_map(|d| d.callsign.as_ref().map(|c| c.0.as_str()))
        .collect();
    let invariants: Vec<grid_wire::Invariant> = standing::load(root)
        .map_err(chart_provider::Error::Config)?
        .into_iter()
        .filter(|inv| {
            let rule = match &inv.rule {
                grid_wire::InvariantRule::ForbidEdge { forbid_edge } => forbid_edge,
                grid_wire::InvariantRule::RequireEdge { require_edge } => require_edge,
            };
            [&rule.src, &rule.dst].into_iter().any(|pattern| {
                callsigns
                    .iter()
                    .any(|c| grid_xte::pattern::matches(&pattern.0, c))
            })
        })
        .collect();

    Ok(ScopeResult {
        snapshot,
        defs,
        in_edges,
        out_edges,
        invariants,
        anchors: Vec::new(),
        churn: Vec::new(),
    })
}

type LiveDef = (
    String,
    String,
    Option<grid_wire::Lang>,
    Option<grid_wire::DefRefMaterialization>,
);

/// Every definition in the live epoch: (datum, callsign, lang, location),
/// in segment order — which is content order, so everything derived from it
/// is deterministic by construction (C2).
fn live_defs(chart: &ChartDelta) -> Vec<LiveDef> {
    let mut all = Vec::new();
    for segment in chart.head().segments() {
        for ord in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(ord);
            if let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) {
                let materialization =
                    segment
                        .location(ord)
                        .map(|loc| grid_wire::DefRefMaterialization {
                            file: loc.path.to_owned(),
                            line: u64::from(loc.line),
                        });
                all.push((
                    datum.to_string(),
                    callsign.to_owned(),
                    segment.lang(ord),
                    materialization,
                ));
            }
        }
    }
    all
}

/// The matched set plus `depth` BFS layers over edges in both directions.
/// Depth zero is the definitions the patterns name, no neighbours — the
/// pack's edges still say what touches them.
fn expand(chart: &ChartDelta, matched: Vec<String>, depth: u64) -> Vec<String> {
    let mut in_scope = matched;
    let mut frontier = in_scope.clone();
    for _ in 0..depth {
        let mut next = Vec::new();
        for segment in chart.head().segments() {
            for ord in 0..segment.def_count() {
                let ord = grid_chart::Ordinal(ord);
                let Some(datum) = segment.datum(ord) else {
                    continue;
                };
                if !frontier.contains(&datum.to_string()) {
                    continue;
                }
                for neighbour in segment
                    .out_edges(ord)
                    .chain(segment.in_edges(ord))
                    .filter_map(|n| segment.datum(n.other))
                {
                    let neighbour = neighbour.to_string();
                    if !in_scope.contains(&neighbour) && !next.contains(&neighbour) {
                        next.push(neighbour);
                    }
                }
            }
        }
        in_scope.extend(next.iter().cloned());
        frontier = next;
    }
    in_scope
}

/// The chart edges touching a datum set, from the set's perspective: an
/// edge whose source is in scope is an out-edge, whose destination is in
/// scope is an in-edge — an edge wholly inside is both, and that is not a
/// contradiction. Both lists sort by (src, relation, dst): same chart, same
/// pack, every time.
fn incident_edges(
    chart: &ChartDelta,
    in_scope: &[String],
) -> (Vec<grid_wire::Edge>, Vec<grid_wire::Edge>) {
    let mut in_edges = Vec::new();
    let mut out_edges = Vec::new();
    for segment in chart.head().segments() {
        for ord in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(ord);
            let Some(src) = segment.datum(ord) else {
                continue;
            };
            let src = src.to_string();
            for n in segment.out_edges(ord) {
                let Some(dst) = segment.datum(n.other) else {
                    continue;
                };
                let dst = dst.to_string();
                let edge = grid_wire::Edge {
                    src: grid_wire::Datum(src.clone()),
                    relation: n.relation,
                    dst: grid_wire::Datum(dst.clone()),
                    zoc: n.zoc,
                    via: None,
                };
                if in_scope.contains(&src) {
                    out_edges.push(edge.clone());
                }
                if in_scope.contains(&dst) {
                    in_edges.push(edge);
                }
            }
        }
    }
    let by_endpoint = |a: &grid_wire::Edge, b: &grid_wire::Edge| {
        (&a.src.0, a.relation as u8, &a.dst.0).cmp(&(&b.src.0, b.relation as u8, &b.dst.0))
    };
    out_edges.sort_by(by_endpoint);
    in_edges.sort_by(by_endpoint);
    (in_edges, out_edges)
}

/// `grid.todo` — the gate verdict rendered as a worklist: what the track
/// promised, split into made-good and pending. This is a *view* over the
/// same judgment `grid.gate.check` computes (CLI plan §5) — the delta, the
/// verdict, and this list can never disagree, because they are one engine.
///
/// `head` defaults to the live snapshot: the worklist question is "what is
/// left *now*", and now is what was last surveyed.
///
/// # Errors
/// [`chart_provider::Error::Track`] when the track id names nothing,
/// [`chart_provider::Error::NoSnapshot`] when nothing has been surveyed, and
/// the usual provider errors for a delta that cannot be computed.
pub fn todo(
    params: &TodoParams,
    root: &std::path::Path,
) -> Result<TodoResult, chart_provider::Error> {
    let track = crate::track::load(&params.track.0).map_err(chart_provider::Error::Track)?;
    let track = standing::fold(
        &track,
        standing::load(root).map_err(chart_provider::Error::Config)?,
    );
    let mut chart = Chart::new(root);
    chart.ensure(&track.base)?;
    let head = match &params.head {
        Some(head) => head.clone(),
        None => SnapshotOrRev::Snapshot {
            snapshot: ChartDelta::open(root)?
                .current()
                .ok_or(chart_provider::Error::NoSnapshot)?,
        },
    };
    if let SnapshotOrRev::GitRev { .. } = head {
        chart.ensure(&head)?;
    }
    let provider = ChartDelta::open(root)?;
    let made_good = provider.made_good(&track.base, &head)?;
    let verdict = grid_xte::verdict(&track, &made_good, &provider);
    let (defs_done, defs_pending) = grid_xte::def_progress(&track, &made_good);
    // A test is done when its comparison came back ok; anything else —
    // failed, skipped, never run — is still work.
    let mut tests_done = Vec::new();
    let mut tests_pending = Vec::new();
    for comparison in &verdict.tests {
        if comparison.ok {
            tests_done.push(comparison.test.clone());
        } else {
            tests_pending.push(comparison.test.clone());
        }
    }

    let done = defs_done.len() + verdict.matched.len() + tests_done.len();
    let total =
        track.delta.defs.len() + track.delta.edges.len() + track.tests.as_ref().map_or(0, Vec::len);
    Ok(TodoResult {
        track_id: grid_wire::TrackId(track.track_id.clone()),
        defs_done,
        defs_pending,
        edges_done: verdict.matched,
        edges_pending: verdict.missing,
        tests_done,
        tests_pending,
        fraction: grid_wire::TodoListFraction {
            num: done as u64,
            den: total as u64,
        },
    })
}

/// `grid uses` (CLI) — who reaches a name: the resolved definitions plus the
/// live chart's *proven* inbound edges. Only proven, at this wave and on
/// principle: "who can reach X" answered with a maybe-edge is how an agent
/// talks itself into a safe-looking blast radius. The verb grows a walker
/// later; the filter does not move.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn uses(
    params: &ResolveParams,
    root: &std::path::Path,
) -> Result<(ResolveResult, Vec<grid_wire::Edge>), chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    let defs = chart.resolve(&params.locator);
    let mut edges = Vec::new();
    for def in &defs {
        let Some(want) = grid_chart::DatumId::parse(&def.datum.0) else {
            continue;
        };
        for segment in chart.head().segments() {
            let Some(ord) = segment.ordinal(&want) else {
                continue;
            };
            for n in segment.in_edges(ord) {
                if n.zoc != grid_wire::Zoc::Proven {
                    continue;
                }
                let Some(src) = segment.datum(n.other) else {
                    continue;
                };
                edges.push(grid_wire::Edge {
                    src: grid_wire::Datum(src.to_string()),
                    relation: n.relation,
                    dst: def.datum.clone(),
                    zoc: n.zoc,
                    via: None,
                });
            }
        }
    }
    edges.sort_by(|a, b| (&a.src.0, a.relation as u8).cmp(&(&b.src.0, b.relation as u8)));
    Ok((defs, edges))
}

/// `grid.wake.ingest` — resolve a test-run wake against the live chart,
/// retain it, and report what it meant.
///
/// Resolution is the `grid.resolve` rule (unique dotted-suffix), so the name
/// a user can resolve is the name a wake can. What cannot resolve is
/// returned, never dropped (C15): an observed edge always names two datums
/// that could be pointed at.
///
/// # Errors
/// Returns a message when the chart store or the wake log cannot be read or
/// written. Ingesting into an unsurveyed repo is an error even for an
/// all-datum wake: symbol resolution must never silently see an empty chart.
pub fn wake_ingest(
    params: &WakeIngestParams,
    root: &std::path::Path,
) -> Result<WakeIngestResult, String> {
    let epoch = epoch(root)?;
    let ingested = grid_wake::ingest(epoch.segments(), &params.wake);
    let store = grid_wake::Store::open(wake_dir(root)).map_err(|e| e.to_string())?;
    store
        .put(&params.wake, &ingested)
        .map_err(|e| e.to_string())?;
    #[allow(clippy::cast_possible_truncation)]
    let observed_edges = ingested.edges.len() as u64;
    Ok(WakeIngestResult {
        observed_edges,
        resolved: ingested.resolved,
        unresolved_symbols: ingested.unresolved_symbols,
    })
}
/// Why `grid.wake.diff` could not answer.
#[derive(Debug)]
pub enum WakeDiffError {
    /// A named wake was never ingested under that id.
    UnknownWake(String),
    /// The retention log could not be read or is corrupt — suspect, the way a
    /// corrupt segment is, not silently empty.
    Store(String),
}
impl std::fmt::Display for WakeDiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownWake(id) => write!(f, "unknown wake id {id}: ingest it first"),
            Self::Store(msg) => f.write_str(msg),
        }
    }
}
/// `grid.wake.diff` — two retained traces compared at call granularity.
///
/// # Errors
/// [`WakeDiffError::UnknownWake`] when either id names nothing retained;
/// [`WakeDiffError::Store`] when the log itself fails.
pub fn wake_diff(
    params: &WakeDiffParams,
    root: &std::path::Path,
) -> Result<grid_wire::WakeDiff, WakeDiffError> {
    let store =
        grid_wake::Store::open(wake_dir(root)).map_err(|e| WakeDiffError::Store(e.to_string()))?;
    let load = |id: &grid_wire::WakeId| {
        store
            .get(&id.0)
            .map_err(|e| WakeDiffError::Store(e.to_string()))?
            .ok_or_else(|| WakeDiffError::UnknownWake(id.0.clone()))
    };
    let (green, red) = (load(&params.green)?, load(&params.red)?);
    Ok(grid_wake::diff(&green, &red))
}
pub(crate) fn wake_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".grid/wakes")
}
