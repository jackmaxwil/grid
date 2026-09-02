//! Method layer shared by CLI and daemon.
use grid_wire::methods::{
    CacheCheckParams, CacheRecordParams, CacheRecordResult, CacheRecordResultNondeterministicItem,
    CacheRecordResultNondeterministicItemPrevious, CacheRecordResultRejectedItem, DeltaParams,
    DeltaResult, GateCheckParams, GateCheckResult, PlanLintParams, PlanLintResult, QueryWalkParams,
    QueryWalkResult, ResolveParams, ResolveResult, ScopeParams, ScopeResult, SnapshotCurrentResult,
    SurveyRunParams, SurveyRunResult, TodoParams, TodoResult, TraceDiffParams, TraceIngestParams,
    TraceIngestResult, VersionResult,
};
use grid_wire::{
    CacheCheckResult, CacheHit, CacheHitVerdict, CacheResultEntryVerdict, GitRev, Snapshot,
    SnapshotId, SnapshotOrRev, SnapshotStats, WalkResult, WalkResultStatus, WalkStats,
};

use grid_gate::DeltaProvider as _;

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

/// `grid.plan.lint` — pure validation, no survey, no snapshot. Pure of the
/// chart, that is: the repo's standing invariants are part of the rules a
/// promise is judged against, and reading them touches no snapshot.
///
/// # Errors
/// Returns a message when the standing invariant file exists but is invalid;
/// a broken rule file must not be mistaken for an absence of rules.
pub fn plan_lint(
    params: &PlanLintParams,
    root: &std::path::Path,
) -> Result<PlanLintResult, String> {
    let plan = standing::fold(&params.plan, standing::load(root)?);
    Ok(grid_gate::lint::lint(&plan))
}

/// `grid.gate.check` — the plan against the delta at head.
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
    gate_check_on(params, root, None)
}

/// The same gate over a chart the caller already opened, when it has one.
///
/// `grid check`'s adherence section holds the store open for every other
/// section; opening a second copy of it for this one was a store map and a
/// name index per check and nothing else.
///
/// # Errors
/// Propagates the provider's error (unknown snapshot, failed survey).
pub fn gate_check_on(
    params: &GateCheckParams,
    root: &std::path::Path,
    opened: Option<&ChartDelta>,
) -> Result<GateCheckResult, chart_provider::Error> {
    let standing = standing::load(root).map_err(chart_provider::Error::Config)?;
    let plan = standing::fold(&params.plan, standing);
    let mut chart = Chart::new(root);
    chart.ensure(&plan.base)?;
    let head = chart.head_of(&params.head)?;
    match opened {
        Some(delta) => grid_gate::check(&plan, &head, delta),
        None => grid_gate::check(&plan, &head, &ChartDelta::open(root)?),
    }
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
    /// this" is wrong by at most this much, and can now say so — and, since
    /// the three populations mean opposite things for an analysis, by how
    /// much of each.
    pub declined: crate::drift::Declined,
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

    /// The snapshot of the tree at `target`, surveyed at the rev if the store
    /// holds none — retained, never made live.
    ///
    /// A snapshot id must already exist — it names content, so it cannot be
    /// conjured — but a rev can always be surveyed, and doing it here is what
    /// lets `grid gate` be the only command CI runs. The answer is always a
    /// snapshot id: a dirty worktree survey is labelled with the rev it was
    /// checked out from, so a rev looked up *after* this call must not be
    /// able to find the worktree instead. And the survey is retained rather
    /// than published live, so a verb that ensured its base on the way in
    /// leaves every later read verb answering about the worktree, not about
    /// the base.
    ///
    /// # Errors
    /// [`chart_provider::Error`] if the rev cannot be surveyed or published.
    pub fn ensure(
        &mut self,
        target: &SnapshotOrRev,
    ) -> Result<SnapshotOrRev, chart_provider::Error> {
        let SnapshotOrRev::GitRev { git_rev } = target else {
            return Ok(target.clone());
        };
        // Already surveyed at this rev: surveying again would produce the same
        // bytes and the same id (D5), so it is pure cost.
        match ChartDelta::open(&self.root)?.snapshot_of(target) {
            Ok(snapshot) => return Ok(SnapshotOrRev::Snapshot { snapshot }),
            Err(chart_provider::Error::NoSnapshotForRev(_)) => {}
            Err(e) => return Err(e),
        }
        let (snapshot, _) = self.survey(
            &SurveyRunParams {
                git_rev: Some(git_rev.clone()),
                worktree: None,
            },
            false,
        )?;
        Ok(SnapshotOrRev::Snapshot {
            snapshot: snapshot.snapshot_id,
        })
    }

    /// The snapshot a verb's *head* names, by id.
    ///
    /// A literal `HEAD` is the working tree — the everyday question is "what
    /// did my uncommitted edits do" — surveyed now and published live. Any
    /// other rev is the committed tree at that rev, surveyed at the rev if
    /// the store lacks it and retained. Named by id either way, so no later
    /// rev lookup can confuse the two.
    ///
    /// # Errors
    /// [`chart_provider::Error`] if the tree cannot be surveyed or published.
    pub fn head_of(
        &mut self,
        target: &SnapshotOrRev,
    ) -> Result<SnapshotOrRev, chart_provider::Error> {
        match target {
            SnapshotOrRev::GitRev { git_rev } if git_rev.0 == "HEAD" => {
                let (snapshot, _) = self.survey_run(&SurveyRunParams {
                    git_rev: None,
                    worktree: None,
                })?;
                Ok(SnapshotOrRev::Snapshot {
                    snapshot: snapshot.snapshot_id,
                })
            }
            other => self.ensure(other),
        }
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
        self.survey(params, true)
    }

    /// The survey itself. `live` publishes the snapshot as the store's live
    /// epoch and logs the edits since the previous one; otherwise it is
    /// retained only, which is what an on-demand base survey wants.
    fn survey(
        &mut self,
        params: &SurveyRunParams,
        live: bool,
    ) -> Result<(SurveyRunResult, Ran), chart_provider::Error> {
        // Surveying "at a rev" means the tree at that rev, read out of the
        // object store; with no rev it is the working tree, labelled with the
        // rev it was checked out from plus the hash of its diff against that
        // rev when it has one — the label says where the tree came from, the
        // hash says it is not that tree. Resolving both is git subprocesses
        // that read nothing the survey writes, so they run alongside the
        // survey instead of after it.
        let named = params.git_rev.as_ref().map_or("HEAD", |rev| rev.0.as_str());
        let root = &self.root;
        let (survey, rev, dirty) = std::thread::scope(|scope| {
            let resolver = scope.spawn(move || {
                let rev = chart_provider::resolve_rev(root, named);
                let dirty = params
                    .git_rev
                    .is_none()
                    .then(|| worktree_diff_hash(root))
                    .flatten();
                (rev, dirty)
            });
            let survey = match params.git_rev.as_ref() {
                Some(rev) => grid_survey::run_at(root, &rev.0, &mut self.cache),
                None => grid_survey::run(root, &mut self.cache),
            };
            let (rev, dirty) = resolver.join().unwrap_or_else(|_| {
                (
                    chart_provider::resolve_rev(root, named),
                    params
                        .git_rev
                        .is_none()
                        .then(|| worktree_diff_hash(root))
                        .flatten(),
                )
            });
            (survey, rev, dirty)
        });
        let survey = survey?;
        // The daemon is the single writer (D6); `publish` takes `&mut self`,
        // which is that rule in the type system.
        let mut store = grid_chart_io::Store::open(chart_provider::store_dir(&self.root))
            .map_err(chart_provider::Error::Store)?;
        let superseded = store.superseded();
        let prev = store.load();
        let hash = dirty.map(|h| *h.as_bytes());
        let id = if live {
            store.publish(&rev, hash, &[survey.image])
        } else {
            store.publish_retained(&rev, hash, &[survey.image])
        }
        .map_err(chart_provider::Error::Store)?;
        // Every modification between the previous live set and this one is a
        // edit: edits made outside `grid edit` are still captured, one edit
        // per survey interval. Best effort — an unwritable log costs
        // history, never the snapshot that already published.
        if live && !prev.segments().is_empty() && prev.snapshot_id() != &id {
            let next = store.load();
            let delta = grid_chart::diff(
                grid_wire::SnapshotId(prev.snapshot_id().as_str().to_owned()),
                prev.segments(),
                grid_wire::SnapshotId(id.as_str().to_owned()),
                next.segments(),
            );
            let edits: Vec<grid_wire::Edit> = delta
                .defs
                .iter()
                .filter(|d| {
                    matches!(
                        d.action,
                        grid_wire::DefChangeAction::Modified | grid_wire::DefChangeAction::Moved
                    )
                })
                .filter_map(|d| {
                    Some(grid_wire::Edit {
                        from: d.before.clone()?,
                        to: d.after.clone()?,
                        git_rev: Some(GitRev(rev.clone())),
                        at: None,
                    })
                })
                .collect();
            let _unwritable = crate::history::append(&self.root, &edits);
        }
        // The three add up to the survey's declined total by construction,
        // which is the only place that identity can be guaranteed rather
        // than asserted.
        let declined = crate::drift::Declined {
            external: survey.declined_external,
            method: survey.declined_method,
            internal: survey.declined_internal,
        };
        Ok((
            Snapshot {
                snapshot_id: SnapshotId(id.as_str().to_owned()),
                git_rev: GitRev(rev),
                created_at: String::new(),
                worktree_diff_hash: dirty.map(|h| format!("b3:{}", h.to_hex())),
                stats: SnapshotStats {
                    defs: survey.defs,
                    edges: survey.edges,
                    langs: Some(survey.langs),
                },
                declined: Some(grid_wire::SnapshotDeclined {
                    external: declined.external,
                    method: declined.method,
                    internal: declined.internal,
                    refused: Some(survey.declined_method_refused),
                }),
            },
            Ran {
                reread: survey.reread,
                superseded,
                declined,
            },
        ))
    }
}

