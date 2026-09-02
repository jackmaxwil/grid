//! grid-gate — spec gate: `Plan` (promised, name-space) vs `Diff`
//! (actual, datum-space) -> `GateVerdict`.
//!
//! Two spaces meet here. A plan is authored before the code exists, so it
//! references code by [`CallsignPattern`]; a `Diff` is a diff of surveyed
//! reality, so it references code by `Datum`. Unification is one step:
//! [`DeltaProvider::callsign`] names each datum, and the named edge is matched
//! against the promise patterns.
//!
//! Everything in this crate is pure: it computes over the values a provider
//! hands it and performs no I/O. The provider is the seam the real chart
//! (Wave 5) slides into behind a file-level stand-in.

pub mod lint;
pub mod pattern;

use grid_wire::{
    Callsign, CallsignPattern, Datum, DefChange, DefChangeAction, Diff, Edge, EdgeChange,
    EdgeChangeAction, EdgeRule, GateVerdict, GateVerdictGate, Grade, Invariant, InvariantRule,
    InvariantViolation, Path, Plan, PlanDefPromise, PlanDefPromiseAction, PlanEdgePromise,
    PlanEdgePromiseAction, SnapshotId, SnapshotOrRev, TestComparison, TestComparisonActual,
    TestComparisonExpected,
};

/// The seam. A provider answers two questions about a pair of snapshots: what
/// changed, and what each changed datum is called. Wave 3 ships a file-level
/// implementation; Wave 5 replaces it with the chart without touching this
/// crate.
pub trait DeltaProvider {
    /// Failure to produce a delta (a missing snapshot, an unreadable tree).
    type Error;

    /// The actual delta between two snapshots or revisions.
    ///
    /// # Errors
    /// Provider-defined: typically an unknown snapshot or a failed diff.
    fn diff(&self, base: &SnapshotOrRev, head: &SnapshotOrRev) -> Result<Diff, Self::Error>;

    /// The callsign a datum is known by, if the provider knows one. A datum
    /// with no name matches no promise pattern — it lands in `surplus`, which
    /// is the honest answer for code the provider cannot name.
    fn callsign(&self, datum: &Datum) -> Option<Callsign>;

    /// What a test actually did at head. Providers with no test report leave
    /// the default: every expected test is `missing`, which holds the gate.
    fn test_outcome(&self, test: &str) -> TestComparisonActual {
        let _ = test;
        TestComparisonActual::Missing
    }
}

/// Rank of an evidence grade, strongest first — `at_or_above` semantics.
const fn rank(z: Grade) -> u8 {
    match z {
        Grade::Proven => 0,
        Grade::Observed => 1,
        Grade::Modeled => 2,
        Grade::Heuristic => 3,
    }
}

/// Promise actions and change actions are the same alphabet in two tenses.
const fn tense(promise: PlanEdgePromiseAction) -> EdgeChangeAction {
    match promise {
        PlanEdgePromiseAction::Add => EdgeChangeAction::Added,
        PlanEdgePromiseAction::Remove => EdgeChangeAction::Removed,
        PlanEdgePromiseAction::Move => EdgeChangeAction::Moved,
    }
}

const fn def_tense(promise: PlanDefPromiseAction) -> grid_wire::DefChangeAction {
    use grid_wire::DefChangeAction as A;
    match promise {
        PlanDefPromiseAction::Add => A::Added,
        PlanDefPromiseAction::Remove => A::Removed,
        PlanDefPromiseAction::Modify => A::Modified,
        PlanDefPromiseAction::Move => A::Moved,
    }
}

/// An edge change with both endpoints named, so promise patterns can see it.
///
/// A name is `None` when the provider cannot name the datum. Patterns are a
/// name-space language: a hash is not a name, and glob-matching one would let
/// `**` silently make good a promise about code nobody can identify. An
/// unnamed endpoint therefore matches no pattern at all — the change lands in
/// `surplus`, which is the honest answer.
struct Named<'a> {
    change: &'a EdgeChange,
    src: Option<String>,
    dst: Option<String>,
}

impl Named<'_> {
    fn endpoints_match(&self, src: &CallsignPattern, dst: &CallsignPattern) -> bool {
        self.src
            .as_ref()
            .is_some_and(|s| pattern::matches(&src.0, s))
            && self
                .dst
                .as_ref()
                .is_some_and(|d| pattern::matches(&dst.0, d))
    }

    fn satisfies(&self, promise: &PlanEdgePromise) -> bool {
        self.change.action == tense(promise.action)
            && self.change.edge.relation == promise.relation
            && self.endpoints_match(&promise.src, &promise.dst)
    }

    fn matches_rule(&self, rule: &EdgeRule) -> bool {
        let floor = rule.at_or_above.unwrap_or(Grade::Heuristic);
        rule.relation.is_none_or(|r| r == self.change.edge.relation)
            && rank(self.change.edge.grade) <= rank(floor)
            && self.endpoints_match(&rule.src, &rule.dst)
    }
}

