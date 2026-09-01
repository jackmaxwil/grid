//! Gate semantics: what makes a verdict hold, and what must never make it.
//!
//! The wire contract states the rule the whole crate exists to implement:
//! "gate is 'hold' iff missing, surplus, `defs_unaccounted`, a failed
//! `TestComparison`, or an invariant violation is non-empty". These tests hold
//! that sentence to its word from both sides.

use grid_wire::*;
use grid_xte::{verdict, DeltaProvider};
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
}
