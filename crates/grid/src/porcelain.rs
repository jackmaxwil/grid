//! The porcelain renderer: typed records on stdout, one per line.
//!
//! Normative grammar in `docs/PORCELAIN.md`. Fields are
//! separated by runs of spaces and column order is stable per record type, so
//! `grep '^edge  miss'` is the explanation and `awk` is the parser. Nothing
//! here writes to stderr: hints are the caller's business.

use std::fmt::Write as _;

use grid_gate::lint::{relation_name, render_edge};
use grid_wire::methods::{CacheRecordResult, TraceIngestResult};
use grid_wire::{
    CacheCheckResult, CacheHitVerdict, DefChangeAction, EdgeChange, GateVerdict, GateVerdictGate,
    LintResult, PlanEdgePromise, TestComparisonActual, TestComparisonExpected,
};

/// The porcelain grammar version this renderer implements.
pub const VERSION: &str = "2.25.0";

/// One record: a tag and its columns. Rendering is the same for every record
/// type, which is what makes the grammar a grammar.
fn record(out: &mut String, tag: &str, columns: &[&str]) {
    // Tags are padded to the width of the longest tag so columns line up for
    // a human; runs of spaces are one separator for a machine either way.
    // A tag at or over that width still gets a separator, or `snapshot` and
    // its first column would run together into one token.
    out.push_str(tag);
    for _ in 0..8usize.saturating_sub(tag.len()).max(2) {
        out.push(' ');
    }
    let mut first = true;
    for col in columns {
        if !first {
            out.push_str("  ");
        }
        first = false;
        out.push_str(col);
    }
    out.push('\n');
}

/// The role column, padded to the widest verdict role (`tolerated`). Runs of
/// spaces are one separator, so padding is for eyes only — but consistent
/// padding is what makes a screenful of records skimmable.
fn role(name: &str) -> String {
    format!("{name:<9}")
}

/// Change glyphs: `+` added, `-` removed, `~` modified, `>` moved.
const fn def_glyph(action: DefChangeAction) -> &'static str {
    match action {
        DefChangeAction::Added => "+",
        DefChangeAction::Removed => "-",
        DefChangeAction::Modified => "~",
        DefChangeAction::Moved => ">",
    }
}

/// The arrow for the edge inside a change.
fn edge_arrow(change: &EdgeChange, name: &NameOf) -> String {
    chart_edge_arrow(&change.edge, name)
}

/// How the renderer asks the chart what a datum is called. `None` means the
/// chart has no name for it, which is a real answer: a datum can outlive the
/// name it had.
pub type NameOf<'a> = dyn Fn(&grid_wire::Datum) -> Option<String> + 'a;

/// Promise arrows drop the leading action word: the verdict role column
/// already says what happened to the promise.
fn promise_arrow(p: &PlanEdgePromise) -> String {
    let rendered = render_edge(p);
    rendered
        .split_once(' ')
        .map_or(rendered.clone(), |(_, rest)| rest.to_string())
}

const fn grade_name(z: grid_wire::Grade) -> &'static str {
    use grid_wire::Grade as Z;
    match z {
        Z::Proven => "proven",
        Z::Observed => "observed",
        Z::Modeled => "modeled",
        Z::Heuristic => "heuristic",
    }
}

const fn expected_name(e: TestComparisonExpected) -> &'static str {
    match e {
        TestComparisonExpected::Pass => "pass",
        TestComparisonExpected::Fail => "fail",
        TestComparisonExpected::Skip => "skip",
    }
}

const fn actual_name(a: TestComparisonActual) -> &'static str {
    match a {
        TestComparisonActual::Pass => "pass",
        TestComparisonActual::Fail => "fail",
        TestComparisonActual::Skip => "skip",
        TestComparisonActual::Missing => "missing",
    }
}

/// Render a gate verdict. The first record is `pass` or `hold` with the plan
/// and snapshot; every divergence follows as its own record.
#[must_use]
pub fn verdict(v: &GateVerdict, name: &NameOf) -> String {
    let mut out = String::new();
    let head = grid_gate::snapshot_display(&v.head);
    let tag = match v.gate {
        GateVerdictGate::Pass => "pass",
        GateVerdictGate::Hold => "hold",
    };
    record(&mut out, tag, &[&v.plan_id, &head]);

    // Verdict roles on `edge` records, padded to the widest of them so the
    // arrow column lines up whichever roles a verdict happens to contain.
    for p in &v.matched {
        record(&mut out, "edge", &[&role("match"), &promise_arrow(p)]);
    }
    for p in &v.missing {
        record(&mut out, "edge", &[&role("miss"), &promise_arrow(p)]);
    }
    for c in &v.surplus {
        record(
            &mut out,
            "edge",
            &[
                &role("surplus"),
                &edge_arrow(c, name),
                grade_name(c.edge.grade),
            ],
        );
    }
    for c in &v.tolerated {
        record(
            &mut out,
            "edge",
            &[
                &role("tolerated"),
                &edge_arrow(c, name),
                grade_name(c.edge.grade),
            ],
        );
    }
    let held_for_interface = v.interface_unaccounted.as_deref().unwrap_or_default();
    for d in &v.defs_unaccounted {
        let mut columns = vec![def_glyph(d.action), d.callsign.0.as_str()];
        if held_for_interface.contains(d) {
            columns.push("unaccounted: interface");
        }
        record(&mut out, "def", &columns);
    }
    for t in &v.tests {
        let outcome = format!("{}={}", expected_name(t.expected), actual_name(t.actual));
        let verdict = if t.ok { "ok" } else { "fail" };
        record(&mut out, "test", &[&role(verdict), &t.test, &outcome]);
    }
    for violation in &v.invariant_violations {
        record(&mut out, "inv", &["violated", &violation.id]);
    }
    out
}

