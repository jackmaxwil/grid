//! The chart-backed delta provider: the real one.
//!
//! Wave 3 gated against `git diff --name-status`, which sees *files* and
//! therefore proves no edges: every promised edge came back `missing` and held
//! the gate. This provider answers from the surveyed chart instead, so a
//! promise about a call is checked against the call.
//!
//! It implements the same [`grid_gate::DeltaProvider`] trait, which is why
//! nothing above it changed: the gate designed the seam and this slid in.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use grid_chart_io::{Epoch, Store};
use grid_gate::DeltaProvider;
use grid_wire::{Callsign, Datum, Diff, SnapshotId, SnapshotOrRev};

/// What to run, and where, to fix a refusal that a survey fixes.
///
/// Every missing-snapshot refusal in the engine has the same recovery and
/// used to say so vaguely: "run `grid survey` first", with no tree named,
/// and with a superseded store — which opens *empty*, so it looks exactly
/// like a store nobody surveyed — giving no reason at all. A refusal a
/// reader cannot act on is a dead end, and against another worktree "run
/// survey" in the wrong directory is worse than a dead end. So the recovery
/// rides the error: one hint line, with the command and the tree, wherever
/// the error surfaces — the CLI, the daemon, a verb that wrapped it (T-10).
#[derive(Debug, Clone)]
pub struct Recovery {
    /// The tree the command runs in.
    root: PathBuf,
    /// The rev to survey, or `None` for the worktree.
    rev: Option<String>,
    /// The format epoch this store superseded, when it has one.
    superseded: Option<u32>,
    /// What the bytes said when the store would not open at all: a segment
    /// that no longer hashes to its name, a manifest that does not parse.
    damaged: Option<String>,
}

impl Recovery {
    /// The recovery for a tree whose store is not open at hand: the command
    /// and the directory, with no superseded epoch to report — a caller that
    /// holds the store says so through [`ChartDelta::recovery`] instead.
    #[must_use]
    pub fn at(root: &Path, rev: Option<&str>) -> Self {
        Self {
            root: root.to_path_buf(),
            rev: rev.map(str::to_owned),
            superseded: None,
            damaged: None,
        }
    }

    /// The rev the caller asked about, when it asked about one.
    #[must_use]
    pub fn rev(&self) -> Option<&str> {
        self.rev.as_deref()
    }
}

impl std::fmt::Display for Recovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(epoch) = self.superseded {
            writeln!(
                f,
                "the chart there was written by segment format epoch {epoch} — superseded, \
                 so it opens empty"
            )?;
        }
        if let Some(what) = &self.damaged {
            writeln!(
                f,
                "the chart there is damaged — {what} — so nothing can be read from it; \
                 `grid survey` drops it and surveys cold"
            )?;
        }
        match &self.rev {
            Some(rev) => write!(
                f,
                "hint: run `grid survey {rev}` in {}",
                self.root.display()
            ),
            None => write!(f, "hint: run `grid survey` in {}", self.root.display()),
        }
    }
}