/// The hash of the working tree's diff against `HEAD`, or `None` for a tree
/// that is the committed one (or has no `HEAD` to differ from).
///
/// `git diff HEAD` sees exactly what the survey sees: tracked files, index
/// and worktree alike, since the survey enumerates by `git ls-files` —
/// minus `.grid/`, grid's own state, which a repo that tracks it would
/// otherwise dirty with every publish. The value only ever labels a
/// snapshot; the snapshot id is still the content.
fn worktree_diff_hash(root: &std::path::Path) -> Option<blake3::Hash> {
    let out = std::process::Command::new("git")
        .args([
            "diff",
            "HEAD",
            "--no-color",
            "--no-ext-diff",
            "--binary",
            "--",
            ".",
            ":(exclude).grid",
        ])
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    (!out.stdout.is_empty()).then(|| blake3::hash(&out.stdout))
}

/// `grid.drift` — dependents of changed interfaces.
///
/// Both endpoints are surveyed on demand, exactly as `grid gate` surveys
/// them, so CI needs no second command. Head is special in one way: a
/// literal `HEAD` surveys the *working tree*, because drift's everyday
/// question is "what did my uncommitted edits leave stale", and because the
/// survey it runs is also where the declined-reference count comes from. Any
/// other rev is surveyed at that rev; a snapshot id is taken as published,
/// and its declined count is then unknown rather than zero. Both endpoints
/// are then named by id: a worktree survey is labelled with its rev, and a
/// rev lookup after it must never find the worktree instead.
///
/// # Errors
/// [`crate::drift::Error::Chart`] when an endpoint does not resolve;
/// [`crate::drift::Error::BudgetRejected`] when an inbound walk ran past its
/// fuel — refused whole, never truncated.
pub fn drift(
    params: &grid_wire::methods::DriftParams,
    root: &std::path::Path,
) -> Result<grid_wire::methods::DriftResult, crate::drift::Error> {
    let ends = drift_endpoints(params, root)?;
    let chart = ChartDelta::open(root).map_err(crate::drift::Error::Chart)?;
    let at = crate::drift::Resolved::open(&chart, &ends.base, &ends.head)?;
    drift_on(root, &chart, &at, &ends, params)
}

/// One drift run's endpoints, surveyed and named — the half of `drift` that
/// must happen *before* the store is opened, because surveying head
/// publishes the epoch a store opened earlier would not be holding.
pub struct Endpoints {
    /// The snapshot dependents were depending on.
    pub base: SnapshotOrRev,
    /// The snapshot they may now be stale against.
    pub head: SnapshotOrRev,
    /// What head's survey declined, or `None` when nobody surveyed head in
    /// this run — unknown, never zero.
    pub declined: Option<crate::drift::Declined>,
}