/// Render a plan: its promises as `todo` records, since a promise is work
/// not yet made good, plus its standing invariants.
#[must_use]
pub fn plan(t: &grid_wire::Plan) -> String {
    let mut out = String::new();
    let base = match &t.base {
        grid_wire::SnapshotOrRev::GitRev { git_rev } => git_rev.0.clone(),
        grid_wire::SnapshotOrRev::Snapshot { snapshot } => grid_gate::snapshot_display(snapshot),
    };
    record(&mut out, "todo", &[&t.plan_id, &base]);
    for d in &t.delta.defs {
        let action = match d.action {
            grid_wire::PlanDefPromiseAction::Add => "+",
            grid_wire::PlanDefPromiseAction::Remove => "-",
            grid_wire::PlanDefPromiseAction::Modify => "~",
            grid_wire::PlanDefPromiseAction::Move => ">",
        };
        record(&mut out, "def", &[action, &d.callsign.0]);
    }
    for e in &t.delta.edges {
        let glyph = match e.action {
            grid_wire::PlanEdgePromiseAction::Add => "+",
            grid_wire::PlanEdgePromiseAction::Remove => "-",
            grid_wire::PlanEdgePromiseAction::Move => ">",
        };
        record(&mut out, "edge", &[glyph, &promise_arrow(e)]);
    }
    // A plan's tests are promises, not comparisons: there is no actual
    // outcome to put in a `test` record's `expected=actual` column yet.
    for t in t.tests.iter().flatten() {
        record(&mut out, "todo", &["test", &t.test, expect_name(t.expect)]);
    }
    for inv in t.invariants.iter().flatten() {
        record(&mut out, "inv", &["standing", &inv.id]);
    }
    out
}

const fn expect_name(e: grid_wire::ExpectedTestExpect) -> &'static str {
    use grid_wire::ExpectedTestExpect as E;
    match e {
        E::Pass => "pass",
        E::Fail => "fail",
        E::Skip => "skip",
    }
}

/// Render a lint result as `warn` records: callsign, then message — the two
/// columns the grammar defines, no more. Severity is not a column because it
/// is the exit code: a plan with errors exits 2, a plan with only warnings
/// exits 0. Callers that need the split structurally read `--json`, which is
/// the `LintResult` verbatim with its two arrays intact.
#[must_use]
pub fn lint(result: &LintResult) -> String {
    let mut out = String::new();
    for finding in result.errors.iter().chain(&result.warnings) {
        record(&mut out, "warn", &[&finding.promise, &finding.reason]);
    }
    out
}

/// Short display form of a datum: `b3:9f2c...` -> `9f2c`.
///
/// The display policy is that callsigns are identity and hashes appear only
/// where identity beyond the name matters — a `def` record's transition column
/// is exactly that place, so it gets the prefix and nothing else does.
/// `file:line` where a definition currently lives, `absent` when unknown.
fn at(m: Option<&grid_wire::DefRefMaterialization>, absent: &str) -> String {
    m.map_or_else(|| absent.to_owned(), |m| format!("{}:{}", m.file, m.line))
}

fn short(datum: &grid_wire::Datum) -> String {
    datum
        .0
        .rsplit_once(':')
        .map_or(datum.0.as_str(), |(_, hex)| hex)
        .chars()
        .take(4)
        .collect()
}

/// Render a snapshot as the single `snapshot` record the grammar defines:
/// id, then the stats block.
#[must_use]
pub fn snapshot(s: &grid_wire::Snapshot, run: Option<(u64, std::time::Duration)>) -> String {
    let mut out = String::new();
    // Displayed identity is the rev, never the raw content id (CLI plan §2) —
    // plus the diff hash's prefix when the tree was dirty, so `@abc1234` and
    // `@abc1234+9f0e1d2c` read as what they are: the tree at the rev, and a
    // worktree checked out from it that differs.
    let id = match s.worktree_diff_hash.as_deref() {
        Some(hash) => format!(
            "@{}+{}",
            s.git_rev.0,
            hash.strip_prefix("b3:")
                .unwrap_or(hash)
                .chars()
                .take(8)
                .collect::<String>()
        ),
        None => format!("@{}", s.git_rev.0),
    };
    // `reread N  <duration>` describe the run, not the snapshot, so they
    // appear only when a run just happened.
    let stats = match run {
        Some((reread, elapsed)) => format!(
            "defs {}  edges {}  reread {reread}  {}ms",
            s.stats.defs,
            s.stats.edges,
            elapsed.as_millis()
        ),
        None => format!("defs {}  edges {}", s.stats.defs, s.stats.edges),
    };
    record(&mut out, "snapshot", &[&id, &stats]);
    // What it charted without what it declined is `grid drift`'s silence.
    if let Some(d) = &s.declined {
        let total = d
            .external
            .saturating_add(d.method)
            .saturating_add(d.internal)
            .saturating_add(d.macros.unwrap_or(0));
        if total > 0 {
            // A snapshot predating a count says nothing rather than `0`.
            let method = match d.refused {
                Some(refused) => format!("method {} of which {refused} refused", d.method),
                None => format!("method {}", d.method),
            };
            let macros = d.macros.map_or(String::new(), |n| format!(", macro {n}"));
            record(
                &mut out,
                "miss",
                &[
                    &total.to_string(),
                    &format!(
                        "references declined (external {}, {method}, internal {}{macros})",
                        d.external, d.internal
                    ),
                ],
            );
        }
    }
    // A file the survey could not read: the same record `grid watch` emits.
    for (paths, why) in [
        (&s.unparseable, "unparseable"),
        (&s.unreadable, "unreadable"),
    ] {
        out.push_str(&refused(paths.as_deref().unwrap_or_default(), why));
    }
    out
}

