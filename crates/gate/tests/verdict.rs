//! Gate semantics: what makes a verdict hold, and what must never make it.
//!
//! The wire contract states the rule the whole crate exists to implement:
//! "gate is 'hold' iff missing, surplus, `defs_unaccounted`, a failed
//! `TestComparison`, or an invariant violation is non-empty". These tests hold
//! that sentence to its word from both sides.

use grid_gate::{verdict, DeltaProvider};
use grid_wire::*;
use proptest::prelude::*;
use std::collections::BTreeMap;

/// A provider that answers from tables: no git, no chart, no I/O.
struct Table {
    names: BTreeMap<String, String>,
    tests: BTreeMap<String, TestComparisonActual>,
}

impl Table {
    fn new() -> Self {
        Self {
            names: BTreeMap::new(),
            tests: BTreeMap::new(),
        }
    }

    fn naming(mut self, datum: &str, callsign: &str) -> Self {
        self.names.insert(datum.into(), callsign.into());
        self
    }

    fn ran(mut self, test: &str, actual: TestComparisonActual) -> Self {
        self.tests.insert(test.into(), actual);
        self
    }
}

impl DeltaProvider for Table {
    type Error = std::convert::Infallible;

    fn diff(&self, _: &SnapshotOrRev, _: &SnapshotOrRev) -> Result<Diff, Self::Error> {
        unreachable!("these tests supply the delta directly")
    }

    fn callsign(&self, datum: &Datum) -> Option<Callsign> {
        self.names.get(&datum.0).map(|n| Callsign(n.clone()))
    }

    fn test_outcome(&self, test: &str) -> TestComparisonActual {
        self.tests
            .get(test)
            .copied()
            .unwrap_or(TestComparisonActual::Missing)
    }
}

fn datum(tag: &str) -> Datum {
    Datum(format!("b3:{}", tag.repeat(64 / tag.len().max(1))))
}

fn plan(edges: Vec<PlanEdgePromise>) -> Plan {
    Plan {
        plan_id: "plan:test".into(),
        pr: None,
        plan_rev: None,
        base: SnapshotOrRev::GitRev {
            git_rev: GitRev("base".into()),
        },
        delta: PlanDelta {
            defs: Vec::new(),
            edges,
        },
        tests: None,
        invariants: None,
        tolerate: None,
    }
}

fn promise(action: PlanEdgePromiseAction, src: &str, dst: &str) -> PlanEdgePromise {
    PlanEdgePromise {
        action,
        src: CallsignPattern(src.into()),
        relation: Relation::Calls,
        dst: CallsignPattern(dst.into()),
    }
}

fn edge_change(action: EdgeChangeAction, src: &str, dst: &str, grade: Grade) -> EdgeChange {
    EdgeChange {
        action,
        edge: Edge {
            src: datum(src),
            relation: Relation::Calls,
            dst: datum(dst),
            grade,
            via: None,
        },
        moved_from: None,
    }
}

fn diff(defs: Vec<DefChange>, edges: Vec<EdgeChange>) -> Diff {
    Diff {
        base: SnapshotId("snap:b3:base".into()),
        head: SnapshotId("snap:b3:head".into()),
        defs,
        edges,
    }
}

#[test]
fn a_promise_the_delta_makes_good_matches() {
    let provider = Table::new()
        .naming(datum("aa").0.as_str(), "cortana.reaper.Reaper.sweep")
        .naming(
            datum("bb").0.as_str(),
            "cortana.reaper.TagContract.validate",
        );
    let plan = plan(vec![promise(
        PlanEdgePromiseAction::Add,
        "cortana.reaper.Reaper.sweep",
        "cortana.reaper.TagContract.*",
    )]);
    let delta = diff(
        vec![],
        vec![edge_change(
            EdgeChangeAction::Added,
            "aa",
            "bb",
            Grade::Proven,
        )],
    );

    let v = verdict(&plan, &delta, &provider);
    assert_eq!(v.gate, GateVerdictGate::Pass);
    assert_eq!(v.matched.len(), 1);
    assert!(v.missing.is_empty() && v.surplus.is_empty());
    assert_eq!(v.reasons.as_deref(), Some(&[][..]));
}

#[test]
fn a_promise_nothing_made_good_is_missing() {
    let plan = plan(vec![promise(
        PlanEdgePromiseAction::Add,
        "cortana.reaper.Reaper.sweep",
        "cortana.reaper.TagContract.validate",
    )]);
    let v = verdict(&plan, &diff(vec![], vec![]), &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.missing.len(), 1);
    assert_eq!(v.reasons.unwrap(), ["1 promised edge never built"]);
}

