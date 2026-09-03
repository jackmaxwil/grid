//! `grid plan` — the plan store and its verbs.
//!
//! A plan is a JSON file under `.grid/plans`, named by its id. It is the
//! promise half of the gate: authored by the planner, read by `gate.check`,
//! and never written by the engine (D6 — survey and `edit.apply` are the only
//! writers; a plan file is planner input, not engine state).

use std::path::{Path, PathBuf};

use grid_wire::Plan;

use crate::{exit, hint, porcelain};

/// Where plans live, relative to the repo root.
const STORE: &str = ".grid/plans";

/// Load a plan from a path, or from the store by id.
///
/// A spec that looks like a path is read as one; anything else is resolved
/// against the store, with or without the id prefix.
///
/// # Errors
/// Returns a human-readable message when the plan cannot be read or parsed.
pub fn load(spec: &str) -> Result<Plan, String> {
    let direct = PathBuf::from(spec);
    let path = if direct.is_file() {
        direct
    } else {
        let file = store_path(spec);
        if !file.is_file() {
            return Err(format!(
                "no plan `{spec}`: not a file, and not in {STORE}\nhint: `grid plan new {spec}` writes a starter plan"
            ));
        }
        file
    };
    parse(&path)
}

fn parse(path: &Path) -> Result<Plan, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not a valid plan: {e}", path.display()))
}

/// The plan id a spec names. `plan:x`, `x` and `x.json` are three spellings
/// of one plan: the store adds the extension itself, so a spec that already
/// carries it wrote `x.json.json` and made the `todo` record's two columns
/// name different things.
fn plan_ident(spec: &str) -> &str {
    let id = spec.strip_prefix("plan:").unwrap_or(spec);
    id.strip_suffix(".json").unwrap_or(id)
}

fn store_path(spec: &str) -> PathBuf {
    Path::new(STORE).join(format!("{}.json", plan_ident(spec)))
}

/// `grid plan new|show|lint`.
pub fn main(args: &[&str], json: bool) -> u8 {
    match args.split_first() {
        Some((&"new", rest)) => new(rest, json),
        Some((&"show", rest)) => show(rest, json),
        Some((&"lint", rest)) => lint(rest, json),
        _ => {
            hint(NEW_USAGE);
            hint("       grid plan show <id> | grid plan lint <id>");
            exit::BAD_INPUT
        }
    }
}

const NEW_USAGE: &str =
    "usage: grid plan new <id> | grid plan new [<id>] --from-diff <base> [--out <file>]";

/// Draft a plan from a diff: one def promise per changed definition, one edge
/// promise per added edge, base pinned to the snapshot the diff was taken
/// from.
///
/// Pure over the diff and a namer, so the drafting rules are testable without
/// a repository. Callsigns are written as literal patterns: the gate matches
/// a literal against itself, and widening one to a glob is a decision only
/// the author can make. What comes out is a *draft* — the agent prunes it,
/// and the gate stays the judge.
///
/// Returns the plan and every added edge left out of it, one rendered line
/// each: an endpoint the chart cannot name has no callsign to promise, and a
/// datum handle is not a callsign pattern.
#[must_use]
pub fn draft(
    plan_id: &str,
    diff: &grid_wire::Diff,
    name: &porcelain::NameOf,
) -> (Plan, Vec<String>) {
    use grid_wire::{
        CallsignPattern, DefChangeAction as Was, EdgeChangeAction, PlanDefPromise,
        PlanDefPromiseAction as Does, PlanEdgePromise, PlanEdgePromiseAction,
    };
    let defs = diff
        .defs
        .iter()
        .map(|d| {
            let action = match d.action {
                Was::Added => Does::Add,
                Was::Removed => Does::Remove,
                Was::Modified => Does::Modify,
                Was::Moved => Does::Move,
            };
            PlanDefPromise {
                action,
                callsign: CallsignPattern(d.callsign.0.clone()),
                // `interface` says whether callers can see the change, and it
                // means nothing on an add or a remove — `plan lint` refuses it
                // there, since neither has a prior sig to hold or move
                // against. Where it does mean something it is copied from the
                // diff and *omitted* when the diff could not say: `false`
                // promises the sig held, which is a claim about a snapshot
                // nobody can ask.
                interface: matches!(action, Does::Modify | Does::Move)
                    .then_some(d.sig_changed)
                    .flatten(),
            }
        })
        .collect();
    let mut edges = Vec::new();
    let mut skipped = Vec::new();
    for change in &diff.edges {
        if change.action != EdgeChangeAction::Added {
            continue;
        }
        let (Some(src), Some(dst)) = (name(&change.edge.src), name(&change.edge.dst)) else {
            skipped.push(porcelain::chart_edge_arrow(&change.edge, name));
            continue;
        };
        edges.push(PlanEdgePromise {
            action: PlanEdgePromiseAction::Add,
            src: CallsignPattern(src),
            relation: change.edge.relation,
            dst: CallsignPattern(dst),
        });
    }
    (
        Plan {
            plan_id: plan_id.to_owned(),
            pr: None,
            plan_rev: None,
            // The snapshot, not the rev that produced it: a plan drafted
            // against a worktree is a promise about *this* base, and a rev
            // label would resolve to whatever that rev means later.
            base: grid_wire::SnapshotOrRev::Snapshot {
                snapshot: diff.base.clone(),
            },
            delta: grid_wire::PlanDelta { defs, edges },
            tests: None,
            invariants: None,
            tolerate: None,
        },
        skipped,
    )
}