/// Render a delta as `def` and `edge` records.
///
/// This is the reference render from the CLI plan: a glyph column, the
/// callsign, and a trailing column that is the datum transition for a def and
/// the evidence grade for an edge.
#[must_use]
pub fn diff(diff: &grid_wire::Diff, name: &NameOf) -> String {
    let mut out = String::new();
    for d in &diff.defs {
        // `old→new` when both exist, otherwise the one that does: an added
        // definition has no before, a removed one has no after.
        // A move's datum held still; the interesting column is the old name.
        let transition = if let Some(from) = &d.moved_from {
            format!("(was {})", from.0)
        } else {
            match (&d.before, &d.after) {
                (Some(before), Some(after)) => format!("{}→{}", short(before), short(after)),
                (None, Some(after)) | (Some(after), None) => short(after),
                (None, None) => String::new(),
            }
        };
        // A trailing `sig` marks a change a caller can see: body-only edits
        // and interface edits are different events, and a reviewer reads the
        // difference off the row. Absent when the interface held, and absent
        // when a pre-epoch-3 snapshot means nobody can say.
        let mut columns = vec![def_glyph(d.action), d.callsign.0.as_str(), &transition];
        if d.sig_changed == Some(true) {
            columns.push("sig");
        }
        record(&mut out, "def", &columns);
    }
    for e in &diff.edges {
        let glyph = match e.action {
            grid_wire::EdgeChangeAction::Added => "+",
            grid_wire::EdgeChangeAction::Removed => "-",
            grid_wire::EdgeChangeAction::Moved => ">",
        };
        // A moved edge's trailing column is where it came from; anything else
        // reports the evidence that establishes it.
        let context = e.moved_from.as_ref().map_or_else(
            || grade_name(e.edge.grade).to_owned(),
            |from| format!("from {}", short(&from.src)),
        );
        record(&mut out, "edge", &[glyph, &edge_arrow(e, name), &context]);
    }
    out
}

/// Render a drift report: one `drift` record per suspect, then the `miss`
/// records that say what was not checked.
///
/// The evidence grade leads, as it does on every query-result `edge` record,
/// because a reader's first question about a suspect is how much to believe
/// it. The arrow points backwards (`dependent <-calls- changed`) because that
/// is the direction the implication runs: the changed definition is the
/// cause, the dependent is what may now be wrong.
///
/// A report with no suspects and nothing unchecked prints nothing at all,
/// which is the grammar's "empty result set" — exit 0, no records.
#[must_use]
pub fn drift(report: &grid_wire::DriftReport) -> String {
    let mut out = String::new();
    for s in &report.suspects {
        let columns = drift_columns(s);
        let refs: Vec<&str> = columns.iter().map(String::as_str).collect();
        record(&mut out, "drift", &refs);
    }
    out.push_str(&stale(report.stale_tests.as_deref().unwrap_or_default()));
    out.push_str(&weak(report.weak.as_deref().unwrap_or_default()));
    out.push_str(&warnings(report.warnings.as_deref().unwrap_or_default()));
    out.push_str(&compat(report.compat.as_deref().unwrap_or_default()));
    out.push_str(&drift_unchecked(&report.unchecked));
    out
}

/// Stale verdicts: a `miss` in the cache's own sense — run, not looked up.
/// The first column is the test name, as a test is addressed everywhere.
#[must_use]
pub fn stale(tests: &[grid_wire::StaleTest]) -> String {
    let mut out = String::new();
    for stale in tests {
        let reason = match stale.reason {
            grid_wire::StaleTestReason::ClosureChanged => "closure-changed",
            grid_wire::StaleTestReason::ClosureRejected => "closure-rejected",
        };
        record(&mut out, "miss", &[&stale.test, reason]);
    }
    out
}

/// Weak tests: `warn`, not `drift` — a heuristic ranks and never gates.
#[must_use]
pub fn weak(tests: &[grid_wire::WeakTest]) -> String {
    let mut out = String::new();
    for weak in tests {
        record(
            &mut out,
            "warn",
            &[&weak.test, &format!("weak-for {}", named(&weak.def))],
        );
    }
    out
}

#[must_use]
pub fn warnings(rows: &[grid_wire::DriftWarning]) -> String {
    let mut out = String::new();
    for warning in rows {
        record(&mut out, "warn", &[&warning.subject, &warning.message]);
    }
    out
}

/// The definition's name, or its `@prefix` when the chart cannot name it.
fn named(def: &grid_wire::DefRef) -> String {
    def.callsign
        .as_ref()
        .map_or_else(|| format!("@{}", short(&def.datum)), |c| c.0.clone())
}

/// `grid check`'s ranked tail and certificate, in the order the plan lists
/// them: `warn <def>  uncovered`, `warn <new>  clone-of <existing>`, the
/// check's own warnings, then one `done  refactor  …` when no gate fired.
/// Everything here ranks; the certificate states three facts and never the
/// phrase "behavior-preserving".
#[must_use]
pub fn check_tail(answer: &grid_wire::CheckResult) -> String {
    let mut out = String::new();
    for def in answer.uncovered.iter().flatten() {
        record(&mut out, "warn", &[&named(def), "uncovered"]);
    }
    for clone in answer.clones.iter().flatten() {
        record(
            &mut out,
            "warn",
            &[
                &named(&clone.def),
                &format!("clone-of {}", named(&clone.existing)),
            ],
        );
    }
    out.push_str(&warnings(answer.warnings.as_deref().unwrap_or_default()));
    if let Some(cert) = answer.refactor.as_ref() {
        let interfaces = match cert.interfaces_moved {
            0 => "interfaces held".to_owned(),
            n => format!("{n} interfaces moved"),
        };
        let coverage = match (cert.replayed_pass, cert.uncovered) {
            (Some(replayed), Some(uncovered)) => {
                format!(
                    "{replayed} covering tests replayed pass · {uncovered} changed defs uncovered"
                )
            }
            // No retained trace: neither count is zero, both are unknown.
            _ => "no retained traces".to_owned(),
        };
        record(
            &mut out,
            "done",
            &["refactor", &format!("{interfaces} · {coverage}")],
        );
    }
    out
}

