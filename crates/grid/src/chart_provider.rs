//! The chart-backed delta provider: the real one.
//!
//! Wave 3 gated against `git diff --name-status`, which sees *files* and
//! therefore proves no edges: every promised edge came back `missing` and held
//! the gate. This provider answers from the surveyed chart instead, so a
//! promise about a call is checked against the call.
//!
//! It implements the same [`DeltaProvider`] trait, which is why nothing above
//! it changed: [`crate::provider`] designed the seam and this is what slid
//! into it.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use grid_chart_io::{Epoch, Store};
use grid_wire::{Callsign, Datum, Diff, SnapshotId, SnapshotOrRev};
use grid_xte::DeltaProvider;

/// Why a chart-backed delta could not be produced.
#[derive(Debug)]
pub enum Error {
    /// The segment store could not be opened or read.
    Store(grid_chart_io::StoreError),
    /// The survey that would have produced a snapshot failed.
    Survey(grid_survey::SurveyError),
    /// A git rev was named, but no snapshot has been published for it. Unlike
    /// an unknown snapshot id this is recoverable by surveying, so it says so.
    NoSnapshotForRev(String),
    /// Repo configuration under `.grid` is present but unreadable. A broken
    /// rule file must not be mistaken for an absence of rules.
    Config(String),
    /// A query needs a snapshot and the store has never held one.
    NoSnapshot,
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
            Self::NoSnapshotForRev(rev) => {
                write!(f, "no snapshot for rev {rev}: run `grid survey` first")
            }
            Self::Config(msg) | Self::Plan(msg) => f.write_str(msg),
            Self::NoSnapshot => f.write_str("no snapshot published: run `grid survey` first"),
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
    /// [`Error::Store`] if the store cannot be opened.
    pub fn open(root: &Path) -> Result<Self, Error> {
        let store = Store::open(store_dir(root))?;
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

    /// Has any snapshot been published for `rev`?
    ///
    /// # Errors
    /// [`Error::Store`] if the store cannot be read.
    pub fn has_rev(&self, rev: &str) -> Result<bool, Error> {
        let rev = resolve_rev(&self.root, rev);
        if self.head.manifest().git_rev == rev && !self.head.segments().is_empty() {
            return Ok(true);
        }
        Ok(self.store.epoch_for_rev(&rev)?.is_some())
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
                if self.head.manifest().git_rev == rev {
                    return Ok(Arc::clone(&self.head));
                }
                self.store
                    .epoch_for_rev(&rev)?
                    .ok_or_else(|| Error::NoSnapshotForRev(git_rev.0.clone()))
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