/// Survey and name both endpoints of a drift question.
///
/// # Errors
/// [`crate::drift::Error::Chart`] when an endpoint cannot be surveyed or
/// resolved.
pub fn drift_endpoints(
    params: &grid_wire::methods::DriftParams,
    root: &std::path::Path,
) -> Result<Endpoints, crate::drift::Error> {
    let mut chart = Chart::new(root);
    let base = chart.ensure(&params.base)?;
    let (head, declined) = match &params.head {
        SnapshotOrRev::GitRev { git_rev } => {
            let at = (git_rev.0 != "HEAD").then(|| git_rev.clone());
            let (snapshot, ran) = chart.survey_run(&SurveyRunParams {
                git_rev: at,
                worktree: None,
            })?;
            (
                SnapshotOrRev::Snapshot {
                    snapshot: snapshot.snapshot_id,
                },
                Some(ran.declined),
            )
        }
        // Nobody re-surveyed head in this run, so nobody can say what its
        // survey declined. Unknown, not zero.
        snapshot @ SnapshotOrRev::Snapshot { .. } => (snapshot.clone(), None),
    };
    Ok(Endpoints {
        base,
        head,
        declined,
    })
}

/// The drift report over a chart and a delta the caller already holds —
/// `grid check`'s path, where every later section reads the same two.
///
/// # Errors
/// [`crate::drift::Error::Covenants`] when a declaration file will not parse;
/// [`crate::drift::Error::BudgetRejected`] when an inbound walk ran past its
/// fuel; [`crate::drift::Error::Stale`] when the trace store or the verdict
/// log could not be read.
pub fn drift_on(
    root: &std::path::Path,
    chart: &ChartDelta,
    at: &crate::drift::Resolved,
    ends: &Endpoints,
    params: &grid_wire::methods::DriftParams,
) -> Result<grid_wire::methods::DriftResult, crate::drift::Error> {
    let (base, head, declined) = (&ends.base, &ends.head, ends.declined);
    // A declaration file that exists but does not parse is a loud error,
    // never an absence of declarations: a covenant nobody can read is not a
    // covenant nobody made.
    let covenants = crate::covenant::load(root).map_err(crate::drift::Error::Covenants)?;
    // Same posture for the invariant file: a rule nobody can read is not an
    // absence of rules, so a broken file is an error rather than an empty set.
    let invariants = if params.compat == Some(true) {
        Some(crate::standing::load(root).map_err(crate::drift::Error::Covenants)?)
    } else {
        None
    };
    let mut report = crate::drift::compute_on(
        root,
        chart,
        at,
        &crate::drift::Query {
            floor: params
                .floor
                .unwrap_or(crate::drift::Query::new(base, head).floor),
            plan: params.plan.as_ref(),
            covenants: &covenants,
            declined,
            compat: invariants.as_deref(),
            ..crate::drift::Query::new(base, head)
        },
    )?;
    if params.tests == Some(true) {
        // Head, because the question is whether a recorded verdict still
        // applies *now* — the closure key is identity math over the current
        // chart, not over the one the verdict was taken against.
        let (epoch, base_epoch) = (&at.head, &at.base);
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
        // Green-but-stale, over step 6's membership — read through the chart
        // this call already holds. Absent membership is an absence of derived
        // data, not a claim that nothing is weak: the report simply does not
        // answer a question it could not ask.
        if let Some(membership) = chart
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

/// `grid.interfere` — semantic overlap with another worktree of this repo.
///
/// # Errors
/// [`crate::interfere::Error`] when either store cannot answer for the base,
/// or when a drift walk refused.
pub fn interfere(
    params: &grid_wire::methods::InterfereParams,
    root: &std::path::Path,
) -> Result<grid_wire::methods::InterfereResult, crate::interfere::Error> {
    crate::interfere::compute(
        root,
        std::path::Path::new(&params.other_root),
        params.base.as_ref().map(|rev| rev.0.as_str()),
    )
}

/// How many charted callsigns a pattern matches.
///
/// Zero is the answer `grid covenant` refuses on: a declaration about code that
/// does not exist is a rule that will never fire.
///
/// # Errors
/// [`chart_provider::Error`] if the store cannot be opened.
pub fn matching(root: &std::path::Path, pattern: &str) -> Result<usize, chart_provider::Error> {
    let chart = ChartDelta::open(root)?;
    Ok(live_defs(&chart)
        .into_iter()
        .filter(|(_, callsign, _, _, _)| grid_gate::pattern::matches(pattern, callsign))
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
        .map(|(_, callsign, _, _, _)| {
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
                cfg: None,
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
                .any(|n| n.grade == grid_wire::Grade::Proven)
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
                cfg: None,
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

/// `grid orphans --traced` — the same rows, each with how many retained
/// traces hit the definition as caller or callee. `trace 0` beside "no
/// proven caller" is the workspace-level unused-`pub` question rustc cannot
/// ask (it checks per crate); still heuristic — `pub` in a published crate
/// stays a maybe.
///
/// # Errors
/// [`chart_provider::Error`] if the store or the trace store cannot be
/// opened — an unreadable trace store is not zero traces.
pub fn orphans_traced(
    root: &std::path::Path,
    lang: Option<grid_wire::Lang>,
    under: Option<&str>,
) -> Result<Vec<grid_wire::Orphan>, chart_provider::Error> {
    let rows = orphans(root, lang, under)?;
    let traces = trace_dir(root)
        .and_then(grid_trace::Store::open)
        .and_then(|store| store.retained())
        .map_err(|e| chart_provider::Error::Config(e.to_string()))?;
    // ponytail: one pass over every retained trace's hops per call; index
    // the store by datum when a repo holds enough traces for this to show.
    let mut hits: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for stored in &traces {
        let seen: std::collections::BTreeSet<grid_chart::DatumId> = stored
            .hops
            .iter()
            .flat_map(|(caller, callee)| [*caller, *callee])
            .collect();
        for datum in seen {
            let count = hits.entry(datum.to_string()).or_default();
            *count = count.saturating_add(1);
        }
    }
    Ok(rows
        .into_iter()
        .map(|def| grid_wire::Orphan {
            traces: Some(hits.get(&def.datum.0).copied().unwrap_or(0)),
            def,
        })
        .collect())
}

/// One hotspot row: a definition scored by complexity times churn.
#[derive(Debug, serde::Serialize)]
pub struct Hotspot {
    pub callsign: String,
    pub datum: String,
    pub complexity: u16,
    pub edits: u64,
    pub score: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<grid_wire::DefRefMaterialization>,
}

/// Where the churn multiplier came from — named so the caller can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChurnSource {
    /// The grid patch log: identity-level, rename-proof.
    Edits,
    /// `git log --numstat`, file-level: every def in a file inherits its
    /// file's commit count. Coarser, but honest on a fresh checkout.
    GitFiles,
    /// Neither exists; the ranking is complexity alone.
    None,
}

/// `grid.hotspots` — complexity × churn, descending. Churn is the number of
/// edits in the datum's identity component of the patch log, so it survives
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
    // Union edits into identity components by plain map-merging: the log is
    // small (one row per recorded modification), so O(edits · chain) is fine.
    // ponytail: replace with union-find when a real log makes this slow.
    let edits = crate::history::read(root);
    let mut component: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut next = 0usize;
    for edit in &edits {
        let ids = (
            component.get(&edit.from.0).copied(),
            component.get(&edit.to.0).copied(),
        );
        let id = match ids {
            (Some(a), _) => a,
            (None, Some(b)) => b,
            (None, None) => {
                next += 1;
                next
            }
        };
        component.insert(edit.from.0.clone(), id);
        component.insert(edit.to.0.clone(), id);
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
    for edit in &edits {
        if let Some(id) = component.get(&edit.to.0) {
            *per_component.entry(*id).or_insert(0) += 1;
        }
    }
    let file_churn = if edits.is_empty() {
        git_file_churn(root)
    } else {
        std::collections::HashMap::new()
    };
    let source = if !edits.is_empty() {
        ChurnSource::Edits
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
                edits: churn,
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
        worktree_diff_hash: chart
            .head()
            .manifest()
            .worktree_diff_hash
            .map(|h| format!("b3:{}", blake3::Hash::from_bytes(h).to_hex())),
        stats: chart.stats(),
        // Published, not re-surveyed here: nobody in this call can say what
        // its survey declined, and unknown is not zero.
        declined: None,
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
    let base = chart.ensure(&params.base)?;
    let head = chart.head_of(&params.head)?;
    ChartDelta::open(root)?.diff(&base, &head)
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

/// `grid.query.walk` — the walk engine over the live chart.
///
/// The chart is a set of segments and [`grid_walk::query`] walks one
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
        return Err(chart.no_snapshot());
    };
    let sweep = params.query.to.is_none();
    let mut merged = WalkResult {
        status: WalkResultStatus::Complete,
        snapshot: snapshot.clone(),
        paths: (!sweep).then(Vec::new),
        reached: sweep.then(Vec::new),
        frontier: None,
        stats: WalkStats {
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
        let Ok(part) = grid_walk::query(segment, &params.query, snapshot.clone()) else {
            continue;
        };
        if part.status == WalkResultStatus::Aground {
            merged.status = WalkResultStatus::Aground;
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
                trace_id: entry.trace_id,
                // A cache check is addressed by datum; nobody named a test.
                test: None,
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
    let mut nondeterministic = Vec::new();

    // A verdict tied to a retained trace also lands in the datum-outcome
    // log: that pairing — this test, this datum version, this verdict — is
    // what bisect searches. Best effort, like every history writer.
    let outcome_test = params.trace_id.as_deref().and_then(|id| {
        trace_dir(root)
            .and_then(grid_trace::Store::open)
            .ok()
            .and_then(|s| s.get(id).ok().flatten())
            .and_then(|stored| stored.trace.command)
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
                let verdict = match entry.verdict {
                    CacheResultEntryVerdict::Pass => grid_cache::Verdict::Pass,
                    CacheResultEntryVerdict::Fail => grid_cache::Verdict::Fail,
                };
                // Same key, opposite verdict: the closure proves the charted
                // code did not change and the environment is the same, so
                // whatever differed is not the charted code. Reported from
                // the last entry — no history table — and stored regardless.
                if let Some(previous) = cache.get(key).filter(|p| p.verdict != verdict) {
                    nondeterministic.push(CacheRecordResultNondeterministicItem {
                        datum: entry.datum.clone(),
                        previous: match previous.verdict {
                            grid_cache::Verdict::Pass => {
                                CacheRecordResultNondeterministicItemPrevious::Pass
                            }
                            grid_cache::Verdict::Fail => {
                                CacheRecordResultNondeterministicItemPrevious::Fail
                            }
                        },
                        previous_at: previous.recorded_at.clone(),
                    });
                }
                cache
                    .put(
                        key,
                        grid_cache::Entry {
                            verdict,
                            recorded_at: recorded_at.clone(),
                            trace_id: params.trace_id.clone(),
                            // Stored so the key can be recomputed later:
                            // `grid drift --tests` asks whether this verdict
                            // still applies, and a key is a hash.
                            env_hash: Some(params.env_hash.clone()),
                        },
                    )
                    .map_err(|e| e.to_string())?;
                recorded += 1;
                if let Some(test) = &outcome_test {
                    let row = crate::history::Outcome {
                        datum: entry.datum.0.clone(),
                        test: test.clone(),
                        fails: entry.verdict == CacheResultEntryVerdict::Fail,
                        at: Some(recorded_at.clone()),
                    };
                    let _unwritable = crate::history::append_outcomes(root, &[row]);
                }
            }
            Err(reject) => rejected.push(CacheRecordResultRejectedItem {
                datum: entry.datum.clone(),
                reason: reject.to_string(),
            }),
        }
    }
    Ok(CacheRecordResult {
        recorded,
        rejected,
        nondeterministic: (!nondeterministic.is_empty()).then_some(nondeterministic),
    })
}

/// Why `grid.test.plan` could not answer.
#[derive(Debug)]
pub enum TestPlanError {
    /// An endpoint did not resolve, or the worktree survey failed.
    Chart(chart_provider::Error),
    /// The trace store or the verdict log could not be read. Neither is an
    /// empty set: a store that fails to open has unknown contents, and a
    /// plan computed as if it were empty would skip tests on nothing.
    Store(String),
}

impl std::fmt::Display for TestPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chart(e) => write!(f, "{e}"),
            Self::Store(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for TestPlanError {}

/// `grid.test.plan` — selection as data over base and a fresh worktree
/// survey. The survey is the head, as it is for `grid drift`, because the
/// everyday question is "what do my uncommitted edits reach".
///
/// Base defaults to the snapshot the last `grid test` recorded against,
/// then to `HEAD`. A remembered snapshot the store no longer holds (an
/// epoch bump drops the store) falls back to `HEAD` rather than refusing:
/// the remembered base is a convenience, not a claim.
///
/// # Errors
/// [`TestPlanError::Chart`] when base cannot be resolved or the worktree
/// cannot be surveyed; [`TestPlanError::Store`] when the traces or the
/// verdict log cannot be read.
pub fn test_plan(
    params: &grid_wire::methods::TestPlanParams,
    root: &std::path::Path,
    chart: &mut Chart,
) -> Result<grid_wire::TestPlan, TestPlanError> {
    test_plan_timed(params, root, chart).map(|(plan, _)| plan)
}

/// How long the two halves of a plan took: the surveys, which are the
/// ledger's `grid survey` row, and the plan itself, which is this step's.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub survey: std::time::Duration,
    pub plan: std::time::Duration,
}

/// [`test_plan`], with the timing the CLI states on stderr.
///
/// # Errors
/// As [`test_plan`].
pub fn test_plan_timed(
    params: &grid_wire::methods::TestPlanParams,
    root: &std::path::Path,
    chart: &mut Chart,
) -> Result<(grid_wire::TestPlan, Timing), TestPlanError> {
    let started = std::time::Instant::now();
    let head_rev = SnapshotOrRev::GitRev {
        git_rev: GitRev("HEAD".to_owned()),
    };
    let base = match &params.base {
        Some(base) => base.clone(),
        None => match std::fs::read_to_string(crate::testplan::last_path(root)) {
            Ok(id) => {
                let remembered = SnapshotOrRev::Snapshot {
                    snapshot: SnapshotId(id.trim().to_owned()),
                };
                let held = ChartDelta::open(root)
                    .and_then(|c| c.epoch_of(&remembered))
                    .is_ok();
                if held {
                    remembered
                } else {
                    head_rev.clone()
                }
            }
            Err(_) => head_rev.clone(),
        },
    };
    // A rev base is surveyed *at the rev*, out of the object store, retained
    // rather than made live, and named by the snapshot that produced — a
    // plan of a snapshot against itself selects nothing, and the worktree
    // survey below must not be mistaken for the base. Idempotent (D5): an
    // already-surveyed rev costs a manifest scan.
    let base = chart.ensure(&base).map_err(TestPlanError::Chart)?;
    let (_, ran) = chart
        .survey_run(&SurveyRunParams {
            git_rev: None,
            worktree: None,
        })
        .map_err(TestPlanError::Chart)?;
    let survey = started.elapsed();
    let planning = std::time::Instant::now();
    let provider = ChartDelta::open(root).map_err(TestPlanError::Chart)?;
    let base_epoch = provider.epoch_of(&base).map_err(TestPlanError::Chart)?;
    let head_epoch = provider.head();
    let diff = grid_chart::diff(
        SnapshotId(base_epoch.snapshot_id().as_str().to_owned()),
        base_epoch.segments(),
        SnapshotId(head_epoch.snapshot_id().as_str().to_owned()),
        head_epoch.segments(),
    );
    let changed = crate::testplan::changed_of(&diff);
    let traces = trace_dir(root)
        .and_then(grid_trace::Store::open)
        .and_then(|store| store.retained())
        .map_err(|e| TestPlanError::Store(e.to_string()))?;
    let cache = grid_cache::Cache::open(cache_path(root))
        .map_err(|e| TestPlanError::Store(e.to_string()))?;
    let plan = crate::testplan::compute(&crate::testplan::Inputs {
        base: base_epoch.segments(),
        head: head_epoch.segments(),
        base_id: diff.base,
        head_id: diff.head,
        changed: &changed,
        traces: &traces,
        cache: &cache,
        env_hash: &params.env_hash,
        tests: &params.tests,
        replay: params.replay.unwrap_or(true),
        fuel: grid_cache::FUEL,
        declined: Some(ran.declined),
    });
    Ok((
        plan,
        Timing {
            survey,
            plan: planning.elapsed(),
        },
    ))
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
/// the edit log that would count edits does not exist until a later wave.
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
        return Err(chart.no_snapshot());
    };
    let all = live_defs(&chart);
    let matched: Vec<String> = all
        .iter()
        .filter(|(_, callsign, _, _, _)| {
            params
                .callsigns
                .iter()
                .any(|p| grid_gate::pattern::matches(&p.0, callsign))
        })
        .map(|(datum, _, _, _, _)| datum.clone())
        .collect();
    // Asking for text asks for the hops the text lives at: a `--sigs 3` pack
    // over a zero-hop walk would certify three hops it never walked. An
    // explicit `--depth` past the tiers still widens the def and edge sections.
    let tiers = params.bodies.is_some() || params.sigs.is_some();
    let bodies_n = params.bodies.unwrap_or(1);
    // A body carries its own sig, so a sig tier below the body tier is not a
    // narrower answer, only a confusing certificate.
    let sigs_m = params.sigs.unwrap_or(3).max(bodies_n);
    let depth = if tiers {
        params.depth.unwrap_or(0).max(sigs_m)
    } else {
        params.depth.unwrap_or(0)
    };
    let mut layers = expand(&chart, matched, depth);
    if let Some(since) = params.since.as_ref() {
        let moved = moved_since(&chart, since)?;
        for layer in &mut layers {
            layer.retain(|datum| moved.contains(datum));
        }
    }
    let in_scope: Vec<String> = layers.iter().flatten().cloned().collect();

    let defs: Vec<grid_wire::DefRef> = all
        .iter()
        .filter(|(datum, _, _, _, _)| in_scope.contains(datum))
        .map(
            |(datum, callsign, lang, materialization, _)| grid_wire::DefRef {
                datum: grid_wire::Datum(datum.clone()),
                callsign: Some(grid_wire::Callsign(callsign.clone())),
                lang: *lang,
                signature: None,
                cfg: None,
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
            let patterns = match &inv.rule {
                grid_wire::InvariantRule::ForbidEdge { forbid_edge } => {
                    vec![&forbid_edge.src, &forbid_edge.dst]
                }
                grid_wire::InvariantRule::RequireEdge { require_edge } => {
                    vec![&require_edge.src, &require_edge.dst]
                }
                // One pattern, over the type itself rather than over an edge.
                grid_wire::InvariantRule::Persisted { persisted } => vec![persisted],
            };
            patterns.into_iter().any(|pattern| {
                callsigns
                    .iter()
                    .any(|c| grid_gate::pattern::matches(&pattern.0, c))
            })
        })
        .collect();

    let (bodies, sigs, horizon) = if tiers {
        let packed = pack_text(root, &all, &layers, bodies_n, sigs_m, params.budget);
        (Some(packed.bodies), Some(packed.sigs), Some(packed.horizon))
    } else {
        (None, None, None)
    };

    Ok(ScopeResult {
        snapshot,
        defs,
        in_edges,
        out_edges,
        invariants,
        anchors: Vec::new(),
        churn: Vec::new(),
        bodies,
        sigs,
        horizon,
    })
}

/// Every live datum whose definition moved since `since` — the head side of
/// every change a membership diff reports. A sig move implies a datum move, so
/// "datum or sig moved" is exactly this set; a definition that was removed has
/// no head side and is not in the live chart to pack anyway.
fn moved_since(
    chart: &ChartDelta,
    since: &SnapshotOrRev,
) -> Result<std::collections::HashSet<String>, chart_provider::Error> {
    // ponytail: resolved against retained epochs, never surveyed — `scope` is
    // a read, and a survey inside it would cost more than the whole pack. A
    // rev nobody has surveyed is `NoSnapshotForRev`, which says so.
    let base = chart.epoch_of(since)?;
    let head = chart.head();
    let diff = grid_chart::diff(
        SnapshotId(base.snapshot_id().as_str().to_owned()),
        base.segments(),
        SnapshotId(head.snapshot_id().as_str().to_owned()),
        head.segments(),
    );
    Ok(diff
        .defs
        .iter()
        .filter_map(|d| Some(d.after.as_ref()?.0.clone()))
        .collect())
}

/// The text tiers and the certificate over them.
struct Packed {
    bodies: Vec<grid_wire::PackText>,
    sigs: Vec<grid_wire::PackText>,
    horizon: grid_wire::PackHorizon,
}

/// One definition's renderable text at one hop: its body as the worktree
/// holds it, and the sig its frontend renders from that body.
struct Renderable {
    def: grid_wire::DefRef,
    callsign: String,
    body: String,
    sig: Option<String>,
}

/// One `Renderable` per definition in each hop layer, layers ordered by hop
/// and definitions inside a layer by callsign — so the pack is a function of
/// the chart's content and not of walk order (C2).
fn renderables(
    root: &std::path::Path,
    all: &[LiveDef],
    layers: &[Vec<String>],
    bodies_n: u64,
    sigs_m: u64,
    budget: Option<u64>,
) -> Vec<Vec<Renderable>> {
    let by_datum: std::collections::HashMap<&str, &LiveDef> =
        all.iter().map(|d| (d.0.as_str(), d)).collect();
    // ponytail: one read per file, memoized across the pack — the plan's cost
    // row is a file read per body, and a hot definition's file is read once.
    let mut files: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut per_hop: Vec<Vec<Renderable>> = Vec::new();
    for layer in layers.iter().take(
        usize::try_from(sigs_m)
            .unwrap_or(usize::MAX)
            .saturating_add(1),
    ) {
        let hop = u64::try_from(per_hop.len()).unwrap_or(u64::MAX);
        let mut here: Vec<Renderable> = Vec::new();
        for datum in layer {
            let Some((_, callsign, lang, materialization, line_end)) =
                by_datum.get(datum.as_str()).copied()
            else {
                continue;
            };
            // No location is no body: a definition grid cannot point at is not
            // one it can quote. It stays in `defs`, where a handle is enough.
            let (Some(loc), Some(line_end)) = (materialization.as_ref(), *line_end) else {
                continue;
            };
            let Some(body) = body_at(root, &mut files, &loc.file, loc.line, u64::from(line_end))
            else {
                continue;
            };
            // The body is the sig's input, so the sig costs one parse and no
            // extra read. Skipped for a body layer when no budget can demote
            // it — nothing would ever ask for it.
            let sig = (hop > bodies_n || budget.is_some())
                .then(|| lang.and_then(|lang| grid_survey::sig_text(lang, &body)))
                .flatten();
            here.push(Renderable {
                def: grid_wire::DefRef {
                    datum: grid_wire::Datum(datum.clone()),
                    callsign: Some(grid_wire::Callsign(callsign.clone())),
                    lang: *lang,
                    signature: None,
                    cfg: None,
                    materialization: materialization.clone(),
                },
                callsign: callsign.clone(),
                body,
                sig,
            });
        }
        here.sort_by(|a, b| a.callsign.cmp(&b.callsign));
        per_hop.push(here);
    }
    per_hop
}

/// Bodies for hops `0..=bodies_n`, sig text for the hops past that up to
/// `sigs_m`, spent against `budget` in whole layers, and the certificate that
/// says how far each tier is complete.
///
/// The budget is spent farthest layer first, and a body layer that will not
/// fit is demoted to its own sigs before it is dropped: a sig where a body was
/// asked for is a smaller answer, while half a body is a wrong one. Nothing
/// here truncates a layer, which is what lets the horizon be believed (C31).
fn pack_text(
    root: &std::path::Path,
    all: &[LiveDef],
    layers: &[Vec<String>],
    bodies_n: u64,
    sigs_m: u64,
    budget: Option<u64>,
) -> Packed {
    let per_hop = renderables(root, all, layers, bodies_n, sigs_m, budget);
    // Byte cost per hop, bodies and sigs, taken once: the budget search reads
    // this table and never the text again.
    let weight: Vec<(u64, u64)> = per_hop
        .iter()
        .map(|here| {
            let sum = |f: fn(&Renderable) -> Option<&String>| {
                here.iter()
                    .map(|d| u64::try_from(f(d).map_or(0, String::len)).unwrap_or(u64::MAX))
                    .fold(0u64, u64::saturating_add)
            };
            (sum(|d| Some(&d.body)), sum(|d| d.sig.as_ref()))
        })
        .collect();
    let cost = |b: Option<u64>, s: Option<u64>| -> u64 {
        weight
            .iter()
            .enumerate()
            .map(|(hop, (body, sig))| {
                let k = u64::try_from(hop).unwrap_or(u64::MAX);
                if b.is_some_and(|b| k <= b) {
                    *body
                } else if s.is_some_and(|s| k <= s) {
                    *sig
                } else {
                    0
                }
            })
            .fold(0u64, u64::saturating_add)
    };

    // Layers past the walk are empty and cost nothing, so the search runs over
    // the populated range and the certificate reports the requested depth when
    // nothing had to be given up: a hop the walk ran and found nothing at is
    // complete, vacuously but truly.
    let top = u64::try_from(per_hop.len().saturating_sub(1)).unwrap_or(u64::MAX);
    let (start_b, start_s) = (bodies_n.min(top), sigs_m.min(top));
    let (mut b, mut s) = (Some(start_b), Some(start_s));
    if let Some(budget) = budget {
        // Each step drops one layer or demotes one, so twice the layer count
        // is a bound, not a hope.
        for _ in 0..=top.saturating_mul(2).saturating_add(4) {
            if cost(b, s) / 4 <= budget {
                break;
            }
            match (b, s) {
                (_, None) => break,
                (Some(bb), Some(ss)) if ss > bb => s = Some(ss.saturating_sub(1)),
                (Some(0), Some(_)) => b = None,
                (Some(bb), Some(_)) => b = Some(bb.saturating_sub(1)),
                (None, Some(0)) => s = None,
                (None, Some(ss)) => s = Some(ss.saturating_sub(1)),
            }
        }
    }
    let untouched = b == Some(start_b) && s == Some(start_s);
    let (reported_b, reported_s) = if untouched {
        (Some(bodies_n), Some(sigs_m))
    } else {
        (b, s)
    };

    let mut bodies = Vec::new();
    let mut sigs = Vec::new();
    for (hop, here) in per_hop.into_iter().enumerate() {
        let k = u64::try_from(hop).unwrap_or(u64::MAX);
        let body_layer = b.is_some_and(|b| k <= b);
        let sig_layer = !body_layer && s.is_some_and(|s| k <= s);
        for d in here {
            if body_layer {
                bodies.push(grid_wire::PackText {
                    def: d.def,
                    hop: k,
                    text: d.body,
                });
            } else if sig_layer {
                if let Some(text) = d.sig {
                    sigs.push(grid_wire::PackText {
                        def: d.def,
                        hop: k,
                        text,
                    });
                }
            }
        }
    }
    Packed {
        bodies,
        sigs,
        horizon: grid_wire::PackHorizon {
            bodies_complete_within: reported_b,
            sigs_complete_within: reported_s,
            // ponytail: bytes/4 as the token estimate — grid ships no
            // tokenizer and will not add one; every rendering states the
            // number as an approximation, and a real count is the upgrade if
            // a caller ever needs the budget to be exact rather than safe.
            approx_tokens: cost(b, s) / 4,
        },
    }
}

/// Source lines `[line, line_end]` of one file in the worktree, 1-based and
/// inclusive — the definition's body as the tree holds it *now*, which is what
/// an agent is about to edit. A file the survey named and the tree no longer
/// has yields nothing rather than a fabricated body.
fn body_at(
    root: &std::path::Path,
    files: &mut std::collections::HashMap<String, Option<String>>,
    file: &str,
    line: u64,
    line_end: u64,
) -> Option<String> {
    let source = files
        .entry(file.to_owned())
        .or_insert_with(|| std::fs::read_to_string(root.join(file)).ok())
        .as_deref()?;
    let skip = usize::try_from(line.saturating_sub(1)).unwrap_or(usize::MAX);
    let take =
        usize::try_from(line_end.saturating_sub(line).saturating_add(1)).unwrap_or(usize::MAX);
    let mut text: String = source
        .lines()
        .skip(skip)
        .take(take)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return None;
    }
    text.push('\n');
    Some(text)
}

type LiveDef = (
    String,
    String,
    Option<grid_wire::Lang>,
    Option<grid_wire::DefRefMaterialization>,
    Option<u32>,
);

/// Every definition in the live epoch: (datum, callsign, lang, location, the
/// location's last line), in segment order — which is content order, so
/// everything derived from it is deterministic by construction (C2). The last
/// line rides along because a body is the source lines `[line, line_end]`,
/// and only the segment knows where a definition ends.
fn live_defs(chart: &ChartDelta) -> Vec<LiveDef> {
    let mut all = Vec::new();
    for segment in chart.head().segments() {
        for ord in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(ord);
            if let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) {
                let loc = segment.location(ord);
                let materialization = loc.as_ref().map(|loc| grid_wire::DefRefMaterialization {
                    file: loc.path.to_owned(),
                    line: u64::from(loc.line),
                });
                all.push((
                    datum.to_string(),
                    callsign.to_owned(),
                    segment.lang(ord),
                    materialization,
                    loc.map(|loc| loc.line_end),
                ));
            }
        }
    }
    all
}

/// The matched set plus `depth` BFS layers over edges in both directions, one
/// entry per hop: layer 0 is the definitions the patterns name, layer k those
/// first reached in k hops. The walk stops as soon as a layer comes back
/// empty — every hop past that is empty too, and a caller asking for a depth
/// past the graph should not pay a segment scan per hop for the privilege. A
/// hop the walk ran and found nothing at is *complete*, not unvisited, which
/// is the distinction the pack's horizon rests on.
///
/// Flattened, this is exactly the order the pre-tier `scope` walked in, so the
/// pack a request without `bodies`/`sigs` gets is byte-identical to before.
fn expand(chart: &ChartDelta, matched: Vec<String>, depth: u64) -> Vec<Vec<String>> {
    let mut layers = vec![matched.clone()];
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
        if next.is_empty() {
            break;
        }
        in_scope.extend(next.iter().cloned());
        layers.push(next.clone());
        frontier = next;
    }
    layers
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
                    grade: n.grade,
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

/// `grid.todo` — the gate verdict rendered as a worklist: what the plan
/// promised, split into made good and pending. This is a *view* over the
/// same judgment `grid.gate.check` computes (CLI plan §5) — the delta, the
/// verdict, and this list can never disagree, because they are one engine.
///
/// `head` defaults to the live snapshot: the worklist question is "what is
/// left *now*", and now is what was last surveyed.
///
/// # Errors
/// [`chart_provider::Error::Plan`] when the plan id names nothing,
/// [`chart_provider::Error::NoSnapshot`] when nothing has been surveyed, and
/// the usual provider errors for a delta that cannot be computed.
pub fn todo(
    params: &TodoParams,
    root: &std::path::Path,
) -> Result<TodoResult, chart_provider::Error> {
    let plan = crate::plan::load(&params.plan.0).map_err(chart_provider::Error::Plan)?;
    let plan = standing::fold(
        &plan,
        standing::load(root).map_err(chart_provider::Error::Config)?,
    );
    let mut chart = Chart::new(root);
    chart.ensure(&plan.base)?;
    let head = if let Some(head) = &params.head {
        chart.head_of(head)?
    } else {
        let live = ChartDelta::open(root)?;
        SnapshotOrRev::Snapshot {
            snapshot: live.current().ok_or_else(|| live.no_snapshot())?,
        }
    };
    let provider = ChartDelta::open(root)?;
    let diff = provider.diff(&plan.base, &head)?;
    let verdict = grid_gate::verdict(&plan, &diff, &provider);
    let (defs_done, defs_pending) = grid_gate::def_progress(&plan, &diff);
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
        plan.delta.defs.len() + plan.delta.edges.len() + plan.tests.as_ref().map_or(0, Vec::len);
    Ok(TodoResult {
        plan_id: grid_wire::PlanId(plan.plan_id.clone()),
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
                if n.grade != grid_wire::Grade::Proven {
                    continue;
                }
                let Some(src) = segment.datum(n.other) else {
                    continue;
                };
                edges.push(grid_wire::Edge {
                    src: grid_wire::Datum(src.to_string()),
                    relation: n.relation,
                    dst: def.datum.clone(),
                    grade: n.grade,
                    via: None,
                });
            }
        }
    }
    edges.sort_by(|a, b| (&a.src.0, a.relation as u8).cmp(&(&b.src.0, b.relation as u8)));
    Ok((defs, edges))
}

/// `grid.trace.ingest` — resolve a test-run trace against the live chart,
/// retain it, and report what it meant.
///
/// Resolution is the `grid.resolve` rule (unique dotted-suffix), so the name
/// a user can resolve is the name a trace can. What cannot resolve is
/// returned, never dropped (C15): an observed edge always names two datums
/// that could be pointed at.
///
/// # Errors
/// Returns a message when the chart store or the trace log cannot be read or
/// written. Ingesting into an unsurveyed repo is an error even for an
/// all-datum trace: symbol resolution must never silently see an empty chart.
pub fn trace_ingest(
    params: &TraceIngestParams,
    root: &std::path::Path,
) -> Result<TraceIngestResult, String> {
    let epoch = epoch(root)?;
    let ingested = grid_trace::ingest(epoch.segments(), &params.trace);
    let store = trace_dir(root)
        .and_then(grid_trace::Store::open)
        .map_err(|e| e.to_string())?;
    store
        .put(&params.trace, &ingested)
        .map_err(|e| e.to_string())?;
    #[allow(clippy::cast_possible_truncation)]
    let observed_edges = ingested.edges.len() as u64;
    Ok(TraceIngestResult {
        observed_edges,
        resolved: ingested.resolved,
        unresolved_symbols: ingested.unresolved_symbols,
    })
}
/// Why `grid.trace.diff` could not answer.
#[derive(Debug)]
pub enum TraceDiffError {
    /// A named trace was never ingested under that id.
    UnknownTrace(String),
    /// The retention log could not be read or is corrupt — suspect, the way a
    /// corrupt segment is, not silently empty.
    Store(String),
}
impl std::fmt::Display for TraceDiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTrace(id) => write!(f, "unknown trace id {id}: ingest it first"),
            Self::Store(msg) => f.write_str(msg),
        }
    }
}
/// `grid.trace.diff` — two retained traces compared at call granularity.
///
/// # Errors
/// [`TraceDiffError::UnknownTrace`] when either id names nothing retained;
/// [`TraceDiffError::Store`] when the log itself fails.
pub fn trace_diff(
    params: &TraceDiffParams,
    root: &std::path::Path,
) -> Result<grid_wire::TraceDiff, TraceDiffError> {
    let store = trace_dir(root)
        .and_then(grid_trace::Store::open)
        .map_err(|e| TraceDiffError::Store(e.to_string()))?;
    let load = |id: &grid_wire::TraceId| {
        store
            .get(&id.0)
            .map_err(|e| TraceDiffError::Store(e.to_string()))?
            .ok_or_else(|| TraceDiffError::UnknownTrace(id.0.clone()))
    };
    let (green, red) = (load(&params.green)?, load(&params.red)?);
    Ok(grid_trace::diff(&green, &red))
}
/// The retained-trace directory, migrating the epoch-0 name once.
///
/// Wire epoch 0 called it `.grid/wakes`; epoch 1 calls it `.grid/traces`
/// (`docs/plans/2026-09-01-naming.md`). When only the old directory is there,
/// it is renamed — one `rename(2)`, so retained traces move whole and are
/// never rewritten. When both are there, grid refuses: retained traces are
/// user data, and picking one would quietly discard the other's runs.
///
/// # Errors
/// [`grid_trace::StoreError::Io`] if the rename fails;
/// [`grid_trace::StoreError::Ambiguous`] if both directories exist.
pub(crate) fn trace_dir(
    root: &std::path::Path,
) -> Result<std::path::PathBuf, grid_trace::StoreError> {
    let current = root.join(".grid/traces");
    let former = root.join(".grid/wakes");
    match (former.is_dir(), current.is_dir()) {
        (true, false) => std::fs::rename(&former, &current)?,
        (true, true) => {
            let (former, current) = (former.display(), current.display());
            return Err(grid_trace::StoreError::Ambiguous(format!(
                "both {former} and {current} exist; grid will not guess which holds the \
                 retained traces — move what you want to keep into {current} and remove \
                 {former}"
            )));
        }
        _ => {}
    }
    Ok(current)
}