/// `orphan` rows with the trailing `trace N` column (`grid orphans
/// --traced`): how many retained traces hit the definition. Zero is the
/// workspace-level unused-`pub` question; still a maybe, never a verdict.
#[must_use]
pub fn orphans(rows: &[grid_wire::Orphan]) -> String {
    let mut out = String::new();
    for row in rows {
        let d = &row.def;
        let callsign = d.callsign.as_ref().map_or("", |c| c.0.as_str());
        let at = at(d.materialization.as_ref(), "");
        let traces = row
            .traces
            .map_or_else(String::new, |n| format!("trace {n}"));
        record(
            &mut out,
            "orphan",
            &[callsign, &short(&d.datum), &at, &traces],
        );
    }
    out
}

/// Deserialize compatibility findings.
///
/// A declared one is an `inv violated` record, because a `persisted`
/// invariant is what turns "this shape moved" into "bytes on disk cannot be
/// read": modeled evidence, and a gate. An undeclared one is a `warn` —
/// heuristic, ranked, and it never moves the exit code.
#[must_use]
pub fn compat(findings: &[grid_wire::CompatFinding]) -> String {
    let mut out = String::new();
    for finding in findings {
        let named = finding.def.callsign.as_ref().map_or_else(
            || format!("@{}", short(&finding.def.datum)),
            |c| c.0.clone(),
        );
        if finding.declared {
            record(
                &mut out,
                "inv",
                &["violated", &format!("persisted:{named}"), &finding.rule],
            );
        } else {
            record(
                &mut out,
                "warn",
                &[&named, &format!("compat: {}", finding.rule)],
            );
        }
    }
    out
}

/// `grid interfere`: what two worktrees collide over.
///
/// A definition both sides moved is a `warn <callsign>  both-touched` — git's
/// merge will find that one too, and a `DefRef` carries no edge, so it has no
/// grade to put in a `drift` record's evidence column and does not gate. The
/// findings that only grid can see are `drift` records with reason
/// `interfere:<other-root>`, carrying the grade of the edge that found them.
#[must_use]
pub fn interfere(report: &grid_wire::InterfereReport) -> String {
    let mut out = String::new();
    for def in &report.both_touched {
        let named = def
            .callsign
            .as_ref()
            .map_or_else(|| format!("@{}", short(&def.datum)), |c| c.0.clone());
        record(&mut out, "warn", &[&named, "both-touched"]);
    }
    for suspect in &report.suspects {
        let columns = drift_columns(suspect);
        let refs: Vec<&str> = columns.iter().map(String::as_str).collect();
        record(&mut out, "drift", &refs);
    }
    out.push_str(&drift_unchecked(&report.unchecked));
    out
}

/// The compile report (`grid check --compile`): a `drift` record per proven
/// dependent the recorded compile did not see, reason `uncompiled:<cfg>`,
/// then a `warn` per dependent whose cfg nobody can decide. Same columns as
/// `grid drift` — a caller the compiler skipped is a dependent left behind,
/// found by a different question.
#[must_use]
pub fn compile(report: &grid_wire::CompileReport) -> String {
    let mut out = String::new();
    for s in &report.uncompiled {
        let columns = drift_columns(s);
        let refs: Vec<&str> = columns.iter().map(String::as_str).collect();
        record(&mut out, "drift", &refs);
    }
    for warning in &report.unknown {
        record(&mut out, "warn", &[&warning.subject, &warning.message]);
    }
    out
}

/// One `delta` record per signed suspect change: the sign, then the same
/// columns a `drift` record carries.
///
/// Signed rather than a current list, so a consumer never diffs lists to see
/// what changed and the stream's algebra is checkable by summation: the sum
/// after any prefix of events is what a fresh `grid drift` would say.
#[must_use]
pub fn deltas(rows: &[grid_wire::DriftDelta]) -> String {
    let mut out = String::new();
    for row in rows {
        let glyph = if row.weight < 0 { "-" } else { "+" };
        let mut columns = vec![glyph.to_owned()];
        columns.extend(drift_columns(&row.suspect));
        let refs: Vec<&str> = columns.iter().map(String::as_str).collect();
        record(&mut out, "delta", &refs);
    }
    out
}

/// The `miss` records a watch event closes with, one per refused path.
#[must_use]
pub fn declined(paths: &[String]) -> String {
    refused(paths, "unparseable")
}

fn refused(paths: &[String], why: &str) -> String {
    let mut out = String::new();
    for path in paths {
        record(&mut out, "miss", &[path, why]);
    }
    out
}

/// One `covenant` record per declared covenant: the two patterns and the
/// boundary they meet across. A declaration, printed the way it was made.
#[must_use]
pub fn covenants(rows: &[grid_wire::Covenant]) -> String {
    let mut out = String::new();
    for c in rows {
        record(&mut out, "covenant", &[&c.a.0, &c.b.0, &c.via]);
    }
    out
}

/// The repo's standing invariants, as `inv standing <id>` — the same record
/// `grid todo` prints for a plan's own, because a rule that holds for the
/// repo and a rule a plan added are the same kind of thing and are checked
/// by the same gate.
#[must_use]
pub fn invariants(rows: &[grid_wire::Invariant]) -> String {
    let mut out = String::new();
    for inv in rows {
        record(&mut out, "inv", &["standing", &inv.id]);
    }
    out
}