/// The action has to agree: building the edge the plan promised to *remove*
/// is not making the promise good, it is the opposite of it.
#[test]
fn the_wrong_tense_does_not_match() {
    let provider = Table::new()
        .naming(datum("aa").0.as_str(), "a")
        .naming(datum("bb").0.as_str(), "b");
    let plan = plan(vec![promise(PlanEdgePromiseAction::Remove, "a", "b")]);
    let delta = diff(
        vec![],
        vec![edge_change(
            EdgeChangeAction::Added,
            "aa",
            "bb",
            Grade::Proven,
        )],
    );

    let v = verdict(&plan, &delta, &provider);
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.missing.len(), 1, "the removal was never made good");
    assert_eq!(v.surplus.len(), 1, "and an unpromised addition appeared");
}

#[test]
fn tolerate_moves_surplus_out_of_the_hold() {
    let provider = Table::new()
        .naming(datum("aa").0.as_str(), "cortana.reaper.Reaper.sweep")
        .naming(datum("bb").0.as_str(), "cortana.reaper.Tag.field");
    let mut t = plan(vec![]);
    t.tolerate = Some(vec![EdgeRule {
        src: CallsignPattern("cortana.reaper.**".into()),
        relation: Some(Relation::Calls),
        dst: CallsignPattern("cortana.reaper.**".into()),
        at_or_above: None,
    }]);
    let delta = diff(
        vec![],
        vec![edge_change(
            EdgeChangeAction::Added,
            "aa",
            "bb",
            Grade::Proven,
        )],
    );

    let v = verdict(&t, &delta, &provider);
    assert_eq!(v.gate, GateVerdictGate::Pass);
    assert_eq!(v.tolerated.len(), 1);
    assert!(
        v.surplus.is_empty(),
        "tolerated changes are never a hold cause"
    );
}

/// `at_or_above` is a floor on evidence strength, so a tolerance written for
/// proven edges does not quietly excuse a heuristic guess.
#[test]
fn tolerance_respects_its_evidence_floor() {
    let provider = Table::new()
        .naming(datum("aa").0.as_str(), "a")
        .naming(datum("bb").0.as_str(), "b");
    let mut t = plan(vec![]);
    t.tolerate = Some(vec![EdgeRule {
        src: CallsignPattern("**".into()),
        relation: None,
        dst: CallsignPattern("**".into()),
        at_or_above: Some(Grade::Proven),
    }]);
    let weak = diff(
        vec![],
        vec![edge_change(
            EdgeChangeAction::Added,
            "aa",
            "bb",
            Grade::Heuristic,
        )],
    );
    let strong = diff(
        vec![],
        vec![edge_change(
            EdgeChangeAction::Added,
            "aa",
            "bb",
            Grade::Proven,
        )],
    );

    assert_eq!(verdict(&t, &weak, &provider).surplus.len(), 1);
    assert_eq!(verdict(&t, &strong, &provider).tolerated.len(), 1);
}

#[test]
fn an_unnamed_datum_matches_no_promise() {
    // The provider knows no names: a promise in name-space cannot be made
    // good by an edge nobody can name. Surplus, not a false match.
    let plan = plan(vec![promise(PlanEdgePromiseAction::Add, "**", "**")]);
    let delta = diff(
        vec![],
        vec![edge_change(
            EdgeChangeAction::Added,
            "aa",
            "bb",
            Grade::Proven,
        )],
    );

    let v = verdict(&plan, &delta, &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.surplus.len(), 1);
}

#[test]
fn def_promises_cover_def_changes_by_pattern() {
    let mut t = plan(vec![]);
    t.delta.defs = vec![PlanDefPromise {
        action: PlanDefPromiseAction::Add,
        callsign: CallsignPattern("cortana.reaper.TagContract.*".into()),
        interface: None,
    }];
    let covered = DefChange {
        moved_from: None,
        sig_changed: None,
        action: DefChangeAction::Added,
        callsign: Callsign("cortana.reaper.TagContract.validate".into()),
        before: None,
        after: Some(datum("bb")),
    };
    let stray = DefChange {
        moved_from: None,
        sig_changed: None,
        action: DefChangeAction::Added,
        callsign: Callsign("cortana.boards.render".into()),
        before: None,
        after: Some(datum("cc")),
    };

    let v = verdict(&t, &diff(vec![covered.clone()], vec![]), &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Pass);

    let v = verdict(&t, &diff(vec![covered, stray], vec![]), &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.defs_unaccounted.len(), 1);
}

fn def_promise(
    action: PlanDefPromiseAction,
    callsign: &str,
    interface: Option<bool>,
) -> PlanDefPromise {
    PlanDefPromise {
        action,
        callsign: CallsignPattern(callsign.into()),
        interface,
    }
}