/// Check a plan against the delta a provider reports at `head`.
///
/// The verdict itemizes every divergence rather than reducing to a boolean:
/// `hold` is the summary, the arrays are the reason. Surplus is computed
/// after `tolerate` filtering (wire contract, `GateVerdict`).
///
/// # Errors
/// Propagates the provider's error when the delta cannot be produced.
pub fn check<P: DeltaProvider>(
    plan: &Plan,
    head: &SnapshotOrRev,
    provider: &P,
) -> Result<GateVerdict, P::Error> {
    let diff = provider.diff(&plan.base, head)?;
    Ok(verdict(plan, &diff, provider))
}

/// Def promises split by whether the `Diff` made them good.
///
/// The gate's `defs_unaccounted` answers the other direction (which changes
/// no promise covers); this is the promise-side view `grid.todo` renders, so
/// the worklist and the verdict are two readings of one judgment, never two
/// engines (CLI plan §5: `todo` is a *view* over the gate).
#[must_use]
pub fn def_progress(plan: &Plan, diff: &Diff) -> (Vec<PlanDefPromise>, Vec<PlanDefPromise>) {
    plan.delta
        .defs
        .iter()
        .cloned()
        .partition(|p| diff.defs.iter().any(|d| covers(p, d)))
}

/// The pure core: a verdict over a delta already in hand.
#[must_use]
pub fn verdict<P: DeltaProvider>(plan: &Plan, diff: &Diff, provider: &P) -> GateVerdict {
    let name = |d: &Datum| provider.callsign(d).map(|c| c.0);
    let named: Vec<Named<'_>> = diff
        .edges
        .iter()
        .map(|change| Named {
            change,
            src: name(&change.edge.src),
            dst: name(&change.edge.dst),
        })
        .collect();

    // Every promise, split by whether some change made it good.
    let (matched, missing): (Vec<_>, Vec<_>) = plan
        .delta
        .edges
        .iter()
        .cloned()
        .partition(|p| named.iter().any(|n| n.satisfies(p)));

    // Every change not accounted for by a promise, split by tolerance.
    let tolerate = plan.tolerate.as_deref().unwrap_or_default();
    let (tolerated, surplus): (Vec<_>, Vec<_>) = named
        .iter()
        .filter(|n| !plan.delta.edges.iter().any(|p| n.satisfies(p)))
        .map(|n| (n.change.clone(), tolerate.iter().any(|r| n.matches_rule(r))))
        .fold(
            (vec![], vec![]),
            |(mut tol, mut sur), (change, is_tolerated)| {
                if is_tolerated {
                    tol.push(change);
                } else {
                    sur.push(change);
                }
                (tol, sur)
            },
        );

    let defs_unaccounted: Vec<DefChange> = diff
        .defs
        .iter()
        .filter(|d| !plan.delta.defs.iter().any(|p| covers(p, d)))
        .cloned()
        .collect();

    // The subset of defs_unaccounted held specifically because a caller could
    // see the change and no promise permitted that: `covers` already refuses
    // to let an `interface: false` promise cover a sig move, so every
    // sig-moved member of defs_unaccounted qualifies by construction — a
    // filter, never a second unification pass, so nothing is held twice.
    // Only a definition that exists on both sides has an interface that can
    // move: an add or a remove carries `sig_changed: None`, and reading that
    // unknown as "moved" labelled every unpromised add `unaccounted:
    // interface`, which reads as a gate reason it is not.
    let interface_unaccounted: Vec<DefChange> = defs_unaccounted
        .iter()
        .filter(|d| {
            matches!(d.action, DefChangeAction::Modified | DefChangeAction::Moved)
                && !matches!(d.sig_changed, Some(false))
        })
        .cloned()
        .collect();

    let tests: Vec<TestComparison> = plan
        .tests
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|expected| {
            let actual = provider.test_outcome(&expected.test);
            TestComparison {
                test: expected.test.clone(),
                expected: expect_of(expected.expect),
                actual,
                ok: agrees(expected.expect, actual),
            }
        })
        .collect();

    let invariant_violations = violations(plan.invariants.as_deref().unwrap_or_default(), &named);

    let reasons = reasons(
        &missing,
        &surplus,
        &defs_unaccounted,
        &tests,
        &invariant_violations,
    );
    let gate = if reasons.is_empty() {
        GateVerdictGate::Pass
    } else {
        GateVerdictGate::Hold
    };

    GateVerdict {
        gate,
        plan_id: plan.plan_id.clone(),
        head: diff.head.clone(),
        matched,
        missing,
        surplus,
        tolerated,
        defs_unaccounted,
        tests,
        invariant_violations,
        reasons: Some(reasons),
        interface_unaccounted: Some(interface_unaccounted),
    }
}

/// A def promise covers a def change when the action agrees, the pattern
/// matches the callsign, and the promise's `interface` claim (if any) agrees
/// with what the sig actually did.
fn covers(promise: &PlanDefPromise, change: &DefChange) -> bool {
    change.action == def_tense(promise.action)
        && pattern::matches(&promise.callsign.0, &change.callsign.0)
        && interface_covers(promise.interface, change.sig_changed)
}

