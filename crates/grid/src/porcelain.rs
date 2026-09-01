//! The porcelain renderer: typed records on stdout, one per line.
//!
//! Normative grammar in `docs/PORCELAIN.md`. Fields are
//! separated by runs of spaces and column order is stable per record type, so
//! `grep '^edge  miss'` is the explanation and `awk` is the parser. Nothing
//! here writes to stderr: hints are the caller's business.

use grid_wire::methods::{CacheRecordResult, WakeIngestResult};
use grid_wire::{
    CacheCheckResult, CacheHitVerdict, DefChangeAction, EdgeChange, LintResult,
    TestComparisonActual, TestComparisonExpected, TrackEdgePromise, XteVerdict, XteVerdictGate,
};
use grid_xte::lint::{relation_name, render_edge};

/// The porcelain grammar version this renderer implements.
pub const VERSION: &str = "1.4.0";

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

/// An edge arrow in callsign space.
///
/// Callsigns are the default identity (PORCELAIN.md display policy), so an
/// arrow prints names whenever the chart can supply them. A datum the chart
/// cannot name falls back to `@<prefix>`, which is still a coordinate a user
/// can paste back in — never a bare 64-hex digest, which is neither readable
/// nor typeable.
fn edge_arrow(change: &EdgeChange, name: &NameOf) -> String {
    let show = |d: &grid_wire::Datum| name(d).unwrap_or_else(|| format!("@{}", short(d)));
    format!(
        "{} -{}-> {}",
        show(&change.edge.src),
        relation_name(change.edge.relation),
        show(&change.edge.dst)
    )
}

/// How the renderer asks the chart what a datum is called. `None` means the
/// chart has no name for it, which is a real answer: a datum can outlive the
/// name it had.
pub type NameOf<'a> = dyn Fn(&grid_wire::Datum) -> Option<String> + 'a;

/// Promise arrows drop the leading action word: the verdict role column
/// already says what happened to the promise.
fn promise_arrow(p: &TrackEdgePromise) -> String {
    let rendered = render_edge(p);
    rendered
        .split_once(' ')
        .map_or(rendered.clone(), |(_, rest)| rest.to_string())
}

const fn zoc_name(z: grid_wire::Zoc) -> &'static str {
    use grid_wire::Zoc as Z;
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

