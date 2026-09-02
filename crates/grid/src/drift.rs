//! `grid drift` — dependents of changed interfaces.
//!
//! The compiler already catches a proven-typed Rust caller of a changed
//! signature. Drift's value is exactly where the compiler is blind:
//! cross-language edges, re-export hops, Python callers of anything, and
//! (later steps) runtime and declared coupling. What it names is the set
//! "dependents that did **not** change" — the ones nobody has even promised
//! to touch.
//!
//! Precisely, over a base and a head snapshot:
//!
//! - The changed set `C` is every definition in `Diff(base, head)` that
//!   was `Removed`, or `Modified` with `sig_changed` `Some(true)` **or**
//!   `None`. Unknown counts as changed: a snapshot from before segment epoch
//!   3 carries no interface identity, and treating that silence as "held"
//!   would drop every dependent in it.
//! - For each `d ∈ C`, the inbound edges *at base* are walked — dependents
//!   were depending on the old interface, so the old graph is the one that
//!   knows about them — filtered to the dependency relations and to the
//!   query's evidence floor.
//! - A dependent `c` is a suspect when `c`'s own datum did not move between
//!   base and head. A dependent that changed at all is assumed handled;
//!   whether it changed *correctly* is the gate's question, not drift's.
//!
//! Observed coupling joins the walk once the floor admits it. A trace's edges
//! are not in the chart — the chart holds what the language proves — so they
//! are read from the trace store and graded by [`grid_trace::grade`] against
//! the *base* chart, the one the dependents were depending on. An edge whose
//! endpoints are both still interned there is `observed`; one whose trace
//! describes code that base no longer holds is `heuristic` with reason
//! `stale-trace`, which puts it below every floor but the weakest. A decayed
//! observation left at `observed` would rot drift into false positives
//! within weeks, which is the whole reason the grade is recomputed per query
//! rather than stored.
//!
//! Declared coupling joins it too. A covenant ([`crate::covenant`]) is a
//! human writing down a pair the language cannot establish — a serde
//! boundary, a JSON contract — and it fires on *datum* change, because the
//! coupling is semantic and a normalizer's body is the contract. Covenant
//! suspects are `modeled` and they gate, which is the one exception to
//! "modeled never gates": a human declared the pair, so the human meant it.
//!
//! Refusals ride with the answer. A drift report that says "no suspects"
//! where it could not see callers is dishonest silence, so
//! [`grid_wire::DriftUnchecked`] carries what was not checked and the
//! porcelain prints it.
use std::collections::{BTreeMap, HashMap, HashSet};

use grid_chart::{DatumId, Ordinal, Segment};
use grid_wire::{
    Callsign, Covenant, Datum, DefChange, DefChangeAction, DefRef, Diff, DriftReport, DriftSuspect,
    DriftUnchecked, Grade, Plan, PlanDefPromiseAction, Relation, SnapshotId, SnapshotOrRev,
};

use crate::chart_provider::{self, ChartDelta};

/// Relations through which a dependent can be broken by an interface change.
/// A `ReadsField` or a `DispatchesTo` is not here because the survey does not
/// prove them; adding one is a decision about evidence, not a widening of a
/// filter.
const DEPENDENCY_RELATIONS: [Relation; 4] = [
    Relation::Calls,
    Relation::Constructs,
    Relation::Implements,
    Relation::WritesField,
];

/// Fuel for one changed definition's inbound walk, borrowed from the cache
/// crate so the two budgets cannot drift apart. Over budget the walk is
/// *rejected*, never truncated: a truncated walk is indistinguishable from a
/// smaller one, and drift would then report fewer suspects and look cleaner
/// for it.
pub(crate) const FUEL: u32 = grid_cache::FUEL;

/// The relation a covenant suspect carries. A covenant is a declared
/// coupling across a serialization boundary — the serde struct and the dict
/// that feeds it — so `serializes_to` is the honest name for it. The `via`
/// word in the reason says which boundary.
const COVENANT_RELATION: Relation = Relation::SerializesTo;