/// The columns a suspect renders as, shared by the `drift` record and the
/// signed `delta` record so the two cannot drift apart.
///
/// Both trailing columns are always present, `-` when there is nothing to
/// say: a column that sometimes vanishes shifts every later one for anything
/// parsing by position, which is the contract.
fn drift_columns(s: &grid_wire::DriftSuspect) -> Vec<String> {
    let name = |d: &grid_wire::DefRef| {
        d.callsign
            .as_ref()
            .map_or_else(|| format!("@{}", short(&d.datum)), |c| c.0.clone())
    };
    let transition = match (&s.sig_before, &s.sig_after) {
        (Some(before), Some(after)) => format!("{}→{}", short(before), short(after)),
        // No sig on one side is the honest unknown that put this suspect in
        // the list at all; the row says so rather than showing half a
        // transition as if it were whole.
        _ => "sig?".to_owned(),
    };
    vec![
        role(grade_name(s.grade)),
        name(&s.dependent),
        format!("<-{}-", relation_name(s.relation)),
        name(&s.changed),
        transition,
        s.reason.clone().unwrap_or_else(|| "-".to_owned()),
        at(s.dependent.materialization.as_ref(), "-"),
    ]
}

/// The `miss` records that close a drift report. Both are refusals, and a
/// refusal that does not print is the dishonest silence the verb exists to
/// avoid — so an unknown count prints too, as unknown.
fn drift_unchecked(unchecked: &grid_wire::DriftUnchecked) -> String {
    let mut out = String::new();
    match unchecked.unresolved_refs {
        Some(0) => {}
        Some(n) => record(
            &mut out,
            "miss",
            &[
                &n.to_string(),
                &match (
                    unchecked.unresolved_external,
                    unchecked.unresolved_method,
                    unchecked.unresolved_internal,
                ) {
                    // The populations mean opposite things; a bare total hides that.
                    (Some(external), Some(method), Some(internal)) => {
                        let macros = unchecked
                            .unresolved_macros
                            .map_or(String::new(), |n| format!(", macro {n}"));
                        let split =
                            format!("external {external}, method {method}, internal {internal}");
                        format!("unresolved references unchecked ({split}{macros})")
                    }
                    _ => "unresolved references unchecked".to_owned(),
                },
            ],
        ),
        None => record(
            &mut out,
            "miss",
            &[
                "?",
                "unresolved references uncounted (head was not surveyed)",
            ],
        ),
    }
    if unchecked.unattributed_entries.is_some_and(|n| n > 0) {
        let count = unchecked
            .unattributed_entries
            .unwrap_or_default()
            .to_string();
        record(&mut out, "miss", &[&count, "unattributed entries"]);
    }
    if unchecked.weak_skipped.is_some_and(|n| n > 0) {
        let count = unchecked.weak_skipped.unwrap_or_default().to_string();
        record(
            &mut out,
            "miss",
            &[&count, "weak-checks skipped (stale coverage)"],
        );
    }
    if unchecked.compat_unchecked.is_some_and(|n| n > 0) {
        let count = unchecked.compat_unchecked.unwrap_or_default().to_string();
        record(
            &mut out,
            "miss",
            &[&count, "compat unchecked (derive not visible)"],
        );
    }
    if unchecked.budget_rejected > 0 {
        record(
            &mut out,
            "miss",
            &[
                &unchecked.budget_rejected.to_string(),
                "inbound walks rejected over budget",
            ],
        );
    }
    out
}

/// `hit` / `miss` records for a cache check (PORCELAIN.md): callsign column
/// carries the datum handle, because a cache check is addressed by identity
/// and a callsign would be ambiguous across time.
#[must_use]
pub fn cache_check(result: &CacheCheckResult) -> String {
    let mut out = String::new();
    for hit in &result.hits {
        let verdict = match hit.verdict {
            CacheHitVerdict::Pass => "pass",
            CacheHitVerdict::Fail => "fail",
        };
        record(&mut out, "hit", &[&hit.datum.0, verdict, &hit.recorded_at]);
    }
    for datum in &result.misses {
        // One reason, always: a miss means run it. Distinguishing "no key" from
        // "unresolvable closure" here would invite treating one as cacheable.
        record(&mut out, "miss", &[&datum.0, "not recorded"]);
    }
    out
}

/// Render resolved definitions as `ref` records: callsign, then the datum
/// prefix that distinguishes them when the callsign does not.
///
/// `root` records: import name, then the directory it roots. The name is
/// first because it is what appears inside every callsign.
#[must_use]
pub fn roots(rows: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, dir) in rows {
        record(&mut out, "root", &[name, dir]);
    }
    out
}

/// `ref` rather than `def` because a `def` record's first column is a change
/// glyph, and a resolved definition did not change — it exists. Overloading
/// `def` here would shift every later column by one for anything parsing by
/// position, which is the whole contract.
#[must_use]
pub fn refs(defs: &[grid_wire::DefRef]) -> String {
    refs_as(defs, "ref")
}

/// The same rows under a different record tag (`orphan`).
#[must_use]
pub fn refs_as(defs: &[grid_wire::DefRef], tag: &str) -> String {
    let mut out = String::new();
    for d in defs {
        let callsign = d.callsign.as_ref().map_or("", |c| c.0.as_str());
        // file:line last, so the row stays pasteable into an editor.
        let at = at(d.materialization.as_ref(), "");
        record(&mut out, tag, &[callsign, &short(&d.datum), &at]);
    }
    out
}

/// One row per hotspot: `hot  callsign  cx N  edits M  file:line`.
#[must_use]
pub fn hotspots(rows: &[crate::rpc::Hotspot]) -> String {
    let mut out = String::new();
    for r in rows {
        let at = at(r.materialization.as_ref(), "");
        record(
            &mut out,
            "hot",
            &[
                &r.callsign,
                &format!("cx {}", r.complexity),
                &format!("edits {}", r.edits),
                &at,
            ],
        );
    }
    out
}

/// One row per edit, newest first: `edit  @from -> @to  rev`.
#[must_use]
pub fn edits(rows: &[grid_wire::Edit]) -> String {
    let mut out = String::new();
    for edit in rows {
        let rev = edit
            .git_rev
            .as_ref()
            .map(|r| r.0.clone())
            .unwrap_or_default();
        record(
            &mut out,
            "edit",
            &[
                &format!("{} -> {}", short(&edit.from), short(&edit.to)),
                &rev,
            ],
        );
    }
    out
}