/// A plan id nobody has to invent: the base snapshot's own tail, so two
/// drafts from one base collide and two from different bases do not.
fn drafted_id(base: &grid_wire::SnapshotId) -> String {
    let tail: String = base
        .0
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("plan:from-diff-{tail}")
}

/// `grid plan new [<id>] [--from-diff <base>] [--out <file>]`.
fn new(args: &[&str], json: bool) -> u8 {
    let mut id = None;
    let mut from_diff = None;
    let mut out = None;
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match *arg {
            "--from-diff" | "--out" => {
                let Some((value, tail)) = rest.split_first() else {
                    hint(&format!("grid: `{arg}` needs a value\n{NEW_USAGE}"));
                    return exit::BAD_INPUT;
                };
                rest = tail;
                if *arg == "--out" {
                    out = Some(*value);
                } else {
                    from_diff = Some(*value);
                }
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{NEW_USAGE}"));
                return exit::BAD_INPUT;
            }
            other if id.is_none() => id = Some(other),
            other => {
                hint(&format!("grid: unexpected argument `{other}`\n{NEW_USAGE}"));
                return exit::BAD_INPUT;
            }
        }
    }
    match from_diff {
        Some(base) => from_working_diff(id, base, out, json),
        None if out.is_some() => {
            hint(&format!("grid: `--out` needs `--from-diff`\n{NEW_USAGE}"));
            exit::BAD_INPUT
        }
        None => starter(id, json),
    }
}

/// Draft the plan from the diff between `base` and the worktree, and write it
/// where the caller asked: a file, or stdout.
fn from_working_diff(id: Option<&str>, base: &str, out: Option<&str>, json: bool) -> u8 {
    let params = grid_wire::methods::DeltaParams {
        base: crate::snapshot_or_rev(base),
        head: grid_wire::SnapshotOrRev::GitRev {
            git_rev: grid_wire::GitRev("HEAD".into()),
        },
    };
    let diff = match crate::rpc::delta(&params, crate::root()) {
        Ok(diff) => diff,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    let plan_id = id.map_or_else(
        || drafted_id(&diff.base),
        |id| {
            if id.starts_with("plan:") {
                id.to_owned()
            } else {
                format!("plan:{id}")
            }
        },
    );
    let (plan, skipped) = draft(&plan_id, &diff, &crate::namer());
    for edge in &skipped {
        hint(&format!(
            "grid: {edge} is not promised — the chart cannot name both of its endpoints"
        ));
    }
    let Ok(body) = serde_json::to_string_pretty(&plan) else {
        hint("grid: cannot serialize plan");
        return exit::INTERNAL;
    };
    if let Some(path) = out {
        if let Err(e) = std::fs::write(path, body.clone() + "\n") {
            hint(&format!("grid: cannot write {path}: {e}"));
            return exit::INTERNAL;
        }
    }
    if json {
        return crate::emit_json(&plan);
    }
    match out {
        None => println!("{body}"),
        Some(path) => println!("todo     {plan_id}  {path}"),
    }
    hint(&format!(
        "hint: {} def and {} edge promises drafted — prune what you did not mean to promise, \
         then `grid plan lint`",
        plan.delta.defs.len(),
        plan.delta.edges.len()
    ));
    exit::OK
}

/// Write a starter plan: valid, empty, and lint-clean but for the warning
/// that it promises nothing — which is exactly the next thing to correct.
fn starter(id: Option<&str>, json: bool) -> u8 {
    let Some(id) = id else {
        hint(NEW_USAGE);
        return exit::BAD_INPUT;
    };
    let plan_id = format!("plan:{}", plan_ident(id));
    let path = store_path(&plan_id);
    if path.exists() {
        hint(&format!(
            "grid: {} already exists\nhint: `grid plan show {plan_id}` reads it",
            path.display()
        ));
        return exit::BAD_INPUT;
    }
    let plan = Plan {
        plan_id: plan_id.clone(),
        pr: None,
        plan_rev: None,
        base: grid_wire::SnapshotOrRev::GitRev {
            git_rev: grid_wire::GitRev("HEAD".into()),
        },
        delta: grid_wire::PlanDelta {
            defs: Vec::new(),
            edges: Vec::new(),
        },
        tests: None,
        invariants: None,
        tolerate: None,
    };
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            hint(&format!("grid: cannot create {}: {e}", dir.display()));
            return exit::INTERNAL;
        }
    }
    let Ok(body) = serde_json::to_string_pretty(&plan) else {
        hint("grid: cannot serialize plan");
        return exit::INTERNAL;
    };
    if let Err(e) = std::fs::write(&path, body + "\n") {
        hint(&format!("grid: cannot write {}: {e}", path.display()));
        return exit::INTERNAL;
    }
    if json {
        return crate::emit_json(&plan);
    }
    println!("todo     {plan_id}  {}", path.display());
    hint("hint: add promises to delta.edges, then `grid plan lint` before you write code");
    exit::OK
}