/// Why drift could not answer.
#[derive(Debug)]
pub enum Error {
    /// An endpoint did not resolve, or the store could not be read.
    Chart(chart_provider::Error),
    /// One or more inbound walks ran past [`FUEL`]. The answer is refused
    /// whole rather than served short.
    BudgetRejected(u32),
    /// The test-staleness enumeration could not read the trace store or the
    /// verdict log. Not an empty answer: a log that fails to open has
    /// unknown contents.
    Stale(String),
    /// The covenant declaration file exists but does not parse. A broken
    /// declaration must never read as an absence of declarations.
    Covenants(String),
    /// The retained traces could not be read. Not an empty observation set:
    /// a store that fails to open has unknown contents, and answering
    /// "nothing was observed" for it would be a fabricated all-clear.
    Trace(grid_trace::StoreError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chart(e) => write!(f, "{e}"),
            Self::Trace(e) => write!(f, "{e}"),
            Self::Covenants(msg) | Self::Stale(msg) => f.write_str(msg),
            Self::BudgetRejected(n) => write!(
                f,
                "{n} inbound walk(s) exceeded fuel ({FUEL}) — refused whole, \
                 because a truncated walk would report fewer suspects and look clean"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<chart_provider::Error> for Error {
    fn from(e: chart_provider::Error) -> Self {
        Self::Chart(e)
    }
}

/// Evidence rank, strongest first — the `at_or_above` order the rest of the
/// engine uses.
pub(crate) const fn rank(z: Grade) -> u8 {
    match z {
        Grade::Proven => 0,
        Grade::Observed => 1,
        Grade::Modeled => 2,
        Grade::Heuristic => 3,
    }
}

/// One changed definition, as drift needs it: the base-side datum to walk
/// inbound from, and the interface transition to report.
pub(crate) struct Changed {
    pub(crate) callsign: String,
    pub(crate) datum: DatumId,
    pub(crate) sig_before: Option<Datum>,
    pub(crate) sig_after: Option<Datum>,
}

/// The changed set `C`. `Modified` with `sig_changed: Some(false)` is
/// excluded — the body moved and no caller can see it — and everything else
/// with a base-side datum is in.
fn changed_set(diff: &Diff) -> Vec<&DefChange> {
    diff.defs
        .iter()
        .filter(|d| match d.action {
            DefChangeAction::Removed => true,
            // `None` is unknown, and unknown is treated as changed.
            DefChangeAction::Modified => d.sig_changed != Some(false),
            // An addition has no old interface to have depended on, and a
            // move is outside the plan's changed-set definition.
            DefChangeAction::Added | DefChangeAction::Moved => false,
        })
        .collect()
}

/// Every callsign the delta reports as changed in any way. A dependent whose
/// callsign is in here changed too, so it is not a suspect.
pub(crate) fn touched(diff: &Diff) -> HashSet<&str> {
    diff.defs.iter().map(|d| d.callsign.0.as_str()).collect()
}

/// The sigs of a live set, by callsign — read once so the report can state
/// the interface transition without a second pass per suspect.
fn sigs_by_callsign<B: AsRef<[u8]>>(segments: &[Segment<B>]) -> HashMap<String, [u8; 32]> {
    let mut out = HashMap::new();
    for segment in segments {
        for ord in 0..segment.def_count() {
            let ord = Ordinal(ord);
            if let (Some(callsign), Some(sig)) = (segment.callsign(ord), segment.sig(ord)) {
                out.insert(callsign.to_owned(), sig);
            }
        }
    }
    out
}

fn wire_datum(id: DatumId) -> Datum {
    Datum(id.to_string())
}

fn sig_datum(sig: Option<&[u8; 32]>) -> Option<Datum> {
    sig.map(|s| wire_datum(DatumId(*s)))
}

/// Does a plan promise that this callsign will be touched?
///
/// A promise to `Modify` or `Remove` is the human saying "I will get to it",
/// and `grid gate` already holds when a promise is not made good. Drift lists
/// what nobody has even promised; the gate judges promises. `Add` does not
/// suppress: promising to add something says nothing about an existing
/// dependent.
fn promised(plan: Option<&Plan>, callsign: &str) -> bool {
    plan.is_some_and(|t| {
        t.delta.defs.iter().any(|p| {
            matches!(
                p.action,
                PlanDefPromiseAction::Modify | PlanDefPromiseAction::Remove
            ) && grid_gate::pattern::matches(&p.callsign.0, callsign)
        })
    })
}

/// One drift question, whole. Bundled rather than passed as six arguments
/// because every field is part of *what was asked*, and a report is only
/// interpretable against all of them.
pub struct Query<'a> {
    /// The snapshot dependents were depending on.
    pub base: &'a SnapshotOrRev,
    /// The snapshot they may now be stale against.
    pub head: &'a SnapshotOrRev,
    /// Weakest evidence an edge may carry and still implicate a dependent.
    pub floor: Grade,
    /// A promise file, if one was named: a suspect the plan promises to
    /// modify or remove is suppressed, because the gate already judges it.
    pub plan: Option<&'a Plan>,
    /// The repo's declared couplings. Empty is a repo that has declared
    /// none, which is not the same as one whose declaration file is
    /// unreadable — that is the caller's error to raise.
    pub covenants: &'a [Covenant],
    /// The head survey's count of references it declined to bind, split by
    /// why, or `None` when nobody surveyed head in this run — unknown, never
    /// zero.
    pub declined: Option<Declined>,
    /// Per-changed-definition inbound-walk budget. A real parameter, not a
    /// constant, because the honest thing to do when it runs out is only
    /// testable if a test can make it run out.
    pub fuel: u32,
    /// The repo's standing invariants when the caller asked for the
    /// Deserialize compatibility pass, `None` when it did not. `Some(&[])` is
    /// a repo that declared nothing persisted, which still checks — the
    /// findings are just heuristic.
    pub compat: Option<&'a [grid_wire::Invariant]>,
}

/// What a survey declined to bind, split by why it declined.
///
/// One number for three populations was the honesty gap: a path into `std`
/// costs a claim about this repo nothing, a call through an untyped receiver
/// is a hole no static rule closes, and a name the tree could have held is a
/// hole in exactly what drift claims. The total is the sum, so the split can
/// never disagree with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Declined {
    /// Names that were never this tree's to hold.
    pub external: u64,
    /// Calls through a receiver whose type would have to be known.
    pub method: u64,
    /// Names this tree could have held and the resolver could not bind.
    pub internal: u64,
}

impl Declined {
    /// Every declined reference, however it was declined.
    #[must_use]
    pub fn total(self) -> u64 {
        self.external
            .saturating_add(self.method)
            .saturating_add(self.internal)
    }
}

impl<'a> Query<'a> {
    /// The everyday question: everything grid can stand behind — proven
    /// edges and live observations — with no plan, no covenants, and the
    /// default budget.
    #[must_use]
    pub fn new(base: &'a SnapshotOrRev, head: &'a SnapshotOrRev) -> Self {
        Self {
            base,
            head,
            floor: Grade::Observed,
            plan: None,
            covenants: &[],
            declined: None,
            fuel: FUEL,
            compat: None,
        }
    }
}

/// Compute drift between two endpoints.
///
/// # Errors
/// [`Error::Chart`] when an endpoint does not resolve;
/// [`Error::BudgetRejected`] when an inbound walk ran past the query's fuel.
pub fn compute(root: &std::path::Path, query: &Query) -> Result<DriftReport, Error> {
    let Query {
        base,
        head,
        floor,
        plan,
        covenants,
        declined,
        fuel: budget,
        compat,
    } = *query;
    let chart = ChartDelta::open(root)?;
    let (base_epoch, head_epoch) = (chart.epoch_of(base)?, chart.epoch_of(head)?);
    let diff = grid_chart::diff(
        SnapshotId(base_epoch.snapshot_id().as_str().to_owned()),
        base_epoch.segments(),
        SnapshotId(head_epoch.snapshot_id().as_str().to_owned()),
        head_epoch.segments(),
    );

    let touched = touched(&diff);
    let changed = changed_set_of(&diff, base_epoch.segments(), head_epoch.segments());

    // Observed coupling, graded against base. Loaded once: the store is a
    // directory read, and doing it per changed definition would make the
    // cost quadratic in the size of the delta.
    let observed = if rank(floor) >= rank(Grade::Observed) {
        observed_inbound(root, base_epoch.segments())?
    } else {
        HashMap::new()
    };

    // Keyed so one dependent implicated by two changed definitions is two
    // rows, and so the output is a function of content rather than of walk
    // order (the determinism property).
    let mut suspects: BTreeMap<(String, String, u8), DriftSuspect> = BTreeMap::new();
    let mut budget_rejected = 0u32;

    for change in &changed {
        let Some(found) = inbound_of(base_epoch.segments(), change, floor, &observed, budget)
        else {
            budget_rejected = budget_rejected.saturating_add(1);
            continue;
        };
        fold_rows(&chart, change, found, &touched, plan, &mut suspects);
    }

    if budget_rejected > 0 {
        return Err(Error::BudgetRejected(budget_rejected));
    }

    // Declared coupling, joined last: a covenant fires on datum change, so
    // it needs the same `touched` set the walk used and nothing else.
    let warnings = fire_covenants(
        covenants,
        &chart,
        base_epoch.segments(),
        head_epoch.segments(),
        &touched,
        &mut suspects,
    );

    // Deserialize compatibility over the same changed set. Last, because it
    // reads source rather than the chart and must not be able to cost the
    // suspect walk anything.
    let compat = compat.map(|invariants| {
        crate::compat::check(
            root,
            &changed,
            base_epoch.segments(),
            head_epoch.segments(),
            &base_epoch.manifest().git_rev,
            // A dirty worktree snapshot read its bytes from the working tree,
            // which is where they still are; anything else is the tree at a
            // rev the object store can serve.
            match head_epoch.manifest().worktree_diff_hash {
                Some(_) => None,
                None => Some(head_epoch.manifest().git_rev.as_str()),
            },
            invariants,
        )
    });

    Ok(DriftReport {
        base: SnapshotId(base_epoch.snapshot_id().as_str().to_owned()),
        head: SnapshotId(head_epoch.snapshot_id().as_str().to_owned()),
        suspects: suspects.into_values().collect(),
        unchecked: DriftUnchecked {
            unresolved_refs: declined.map(Declined::total),
            unresolved_external: declined.map(|d| d.external),
            unresolved_method: declined.map(|d| d.method),
            unresolved_internal: declined.map(|d| d.internal),
            budget_rejected: u64::from(budget_rejected),
            unattributed_entries: None,
            weak_skipped: None,
            compat_unchecked: compat.as_ref().map(|found| found.unchecked),
        },
        warnings: (!warnings.is_empty()).then_some(warnings),
        // Filled in by the caller when the report was asked for tests: both
        // are different questions over different stores.
        stale_tests: None,
        weak: None,
        compat: compat.map(|found| found.findings),
    })
}

/// Join the repo's declared couplings into the suspect set.
///
/// Returns the warnings: a pattern that matches nothing at drift time is a
/// declaration that stopped being checked, and a rule that quietly stops
/// being checked is worse than no rule.
fn fire_covenants<B: AsRef<[u8]>>(
    covenants: &[Covenant],
    chart: &ChartDelta,
    segments: &[Segment<B>],
    head: &[Segment<B>],
    touched: &HashSet<&str>,
    suspects: &mut BTreeMap<(String, String, u8), DriftSuspect>,
) -> Vec<grid_wire::DriftWarning> {
    let names = base_callsigns(segments);
    let all: Vec<&str> = names.iter().map(String::as_str).collect();
    let head_names = base_callsigns(head);
    let live: Vec<&str> = head_names.iter().map(String::as_str).collect();
    let touched_names: Vec<&str> = touched.iter().copied().collect();
    let (fired, warnings) = crate::covenant::fire(covenants, &all, &live, &touched_names);
    for hit in fired {
        let Some(datum) = datum_of(segments, &hit.held) else {
            continue;
        };
        let key = (
            hit.held.clone(),
            hit.moved.clone(),
            grid_chart::relation_tag(COVENANT_RELATION),
        );
        suspects.entry(key).or_insert_with(|| DriftSuspect {
            dependent: def_ref(chart, datum, Some(&hit.held)),
            changed: datum_of(segments, &hit.moved).map_or_else(
                // The partner may have been removed outright, and then base
                // has no datum for it: name it and say nothing more.
                || DefRef {
                    datum: Datum(String::new()),
                    callsign: Some(Callsign(hit.moved.clone())),
                    lang: None,
                    signature: None,
                    cfg: None,
                    materialization: None,
                },
                |d| def_ref(chart, d, Some(&hit.moved)),
            ),
            relation: COVENANT_RELATION,
            grade: Grade::Modeled,
            // A covenant fires on the body, not the interface, so there is
            // no sig transition to show — and showing one would suggest the
            // wrong reason for the finding.
            sig_before: None,
            sig_after: None,
            reason: Some(hit.reason),
        });
    }
    warnings
}

/// Every callsign a live set names, sorted, so covenant matching is a
/// function of content rather than of segment order. Used for both endpoints
/// of the covenant join: base decides what fired, head decides what is still
/// being checked.
fn base_callsigns<B: AsRef<[u8]>>(segments: &[Segment<B>]) -> Vec<String> {
    let mut out: Vec<String> = segments
        .iter()
        .flat_map(|s| {
            (0..s.def_count()).filter_map(move |ord| s.callsign(Ordinal(ord)).map(str::to_owned))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The datum a callsign answers to in a live set.
fn datum_of<B: AsRef<[u8]>>(segments: &[Segment<B>], callsign: &str) -> Option<DatumId> {
    segments.iter().find_map(|s| {
        (0..s.def_count()).find_map(|ord| {
            let ord = Ordinal(ord);
            (s.callsign(ord) == Some(callsign)).then(|| s.datum(ord))?
        })
    })
}

/// The dependents of one changed definition, from every source of evidence
/// the floor admits: proven edges in the base chart, then traced callers.
///
/// `Ok(None)` is the budget refusal — the *whole* answer for this definition
/// is withheld, because a partial list is the one output that reads as
/// complete while being short.
pub(crate) fn inbound_of<B: AsRef<[u8]>>(
    segments: &[Segment<B>],
    change: &Changed,
    floor: Grade,
    observed: &HashMap<DatumId, Vec<grid_trace::Graded>>,
    mut fuel: u32,
) -> Option<Vec<Inbound>> {
    let mut found = Vec::new();
    for segment in segments {
        let Some(ord) = segment.ordinal(&change.datum) else {
            continue;
        };
        for edge in segment.in_edges(ord) {
            fuel = fuel.checked_sub(1)?;
            if !DEPENDENCY_RELATIONS.contains(&edge.relation) || rank(edge.grade) > rank(floor) {
                continue;
            }
            let (Some(datum), Some(callsign)) =
                (segment.datum(edge.other), segment.callsign(edge.other))
            else {
                continue;
            };
            found.push(Inbound {
                datum,
                callsign: Some(callsign.to_owned()),
                relation: edge.relation,
                grade: edge.grade,
                reason: None,
            });
        }
    }
    // A traced caller of this definition is a dependent the language never
    // proved. It is charged no fuel: the store was read once, and the budget
    // bounds the graph walk, which is the unbounded part.
    for graded in observed.get(&change.datum).into_iter().flatten() {
        if rank(graded.grade) > rank(floor) {
            continue;
        }
        found.push(Inbound {
            datum: graded.edge.caller,
            callsign: name_at(segments, graded.edge.caller),
            relation: Relation::Calls,
            grade: graded.grade,
            reason: graded.reason.map(str::to_owned),
        });
    }
    Some(found)
}

/// The changed set, with each definition's base-side datum and the interface
/// transition to report. Split out of [`compute`] so the one place that
/// decides *what counts as changed* is readable on its own.
pub(crate) fn changed_set_of<A: AsRef<[u8]>, B: AsRef<[u8]>>(
    diff: &Diff,
    base: &[Segment<A>],
    head: &[Segment<B>],
) -> Vec<Changed> {
    let base_sigs = sigs_by_callsign(base);
    let head_sigs = sigs_by_callsign(head);
    changed_set(diff)
        .into_iter()
        .filter_map(|d| {
            let datum = DatumId::parse(&d.before.as_ref()?.0)?;
            Some(Changed {
                callsign: d.callsign.0.clone(),
                datum,
                sig_before: sig_datum(base_sigs.get(&d.callsign.0)),
                sig_after: sig_datum(head_sigs.get(&d.callsign.0)),
            })
        })
        .collect()
}

/// Fold one changed definition's inbound dependents into the suspect map.
///
/// Split out of [`compute`] because watch mode calls it for exactly the
/// definitions an event moved, and the two must build the same rows — a
/// second implementation of "what a suspect is" is the one thing an
/// incremental view cannot afford.
pub(crate) fn fold_rows(
    chart: &ChartDelta,
    change: &Changed,
    found: Vec<Inbound>,
    touched: &HashSet<&str>,
    plan: Option<&Plan>,
    suspects: &mut BTreeMap<(String, String, u8), DriftSuspect>,
) {
    for dep in found {
        // A dependent base cannot name changed nothing between base and head
        // *under a name*, so neither check can implicate it — and neither can
        // excuse it.
        if dep
            .callsign
            .as_deref()
            .is_some_and(|c| touched.contains(c) || promised(plan, c))
        {
            continue;
        }
        let key = (
            dep.key(),
            change.callsign.clone(),
            grid_chart::relation_tag(dep.relation),
        );
        // Strongest evidence wins a duplicate: a pair proven *and* observed is
        // proven, and reporting it twice at two grades would read as two
        // findings.
        match suspects.entry(key) {
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                if rank(dep.grade) < rank(slot.get().grade) {
                    slot.get_mut().grade = dep.grade;
                    slot.get_mut().reason = dep.reason;
                }
            }
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(DriftSuspect {
                    dependent: def_ref(chart, dep.datum, dep.callsign.as_deref()),
                    changed: def_ref(chart, change.datum, Some(&change.callsign)),
                    relation: dep.relation,
                    grade: dep.grade,
                    sig_before: change.sig_before.clone(),
                    sig_after: change.sig_after.clone(),
                    reason: dep.reason,
                });
            }
        }
    }
}

/// One inbound dependency found for a changed definition, whatever proved it.
pub(crate) struct Inbound {
    pub(crate) datum: DatumId,
    /// The name base knows this dependent by, or `None` when base does not
    /// intern it at all — which happens exactly on a decayed trace, where
    /// the caller was edited between the trace and base. Reported by datum
    /// handle rather than dropped: "something that was observed calling this
    /// has since been rewritten" is a real, weak answer, and dropping it
    /// would be the silence the report shape exists to prevent.
    pub(crate) callsign: Option<String>,
    pub(crate) relation: Relation,
    pub(crate) grade: Grade,
    pub(crate) reason: Option<String>,
}

impl Inbound {
    /// The key this dependent is deduplicated and suppressed under: its name
    /// where base has one, its identity where base does not.
    fn key(&self) -> String {
        self.callsign
            .clone()
            .unwrap_or_else(|| self.datum.to_string())
    }
}

/// Observed edges from the retained traces, graded against `segments` and
/// indexed by the definition they point *at* — the shape the inbound walk
/// asks in.
///
/// A store that cannot be read is an error, never an empty map: answering
/// "nothing was observed" for a store that has observations is the silence
/// this whole report shape exists to prevent.
pub(crate) fn observed_inbound<B: AsRef<[u8]>>(
    root: &std::path::Path,
    segments: &[Segment<B>],
) -> Result<HashMap<DatumId, Vec<grid_trace::Graded>>, Error> {
    let store = crate::rpc::trace_dir(root)
        .and_then(grid_trace::Store::open)
        .map_err(Error::Trace)?;
    let mut out: HashMap<DatumId, Vec<grid_trace::Graded>> = HashMap::new();
    for stored in store.retained().map_err(Error::Trace)? {
        for edge in stored.edges() {
            let graded = grid_trace::grade(segments, edge);
            out.entry(graded.edge.callee).or_default().push(graded);
        }
    }
    // Deterministic: a store's directory order is not a contract.
    for edges in out.values_mut() {
        edges.sort_by_key(|g| (g.edge.caller, rank(g.grade)));
        edges.dedup_by_key(|g| g.edge.caller);
    }
    Ok(out)
}

/// The callsign a datum answers to in this live set, if it is interned here.
/// A demoted edge often names a datum base no longer holds; such a dependent
/// cannot be named, and an unnameable suspect is not reported at all.
fn name_at<B: AsRef<[u8]>>(segments: &[Segment<B>], datum: DatumId) -> Option<String> {
    segments.iter().find_map(|s| {
        s.ordinal(&datum)
            .and_then(|ord| s.callsign(ord))
            .map(str::to_owned)
    })
}

/// A `DefRef` with the location the live chart knows, when it knows one. The
/// row is meant to be pasteable into an editor, so `file:line` matters more
/// here than anywhere else in the report.
pub(crate) fn def_ref(chart: &ChartDelta, datum: DatumId, callsign: Option<&str>) -> DefRef {
    let wire = wire_datum(datum);
    let materialization = chart.locate(&wire);
    DefRef {
        datum: wire,
        callsign: callsign.map(|c| Callsign(c.to_owned())),
        lang: None,
        signature: None,
        cfg: None,
        materialization,
    }
}

#[cfg(test)]
mod tests {
    //! The refusals, tested where they can be made to happen: a fuel bound
    //! small enough to run out is a parameter of the query, not a constant
    //! nobody can reach.
    use super::{compute, Error, Query};
    use grid_chart::{build, DatumId, Def, DefFacts, Edge};
    use grid_wire::{GitRev, Grade, Relation, SnapshotOrRev};

    fn def(callsign: &str, datum: u8, sig: u8) -> Def {
        Def {
            datum: DatumId([datum; 32]),
            callsign: callsign.to_owned(),
            lang: Some(grid_wire::Lang::Rs),
            facts: Some(DefFacts {
                path: "src/lib.rs".into(),
                line: 1,
                line_end: 1,
                span: (0, 1),
                shape: [datum; 32],
                complexity: 1,
                sig: [sig; 32],
                cfg: String::new(),
            }),
        }
    }

    fn calls(src: u8, dst: u8) -> Edge {
        Edge {
            src: DatumId([src; 32]),
            dst: DatumId([dst; 32]),
            relation: Relation::Calls,
            grade: Grade::Proven,
        }
    }

    /// A store with two published snapshots: `callee` changes its interface
    /// between them, `caller` does not change at all.
    fn seeded() -> (tempfile::TempDir, SnapshotOrRev, SnapshotOrRev) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = grid_chart_io::Store::open(crate::chart_provider::store_dir(dir.path()))
            .expect("store");
        // Two callers, so a *truncated* walk would find one of them and a
        // refused walk finds neither — the difference the test below is for.
        let base = build(
            &[
                def("m.caller", 1, 11),
                def("m.other", 4, 44),
                def("m.callee", 2, 22),
            ],
            &[calls(1, 2), calls(4, 2)],
        )
        .expect("base image");
        let head = build(
            &[
                def("m.caller", 1, 11),
                def("m.other", 4, 44),
                def("m.callee", 3, 33),
            ],
            &[calls(1, 3), calls(4, 3)],
        )
        .expect("head image");
        store.publish("base", None, &[base]).expect("publish base");
        store.publish("head", None, &[head]).expect("publish head");
        let rev = |r: &str| SnapshotOrRev::GitRev {
            git_rev: GitRev(r.to_owned()),
        };
        (dir, rev("base"), rev("head"))
    }

    #[test]
    fn a_changed_interface_implicates_its_unchanged_caller() {
        let (dir, base, head) = seeded();
        let report = compute(dir.path(), &Query::new(&base, &head)).expect("drift answers");
        let names: Vec<_> = report
            .suspects
            .iter()
            .map(|s| {
                (
                    s.dependent.callsign.clone().expect("named").0,
                    s.changed.callsign.clone().expect("named").0,
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("m.caller".to_owned(), "m.callee".to_owned()),
                ("m.other".to_owned(), "m.callee".to_owned()),
            ]
        );
        assert_eq!(report.unchecked.budget_rejected, 0);
    }

    /// Conservatism, the load-bearing one: a walk that runs past its fuel
    /// refuses the whole answer. A truncated walk would report *fewer*
    /// suspects and look cleaner for it, which is the one failure mode a
    /// drift verb cannot afford.
    #[test]
    fn a_walk_over_budget_refuses_rather_than_truncates() {
        let (dir, base, head) = seeded();
        // Fuel for exactly one of the two inbound edges: enough to find a
        // suspect, not enough to find them all. A truncating implementation
        // answers `Ok` with one suspect and looks clean; this must not.
        let query = Query {
            fuel: 1,
            ..Query::new(&base, &head)
        };
        match compute(dir.path(), &query) {
            Err(Error::BudgetRejected(n)) => assert_eq!(n, 1),
            Err(e) => panic!("wrong refusal: {e}"),
            Ok(report) => panic!(
                "a budget-exhausted walk must refuse, not answer with {} suspects",
                report.suspects.len()
            ),
        }
    }

    /// The refusal names its cause. A rejection that printed as a clean
    /// "no suspects" would be the dishonest silence the whole report shape
    /// exists to prevent.
    #[test]
    fn the_refusal_says_what_it_refused() {
        let text = Error::BudgetRejected(3).to_string();
        assert!(text.contains('3'), "{text}");
        assert!(text.contains("refused whole"), "{text}");
    }
}