/// Why a chart-backed delta could not be produced.
#[derive(Debug)]
pub enum Error {
    /// The segment store could not be opened or read.
    Store(grid_chart_io::StoreError),
    /// The survey that would have produced a snapshot failed.
    Survey(grid_survey::SurveyError),
    /// A git rev was named, but the store holds no snapshot of the tree at
    /// it — a dirty worktree survey labelled with the rev does not count.
    /// Unlike an unknown snapshot id this is recoverable by surveying, so it
    /// carries the [`Recovery`] that says how.
    NoSnapshotForRev(Recovery),
    /// Repo configuration under `.grid` is present but unreadable. A broken
    /// rule file must not be mistaken for an absence of rules.
    Config(String),
    /// A query needs a snapshot and the store has never held one — or holds
    /// one written by a superseded format, which opens empty and is the same
    /// fix for a different reason. The [`Recovery`] states which.
    NoSnapshot(Recovery),
    /// A snapshot other than the live one was named. The engine holds
    /// exactly one live epoch; anything else cannot be answered from it.
    NotLive(String),
    /// A named plan does not exist or does not parse.
    Plan(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(e) => write!(f, "chart: {e}"),
            Self::Survey(e) => write!(f, "survey: {e}"),
            Self::NoSnapshotForRev(how) => {
                write!(
                    f,
                    "no snapshot of the tree at {} — {how}",
                    how.rev().unwrap_or("that rev")
                )
            }
            Self::Config(msg) | Self::Plan(msg) => f.write_str(msg),
            Self::NoSnapshot(how) if how.damaged.is_some() => {
                write!(f, "no readable snapshot — {how}")
            }
            Self::NoSnapshot(how) => write!(f, "no snapshot published — {how}"),
            Self::NotLive(id) => write!(f, "snapshot {id} is not the live epoch"),
        }
    }
}

impl std::error::Error for Error {}

impl From<grid_chart_io::StoreError> for Error {
    fn from(e: grid_chart_io::StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<grid_survey::SurveyError> for Error {
    fn from(e: grid_survey::SurveyError) -> Self {
        Self::Survey(e)
    }
}

/// The short sha a rev names, or the input unchanged if git cannot say.
///
/// Every rev crossing into the store is resolved through here. Snapshots are
/// recorded against a sha, so looking one up by `HEAD~1` has to ask the same
/// question the survey answered — otherwise the store would hold a snapshot
/// that no lookup can find.
#[must_use]
pub fn resolve_rev(root: &Path, rev: &str) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", rev])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || rev.to_owned(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_owned(),
        )
}

/// Where a repo's segment store lives.
#[must_use]
pub fn store_dir(root: &Path) -> PathBuf {
    root.join(".grid/chart")
}

/// A delta provider over a segment store.
///
/// The callsign map is built once at construction from the head epoch, because
/// [`DeltaProvider::callsign`] is called once per changed edge endpoint and
/// walking every segment for each would be quadratic in the size of the diff.
pub struct ChartDelta {
    root: PathBuf,
    store: Store,
    /// Datum wire form -> callsign, from the head epoch. Name-space and
    /// datum-space meet here, and this map is the whole of the unification.
    names: HashMap<String, String>,
    head: Arc<Epoch>,
}

