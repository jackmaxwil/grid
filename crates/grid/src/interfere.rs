//! `grid interfere` — the semantic overlap between two worktrees.
//!
//! Parallel sessions in worktrees merge on text. Git sees files; a moved
//! signature here and a caller of it there share none, so the merge is clean
//! and the tree is broken. Grid holds both charts, so it can see it:
//!
//! - `touched(X)` is every definition whose datum moved between the common
//!   base and `X`'s live snapshot — one membership merge-diff per store,
//!   base-side, so the two sets are comparable coordinates.
//! - `drift(X)` is `grid drift base X` in `X`'s own store: dependents of a
//!   changed interface that `X` itself did not touch.
//! - The findings are `touched(A) ∩ touched(B)` — both edited it, which git
//!   will also find — and each side's drift suspects that land in the other
//!   side's edits, which it will not.
//!
//! The other root's store is opened **read-only** and never written: grid
//! surveys nothing there, and a store it cannot read is a refusal rather than
//! an answer of "no interference". Those are opposite answers, and only one
//! of them is available.

use std::collections::BTreeSet;
use std::path::Path;

use grid_chart::{Change, DatumId, Membership};
use grid_wire::{
    Datum, DefRef, DriftSuspect, DriftUnchecked, GitRev, InterfereReport, SnapshotId, SnapshotOrRev,
};

use crate::chart_provider::{self, ChartDelta};