fn def_change(action: DefChangeAction, callsign: &str, sig_changed: Option<bool>) -> DefChange {
    DefChange {
        moved_from: None,
        sig_changed,
        action,
        callsign: Callsign(callsign.into()),
        before: Some(datum("aa")),
        after: Some(datum("bb")),
    }
}

/// `interface: true` claims a caller can see the change: made good by a sig
/// move, and — conservatively — by an unknown transition too (a pre-epoch-3
/// snapshot on either side).
#[test]
fn interface_true_is_made_good_by_a_sig_move_or_unknown() {
    let mut t = plan(vec![]);
    t.delta.defs = vec![def_promise(
        PlanDefPromiseAction::Modify,
        "cortana.reaper.Reaper.sweep",
        Some(true),
    )];
    for sig_changed in [Some(true), None] {
        let change = def_change(
            DefChangeAction::Modified,
            "cortana.reaper.Reaper.sweep",
            sig_changed,
        );
        let v = verdict(&t, &diff(vec![change], vec![]), &Table::new());
        assert_eq!(v.gate, GateVerdictGate::Pass, "sig_changed={sig_changed:?}");
    }
}

/// `interface: true` is not made good by a change that provably held the
/// sig — the promise claimed a caller would see something, and nothing
/// moved. Unaccounted, but not for the `interface` reason: the sig held.
#[test]
fn interface_true_is_not_made_good_by_a_held_sig() {
    let mut t = plan(vec![]);
    t.delta.defs = vec![def_promise(
        PlanDefPromiseAction::Modify,
        "cortana.reaper.Reaper.sweep",
        Some(true),
    )];
    let change = def_change(
        DefChangeAction::Modified,
        "cortana.reaper.Reaper.sweep",
        Some(false),
    );
    let v = verdict(&t, &diff(vec![change], vec![]), &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.defs_unaccounted.len(), 1);
    assert!(v.interface_unaccounted.unwrap().is_empty());
}

/// `interface: false` claims the interface holds: made good only by a sig
/// that provably did not move.
#[test]
fn interface_false_is_made_good_only_by_a_held_sig() {
    let mut t = plan(vec![]);
    t.delta.defs = vec![def_promise(
        PlanDefPromiseAction::Modify,
        "cortana.reaper.Reaper.sweep",
        Some(false),
    )];
    let change = def_change(
        DefChangeAction::Modified,
        "cortana.reaper.Reaper.sweep",
        Some(false),
    );
    let v = verdict(&t, &diff(vec![change], vec![]), &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Pass);
}

/// A sig move under an `interface: false` promise is unaccounted with reason
/// `interface` — it lands in both lists, held once.
#[test]
fn interface_false_is_violated_by_a_sig_move() {
    let mut t = plan(vec![]);
    t.delta.defs = vec![def_promise(
        PlanDefPromiseAction::Modify,
        "cortana.reaper.Reaper.sweep",
        Some(false),
    )];
    let change = def_change(
        DefChangeAction::Modified,
        "cortana.reaper.Reaper.sweep",
        Some(true),
    );
    let v = verdict(&t, &diff(vec![change.clone()], vec![]), &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.defs_unaccounted, vec![change.clone()]);
    assert_eq!(v.interface_unaccounted.unwrap(), vec![change]);
}

/// Conservatism, seen red first: a pre-epoch-3 snapshot on either side of the
/// diff makes `sig_changed` `None` — unknown, not "held". A naive reading
/// that treats unknown as "did not move" would let an `interface: false`
/// promise pass here; the conservative reading holds instead, because a
/// promise this specific about invisibility must never be made good by
/// "nobody can say".
#[test]
fn interface_false_conservatism_an_unknown_sig_still_holds() {
    let mut t = plan(vec![]);
    t.delta.defs = vec![def_promise(
        PlanDefPromiseAction::Modify,
        "cortana.reaper.Reaper.sweep",
        Some(false),
    )];
    let change = def_change(
        DefChangeAction::Modified,
        "cortana.reaper.Reaper.sweep",
        None,
    );
    let v = verdict(&t, &diff(vec![change.clone()], vec![]), &Table::new());
    assert_eq!(
        v.gate,
        GateVerdictGate::Hold,
        "unknown must not read as \"held\" under an interface:false promise"
    );
    assert_eq!(v.interface_unaccounted.unwrap(), vec![change]);
}