/// Render a gate verdict. The first record is `pass` or `hold` with the track
/// and snapshot; every divergence follows as its own record.
#[must_use]
pub fn verdict(v: &XteVerdict, name: &NameOf) -> String {
    let mut out = String::new();
    let head = grid_xte::snapshot_display(&v.head);
    let tag = match v.gate {
        XteVerdictGate::Pass => "pass",
        XteVerdictGate::Hold => "hold",
    };
    record(&mut out, tag, &[&v.track_id, &head]);

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
            &[&role("surplus"), &edge_arrow(c, name), zoc_name(c.edge.zoc)],
        );
    }
    for c in &v.tolerated {
        record(
            &mut out,
            "edge",
            &[
                &role("tolerated"),
                &edge_arrow(c, name),
                zoc_name(c.edge.zoc),
            ],
        );
    }
    for d in &v.defs_unaccounted {
        record(&mut out, "def", &[def_glyph(d.action), &d.callsign.0]);
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

/// Render a track: its promises as `todo` records, since a promise is work
/// not yet made good, plus its standing invariants.
#[must_use]
pub fn track(t: &grid_wire::Track) -> String {
    let mut out = String::new();
    let base = match &t.base {
        grid_wire::SnapshotOrRev::GitRev { git_rev } => git_rev.0.clone(),
        grid_wire::SnapshotOrRev::Snapshot { snapshot } => grid_xte::snapshot_display(snapshot),
    };
    record(&mut out, "todo", &[&t.track_id, &base]);
    for d in &t.delta.defs {
        let action = match d.action {
            grid_wire::TrackDefPromiseAction::Add => "+",
            grid_wire::TrackDefPromiseAction::Remove => "-",
            grid_wire::TrackDefPromiseAction::Modify => "~",
            grid_wire::TrackDefPromiseAction::Move => ">",
        };
        record(&mut out, "def", &[action, &d.callsign.0]);
    }
    for e in &t.delta.edges {
        let glyph = match e.action {
            grid_wire::TrackEdgePromiseAction::Add => "+",
            grid_wire::TrackEdgePromiseAction::Remove => "-",
            grid_wire::TrackEdgePromiseAction::Move => ">",
        };
        record(&mut out, "edge", &[glyph, &promise_arrow(e)]);
    }
    // A track's tests are promises, not comparisons: there is no actual
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
/// is the exit code: a track with errors exits 2, a track with only warnings
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
    // Displayed identity is the rev, never the raw content id (CLI plan §2).
    let id = format!("@{}", s.git_rev.0);
    // The full stats block is `defs N  edges N  reread N  <duration>`. The
    // last two describe the run rather than the snapshot, so they appear only
    // when a run just happened — reporting a snapshot read back from the
    // store would have nothing truthful to put there.
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
    out
}

/// Render a delta as `def` and `edge` records.
///
/// This is the reference render from the CLI plan: a glyph column, the
/// callsign, and a trailing column that is the datum transition for a def and
/// the evidence grade for an edge.
#[must_use]
pub fn diff(made_good: &grid_wire::MadeGood, name: &NameOf) -> String {
    let mut out = String::new();
    for d in &made_good.defs {
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
    for e in &made_good.edges {
        let glyph = match e.action {
            grid_wire::EdgeChangeAction::Added => "+",
            grid_wire::EdgeChangeAction::Removed => "-",
            grid_wire::EdgeChangeAction::Moved => ">",
        };
        // A moved edge's trailing column is where it came from; anything else
        // reports the evidence that establishes it.
        let context = e.moved_from.as_ref().map_or_else(
            || zoc_name(e.edge.zoc).to_owned(),
            |from| format!("from {}", short(&from.src)),
        );
        record(&mut out, "edge", &[glyph, &edge_arrow(e, name), &context]);
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
        let at = d
            .materialization
            .as_ref()
            .map(|m| format!("{}:{}", m.file, m.line))
            .unwrap_or_default();
        record(&mut out, tag, &[callsign, &short(&d.datum), &at]);
    }
    out
}

/// One row per hotspot: `hot  callsign  cx N  legs M  file:line`.
#[must_use]
pub fn hotspots(rows: &[crate::rpc::Hotspot]) -> String {
    let mut out = String::new();
    for r in rows {
        let at = r
            .materialization
            .as_ref()
            .map(|m| format!("{}:{}", m.file, m.line))
            .unwrap_or_default();
        record(
            &mut out,
            "hot",
            &[
                &r.callsign,
                &format!("cx {}", r.complexity),
                &format!("legs {}", r.legs),
                &at,
            ],
        );
    }
    out
}

/// One row per leg, newest first: `leg  @from -> @to  rev`.
#[must_use]
pub fn legs(rows: &[grid_wire::Leg]) -> String {
    let mut out = String::new();
    for leg in rows {
        let rev = leg
            .git_rev
            .as_ref()
            .map(|r| r.0.clone())
            .unwrap_or_default();
        record(
            &mut out,
            "leg",
            &[&format!("{} -> {}", short(&leg.from), short(&leg.to)), &rev],
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
    out
}

/// An edge arrow for a chart edge (not a change): same shape as
/// `edge_arrow`, but nothing was added or removed — the edge *is*.
fn chart_edge_arrow(edge: &grid_wire::Edge, name: &NameOf) -> String {
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
            &[&role(zoc_name(edge.zoc)), &chart_edge_arrow(edge, name)],
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
/// promise rendered exactly as the track store renders a fresh one — a
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
            grid_wire::TrackDefPromiseAction::Add => "+",
            grid_wire::TrackDefPromiseAction::Remove => "-",
            grid_wire::TrackDefPromiseAction::Modify => "~",
            grid_wire::TrackDefPromiseAction::Move => ">",
        };
        record(&mut out, "def", &[action, &d.callsign.0]);
    }
    for e in &list.edges_pending {
        let glyph = match e.action {
            grid_wire::TrackEdgePromiseAction::Add => "+",
            grid_wire::TrackEdgePromiseAction::Remove => "-",
            grid_wire::TrackEdgePromiseAction::Move => ">",
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

/// `done` / `warn` records for a wake ingest: how much of the trace became
/// edges, and every symbol that did not (C15 — returned, never dropped).
#[must_use]
pub fn wake_ingest(result: &WakeIngestResult, total: u64) -> String {
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
/// `fork` / `edge` records for a wake diff: where the traces parted, then
/// what each does that the other never did. Handles are datum-short because a
/// wake is addressed by content, like a cache verdict.
#[must_use]
pub fn wake_diff(diff: &grid_wire::WakeDiff) -> String {
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
    let arrow = |e: &grid_wire::Edge| {
        format!(
            "{} -{}-> {}",
            short(&e.src),
            relation_name(e.relation),
            short(&e.dst)
        )
    };
    for e in &diff.removed {
        record(&mut out, "edge", &["-", &arrow(e), zoc_name(e.zoc)]);
    }
    for e in &diff.added {
        record(&mut out, "edge", &["+", &arrow(e), zoc_name(e.zoc)]);
    }
    out
}