/// `done` for what was stored, `warn` per refusal. A rejection is not an
/// error — it is the cache declining to lie — so it prints and exits 0.
#[must_use]
pub fn cache_record(result: &CacheRecordResult) -> String {
    let mut out = String::new();
    let stored = result.recorded.to_string();
    let total = (result.recorded + result.rejected.len() as u64).to_string();
    record(&mut out, "done", &[&format!("{stored}/{total}")]);
    for item in &result.rejected {
        record(&mut out, "warn", &[&item.datum.0, &item.reason]);
    }
    for item in result.nondeterministic.iter().flatten() {
        record(
            &mut out,
            "warn",
            &[&item.datum.0, &nondeterministic(item, result_verdict(item))],
        );
    }
    out
}

/// The verdict a nondeterministic record just stored is the opposite of the
/// one it found, by definition of the finding.
fn result_verdict(
    item: &grid_wire::methods::CacheRecordResultNondeterministicItem,
) -> &'static str {
    use grid_wire::methods::CacheRecordResultNondeterministicItemPrevious as P;
    match item.previous {
        P::Pass => "fail",
        P::Fail => "pass",
    }
}

/// The `nondeterministic` message for `grid test`'s `warn` record: the same
/// words as the cache verb's, so the two cannot drift apart.
#[must_use]
pub fn nondeterministic_message(
    item: &grid_wire::methods::CacheRecordResultNondeterministicItem,
) -> String {
    nondeterministic(item, result_verdict(item))
}

/// The `nondeterministic` message: the claim is exactly "not the charted
/// code" — same closure key, same environment, two answers. The word is
/// never *flake*: a race is code, and grid does not know which.
fn nondeterministic(
    item: &grid_wire::methods::CacheRecordResultNondeterministicItem,
    now: &str,
) -> String {
    use grid_wire::methods::CacheRecordResultNondeterministicItemPrevious as P;
    let previous = match item.previous {
        P::Pass => "pass",
        P::Fail => "fail",
    };
    format!(
        "nondeterministic  same closure key, same env: {previous} {}, {now} now",
        item.previous_at
    )
}

/// Render a `grid test` run: `hit` per replayed verdict, `miss` per selected
/// test with the reason it must run, `test` per outcome, `warn` per finding,
/// and the closing `done  tests R/N ran · H replayed · F failed`.
///
/// The first column of `hit` and `miss` is the test id here rather than a
/// datum handle: a test is addressed by the runner's name everywhere in this
/// verb, and the datum rides in `--json`.
#[must_use]
pub fn test_run(run: &grid_wire::TestRun) -> String {
    let mut out = String::new();
    for hit in &run.plan.replayed {
        let verdict = match hit.verdict {
            CacheHitVerdict::Pass => "pass",
            CacheHitVerdict::Fail => "fail",
        };
        let test = hit
            .test
            .as_ref()
            .map_or(hit.datum.0.as_str(), |t| t.0.as_str());
        record(&mut out, "hit", &[test, verdict, &hit.recorded_at]);
    }
    for sel in &run.plan.selected {
        let why = match sel.why {
            grid_wire::TestSelectionWhy::Reached => "closure-changed",
            grid_wire::TestSelectionWhy::NoTrace => "no-trace",
            // An unresolved test always runs, which is right and says
            // nothing; the reader still deserves to know *why* the chart
            // cannot name it, because "the module is charted and this name
            // is not" is a fact about the test and "nothing here is charted"
            // is one about the survey.
            grid_wire::TestSelectionWhy::Unresolved => match sel.unresolved {
                Some(grid_wire::TestSelectionUnresolved::Ambiguous) => "unresolved:ambiguous",
                Some(grid_wire::TestSelectionUnresolved::ModuleCharted) => {
                    "unresolved:module-charted"
                }
                Some(grid_wire::TestSelectionUnresolved::NotCharted) => "unresolved:not-charted",
                None => "unresolved",
            },
            grid_wire::TestSelectionWhy::Aground => "aground",
            grid_wire::TestSelectionWhy::NotRecorded => "not-recorded",
            grid_wire::TestSelectionWhy::Unwatched => "unwatched",
        };
        record(&mut out, "miss", &[&sel.test.0, why]);
    }
    for outcome in &run.ran {
        // A test is expected to pass; the column says what it did.
        let (verdict, actual) = match outcome.verdict {
            grid_wire::TestOutcomeVerdict::Pass => ("ok", "pass"),
            grid_wire::TestOutcomeVerdict::Fail => ("fail", "fail"),
        };
        record(
            &mut out,
            "test",
            &[&role(verdict), &outcome.test.0, &format!("pass={actual}")],
        );
    }
    for warning in &run.warnings {
        record(&mut out, "warn", &[&warning.subject, &warning.message]);
    }
    let total = run.plan.selected.len() + run.plan.replayed.len();
    // Failed is the exit code's own count: run failures plus replayed
    // `fail` verdicts, so the line and the exit cannot disagree.
    record(
        &mut out,
        "done",
        &[&format!(
            "tests {}/{total} ran · {} replayed · {} failed",
            run.ran.len(),
            run.plan.replayed.len(),
            crate::testrun::failed(run)
        )],
    );
    out
}

/// The arrow every edge is printed as, changed or not.
///
/// Callsigns are the default identity (PORCELAIN.md display policy). A datum
/// the chart cannot name falls back to `@<prefix>`, still a coordinate a user
/// can paste back in — never a bare 64-hex digest, which is neither.
#[must_use]
pub fn chart_edge_arrow(edge: &grid_wire::Edge, name: &NameOf) -> String {
    let show = |d: &grid_wire::Datum| name(d).unwrap_or_else(|| format!("@{}", short(d)));
    format!(
        "{} -{}-> {}",
        show(&edge.src),
        relation_name(edge.relation),
        show(&edge.dst)
    )
}