fn show(args: &[&str], json: bool) -> u8 {
    let Some(spec) = args.first() else {
        hint("usage: grid plan show <id>");
        return exit::BAD_INPUT;
    };
    let plan = match load(spec) {
        Ok(t) => t,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    if json {
        return crate::emit_json(&plan);
    }
    print!("{}", porcelain::plan(&plan));
    exit::OK
}

/// Lint exits 2 on errors — an unusable plan is bad input, not a divergence
/// verdict: no code has been written yet for reality to diverge from.
fn lint(args: &[&str], json: bool) -> u8 {
    let Some(spec) = args.first() else {
        hint("usage: grid plan lint <id>");
        return exit::BAD_INPUT;
    };
    let plan = match load(spec) {
        Ok(t) => t,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    let result =
        match crate::rpc::plan_lint(&grid_wire::methods::PlanLintParams { plan }, Path::new(".")) {
            Ok(r) => r,
            Err(e) => {
                hint(&format!("grid: {e}"));
                return exit::BAD_INPUT;
            }
        };
    let bad = !result.errors.is_empty();
    if json {
        let code = crate::emit_json(&result);
        return if code == exit::OK && bad {
            exit::BAD_INPUT
        } else {
            code
        };
    }
    print!("{}", porcelain::lint(&result));
    if bad {
        hint("grid: plan is not gateable until these are corrected");
        return exit::BAD_INPUT;
    }
    exit::OK
}

#[cfg(test)]
mod tests {
    //! The drafting rules, over a diff built by hand: the promise each kind
    //! of change becomes, and what a draft refuses to claim.
    use grid_wire::{
        Callsign, Datum, DefChange, DefChangeAction, Diff, Edge, EdgeChange, EdgeChangeAction,
        Grade, PlanDefPromiseAction, PlanEdgePromiseAction, Relation, SnapshotId,
    };

    fn datum(fill: char) -> Datum {
        Datum(format!("b3:{}", String::from(fill).repeat(64)))
    }

    fn def(action: DefChangeAction, callsign: &str, sig_changed: Option<bool>) -> DefChange {
        DefChange {
            action,
            callsign: Callsign(callsign.to_owned()),
            before: Some(datum('1')),
            after: Some(datum('2')),
            moved_from: None,
            sig_changed,
        }
    }

    fn edge(src: Datum, dst: Datum) -> EdgeChange {
        EdgeChange {
            action: EdgeChangeAction::Added,
            edge: Edge {
                src,
                relation: Relation::Calls,
                dst,
                grade: Grade::Proven,
                via: None,
            },
            moved_from: None,
        }
    }

    /// Names `1…` and `2…`; anything else the chart cannot name.
    fn namer() -> impl Fn(&Datum) -> Option<String> {
        |d: &Datum| match d.0.as_str() {
            s if s == datum('1').0 => Some("m.a".to_owned()),
            s if s == datum('2').0 => Some("m.b".to_owned()),
            _ => None,
        }
    }

    fn drafted(diff: &Diff) -> (grid_wire::Plan, Vec<String>) {
        super::draft("plan:d", diff, &namer())
    }

    /// Ground truth: each change kind becomes its promise, the base is the
    /// snapshot the diff was taken from rather than the rev label that found
    /// it, an added edge becomes an `add` promise in callsign space — and
    /// `grid plan lint` accepts the result unchanged, which is the whole
    /// point of drafting it.
    #[test]
    fn a_draft_is_the_diff_as_promises_and_lints_clean() {
        let diff = Diff {
            base: SnapshotId("snap:b3:base".into()),
            head: SnapshotId("snap:b3:head".into()),
            defs: vec![
                def(DefChangeAction::Added, "m.new", None),
                def(DefChangeAction::Removed, "m.gone", None),
                def(DefChangeAction::Modified, "m.body", Some(false)),
                def(DefChangeAction::Moved, "m.sig", Some(true)),
            ],
            edges: vec![
                edge(datum('1'), datum('2')),
                // One endpoint the chart cannot name: no callsign, no promise.
                edge(datum('1'), datum('9')),
            ],
        };
        let (plan, skipped) = drafted(&diff);
        assert_eq!(
            plan.base,
            grid_wire::SnapshotOrRev::Snapshot {
                snapshot: SnapshotId("snap:b3:base".into())
            }
        );
        let promises: Vec<(PlanDefPromiseAction, &str, Option<bool>)> = plan
            .delta
            .defs
            .iter()
            .map(|p| (p.action, p.callsign.0.as_str(), p.interface))
            .collect();
        assert_eq!(
            promises,
            [
                (PlanDefPromiseAction::Add, "m.new", None),
                (PlanDefPromiseAction::Remove, "m.gone", None),
                (PlanDefPromiseAction::Modify, "m.body", Some(false)),
                (PlanDefPromiseAction::Move, "m.sig", Some(true)),
            ]
        );
        assert_eq!(plan.delta.edges.len(), 1);
        assert_eq!(plan.delta.edges[0].action, PlanEdgePromiseAction::Add);
        assert_eq!(plan.delta.edges[0].src.0, "m.a");
        assert_eq!(plan.delta.edges[0].dst.0, "m.b");
        assert_eq!(
            skipped.len(),
            1,
            "the unnameable edge is reported: {skipped:?}"
        );

        let found = grid_gate::lint::lint(&plan);
        assert!(
            found.errors.is_empty(),
            "a draft `grid plan lint` refuses is not a draft: {:?}",
            found.errors
        );
    }

    /// Conservatism: a definition whose `sig_changed` the diff could not
    /// establish — a pre-epoch-3 snapshot on one side — gets no `interface`
    /// claim at all. `false` promises that callers cannot see the change,
    /// and the gate would then hold on a sig move the draft never looked at;
    /// omitted is silent on the question, which is what unknown means.
    #[test]
    fn an_unknown_sig_change_is_never_promised_as_a_held_interface() {
        for action in [DefChangeAction::Modified, DefChangeAction::Moved] {
            let diff = Diff {
                base: SnapshotId("snap:b3:base".into()),
                head: SnapshotId("snap:b3:head".into()),
                defs: vec![def(action, "m.unknown", None)],
                edges: Vec::new(),
            };
            let (plan, _) = drafted(&diff);
            assert_eq!(
                plan.delta.defs[0].interface, None,
                "{action:?}: unknown was promised as a claim"
            );
        }
        // And an add or a remove never carries one either, whatever the diff
        // says — `grid plan lint` refuses `interface` on both.
        for action in [DefChangeAction::Added, DefChangeAction::Removed] {
            let diff = Diff {
                base: SnapshotId("snap:b3:base".into()),
                head: SnapshotId("snap:b3:head".into()),
                defs: vec![def(action, "m.x", Some(true))],
                edges: Vec::new(),
            };
            let (plan, _) = drafted(&diff);
            assert_eq!(plan.delta.defs[0].interface, None, "{action:?}");
            assert!(grid_gate::lint::lint(&plan).errors.is_empty());
        }
    }
}