#[test]
fn a_test_that_disagrees_holds_the_gate() {
    let mut t = plan(vec![]);
    t.tests = Some(vec![ExpectedTest {
        test: "tests/test_tag_contract.py::test_retry_window".into(),
        expect: ExpectedTestExpect::Pass,
        new: Some(true),
    }]);

    // No runner report: the test reads `missing`, which is a disagreement.
    let v = verdict(&t, &diff(vec![], vec![]), &Table::new());
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.tests[0].actual, TestComparisonActual::Missing);

    let ran = Table::new().ran(
        "tests/test_tag_contract.py::test_retry_window",
        TestComparisonActual::Pass,
    );
    let v = verdict(&t, &diff(vec![], vec![]), &ran);
    assert_eq!(v.gate, GateVerdictGate::Pass);
    assert!(v.tests[0].ok);
}

#[test]
fn a_forbidden_edge_is_witnessed_by_the_edge_itself() {
    let provider = Table::new()
        .naming(datum("aa").0.as_str(), "cortana.boards.render")
        .naming(datum("bb").0.as_str(), "cortana.persistence.write");
    let mut t = plan(vec![promise(
        PlanEdgePromiseAction::Add,
        "cortana.boards.render",
        "cortana.persistence.write",
    )]);
    t.invariants = Some(vec![Invariant {
        id: "boards-skip-persistence".into(),
        description: None,
        rule: InvariantRule::ForbidEdge {
            forbid_edge: EdgeRule {
                src: CallsignPattern("cortana.boards.**".into()),
                relation: Some(Relation::Calls),
                dst: CallsignPattern("cortana.persistence.**".into()),
                at_or_above: None,
            },
        },
    }]);
    let delta = diff(
        vec![],
        vec![edge_change(
            EdgeChangeAction::Added,
            "aa",
            "bb",
            Grade::Proven,
        )],
    );

    let v = verdict(&t, &delta, &provider);
    // The promise matched — and the gate still holds. An invariant outranks a
    // promise: promising to break a standing rule does not license breaking it.
    assert_eq!(v.matched.len(), 1);
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.invariant_violations.len(), 1);
    assert_eq!(v.invariant_violations[0].witness.edges.len(), 1);
    assert_eq!(
        v.reasons.unwrap(),
        ["invariant boards-skip-persistence violated"]
    );
}

proptest! {
    /// The contract's hold rule, from both directions: the gate holds exactly
    /// when some itemized divergence exists. No other input can move it.
    #[test]
    fn gate_holds_iff_something_diverges(
        promises in prop::collection::vec(0usize..3, 0..4),
        changes in prop::collection::vec(0usize..3, 0..4),
    ) {
        let names = ["a", "b", "c"];
        let provider = names
            .iter()
            .enumerate()
            .fold(Table::new(), |t, (i, n)| t.naming(datum(&"a".repeat(i + 1)).0.as_str(), n));
        let t = plan(
            promises
                .iter()
                .map(|&i| promise(PlanEdgePromiseAction::Add, names[i], names[i]))
                .collect(),
        );
        let tags: Vec<String> = (0..3).map(|i| "a".repeat(i + 1)).collect();
        let delta = diff(
            vec![],
            changes
                .iter()
                .map(|&i| edge_change(EdgeChangeAction::Added, &tags[i], &tags[i], Grade::Proven))
                .collect(),
        );

        let v = verdict(&t, &delta, &provider);
        let diverged = !v.missing.is_empty()
            || !v.surplus.is_empty()
            || !v.defs_unaccounted.is_empty()
            || v.tests.iter().any(|t| !t.ok)
            || !v.invariant_violations.is_empty();
        prop_assert_eq!(v.gate == GateVerdictGate::Hold, diverged);
        // The reasons list is the decision, not a decoration on it.
        prop_assert_eq!(v.reasons.as_ref().is_some_and(|r| !r.is_empty()), diverged);
    }

    /// Every change is accounted for exactly once: matched promises consume
    /// changes, the rest split into tolerated and surplus with nothing lost
    /// and nothing double-counted.
    #[test]
    fn every_change_lands_in_exactly_one_bucket(
        changes in prop::collection::vec((0usize..3, any::<bool>()), 0..5),
    ) {
        let names = ["a", "b", "c"];
        let tags: Vec<String> = (0..3).map(|i| "a".repeat(i + 1)).collect();
        let provider = names
            .iter()
            .enumerate()
            .fold(Table::new(), |t, (i, n)| t.naming(datum(&tags[i]).0.as_str(), n));
        let mut t = plan(vec![promise(PlanEdgePromiseAction::Add, "a", "a")]);
        t.tolerate = Some(vec![EdgeRule {
            src: CallsignPattern("b".into()),
            relation: None,
            dst: CallsignPattern("b".into()),
            at_or_above: None,
        }]);
        let delta = diff(
            vec![],
            changes
                .iter()
                .map(|&(i, _)| edge_change(EdgeChangeAction::Added, &tags[i], &tags[i], Grade::Proven))
                .collect(),
        );

        let v = verdict(&t, &delta, &provider);
        let accounted = v.surplus.len() + v.tolerated.len();
        let matched_changes = delta
            .edges
            .iter()
            .filter(|e| provider.callsign(&e.edge.src) == Some(Callsign("a".into())))
            .count();
        prop_assert_eq!(accounted + matched_changes, delta.edges.len());
    }

    /// `interface: None` is today's behavior, and today's behavior never
    /// looked at `sig_changed` — so a covering promise with no `interface`
    /// claim stays byte-identical to before the field existed, whatever the
    /// sig did. This is the property the field's additivity promised: every
    /// plan on record (which carries no `interface` and so deserializes to
    /// `None`) sees no change in verdict from adding the field.
    #[test]
    fn interface_none_is_blind_to_sig_changed(sig_changed in prop::option::of(any::<bool>())) {
        let mut t = plan(vec![]);
        t.delta.defs = vec![def_promise(
            PlanDefPromiseAction::Modify,
            "cortana.reaper.Reaper.sweep",
            None,
        )];
        let change = def_change(
            DefChangeAction::Modified,
            "cortana.reaper.Reaper.sweep",
            sig_changed,
        );
        let v = verdict(&t, &diff(vec![change], vec![]), &Table::new());
        prop_assert_eq!(v.gate, GateVerdictGate::Pass);
        prop_assert!(v.defs_unaccounted.is_empty());
    }
}

