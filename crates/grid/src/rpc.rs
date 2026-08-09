//! The method layer: one function per wire method, shared verbatim by the CLI
//! and the daemon.
//!
//! The CLI is not a second implementation of the daemon — both call these
//! functions, so `grid gate --json` and a `grid.gate.check` request over the
//! socket cannot drift.

use grid_wire::methods::{
    DeltaParams, DeltaResult, GateCheckParams, GateCheckResult, ResolveParams, ResolveResult,
    SnapshotCurrentResult, SurveyRunParams, SurveyRunResult, TrackLintParams, TrackLintResult,
    VersionResult,
};
use grid_wire::{GitRev, Snapshot, SnapshotId, SnapshotOrRev, SnapshotStats};
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
    ) -> Result<(SurveyRunResult, u64), chart_provider::Error> {
        // Surveying "at a rev" means the tree at that rev, read out of the
        // object store; with no rev it is the working tree, labelled with the
        // rev it was checked out from.
        let (survey, rev) = match params.git_rev.as_ref() {
            Some(rev) => (
                grid_survey::run_at(&self.root, &rev.0, &mut self.cache)?,
                chart_provider::resolve_rev(&self.root, &rev.0),
            ),
            None => (
                grid_survey::run(&self.root, &mut self.cache)?,
                chart_provider::resolve_rev(&self.root, "HEAD"),
            ),
        };
        // The daemon is the single writer (D6); `publish` takes `&mut self`,
        // which is that rule in the type system.
        let mut store = grid_chart_io::Store::open(chart_provider::store_dir(&self.root))
            .map_err(chart_provider::Error::Store)?;
        let id = store
            .publish(&rev, None, &[survey.image])
            .map_err(chart_provider::Error::Store)?;
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
            // How many files came from the cache rather than the parser. Not
            // a wire field — it describes this *run*, not the snapshot, which
            // is why it is returned beside the snapshot instead of inside it.
            survey.reread,
        ))
    }
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