/// Does a promise's `interface` claim agree with an observed sig transition?
///
/// `None` is today's behavior: either transition makes the promise good.
/// `Some(true)` claims a caller can see the change, made good only by a sig
/// move — and an unknown transition (a pre-epoch-3 snapshot on either side)
/// is treated as a move, the conservative reading. `Some(false)` claims the
/// interface held, made good only by a sig that provably did not move —
/// unknown is *not* "held" here either, for the same reason: a promise this
/// specific about invisibility should not be made good by "nobody can say".
const fn interface_covers(interface: Option<bool>, sig_changed: Option<bool>) -> bool {
    match interface {
        None => true,
        Some(true) => !matches!(sig_changed, Some(false)),
        Some(false) => matches!(sig_changed, Some(false)),
    }
}

const fn expect_of(e: grid_wire::ExpectedTestExpect) -> TestComparisonExpected {
    use grid_wire::ExpectedTestExpect as E;
    match e {
        E::Pass => TestComparisonExpected::Pass,
        E::Fail => TestComparisonExpected::Fail,
        E::Skip => TestComparisonExpected::Skip,
    }
}

const fn agrees(expected: grid_wire::ExpectedTestExpect, actual: TestComparisonActual) -> bool {
    use grid_wire::ExpectedTestExpect as E;
    use TestComparisonActual as A;
    matches!(
        (expected, actual),
        (E::Pass, A::Pass) | (E::Fail, A::Fail) | (E::Skip, A::Skip)
    )
}

/// Standing invariants, checked against the edges this delta added.
///
/// The witness is the single hop that violates the rule — the concrete
/// evidence chain, one edge long, which is all a file-level delta can prove.
/// Multi-hop witnesses arrive with the walk engine (Wave 9).
fn violations(invariants: &[Invariant], named: &[Named<'_>]) -> Vec<InvariantViolation> {
    let mut out = Vec::new();
    for inv in invariants {
        match &inv.rule {
            InvariantRule::ForbidEdge { forbid_edge } => {
                for n in named
                    .iter()
                    .filter(|n| n.change.action == EdgeChangeAction::Added)
                    .filter(|n| n.matches_rule(forbid_edge))
                {
                    out.push(InvariantViolation {
                        id: inv.id.clone(),
                        witness: witness(&n.change.edge),
                    });
                }
            }
            // A require_edge invariant is a standing query over the whole
            // chart, not over a delta: a delta that does not remove the
            // required edge says nothing about whether it exists. Only a
            // removal that matches is a violation this crate can witness.
            InvariantRule::RequireEdge { require_edge } => {
                for n in named
                    .iter()
                    .filter(|n| n.change.action == EdgeChangeAction::Removed)
                    .filter(|n| n.matches_rule(require_edge))
                {
                    out.push(InvariantViolation {
                        id: inv.id.clone(),
                        witness: witness(&n.change.edge),
                    });
                }
            }
            // A `persisted` invariant is a claim about bytes on disk, not
            // about edges. Nothing in a structural delta can witness it; the
            // Deserialize compatibility pass is what checks it, and it says
            // so where it runs rather than passing silently here.
            InvariantRule::Persisted { .. } => {}
        }
    }
    out
}

fn witness(edge: &Edge) -> Path {
    Path {
        nodes: vec![edge.src.clone(), edge.dst.clone()],
        edges: vec![edge.clone()],
    }
}

/// One human-readable line per contributing hold cause; empty on pass. This
/// list *is* the gate decision: `hold` iff it is non-empty.
fn reasons(
    missing: &[PlanEdgePromise],
    surplus: &[EdgeChange],
    defs_unaccounted: &[DefChange],
    tests: &[TestComparison],
    violations: &[InvariantViolation],
) -> Vec<String> {
    let mut out = Vec::new();
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            one.to_string()
        } else {
            many.to_string()
        }
    };
    if !missing.is_empty() {
        out.push(format!(
            "{} promised {} never built",
            missing.len(),
            plural(missing.len(), "edge", "edges")
        ));
    }
    if !surplus.is_empty() {
        out.push(format!(
            "{} unpromised {} built",
            surplus.len(),
            plural(surplus.len(), "edge", "edges")
        ));
    }
    if !defs_unaccounted.is_empty() {
        out.push(format!(
            "{} changed {} covered by no promise",
            defs_unaccounted.len(),
            plural(defs_unaccounted.len(), "definition", "definitions")
        ));
    }
    let failed = tests.iter().filter(|t| !t.ok).count();
    if failed > 0 {
        out.push(format!(
            "{failed} expected {} did not agree",
            plural(failed, "test", "tests")
        ));
    }
    for v in violations {
        out.push(format!("invariant {} violated", v.id));
    }
    out
}

/// `SnapshotOrRev` displayed the way porcelain shows it.
#[must_use]
pub fn snapshot_display(id: &SnapshotId) -> String {
    // snap:b3:<64 hex> -> @<7 hex>, the surveyed rev's short form.
    id.0.rsplit(':').next().map_or_else(
        || id.0.clone(),
        |hex| format!("@{}", &hex[..hex.len().min(7)]),
    )
}