/// An unpromised add or remove is unaccounted, but not *for the interface*:
/// nothing existed on both sides for a caller to see move, and labelling it
/// `unaccounted: interface` reads as a gate reason it is not.
#[test]
fn an_unpromised_add_is_unaccounted_but_not_for_its_interface() {
    let t = plan(vec![]);
    let added = DefChange {
        moved_from: None,
        sig_changed: None,
        action: DefChangeAction::Added,
        callsign: Callsign("cortana.reaper.Reaper.extra".into()),
        before: None,
        after: Some(datum("cc")),
    };
    let removed = DefChange {
        moved_from: None,
        sig_changed: None,
        action: DefChangeAction::Removed,
        callsign: Callsign("cortana.reaper.Reaper.gone".into()),
        before: Some(datum("dd")),
        after: None,
    };
    let v = verdict(
        &t,
        &diff(vec![added.clone(), removed.clone()], vec![]),
        &Table::new(),
    );
    assert_eq!(v.gate, GateVerdictGate::Hold);
    assert_eq!(v.defs_unaccounted, vec![added, removed]);
    assert!(
        v.interface_unaccounted.unwrap().is_empty(),
        "an add or a remove has no interface that moved"
    );
}

/// The gate's own sentence — hold iff something is missing, surplus or
/// unaccounted — has always covered edge promises and never def promises. A
/// plan that promises a definition nobody wrote passed, while `grid todo` on
/// the same plan listed it as pending: two verbs, one plan, contradictory
/// answers, and the gate is the one CI would trust.
#[test]
fn a_def_promise_nothing_made_good_holds_the_gate() {
    let mut p = plan(vec![]);
    p.delta.defs = vec![def_promise(
        PlanDefPromiseAction::Add,
        "app.never_written",
        None,
    )];

    // A delta that touches nothing at all: the promise cannot have been kept.
    let out = verdict(&p, &diff(vec![], vec![]), &Table::new());

    assert_eq!(
        out.gate,
        GateVerdictGate::Hold,
        "a promise nobody made good passed the gate; reasons: {:?}",
        out.reasons
    );
    assert_eq!(
        out.defs_missing.as_deref().unwrap_or_default().len(),
        1,
        "the unfulfilled promise is named, not merely counted"
    );
}

/// The other side of the same rule: a promise the delta does make good must
/// not hold anything, or every plan that is finished would fail its own gate.
#[test]
fn a_def_promise_the_delta_makes_good_does_not_hold() {
    let mut p = plan(vec![]);
    p.delta.defs = vec![def_promise(PlanDefPromiseAction::Add, "app.written", None)];
    let change = def_change(DefChangeAction::Added, "app.written", None);

    let out = verdict(&p, &diff(vec![change], vec![]), &Table::new());

    assert_eq!(
        out.gate,
        GateVerdictGate::Pass,
        "a kept promise held the gate; reasons: {:?}",
        out.reasons
    );
    assert!(out.defs_missing.as_deref().unwrap_or_default().is_empty());
}