impl ChartDelta {
    /// Open the store under `root` and index the live epoch's names.
    ///
    /// # Errors
    /// [`Error::NoSnapshot`] if the store is damaged — its recovery is the
    /// same survey every other missing snapshot wants, and the refusal says
    /// so; [`Error::Store`] if it cannot be opened for any other reason.
    pub fn open(root: &Path) -> Result<Self, Error> {
        let store = Store::open(store_dir(root)).map_err(|e| {
            if e.is_damage() {
                Error::NoSnapshot(Recovery {
                    root: root.to_path_buf(),
                    rev: None,
                    superseded: None,
                    damaged: Some(e.to_string()),
                })
            } else {
                Error::Store(e)
            }
        })?;
        let head = store.load();
        let mut names = HashMap::new();
        for segment in head.segments() {
            for ord in 0..segment.def_count() {
                let ord = grid_chart::Ordinal(ord);
                if let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) {
                    names.insert(datum.to_string(), callsign.to_owned());
                }
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            store,
            names,
            head,
        })
    }

    /// The live snapshot id, or `None` for a store that has never been
    /// surveyed. An empty live set is "no snapshot", not a snapshot of
    /// nothing.
    #[must_use]
    pub fn current(&self) -> Option<SnapshotId> {
        (!self.head.segments().is_empty()).then(|| wire_id(self.head.snapshot_id()))
    }

    /// The command that would give this store what the caller asked for,
    /// and the tree to run it in — with the superseded epoch when there is
    /// one, because "opens empty because it is old" and "opens empty because
    /// nobody surveyed" are the same fix for different reasons.
    #[must_use]
    pub fn recovery(&self, rev: Option<&str>) -> Recovery {
        Recovery {
            root: self.root.clone(),
            rev: rev.map(str::to_owned),
            superseded: self.store.superseded(),
            damaged: None,
        }
    }

    /// The refusal for a query that needs a snapshot this store does not
    /// hold, with its recovery.
    #[must_use]
    pub fn no_snapshot(&self) -> Error {
        Error::NoSnapshot(self.recovery(None))
    }

    /// The format epoch of a manifest this store refused to load.
    ///
    /// A superseded store opens *empty*, which is indistinguishable from a
    /// store nobody has surveyed unless the caller can ask this — and for a
    /// store in someone else's worktree the two demand opposite answers.
    #[must_use]
    pub const fn superseded(&self) -> Option<u32> {
        self.store.superseded()
    }

    /// The membership image beside the chart, when one has been built.
    ///
    /// `None` is an absence of derived data, not a claim that nothing
    /// changed; the next survey rebuilds it.
    ///
    /// # Errors
    /// [`Error::Store`] if the image exists and cannot be read.
    /// The concrete byte holder is named rather than hidden behind `impl
    /// AsRef<[u8]>`: an opaque type here would unify with the segment type of
    /// whatever the caller joins the image against, and the caller joins it
    /// against this store's own segments.
    pub fn membership(
        &self,
    ) -> Result<Option<grid_chart::Membership<grid_chart_io::Bytes>>, Error> {
        Ok(self.store.membership()?)
    }

    /// The live epoch itself, for read-only queries that walk segments
    /// (scope, uses). Mutation stays behind `publish`: this hands out a
    /// read view, and `Epoch` exposes nothing else.
    #[must_use]
    pub fn head(&self) -> &Arc<Epoch> {
        &self.head
    }

    /// The rev the live epoch was surveyed at.
    #[must_use]
    pub fn git_rev(&self) -> &str {
        &self.head.manifest().git_rev
    }

    /// When the live snapshot was first published here, if the store can say.
    #[must_use]
    pub fn published_at(&self) -> Option<std::time::SystemTime> {
        self.store.published_at(self.head.snapshot_id()).ok()
    }

    /// Live-epoch counts, for the wire `Snapshot`.
    #[must_use]
    pub fn stats(&self) -> grid_wire::SnapshotStats {
        let (mut defs, mut edges) = (0u64, 0u64);
        let mut langs = Vec::new();
        for segment in self.head.segments() {
            defs += u64::from(segment.def_count());
            edges += u64::from(segment.edge_count());
            for ord in 0..segment.def_count() {
                if let Some(lang) = segment.lang(grid_chart::Ordinal(ord)) {
                    if !langs.contains(&lang) {
                        langs.push(lang);
                    }
                }
            }
        }
        langs.sort_by_key(|l| format!("{l:?}"));
        grid_wire::SnapshotStats {
            defs,
            edges,
            langs: Some(langs),
        }
    }

    /// Resolve a locator against the live epoch.
    ///
    /// A callsign is matched by *unique suffix*, which is the input rule the
    /// CLI plan names: `validate` finds `pkg.mod.TagContract.validate` when
    /// nothing else ends that way. Ambiguity returns every candidate, so the
    /// caller can print them as pasteable alternatives rather than guessing.
    #[must_use]
    pub fn resolve(&self, locator: &grid_wire::Locator) -> Vec<grid_wire::DefRef> {
        use grid_wire::Locator as L;
        let mut out = Vec::new();
        for segment in self.head.segments() {
            for ord in 0..segment.def_count() {
                let ord = grid_chart::Ordinal(ord);
                let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord))
                else {
                    continue;
                };
                let hit = match locator {
                    L::Datum { datum: want } => datum.to_string() == want.0,
                    L::Callsign { callsign: want } => suffix_matches(callsign, &want.0),
                    L::Handle { handle } => handle_matches(callsign, &datum.to_string(), &handle.0),
                    L::Position { position } => datum.to_string() == position.datum.0,
                    L::FileLine { file, line } => segment.location(ord).is_some_and(|loc| {
                        loc.path == file.as_str()
                            && u64::from(loc.line) <= *line
                            && *line <= u64::from(loc.line_end)
                    }),
                };
                if hit {
                    let materialization =
                        segment
                            .location(ord)
                            .map(|loc| grid_wire::DefRefMaterialization {
                                file: loc.path.to_owned(),
                                line: u64::from(loc.line),
                            });
                    out.push(grid_wire::DefRef {
                        datum: Datum(datum.to_string()),
                        callsign: Some(Callsign(callsign.to_owned())),
                        lang: segment.lang(ord),
                        signature: None,
                        cfg: segment.cfg(ord).map(str::to_owned),
                        materialization,
                    });
                }
            }
        }
        // Deterministic order by name, then identity: two definitions can
        // share a callsign, and the wire contract requires a stable list.
        let key = |d: &grid_wire::DefRef| {
            (
                d.callsign.as_ref().map(|c| c.0.clone()).unwrap_or_default(),
                d.datum.0.clone(),
            )
        };
        out.sort_by_key(key);
        out.dedup_by(|a, b| a.datum == b.datum && a.callsign == b.callsign);
        out
    }

    /// Is the live epoch the tree at `rev` — surveyed at it, or a clean
    /// worktree checked out at it?
    ///
    /// A dirty worktree survey is labelled with the rev it was checked out
    /// from, and that label is provenance, not identity: the tree it charted
    /// is not the tree at the rev. Every rev-addressed lookup goes through
    /// here, so none of them can hand back the worktree as "the base".
    fn live_is(&self, rev: &str) -> bool {
        let manifest = self.head.manifest();
        manifest.git_rev == rev
            && manifest.worktree_diff_hash.is_none()
            && !self.head.segments().is_empty()
    }

    /// Where a datum currently materializes, if the live epoch knows.
    ///
    /// Advisory, like every materialization: it is what makes a row
    /// pasteable into an editor, not an address anything is stored under.
    #[must_use]
    pub fn locate(&self, datum: &Datum) -> Option<grid_wire::DefRefMaterialization> {
        let id = grid_chart::DatumId::parse(&datum.0)?;
        for segment in self.head.segments() {
            if let Some(ord) = segment.ordinal(&id) {
                if let Some(loc) = segment.location(ord) {
                    return Some(grid_wire::DefRefMaterialization {
                        file: loc.path.to_owned(),
                        line: u64::from(loc.line),
                    });
                }
            }
        }
        None
    }

    /// The epoch a `SnapshotOrRev` names.
    ///
    /// A snapshot id is looked up directly. A rev is matched against published
    /// manifests, since a manifest records the rev it was surveyed at — so a
    /// caller holding only a rev (which is what CI holds) still resolves.
    ///
    /// # Errors
    /// [`Error::NoSnapshotForRev`] when a rev has never been surveyed,
    /// [`Error::Store`] when the store cannot be read.
    pub fn epoch_of(&self, target: &SnapshotOrRev) -> Result<Arc<Epoch>, Error> {
        match target {
            SnapshotOrRev::Snapshot { snapshot } => Ok(self.store.epoch_of(&chart_id(snapshot))?),
            SnapshotOrRev::GitRev { git_rev } => {
                let rev = resolve_rev(&self.root, &git_rev.0);
                // The live epoch first: gating head against base is the common
                // case and head is usually what was just surveyed.
                if self.live_is(&rev) {
                    return Ok(Arc::clone(&self.head));
                }
                self.store
                    .epoch_for_rev(&rev)?
                    .ok_or_else(|| Error::NoSnapshotForRev(self.recovery(Some(&git_rev.0))))
            }
        }
    }

    /// The snapshot id a `SnapshotOrRev` names, without mapping its segments.
    ///
    /// A snapshot id is a hash of a manifest, so naming a snapshot never
    /// needs the chart it indexes. A query that only wants the coordinate —
    /// a membership diff reads a separate image — takes this rather than
    /// [`ChartDelta::epoch_of`], which maps and validates every segment of an
    /// epoch it will never walk.
    ///
    /// # Errors
    /// [`Error::NoSnapshotForRev`] when a rev has never been surveyed,
    /// [`Error::Store`] when the store cannot be read.
    pub fn snapshot_of(&self, target: &SnapshotOrRev) -> Result<SnapshotId, Error> {
        match target {
            SnapshotOrRev::Snapshot { snapshot } => Ok(snapshot.clone()),
            SnapshotOrRev::GitRev { git_rev } => {
                let rev = resolve_rev(&self.root, &git_rev.0);
                if self.live_is(&rev) {
                    return Ok(wire_id(self.head.snapshot_id()));
                }
                self.store
                    .snapshot_for_rev(&rev)?
                    .map(|id| wire_id(&id))
                    .ok_or_else(|| Error::NoSnapshotForRev(self.recovery(Some(&git_rev.0))))
            }
        }
    }
}