/// Why two worktrees could not be compared.
#[derive(Debug)]
pub enum Error {
    /// A store could not be opened, holds no snapshot, is superseded, or does
    /// not hold the base. Bad input in every case, with the recovery named:
    /// silence about a store grid could not read is the one answer this verb
    /// must never give.
    Unreadable(String),
    /// Git could not name the two heads or their merge base.
    Git(String),
    /// One side's drift walk refused.
    Drift(crate::drift::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(why) | Self::Git(why) => f.write_str(why),
            Self::Drift(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

/// Compare this worktree with another against their common base.
///
/// # Errors
/// [`Error::Unreadable`] when either store cannot answer for the base,
/// [`Error::Git`] when the merge base cannot be found,
/// [`Error::Drift`] when a drift walk refused.
pub fn compute(root: &Path, other: &Path, base: Option<&str>) -> Result<InterfereReport, Error> {
    let other = other
        .canonicalize()
        .map_err(|error| Error::Unreadable(format!("{}: {error}", other.display())))?;
    if other == root.canonicalize().unwrap_or_else(|_| root.to_path_buf()) {
        return Err(Error::Unreadable(
            "a worktree does not interfere with itself".into(),
        ));
    }
    let base = match base {
        Some(rev) => rev.to_owned(),
        None => merge_base(root, &other)?,
    };

    // Mine first, and its head captured before anything is surveyed: an
    // on-demand base survey publishes a new live epoch, and reading `current`
    // afterwards would hand back base as if it were head.
    let mine_id = ChartDelta::open(root)
        .map_err(|error| unreadable(root, &error))?
        .current()
        .ok_or_else(|| unreadable_msg(root, "no snapshot published: run `grid survey` first"))?;
    // Base is surveyed on demand here and only here — this is grid's own
    // store, and a survey of a rev is idempotent.
    crate::rpc::Chart::new(root)
        .ensure(&SnapshotOrRev::GitRev {
            git_rev: GitRev(base.clone()),
        })
        .map_err(|error| unreadable(root, &error))?;

    let mine = ChartDelta::open(root).map_err(|error| unreadable(root, &error))?;
    let theirs = open_read_only(&other)?;
    let theirs_id = theirs
        .current()
        .ok_or_else(|| unreadable_msg(&other, "no snapshot published"))?;

    let my_base = snapshot_at(&mine, root, &base)?;
    let their_base = snapshot_at(&theirs, &other, &base)?;

    let my_touched = touched(&mine, root, &my_base, &mine_id)?;
    let their_touched = touched(&theirs, &other, &their_base, &theirs_id)?;

    let (my_drift, my_unchecked) = drift_of(root, &base, &mine_id)?;
    let (their_drift, their_unchecked) = drift_of(&other, &base, &theirs_id)?;

    let reason = format!("interfere:{}", other.display());
    let mut suspects: Vec<DriftSuspect> = my_drift
        .into_iter()
        .filter(|s| lands_in(&their_touched, &s.dependent))
        .chain(
            their_drift
                .into_iter()
                .filter(|s| lands_in(&my_touched, &s.dependent)),
        )
        .map(|mut s| {
            s.reason = Some(reason.clone());
            s
        })
        .collect();
    // Deterministic, and symmetric in the two roots: the same pair of stores
    // compared from either side must produce the same list.
    suspects.sort_by_key(key);
    suspects.dedup_by(|a, b| key(a) == key(b));

    Ok(InterfereReport {
        base: GitRev(base),
        mine: mine_id,
        theirs: theirs_id,
        both_touched: named(&mine, &my_base, &my_touched, &their_touched)?,
        suspects,
        unchecked: DriftUnchecked {
            unresolved_refs: None,
            unresolved_external: None,
            unresolved_method: None,
            unresolved_internal: None,
            budget_rejected: my_unchecked.saturating_add(their_unchecked),
            unattributed_entries: None,
            weak_skipped: None,
            compat_unchecked: None,
        },
    })
}

fn key(s: &DriftSuspect) -> (String, String, String) {
    let name = |d: &DefRef| {
        d.callsign
            .as_ref()
            .map_or_else(|| d.datum.0.clone(), |c| c.0.clone())
    };
    (
        name(&s.dependent),
        name(&s.changed),
        format!("{:?}", s.grade),
    )
}

fn lands_in(touched: &BTreeSet<DatumId>, dependent: &DefRef) -> bool {
    DatumId::parse(&dependent.datum.0).is_some_and(|datum| touched.contains(&datum))
}

/// The definitions both sides moved, named as base held them.
///
/// Named out of the *base* epoch: a datum both sides moved away from is by
/// construction not in either head, so the live name index has nothing to say
/// about it. Loaded only when the intersection is non-empty, which is the
/// common case of no conflict at all.
fn named(
    mine: &ChartDelta,
    base: &SnapshotId,
    a: &BTreeSet<DatumId>,
    b: &BTreeSet<DatumId>,
) -> Result<Vec<DefRef>, Error> {
    let both: Vec<DatumId> = a.intersection(b).copied().collect();
    if both.is_empty() {
        return Ok(Vec::new());
    }
    let epoch = mine
        .epoch_of(&SnapshotOrRev::Snapshot {
            snapshot: base.clone(),
        })
        .map_err(|error| Error::Unreadable(error.to_string()))?;
    let mut names: std::collections::HashMap<DatumId, (String, Option<Location>)> =
        std::collections::HashMap::new();
    for segment in epoch.segments() {
        for ord in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(ord);
            let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) else {
                continue;
            };
            if both.binary_search(&datum).is_ok() {
                let at = segment.location(ord).map(|loc| Location {
                    file: loc.path.to_owned(),
                    line: u64::from(loc.line),
                });
                names.insert(datum, (callsign.to_owned(), at));
            }
        }
    }
    let mut out: Vec<DefRef> = both
        .into_iter()
        .map(|datum| {
            let held = names.get(&datum);
            DefRef {
                datum: Datum(datum.to_string()),
                callsign: held.map(|(name, _)| grid_wire::Callsign(name.clone())),
                materialization: held.and_then(|(_, at)| at.clone()).map(|at| {
                    grid_wire::DefRefMaterialization {
                        file: at.file,
                        line: at.line,
                    }
                }),
                lang: None,
                signature: None,
                cfg: None,
            }
        })
        .collect();
    out.sort_by_key(|d| {
        d.callsign
            .as_ref()
            .map_or_else(|| d.datum.0.clone(), |c| c.0.clone())
    });
    Ok(out)
}

/// Where a base-side definition materialized, kept beside its name so the
/// base epoch is walked once.
#[derive(Clone)]
struct Location {
    file: String,
    line: u64,
}

/// Every definition whose datum moved between `base` and `head` in one store,
/// as base held it. One merge-diff over two sorted membership rows.
fn touched(
    chart: &ChartDelta,
    root: &Path,
    base: &SnapshotId,
    head: &SnapshotId,
) -> Result<BTreeSet<DatumId>, Error> {
    let membership = chart
        .membership()
        .map_err(|error| unreadable(root, &error))?
        .ok_or_else(|| {
            unreadable_msg(
                root,
                "no membership image beside the chart: re-run `grid survey`",
            )
        })?;
    changed(&membership, &base.0, &head.0)
        .map_err(|why| unreadable_msg(root, &why))
        .map(|changes| {
            changes
                .into_iter()
                .filter_map(|change| match change {
                    Change::Removed(datum) => Some(datum),
                    Change::Added(_) => None,
                })
                .collect()
        })
}

fn changed<B: AsRef<[u8]>>(
    membership: &Membership<B>,
    base: &str,
    head: &str,
) -> Result<Vec<Change>, String> {
    membership
        .changed_between(base, head)
        .map_err(|error| error.to_string())
}

/// One side's drift suspects and its budget refusals.
fn drift_of(root: &Path, base: &str, head: &SnapshotId) -> Result<(Vec<DriftSuspect>, u64), Error> {
    let base = SnapshotOrRev::GitRev {
        git_rev: GitRev(base.to_owned()),
    };
    let head = SnapshotOrRev::Snapshot {
        snapshot: head.clone(),
    };
    let report = crate::drift::compute(root, &crate::drift::Query::new(&base, &head))
        .map_err(Error::Drift)?;
    Ok((report.suspects, report.unchecked.budget_rejected))
}

/// The snapshot a store holds for `rev`, or a refusal naming the command that
/// would produce one. Never surveys: the other root is read-only, and being
/// silent about a store that lacks the base would report "no interference"
/// for a comparison that never happened.
fn snapshot_at(chart: &ChartDelta, root: &Path, rev: &str) -> Result<SnapshotId, Error> {
    chart
        .snapshot_of(&SnapshotOrRev::GitRev {
            git_rev: GitRev(rev.to_owned()),
        })
        .map_err(|_| {
            unreadable_msg(
                root,
                &format!("no snapshot for base {rev} — run `grid survey {rev}` there first"),
            )
        })
}

/// Open the other root's store without touching it.
///
/// The directory is checked first because opening a store creates it: a root
/// grid has never surveyed must come back as a refusal, not as a new empty
/// directory in someone else's worktree.
fn open_read_only(other: &Path) -> Result<ChartDelta, Error> {
    if !chart_provider::store_dir(other).is_dir() {
        return Err(unreadable_msg(
            other,
            "no chart there — run `grid survey` in that worktree first",
        ));
    }
    let chart = ChartDelta::open(other).map_err(|error| unreadable(other, &error))?;
    if let Some(epoch) = chart.superseded() {
        return Err(unreadable_msg(
            other,
            &format!(
                "chart written by segment format epoch {epoch} — superseded, \
                 so it opens empty; re-run `grid survey` there"
            ),
        ));
    }
    Ok(chart)
}

/// The common ancestor of the two worktrees' heads.
fn merge_base(root: &Path, other: &Path) -> Result<String, Error> {
    let theirs = git(other, &["rev-parse", "HEAD"])?;
    git(root, &["merge-base", "HEAD", &theirs])
}

fn git(dir: &Path, args: &[&str]) -> Result<String, Error> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|error| Error::Git(format!("git {}: {error}", args.join(" "))))?;
    if !out.status.success() {
        return Err(Error::Git(format!(
            "git {} in {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn unreadable(root: &Path, error: &dyn std::fmt::Display) -> Error {
    unreadable_msg(root, &error.to_string())
}

fn unreadable_msg(root: &Path, why: &str) -> Error {
    Error::Unreadable(format!("{}: {why}", root.display()))
}
