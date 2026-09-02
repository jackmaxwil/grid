//! `plan.lint` — pure validation of a promise before code exists.
//!
//! No survey, no snapshot, no chart: everything checkable about a plan from
//! the plan alone plus the standing invariants it carries. Impossible plans
//! die at plan time (CLI plan §5, Planning).

use grid_wire::{
    EdgeRule, Invariant, InvariantRule, LintFinding, LintResult, Plan, PlanDefPromise,
    PlanDefPromiseAction, PlanEdgePromise, PlanEdgePromiseAction,
};

use crate::pattern;

/// The canonical rendering of an edge promise — the same shape the porcelain
/// `edge` record uses, so a lint finding is greppable next to a gate verdict.
#[must_use]
pub fn render_edge(p: &PlanEdgePromise) -> String {
    let action = match p.action {
        PlanEdgePromiseAction::Add => "add",
        PlanEdgePromiseAction::Remove => "remove",
        PlanEdgePromiseAction::Move => "move",
    };
    let relation = relation_name(p.relation);
    format!("{action} {} -{relation}-> {}", p.src.0, p.dst.0)
}

/// The wire spelling of a relation, which is also its porcelain spelling.
#[must_use]
pub fn relation_name(r: grid_wire::Relation) -> &'static str {
    use grid_wire::Relation as R;
    match r {
        R::Calls => "calls",
        R::Constructs => "constructs",
        R::ReadsField => "reads_field",
        R::WritesField => "writes_field",
        R::Implements => "implements",
        R::DispatchesTo => "dispatches_to",
        R::FlowsInto => "flows_into",
        R::SerializesTo => "serializes_to",
        R::DeserializesFrom => "deserializes_from",
        R::RegistersUnder => "registers_under",
        R::SendsOver => "sends_over",
        R::LinksAgainst => "links_against",
    }
}

fn render_def(p: &PlanDefPromise) -> String {
    use grid_wire::PlanDefPromiseAction as A;
    let action = match p.action {
        A::Add => "add",
        A::Remove => "remove",
        A::Modify => "modify",
        A::Move => "move",
    };
    format!("{action} {}", p.callsign.0)
}

/// Validate a plan. Errors make the plan unusable; warnings are smells the
/// planner should see but which do not block.
#[must_use]
pub fn lint(plan: &Plan) -> LintResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if !valid_track_id(&plan.plan_id) {
        errors.push(LintFinding {
            promise: plan.plan_id.clone(),
            reason: "plan_id must match ^plan:[A-Za-z0-9._-]{1,64}$".into(),
        });
    }

    for p in &plan.delta.defs {
        if let Some(reason) = bad_pattern(&p.callsign.0) {
            errors.push(LintFinding {
                promise: render_def(p),
                reason,
            });
        }
        // `interface` distinguishes a sig move from a body-only one; an add
        // or remove has no prior sig to hold or move against, so the claim
        // has nothing to mean.
        if p.interface.is_some()
            && matches!(
                p.action,
                PlanDefPromiseAction::Add | PlanDefPromiseAction::Remove
            )
        {
            errors.push(LintFinding {
                promise: render_def(p),
                reason: "interface only applies to a modify or move promise".into(),
            });
        }
    }

    let invariants = plan.invariants.as_deref().unwrap_or_default();
    for p in &plan.delta.edges {
        let rendered = render_edge(p);
        for side in [&p.src.0, &p.dst.0] {
            if let Some(reason) = bad_pattern(side) {
                errors.push(LintFinding {
                    promise: rendered.clone(),
                    reason,
                });
            }
        }
        // A promise that would build an edge a standing invariant forbids is
        // an impossible plan: the gate it must pass forbids what it promises.
        if p.action == PlanEdgePromiseAction::Add {
            for inv in invariants {
                if let InvariantRule::ForbidEdge { forbid_edge } = &inv.rule {
                    if subsumes(forbid_edge, p) {
                        errors.push(LintFinding {
                            promise: rendered.clone(),
                            reason: format!("violates invariant {}", inv.id),
                        });
                    }
                }
            }
        }
    }

    warnings.extend(duplicate_ids(invariants));
    if plan.delta.defs.is_empty() && plan.delta.edges.is_empty() {
        warnings.push(LintFinding {
            promise: plan.plan_id.clone(),
            reason: "plan promises nothing: every gate against it passes vacuously".into(),
        });
    }

    LintResult { errors, warnings }
}

fn duplicate_ids(invariants: &[Invariant]) -> Vec<LintFinding> {
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for inv in invariants {
        if seen.contains(&inv.id.as_str()) {
            out.push(LintFinding {
                promise: inv.id.clone(),
                reason: "duplicate invariant id".into(),
            });
        }
        seen.push(&inv.id);
    }
    out
}

/// Does a forbidding rule cover everything this promise could produce? The
/// promise is impossible only if *every* callsign it could name is forbidden,
/// which is decided by pattern subsumption, not by matching a sample.
fn subsumes(rule: &EdgeRule, promise: &PlanEdgePromise) -> bool {
    rule.relation.is_none_or(|r| r == promise.relation)
        && pattern::subsumes(&rule.src.0, &promise.src.0)
        && pattern::subsumes(&rule.dst.0, &promise.dst.0)
}

fn valid_track_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("plan:") else {
        return false;
    };
    (1..=64).contains(&rest.len())
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Patterns that can never match anything, and so can never be made good.
fn bad_pattern(p: &str) -> Option<String> {
    if p.is_empty() {
        return Some("empty callsign pattern".into());
    }
    if p.split('.').any(str::is_empty) {
        return Some(format!("callsign pattern `{p}` has an empty segment"));
    }
    if p.split('.')
        .any(|s| s.contains('*') && s != "*" && s != "**")
    {
        return Some(format!(
            "callsign pattern `{p}` mixes a glob with literal text in one segment; \
             use `*` (one segment) or `**` (any number)"
        ));
    }
    None
}