/// Wire `SnapshotId` as the chart's. Both are `snap:b3:<hex>`; the two types
/// exist so the wire crate stays free of chart internals.
fn chart_id(id: &SnapshotId) -> grid_chart::SnapshotId {
    grid_chart::SnapshotId::from_wire(&id.0)
}

/// The chart's `SnapshotId` as the wire's.
fn wire_id(id: &grid_chart::SnapshotId) -> SnapshotId {
    SnapshotId(id.as_str().to_owned())
}

impl DeltaProvider for ChartDelta {
    type Error = Error;

    fn diff(&self, base: &SnapshotOrRev, head: &SnapshotOrRev) -> Result<Diff, Self::Error> {
        let (base_epoch, head_epoch) = (self.epoch_of(base)?, self.epoch_of(head)?);
        Ok(grid_chart::diff(
            wire_id(base_epoch.snapshot_id()),
            base_epoch.segments(),
            wire_id(head_epoch.snapshot_id()),
            head_epoch.segments(),
        ))
    }

    fn callsign(&self, datum: &Datum) -> Option<Callsign> {
        self.names.get(&datum.0).map(|c| Callsign(c.clone()))
    }

    // `test_outcome` keeps the trait default: the chart records structure, not
    // test runs. Real outcomes arrive with trace ingestion (Wave 8).
}

