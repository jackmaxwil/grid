//! `plan.lint`: everything checkable about a promise before code exists.

use grid_gate::lint::lint;
use grid_wire::*;

fn plan(edges: Vec<PlanEdgePromise>, invariants: Vec<Invariant>) -> Plan {
    Plan {
        plan_id: "plan:pr-390-plan-v1".into(),
        pr: None,
        plan_rev: None,
        base: SnapshotOrRev::GitRev {
            git_rev: GitRev("bc21972".into()),
        },
        delta: PlanDelta {
            defs: Vec::new(),
            edges,
        },
        tests: None,
        invariants: (!invariants.is_empty()).then_some(invariants),
        tolerate: None,
    }
}

fn forbid(id: &str, src: &str, dst: &str) -> Invariant {
    Invariant {
        id: id.into(),
        description: None,
        rule: InvariantRule::ForbidEdge {
            forbid_edge: EdgeRule {
                src: CallsignPattern(src.into()),
                relation: Some(Relation::Calls),
                dst: CallsignPattern(dst.into()),
                at_or_above: None,
            },
        },
    }
}

fn add(src: &str, dst: &str) -> PlanEdgePromise {
    PlanEdgePromise {
        action: PlanEdgePromiseAction::Add,
        src: CallsignPattern(src.into()),
        relation: Relation::Calls,
        dst: CallsignPattern(dst.into()),
    }
}

/// The contract's own example, to the letter.
#[test]
fn an_impossible_plan_dies_at_plan_time() {
    let t = plan(
        vec![add("cortana.boards.render", "cortana.persistence.write")],
        vec![forbid(
            "boards-skip-persistence",
            "cortana.boards.**",
            "cortana.persistence.**",
        )],
    );
    let result = lint(&t);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(
        result.errors[0].promise,
        "add cortana.boards.render -calls-> cortana.persistence.write"
    );
    assert_eq!(
        result.errors[0].reason,
        "violates invariant boards-skip-persistence"
    );
    assert!(result.warnings.is_empty());
}

/// The lint calls a plan impossible only when *every* callsign the promise
/// could name is forbidden. A promise that merely overlaps a forbidden region
/// might still be made good inside it, and rejecting it would be a false
/// impossibility — the failure mode that makes a linter get switched off.
#[test]
fn overlap_is_not_impossibility() {
    let t = plan(
        vec![add("cortana.**", "cortana.**")],
        vec![forbid(
            "narrow",
            "cortana.boards.**",
            "cortana.persistence.**",
        )],
    );
    assert!(
        lint(&t).errors.is_empty(),
        "a wide promise can be made good outside the forbidden region"
    );

    // But a promise entirely inside the forbidden region cannot.
    let t = plan(
        vec![add("cortana.boards.*", "cortana.persistence.*")],
        vec![forbid(
            "narrow",
            "cortana.boards.**",
            "cortana.persistence.**",
        )],
    );
    assert_eq!(lint(&t).errors.len(), 1);
}

/// A relation-specific rule says nothing about other relations.
#[test]
fn a_rule_scoped_to_a_relation_stays_there() {
    let mut t = plan(
        vec![add("a.**", "b.**")],
        vec![forbid("calls-only", "a.**", "b.**")],
    );
    assert_eq!(lint(&t).errors.len(), 1);
    t.delta.edges[0].relation = Relation::ReadsField;
    assert!(lint(&t).errors.is_empty());
}

/// Removing an edge an invariant forbids is the fix, not the violation.
#[test]
fn only_additions_can_violate_a_forbid_rule() {
    let mut t = plan(
        vec![add("a.x", "b.y")],
        vec![forbid("no-a-to-b", "a.**", "b.**")],
    );
    assert_eq!(lint(&t).errors.len(), 1);
    t.delta.edges[0].action = PlanEdgePromiseAction::Remove;
    assert!(lint(&t).errors.is_empty());
}

#[test]
fn unmatchable_patterns_are_errors() {
    let mut t = plan(vec![add("a..b", "c")], vec![]);
    assert_eq!(lint(&t).errors.len(), 1, "an empty segment matches nothing");

    t.delta.edges = vec![add("a.re*", "c")];
    let errors = lint(&t).errors;
    assert_eq!(errors.len(), 1);
    assert!(errors[0].reason.contains("mixes a glob with literal text"));

    t.delta.edges = vec![add("", "c")];
    assert_eq!(lint(&t).errors.len(), 1);
}

#[test]
fn a_malformed_track_id_is_an_error() {
    let mut t = plan(vec![add("a", "b")], vec![]);
    t.plan_id = "pr-390".into();
    let errors = lint(&t).errors;
    assert_eq!(errors.len(), 1);
    assert!(errors[0].reason.contains("plan:"));
}

#[test]
fn a_vacuous_track_warns_but_is_usable() {
    let result = lint(&plan(vec![], vec![]));
    assert!(result.errors.is_empty());
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].reason.contains("passes vacuously"));
}

#[test]
fn duplicate_invariant_ids_warn() {
    let t = plan(
        vec![],
        vec![
            forbid("same", "a.**", "b.**"),
            forbid("same", "c.**", "d.**"),
        ],
    );
    let warnings = lint(&t).warnings;
    assert!(warnings
        .iter()
        .any(|w| w.reason == "duplicate invariant id"));
}