/// `edge` records whose first column is the evidence grade, for query
/// results (`scope`, `uses`): these edges are neither changes (no glyph)
/// nor verdicts (no role), and the grammar's evidence column moves to the
/// front for exactly that case (PORCELAIN.md, edge record).
fn edge_records(out: &mut String, edges: &[grid_wire::Edge], name: &NameOf) {
    for edge in edges {
        record(
            out,
            "edge",
            &[&role(grade_name(edge.grade)), &chart_edge_arrow(edge, name)],
        );
    }
}

/// Render a scope pack: `ref` records for the definitions (they exist;
/// nothing changed), `edge` records for the chart around them, and `inv`
/// records for the standing rules in scope. Anchors and churn print nothing
/// while their stores do not exist — an empty section is a true answer, but
/// a record type with no records is not a column shift.
#[must_use]
pub fn scope(pack: &grid_wire::ScopePack, name: &NameOf) -> String {
    let mut out = refs(&pack.defs);
    // An edge wholly inside the scope is both an in-edge and an out-edge;
    // print it once.
    let mut edges = pack.in_edges.clone();
    edges.extend(
        pack.out_edges
            .iter()
            .filter(|e| !pack.in_edges.contains(e))
            .cloned(),
    );
    edges.sort_by(|a, b| {
        (&a.src.0, a.relation as u8, &a.dst.0).cmp(&(&b.src.0, b.relation as u8, &b.dst.0))
    });
    edge_records(&mut out, &edges, name);
    for inv in &pack.invariants {
        record(&mut out, "inv", &["standing", &inv.id]);
    }
    out
}