/// Does `callsign` end with `want` on a dotted-segment boundary?
///
/// `validate` matches `pkg.TagContract.validate` but not `revalidate`: a
/// suffix that starts mid-name is not a name, and matching it would resolve a
/// locator to a definition the user did not ask for.
/// Match `name@hexprefix` against a callsign and its datum. The suffix after
/// the LAST `@` is a datum pin only when it is pure hex — a Rust trait-impl
/// callsign (`Foo@Frob.go`) carries `@` in the name itself, and its
/// trait/method suffix always holds a `.` or a non-hex letter. (A trait
/// spelled only in hex letters and pinned bare would misparse; none exists.)
pub(crate) fn handle_matches(callsign: &str, datum: &str, handle: &str) -> bool {
    match handle.rsplit_once('@') {
        Some((name, pin)) if !pin.is_empty() && pin.bytes().all(|b| b.is_ascii_hexdigit()) => {
            suffix_matches(callsign, name)
                && datum.strip_prefix("b3:").unwrap_or(datum).starts_with(pin)
        }
        _ => suffix_matches(callsign, handle),
    }
}

pub(crate) fn suffix_matches(callsign: &str, want: &str) -> bool {
    callsign == want
        || callsign
            .strip_suffix(want)
            .is_some_and(|head| head.ends_with('.'))
}