/// Render a scope pack that carries text: a document, not records. Headers
/// name what each block is and how far out it sits (`## <callsign>
/// <file:line>  (body, hop k)`, `## sig <callsign>  <file:line>  (hop k)`),
/// and one `# horizon:` line closes it with the depth each tier is complete
/// to and the token estimate. PORCELAIN.md's text-mode paragraph: the prose
/// is the answer here and `--json` is the record form.
///
/// The horizon line is unconditional. A pack that packed nothing still says
/// so — a bounded answer whose bound is missing is worse than no answer.
#[must_use]
pub fn scope_document(pack: &grid_wire::ScopePack) -> String {
    let mut out = String::new();
    let at = |t: &grid_wire::PackText| at(t.def.materialization.as_ref(), "-");
    let name = |t: &grid_wire::PackText| {
        t.def
            .callsign
            .as_ref()
            .map_or_else(|| t.def.datum.0.clone(), |c| c.0.clone())
    };
    for text in pack.bodies.iter().flatten() {
        let _ = writeln!(
            out,
            "## {}  {}  (body, hop {})",
            name(text),
            at(text),
            text.hop
        );
        out.push_str(&text.text);
        out.push('\n');
    }
    for text in pack.sigs.iter().flatten() {
        let _ = writeln!(
            out,
            "## sig  {}  {}  (hop {})",
            name(text),
            at(text),
            text.hop
        );
        out.push_str(&text.text);
        if !text.text.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    if let Some(h) = &pack.horizon {
        let hops = |n: Option<u64>| match n {
            None => "none".to_owned(),
            Some(1) => "1 hop".to_owned(),
            Some(n) => format!("{n} hops"),
        };
        let _ = writeln!(
            out,
            "# horizon: bodies complete through {}, sigs through {}, ≈{} tokens (bytes/4)",
            hops(h.bodies_complete_within),
            h.sigs_complete_within
                .map_or_else(|| "none".to_owned(), |n| n.to_string()),
            h.approx_tokens
        );
    }
    out
}

/// Render `grid uses`: the proven inbound edges of what the name resolved
/// to, evidence first.
#[must_use]
pub fn uses(edges: &[grid_wire::Edge], name: &NameOf) -> String {
    let mut out = String::new();
    edge_records(&mut out, edges, name);
    out
}

/// Render a worklist: one `done` record carrying the overall fraction and
/// the per-kind split (the CLI plan's `done 3/5 promises · edges 2/3 ·
/// defs 1/1 · tests 0/1`, split at the column boundary), then every pending
/// promise rendered exactly as the plan store renders a fresh one — a
/// promise is the same work whether it is being planned or chased.
#[must_use]
pub fn todo(list: &grid_wire::TodoList) -> String {
    let mut out = String::new();
    let split = |done: usize, pending: usize| format!("{done}/{}", done + pending);
    let detail = format!(
        "edges {} · defs {} · tests {}",
        split(list.edges_done.len(), list.edges_pending.len()),
        split(list.defs_done.len(), list.defs_pending.len()),
        split(list.tests_done.len(), list.tests_pending.len()),
    );
    record(
        &mut out,
        "done",
        &[
            &format!("{}/{}", list.fraction.num, list.fraction.den),
            &detail,
        ],
    );
    for d in &list.defs_pending {
        let action = match d.action {
            grid_wire::PlanDefPromiseAction::Add => "+",
            grid_wire::PlanDefPromiseAction::Remove => "-",
            grid_wire::PlanDefPromiseAction::Modify => "~",
            grid_wire::PlanDefPromiseAction::Move => ">",
        };
        record(&mut out, "def", &[action, &d.callsign.0]);
    }
    for e in &list.edges_pending {
        let glyph = match e.action {
            grid_wire::PlanEdgePromiseAction::Add => "+",
            grid_wire::PlanEdgePromiseAction::Remove => "-",
            grid_wire::PlanEdgePromiseAction::Move => ">",
        };
        record(&mut out, "edge", &[glyph, &promise_arrow(e)]);
    }
    for t in &list.tests_pending {
        record(&mut out, "todo", &["test", t]);
    }
    out
}

/// Render applied edits as `def` records: `~` with the datum transition for
/// a replacement, `+` for what a creation brought into being. A created
/// file that defines nothing prints its path — the path is the change.
#[must_use]
pub fn edit_applied(edits: &[crate::edit::EditDetail]) -> String {
    let mut out = String::new();
    for detail in edits {
        match detail {
            crate::edit::EditDetail::Modified { callsign, was, now } => record(
                &mut out,
                "def",
                &["~", callsign, &format!("{}→{}", short(was), short(now))],
            ),
            crate::edit::EditDetail::CreatedDef { callsign, now } => {
                record(&mut out, "def", &["+", callsign, &short(now)]);
            }
            crate::edit::EditDetail::CreatedFile { path } => {
                record(&mut out, "def", &["+", path]);
            }
        }
    }
    out
}

/// Render CAS conflicts: the target as typed, then `was→now` — the stale
/// read and the identity that replaced it, both pasteable.
#[must_use]
pub fn edit_conflicts(conflicts: &[grid_wire::EditResultConflictsItem]) -> String {
    let mut out = String::new();
    for c in conflicts {
        let was = c.was.as_ref().map_or_else(|| "?".to_string(), short);
        record(
            &mut out,
            "conflict",
            &[&c.target, &format!("{was}→{}", short(&c.now))],
        );
    }
    out
}

/// `done` / `warn` records for a trace ingest: how much of the trace became
/// edges, and every symbol that did not (C15 — returned, never dropped).
#[must_use]
pub fn trace_ingest(result: &TraceIngestResult, total: u64) -> String {
    let mut out = String::new();
    record(
        &mut out,
        "done",
        &[&format!("{}/{}", result.resolved, total)],
    );
    for symbol in &result.unresolved_symbols {
        record(&mut out, "warn", &[symbol, "unresolved"]);
    }
    out
}
/// `miss` / `done` records for a trace transport: every id the destination
/// already holds under other bytes, then what the transport did.
///
/// Existing tags, as `grid test` uses existing tags: a refused id is a `miss`
/// with its reason, and the closing `done` carries the counts. The first
/// column of the `miss` is a trace id — a run's own name, which is what this
/// verb addresses things by.
#[must_use]
pub fn trace_transport(
    moved: &grid_trace::Moved,
    exporting: bool,
    dir: &std::path::Path,
) -> String {
    let mut out = String::new();
    for id in &moved.conflicts {
        record(
            &mut out,
            "miss",
            &[id, "differs from the trace already there"],
        );
    }
    let verb = if exporting { "exported" } else { "imported" };
    record(
        &mut out,
        "done",
        &[&format!(
            "traces {} {verb} · {} unchanged · {} refused  {}",
            moved.written.len(),
            moved.unchanged.len(),
            moved.conflicts.len(),
            dir.display()
        )],
    );
    out
}

/// The `fork` record of a trace diff: where two runs of the same test parted.
/// Empty when they never did.
#[must_use]
pub fn fork(diff: &grid_wire::TraceDiff) -> String {
    let mut out = String::new();
    if let Some(fork) = &diff.first_divergence {
        let callee =
            |c: &Option<grid_wire::Datum>| c.as_ref().map_or_else(|| "-".to_string(), short);
        record(
            &mut out,
            "fork",
            &[
                &short(&fork.caller),
                &callee(&fork.green_callee),
                &callee(&fork.red_callee),
            ],
        );
    }
    out
}

/// Ranked `hit` records for `grid suspects`, in either evidence mode.
///
/// The second column is the evidence: an Ochiai score, or the word `closure`
/// when the row was ranked by hop distance instead. The third says why it
/// ranked; the callsign and `file:line` close the row (`-` when the chart
/// no longer holds the datum), so it can be acted on as printed.
#[must_use]
pub fn suspects(result: &grid_wire::Suspects) -> String {
    let mut out = String::new();
    for item in &result.ranked {
        let mode = match (item.hops, item.score) {
            (Some(_), _) => "closure".to_owned(),
            (None, score) => score.map_or_else(|| "-".to_owned(), |s| format!("{s:.6}")),
        };
        let callsign = item.callsign.as_ref().map_or("-", |c| c.0.as_str());
        let at = at(item.materialization.as_ref(), "-");
        record(
            &mut out,
            "hit",
            &[&item.datum.0, &mode, &item.via, callsign, &at],
        );
    }
    out
}

/// `fork` / `edge` records for a trace diff: where the traces parted, then
/// what each does that the other never did. Handles are datum-short because a
/// trace is addressed by content, like a cache verdict.
#[must_use]
pub fn trace_diff(diff: &grid_wire::TraceDiff) -> String {
    let mut out = fork(diff);
    let arrow = |e: &grid_wire::Edge| {
        format!(
            "{} -{}-> {}",
            short(&e.src),
            relation_name(e.relation),
            short(&e.dst)
        )
    };
    for e in &diff.removed {
        record(&mut out, "edge", &["-", &arrow(e), grade_name(e.grade)]);
    }
    for e in &diff.added {
        record(&mut out, "edge", &["+", &arrow(e), grade_name(e.grade)]);
    }
    out
}

/// `hit` then `done` for a bisect: the culprit, then the search that found it.
/// The tag is `grid suspects`'s, because the two verbs answer one question —
/// which change broke this test — from different evidence.
#[must_use]
pub fn bisect(result: &grid_wire::BisectResult) -> String {
    let mut out = String::new();
    if let Some(culprit) = &result.culprit {
        record(&mut out, "hit", &[&culprit.0, "culprit"]);
    }
    let steps = format!("steps {}", result.steps);
    let hits = format!("cache_hits {}", result.cache_hits);
    record(&mut out, "done", &[&steps, &hits]);
    out
}
