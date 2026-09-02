//! `grid check` — everything the change did that nobody checked, in one call.
//!
//! The composite. Each finding grid can make about a change is its own verb;
//! an agent needs one call after an edit and one exit code. This module
//! composes those verbs and adds nothing of its own except the join:
//!
//! - **gates** (any → exit 3): drift suspects, the compile record's
//!   uncompiled callers, adherence to a plan when one is named, stale
//!   verdicts, covenant fires (they are drift suspects at `modeled`), and a
//!   compat finding on a type declared `persisted`;
//! - **ranked tail** (never moves the exit): weak tests, uncovered
//!   definitions, clone-of, undeclared compat findings;
//! - a **refactor certificate** — one `done` record over the same data when
//!   no gate fired: interfaces held or moved, covering tests replayed pass,
//!   changed definitions uncovered. Three facts; the phrase
//!   "behavior-preserving" appears nowhere.
//!
//! The composition rule: every section is the corresponding verb's own
//! result — [`grid_wire::DriftReport`] from `grid.drift`,
//! [`grid_wire::CompileReport`] from `grid check --compile`'s record,
//! [`grid_wire::GateVerdict`] from `grid.gate.check`, the stale, weak and
//! compat lists from `grid drift --tests` / `--compat`, uncovered from
//! `grid.test.plan`'s walk — never a re-implementation. A section that
//! refuses (aground, unknown snapshot, unreadable store) refuses the whole
//! check with that reason, never a check with a section silently missing.
//!
//! With `--plan`, the verdict is the gate's and `--why` carries the
//! reasoning: for every promise the gate matched, the paths through the chart
//! that make it good. Only fulfilling paths ship — a promise with no path
//! under the budget carries no `hops` entry rather than an empty one, because
//! an item here means "this is why the gate said match". Traversal is
//! [`grid_walk::query`], the engine `grid.query.walk` serves, so a path a
//! reviewer sees is a path the query verb would produce, at the grade the
//! chart stored for it.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use grid_chart::{DatumId, Ordinal, Segment};
use grid_gate::DeltaProvider as _;
use grid_wire::methods::{CheckParams, DriftParams};
use grid_wire::{
    Callsign, CheckResult, CheckWhy, CheckWhyFulfillingItem, CloneFinding, Datum, DefChangeAction,
    DefRef, DefRefMaterialization, DriftWarning, Fuel, GateVerdictGate, Grade, Locator,
    PlanEdgePromise, RefactorCertificate, SnapshotId, SnapshotOrRev, TestId, TestSelectionWhy,
    WalkQuery, WalkQueryDirection,
};

use crate::chart_provider::{self, ChartDelta};
use crate::{rpc, standing};

/// Budget for one promise's path search.
///
/// A review is interactive, so the walk is bounded by default rather than by
/// the caller remembering to bound it. Four hops is the honest reach for
/// "these calls make that promise good": beyond it a path stops being an
/// explanation a reviewer can hold in their head.
const MAX_HOPS: u64 = 4;
const MAX_NODES: u64 = 4096;
const MAX_MS: u64 = 250;

/// How many endpoints one pattern may expand to before the search stops
/// widening. A `**` pattern over a large repo would otherwise turn one
/// promise into a cross product; the cap is deterministic (candidates are
/// sorted before it applies), so two runs cut the same set.
const MAX_ENDPOINTS: usize = 32;

/// The base a plan-less check measures from: `grid drift`'s default.
const DEFAULT_BASE: &str = "HEAD~1";

/// Where `--quick` remembers the lines it printed last time, so it can tell
/// a count that moved from one that did not. Engine state under `.grid`,
/// safe to delete: losing it costs one repeated line.
const QUICK_STATE: &str = ".grid/check-quick.last";

/// Is this one of the two lines a `--quick` hook prints unchanged every
/// turn — the drift section's declined-reference count, and the check's own
/// "no compile record" warning?
///
/// Matched on the rendered line rather than on the wire fields because what
/// repeats is the *text*: two runs whose counts differ by one produce
/// different lines, and that is exactly the difference worth printing.
fn repeated(line: &str) -> bool {
    let mut columns = line.split_whitespace();
    match columns.next() {
        Some("miss") => line.contains("unresolved references "),
        Some("warn") => columns.next() == Some("-") && line.contains("no compile record since @"),
        _ => false,
    }
}

/// `--quick`'s repeat filter: drop the two lines above when their content is
/// what this store printed last time.
///
/// A `PostToolUse` hook fires once per edit and those two lines are the same
/// text every turn — two lines of unchanged context into an agent's window,
/// forever. The counts are honest and worth saying; saying the same ones
/// again says nothing. What is remembered is a hash of the lines themselves,
/// so a count that *moved* — one more declined reference, a compile record
/// that just went stale — is printed. A state file grid cannot read or write
/// suppresses nothing: forgetting must cost a repeated line, never a dropped
/// one. The full `grid check` never calls this, and `--json` is the record
/// form and is never filtered.
#[must_use]
pub fn quiet(text: &str, root: &Path) -> String {
    let lines: Vec<&str> = text.lines().filter(|line| repeated(line)).collect();
    if lines.is_empty() {
        return text.to_owned();
    }
    let now = blake3::hash(lines.join("\n").as_bytes())
        .to_hex()
        .to_string();
    let path = root.join(QUICK_STATE);
    if std::fs::read_to_string(&path).is_ok_and(|last| last.trim() == now) {
        let mut out = String::new();
        for line in text.lines().filter(|line| !repeated(line)) {
            out.push_str(line);
            out.push('\n');
        }
        return out;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, now + "\n");
    text.to_owned()
}

/// Why a check could not be produced.
#[derive(Debug)]
pub enum Error {
    /// The chart could not answer (unknown snapshot, failed survey, unknown
    /// plan).
    Chart(chart_provider::Error),
    /// A trace id was supplied and the log does not hold it. Coverage without
    /// the trace it names would be a different question silently answered.
    UnknownTrace(String),
    /// The drift section refused — aground, an unreadable trace store or
    /// verdict log, an endpoint that does not resolve, a declaration file
    /// that does not parse. Carried whole so the exit is drift's own.
    Drift(crate::drift::Error),
    /// A section grid cannot stand behind: the coverage walk ran past its
    /// fuel, or a store it needed would not open. Refused whole rather than
    /// served without the section.
    Refused(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chart(e) => write!(f, "{e}"),
            Self::Drift(e) => write!(f, "{e}"),
            Self::UnknownTrace(id) => write!(f, "no retained trace `{id}`"),
            Self::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for Error {}

impl From<chart_provider::Error> for Error {
    fn from(e: chart_provider::Error) -> Self {
        Self::Chart(e)
    }
}

/// `grid.check` — the composite.
///
/// Drift runs first because it surveys the endpoints; every later section
/// reads the snapshots it published, named by id, so nothing is surveyed
/// twice and no section can see a different tree.
///
/// # Errors
/// [`Error::Chart`] when the plan, the base or the head cannot be resolved;
/// [`Error::Drift`] when the drift or compile section refused;
/// [`Error::Refused`] when the coverage walk refused;
/// [`Error::UnknownTrace`] when a supplied trace id names nothing retained.
pub fn answer(params: &CheckParams, root: &Path) -> Result<CheckResult, Error> {
    let quick = params.quick == Some(true);
    let plan = match params.plan.as_ref() {
        Some(id) => {
            let plan = crate::plan::load(&id.0).map_err(chart_provider::Error::Plan)?;
            Some(standing::fold(
                &plan,
                standing::load(root).map_err(chart_provider::Error::Config)?,
            ))
        }
        None => None,
    };
    let base = params
        .base
        .clone()
        .or_else(|| plan.as_ref().map(|p| p.base.clone()))
        .unwrap_or_else(|| SnapshotOrRev::GitRev {
            git_rev: grid_wire::GitRev(DEFAULT_BASE.to_owned()),
        });

    // The drift section, whole: `grid drift --tests --compat`'s own report,
    // with the three lists it carries lifted into their own sections. Quick
    // is drift at the proven floor — no trace store, verdict log or source
    // read — plus the compile record.
    //
    // The endpoints are surveyed first, then the store is opened *once* and
    // both epochs mapped and diffed *once*; every section below takes that
    // chart and that delta rather than asking the store the same three
    // questions again. The order is forced, not incidental: surveying head
    // publishes an epoch, so a store opened before the survey would answer
    // about the tree from before the edit.
    let drift_params = DriftParams {
        base: base.clone(),
        head: params.head.clone(),
        floor: quick.then_some(Grade::Proven),
        plan: plan.clone(),
        tests: (!quick).then_some(true),
        compat: (!quick).then_some(true),
    };
    let ends = rpc::drift_endpoints(&drift_params, root).map_err(Error::Drift)?;
    let provider = ChartDelta::open(root)?;
    let opened =
        crate::drift::Resolved::open(&provider, &ends.base, &ends.head).map_err(Error::Drift)?;
    let mut drift =
        rpc::drift_on(root, &provider, &opened, &ends, &drift_params).map_err(Error::Drift)?;
    let stale = drift.stale_tests.take();
    let weak = drift.weak.take();
    let compat = drift.compat.take();
    let head_id = drift.head.clone();
    let at = |id: &SnapshotId| SnapshotOrRev::Snapshot {
        snapshot: id.clone(),
    };

    // The compile record is read, never written, here: `--compile` wrote it
    // before asking. A record that names another snapshot than the one this
    // check surveyed describes a compile of a different tree, and gates
    // nothing; the warning says so rather than the section going quiet.
    let mut warnings = Vec::new();
    let compile = match crate::compile::load(root).map_err(chart_provider::Error::Config)? {
        Some(record) if record.snapshot == drift.head => {
            Some(crate::compile::report_on(&provider, &opened, &record).map_err(Error::Drift)?)
        }
        _ => {
            warnings.push(DriftWarning {
                subject: "-".to_owned(),
                message: format!("no compile record since @{}", prefix_of(&drift.head.0)),
            });
            None
        }
    };

    let (verdict, why) = match plan.as_ref() {
        Some(plan) => adherence(&at(&head_id), params.trace.as_ref(), plan, root, &provider)?,
        None => (None, None),
    };

    let mut result = CheckResult {
        verdict,
        why,
        drift: Some(drift),
        compile,
        stale,
        weak,
        uncovered: None,
        clones: None,
        compat,
        refactor: None,
        warnings: (!warnings.is_empty()).then_some(warnings),
    };
    if quick {
        return Ok(result);
    }

    let (base_epoch, head_epoch, diff) = (&opened.base, &opened.head, &opened.diff);
    result.clones = Some(clones(diff, head_epoch.segments()));
    let coverage = traced_coverage(root, diff, base_epoch.segments(), head_epoch.segments())?;
    result.uncovered = coverage.as_ref().map(|c| c.uncovered.clone());
    if !gated(&result) {
        result.refactor = Some(RefactorCertificate {
            interfaces_moved: count(interfaces_moved(diff)),
            replayed_pass: coverage.as_ref().map(|c| c.replayed_pass),
            uncovered: coverage.as_ref().map(|c| count(c.uncovered.len())),
        });
    }
    Ok(result)
}

/// Did any gating section fire? The exit code is 3 exactly when this holds,
/// and the refactor certificate exists exactly when it does not — one
/// function, so the two can never disagree.
#[must_use]
pub fn gated(answer: &CheckResult) -> bool {
    answer
        .drift
        .as_ref()
        .is_some_and(|d| !d.suspects.is_empty())
        || answer
            .compile
            .as_ref()
            .is_some_and(|c| !c.uncompiled.is_empty())
        || answer
            .verdict
            .as_ref()
            .is_some_and(|v| v.gate == GateVerdictGate::Hold)
        || answer.stale.as_ref().is_some_and(|s| !s.is_empty())
        || answer.compat.iter().flatten().any(|f| f.declared)
}

/// The adherence section: the gate's verdict and the reasoning behind it.
///
/// The verdict is computed here the way `grid.todo` computes it: one engine,
/// so a review and a gate can never disagree about whether a promise was
/// kept.
fn adherence(
    head: &SnapshotOrRev,
    trace: Option<&grid_wire::TraceId>,
    plan: &grid_wire::Plan,
    root: &Path,
    provider: &ChartDelta,
) -> Result<(Option<grid_wire::GateVerdict>, Option<CheckWhy>), Error> {
    // Head by the id drift surveyed, never by the rev the caller typed: a
    // worktree survey is labelled with its rev, and looking the rev up here
    // would be the committed tree, not the one drift just judged.
    let diff = provider.diff(&plan.base, head)?;
    let verdict = rpc::gate_check_on(
        &grid_wire::methods::GateCheckParams {
            plan: plan.clone(),
            head: head.clone(),
        },
        root,
        Some(provider),
    )?;
    let snapshot = provider.current().ok_or_else(|| provider.no_snapshot())?;
    let names = callsigns(provider);
    let fulfilling = verdict
        .matched
        .iter()
        .filter_map(|promise| {
            let hops = hops_for(provider, &names, promise, &snapshot);
            (!hops.is_empty()).then(|| CheckWhyFulfillingItem {
                promise: promise.clone(),
                hops,
            })
        })
        .collect();
    let why = CheckWhy {
        surplus: verdict.surplus.clone(),
        coverage: coverage(&diff, trace, root)?,
        // Grid stores no anchors: nothing writes them, so the honest
        // answer is none, not an invented one. The field is the
        // consumer's (an Artifacts review, say) once a store exists.
        open_anchors: Vec::new(),
        fulfilling,
    };
    Ok((Some(verdict), Some(why)))
}

/// What the retained traces say about the change: the changed definitions
/// no traced test reaches, and how many covering tests already have a pass
/// on record at head.
struct Coverage {
    uncovered: Vec<DefRef>,
    replayed_pass: u64,
}

/// `grid.test.plan`'s walk with the retained traces as the inventory: every
/// test a trace names is a test, and the plan's `uncovered` is the answer.
/// No trace, no claim — `None` is "nothing is known", never "nothing is
/// uncovered".
///
/// `replayed_pass` is the reached tests whose closure key at head, in an
/// environment their verdicts were recorded in, is a recorded pass: the
/// attribution `grid drift --tests` uses, through the trace id the cache
/// line carries.
fn traced_coverage<B: AsRef<[u8]>>(
    root: &Path,
    diff: &grid_wire::Diff,
    base: &[Segment<B>],
    head: &[Segment<B>],
) -> Result<Option<Coverage>, Error> {
    let refused = |e: &dyn std::fmt::Display| Error::Refused(e.to_string());
    let traces = rpc::trace_dir(root)
        .and_then(grid_trace::Store::open)
        .and_then(|store| store.retained())
        .map_err(|e| refused(&e))?;
    if traces.is_empty() {
        return Ok(None);
    }
    let cache = grid_cache::Cache::open(rpc::cache_path(root)).map_err(|e| refused(&e))?;
    let mut tests: Vec<TestId> = traces
        .iter()
        .filter_map(|s| s.trace.command.clone())
        .map(TestId)
        .collect();
    tests.sort_by(|a, b| a.0.cmp(&b.0));
    tests.dedup_by(|a, b| a.0 == b.0);
    let changed = crate::testplan::changed_of(diff);
    // ponytail: the plan is recomputed here rather than read from the last
    // `grid test`, which persists only its base; persist the plan when a
    // repo's trace inventory makes this walk measurable.
    let plan = crate::testplan::compute(&crate::testplan::Inputs {
        base,
        head,
        base_id: diff.base.clone(),
        head_id: diff.head.clone(),
        changed: &changed,
        traces: &traces,
        cache: &cache,
        env_hash: "",
        tests: &tests,
        replay: false,
        fuel: crate::drift::FUEL,
        declined: None,
    });
    if plan.unchecked.budget_rejected > 0 {
        return Err(Error::Refused(format!(
            "the coverage walk exceeded fuel ({}) — refused whole, because a truncated \
             walk would report fewer uncovered definitions and look clean",
            crate::drift::FUEL
        )));
    }

    let test_of: BTreeMap<&str, &str> = traces
        .iter()
        .filter_map(|w| Some((w.trace.trace_id.as_str(), w.trace.command.as_deref()?)))
        .collect();
    let mut envs: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (_, entry) in cache.entries() {
        if let Some((test, env)) = entry
            .trace_id
            .as_deref()
            .and_then(|id| test_of.get(id))
            .zip(entry.env_hash.as_deref())
        {
            envs.entry(test).or_default().insert(env);
        }
    }
    let head_chart = grid_cache::Chart::new(head);
    let replayed_pass = plan
        .selected
        .iter()
        .filter(|s| s.why == TestSelectionWhy::Reached)
        .filter(|s| {
            let Some(datum) = s.datum.as_ref().and_then(|d| DatumId::parse(&d.0)) else {
                return false;
            };
            envs.get(s.test.0.as_str()).is_some_and(|envs| {
                envs.iter().any(|env| {
                    head_chart
                        .key(datum, grid_cache::Kind::Test, env)
                        .ok()
                        .and_then(|key| cache.get(key))
                        .is_some_and(|entry| entry.verdict == grid_cache::Verdict::Pass)
                })
            })
        })
        .count();
    Ok(Some(Coverage {
        uncovered: plan.uncovered,
        replayed_pass: count(replayed_pass),
    }))
}

/// Definitions added since base whose name-erased shape equals an existing
/// definition's — exact match only, the `Added` half of the diff joined
/// against head's shape column. A definition the diff calls `Moved` (a
/// rename) is not added and counts as existing.
fn clones<B: AsRef<[u8]>>(diff: &grid_wire::Diff, head: &[Segment<B>]) -> Vec<CloneFinding> {
    let added: BTreeSet<&str> = diff
        .defs
        .iter()
        .filter(|d| d.action == DefChangeAction::Added)
        .map(|d| d.callsign.0.as_str())
        .collect();
    if added.is_empty() {
        return Vec::new();
    }
    let mut existing: HashMap<[u8; 32], Vec<DefRef>> = HashMap::new();
    let mut fresh: Vec<(DefRef, [u8; 32])> = Vec::new();
    for segment in head {
        for i in 0..segment.def_count() {
            let ord = Ordinal(i);
            let (Some(callsign), Some(shape), Some(def)) = (
                segment.callsign(ord),
                segment.shape(ord),
                def_ref(segment, ord),
            ) else {
                continue;
            };
            if added.contains(callsign) {
                fresh.push((def, shape));
            } else {
                existing.entry(shape).or_default().push(def);
            }
        }
    }
    let mut out: Vec<CloneFinding> = fresh
        .iter()
        .flat_map(|(def, shape)| {
            existing
                .get(shape)
                .into_iter()
                .flatten()
                .map(|old| CloneFinding {
                    def: def.clone(),
                    existing: old.clone(),
                })
        })
        .collect();
    let name = |d: &DefRef| d.callsign.as_ref().map(|c| c.0.clone());
    out.sort_by(|a, b| (name(&a.def), name(&a.existing)).cmp(&(name(&b.def), name(&b.existing))));
    out
}

/// Drift's changed set, counted: every definition whose interface moved or
/// whose transition is unknown — unknown is treated as moved, as drift
/// treats it.
fn interfaces_moved(diff: &grid_wire::Diff) -> usize {
    diff.defs
        .iter()
        .filter(|d| match d.action {
            DefChangeAction::Removed => true,
            DefChangeAction::Modified => d.sig_changed != Some(false),
            DefChangeAction::Added | DefChangeAction::Moved => false,
        })
        .count()
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// The eight-character prefix of a snapshot id's hash, as `@prefix`.
fn prefix_of(id: &str) -> &str {
    let hex = id.rsplit_once(':').map_or(id, |(_, hex)| hex);
    hex.get(..8).unwrap_or(hex)
}

/// A definition at head, named and located.
fn def_ref<B: AsRef<[u8]>>(segment: &Segment<B>, ord: Ordinal) -> Option<DefRef> {
    Some(DefRef {
        datum: Datum(segment.datum(ord)?.to_string()),
        callsign: Some(Callsign(segment.callsign(ord)?.to_owned())),
        lang: segment.lang(ord),
        signature: None,
        cfg: None,
        materialization: segment.location(ord).map(|loc| DefRefMaterialization {
            file: loc.path.to_owned(),
            line: u64::from(loc.line),
        }),
    })
}

/// Render a check as one markdown document for a pull-request comment.
///
/// The porcelain is the record form and stays exactly what it was; this is
/// the same judgment for a reader who is about to leave a review comment,
/// where thirty records with padded columns are a wall and a heading with
/// bullets is a paragraph. Nothing is computed here — every bullet is a field
/// of the same [`grid_wire::CheckResult`], in the same gate-first order the
/// porcelain prints, so the two cannot disagree about what gated.
///
/// Every bullet is `callsign — reason — file:line`, and the three columns
/// keep their meaning across every section: what, why, where. A section with
/// nothing in it is omitted rather than printed empty, because an empty
/// heading in a review comment reads as a check that ran and found nothing
/// when it may be a check that never ran (`--quick` asks for no traces at
/// all); the footer says what was not checked.
#[must_use]
pub fn markdown(answer: &CheckResult, name: &crate::porcelain::NameOf) -> String {
    let show = |d: &Datum| name(d).unwrap_or_else(|| format!("@{}", prefix(d)));
    let mut out = String::new();
    let (base, head) = answer.drift.as_ref().map_or_else(
        || ("?".to_owned(), "?".to_owned()),
        |d| {
            (
                format!("@{}", prefix_of(&d.base.0)),
                format!("@{}", prefix_of(&d.head.0)),
            )
        },
    );
    let _ = writeln!(out, "# grid check {base} → {head}\n");
    out.push_str(if gated(answer) {
        "**Exit 3** — something this change did was not checked. Every bullet under \
         *Gates* is evidence grid can stand behind; the tail below ranks and does not gate.\n"
    } else {
        "**Exit 0** — no gate fired. The tail below ranks and never gates.\n"
    });

    let blocking = gate_sections(answer, &show);
    if !blocking.is_empty() {
        out.push_str("\n## Gates\n");
        out.push_str(&blocking);
    }
    let ranked = tail_sections(answer, &show);
    if !ranked.is_empty() {
        out.push_str("\n## Tail\n");
        out.push_str(&ranked);
    }
    if let Some(cert) = answer.refactor.as_ref() {
        let _ = writeln!(out, "\n## Certificate\n\n- {}", certificate(cert));
    }

    let mut footer: Vec<String> = answer
        .drift
        .as_ref()
        .map(|d| unchecked_lines(&d.unchecked))
        .unwrap_or_default();
    // `-` is the porcelain's "no subject" column; a document has no columns,
    // so the message stands on its own.
    let line = |w: &DriftWarning| {
        if w.subject == "-" {
            w.message.clone()
        } else {
            format!("{}: {}", w.subject, w.message)
        }
    };
    // Every `warn` the records emit, from all three places that emit one: the
    // composite's own, the drift report's (a covenant whose pattern matches
    // nothing, a test whose verdicts can no longer be re-keyed) and the
    // compile report's (a cfg predicate grid could not decide). This is the
    // half of the document nobody notices is missing: a gate that fired is
    // loud in either form, but a check that quietly stopped running looks
    // exactly like a check that passed, and the document is the surface a
    // human reviews from.
    footer.extend(
        answer
            .warnings
            .iter()
            .flatten()
            .chain(
                answer
                    .drift
                    .iter()
                    .flat_map(|d| d.warnings.iter().flatten()),
            )
            .chain(answer.compile.iter().flat_map(|c| &c.unknown))
            .map(line),
    );
    if !footer.is_empty() {
        let _ = writeln!(out, "\n---\n\nNot checked: {}", footer.join("; "));
    }
    out
}

/// The sections that move the exit code, in the porcelain's own order.
fn gate_sections(answer: &CheckResult, show: &impl Fn(&Datum) -> String) -> String {
    let mut out = String::new();
    let suspects = || answer.drift.iter().flat_map(|d| &d.suspects);
    section(
        &mut out,
        "Drift suspects",
        suspects()
            .filter(|s| !is_covenant(s))
            .map(|s| suspect_bullet(s, show)),
    );
    section(
        &mut out,
        "Covenant fires",
        suspects()
            .filter(|s| is_covenant(s))
            .map(|s| suspect_bullet(s, show)),
    );
    section(
        &mut out,
        "Uncompiled callers",
        answer
            .compile
            .iter()
            .flat_map(|c| &c.uncompiled)
            .map(|s| suspect_bullet(s, show)),
    );
    section(
        &mut out,
        "Adherence",
        answer
            .verdict
            .iter()
            .filter(|v| v.gate == GateVerdictGate::Hold)
            .flat_map(|v| adherence_bullets(v, show)),
    );
    section(
        &mut out,
        "Stale verdicts",
        answer.stale.iter().flatten().map(|s| {
            bullet(
                &s.test,
                match s.reason {
                    grid_wire::StaleTestReason::ClosureChanged => "closure-changed",
                    grid_wire::StaleTestReason::ClosureRejected => "closure-rejected",
                },
                None,
            )
        }),
    );
    section(
        &mut out,
        "Declared compat",
        answer
            .compat
            .iter()
            .flatten()
            .filter(|f| f.declared)
            .map(|f| compat_bullet(f, show)),
    );
    out
}

/// The sections that rank and never move the exit code.
fn tail_sections(answer: &CheckResult, show: &impl Fn(&Datum) -> String) -> String {
    let mut out = String::new();
    section(
        &mut out,
        "Weak tests",
        answer.weak.iter().flatten().map(|w| {
            bullet(
                &w.test,
                &format!("proved nothing about {}", named(&w.def, show)),
                w.def.materialization.as_ref(),
            )
        }),
    );
    section(
        &mut out,
        "Uncovered",
        answer.uncovered.iter().flatten().map(|d| {
            bullet(
                &named(d, show),
                "no selected test reaches it",
                d.materialization.as_ref(),
            )
        }),
    );
    section(
        &mut out,
        "Clone-of",
        answer.clones.iter().flatten().map(|c| {
            bullet(
                &named(&c.def, show),
                &format!("clone-of {}", named(&c.existing, show)),
                c.def.materialization.as_ref(),
            )
        }),
    );
    section(
        &mut out,
        "Compat",
        answer
            .compat
            .iter()
            .flatten()
            .filter(|f| !f.declared)
            .map(|f| compat_bullet(f, show)),
    );
    out
}

/// A covenant fire is a drift suspect whose reason names the declaration —
/// the same row, listed under its own heading because a human declared it
/// and a reader wants to know which of the two kinds fired.
fn is_covenant(suspect: &grid_wire::DriftSuspect) -> bool {
    suspect
        .reason
        .as_deref()
        .is_some_and(|r| r.starts_with("covenant:"))
}

/// One `### <title>` block, or nothing at all when the iterator is empty.
fn section(out: &mut String, title: &str, rows: impl Iterator<Item = String>) {
    let rows: Vec<String> = rows.collect();
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n### {title}\n");
    for row in rows {
        out.push_str(&row);
    }
}

/// One bullet: the callsign in backticks, the reason, and `file:line` in
/// backticks — the one shape every section uses, so the three columns keep
/// their meaning throughout the document: what, why, where.
fn bullet(what: &str, why: &str, at: Option<&DefRefMaterialization>) -> String {
    match at {
        Some(at) => format!("- `{what}` — {why} — `{}:{}`\n", at.file, at.line),
        None => format!("- `{what}` — {why}\n"),
    }
}

fn named(def: &DefRef, show: &impl Fn(&Datum) -> String) -> String {
    def.callsign
        .as_ref()
        .map_or_else(|| show(&def.datum), |c| c.0.clone())
}

/// A suspect's bullet: the dependent, what it depends on and how, and where
/// the dependent is — the `drift` record's own columns as a sentence.
fn suspect_bullet(suspect: &grid_wire::DriftSuspect, show: &impl Fn(&Datum) -> String) -> String {
    let relation = grid_gate::lint::relation_name(suspect.relation);
    let changed = named(&suspect.changed, show);
    let grade = grade(suspect.grade);
    let why = match suspect.reason.as_deref() {
        Some(reason) => format!("{relation} `{changed}` ({grade}, {reason})"),
        None => format!("{relation} `{changed}` ({grade})"),
    };
    bullet(
        &named(&suspect.dependent, show),
        &why,
        suspect.dependent.materialization.as_ref(),
    )
}

fn compat_bullet(finding: &grid_wire::CompatFinding, show: &impl Fn(&Datum) -> String) -> String {
    bullet(
        &named(&finding.def, show),
        &format!("compat: {}", finding.rule),
        finding.def.materialization.as_ref(),
    )
}

/// The gate's divergences, each as its own bullet: a promise never built, an
/// edge nobody promised, a definition nobody accounted for.
fn adherence_bullets(
    verdict: &grid_wire::GateVerdict,
    show: &impl Fn(&Datum) -> String,
) -> Vec<String> {
    let mut out: Vec<String> = verdict
        .missing
        .iter()
        .map(|p| {
            bullet(
                &format!("{} -> {}", p.src.0, p.dst.0),
                "missing: promised and never built",
                None,
            )
        })
        .collect();
    out.extend(verdict.surplus.iter().map(|e| {
        bullet(
            &format!("{} -> {}", show(&e.edge.src), show(&e.edge.dst)),
            "surplus: built and never promised",
            None,
        )
    }));
    out.extend(verdict.defs_unaccounted.iter().map(|d| {
        bullet(
            &d.callsign.0,
            "unaccounted: changed and never promised",
            None,
        )
    }));
    out.extend(
        verdict
            .invariant_violations
            .iter()
            .map(|v| bullet(&v.id, "invariant violated", None)),
    );
    out
}

/// The refactor certificate, as the one sentence the porcelain prints.
fn certificate(cert: &RefactorCertificate) -> String {
    let interfaces = match cert.interfaces_moved {
        0 => "interfaces held".to_owned(),
        n => format!("{n} interfaces moved"),
    };
    match (cert.replayed_pass, cert.uncovered) {
        (Some(replayed), Some(uncovered)) => format!(
            "**refactor** — {interfaces} · {replayed} covering tests replayed pass · \
             {uncovered} changed defs uncovered"
        ),
        _ => format!("**refactor** — {interfaces} · no retained traces"),
    }
}

/// What the check could not check, as the footer sentence.
fn unchecked_lines(unchecked: &grid_wire::DriftUnchecked) -> Vec<String> {
    let mut out = Vec::new();
    match unchecked.unresolved_refs {
        Some(n) => out.push(format!("{n} unresolved references")),
        None => out
            .push("an unknown number of unresolved references (head was not surveyed)".to_owned()),
    }
    let mut count = |n: Option<u64>, what: &str| {
        if let Some(n) = n.filter(|n| *n > 0) {
            out.push(format!("{n} {what}"));
        }
    };
    count(unchecked.unattributed_entries, "unattributed verdicts");
    count(
        unchecked.weak_skipped,
        "weak-checks skipped (stale coverage)",
    );
    count(
        unchecked.compat_unchecked,
        "compat unchecked (derive not visible)",
    );
    out
}

/// Every callsign the live epoch knows, sorted — the name-space side of the
/// pattern match, gathered once rather than per promise.
fn callsigns(provider: &ChartDelta) -> Vec<String> {
    let mut out: Vec<String> = provider
        .head()
        .segments()
        .iter()
        .flat_map(|segment| {
            (0..segment.def_count())
                .filter_map(|ord| segment.callsign(grid_chart::Ordinal(ord)))
                .map(str::to_owned)
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The concrete endpoints a promise pattern names, capped deterministically.
fn endpoints(names: &[String], pattern: &str) -> Vec<Locator> {
    names
        .iter()
        .filter(|name| grid_gate::pattern::matches(pattern, name))
        .take(MAX_ENDPOINTS)
        .map(|name| Locator::Callsign {
            callsign: Callsign(name.clone()),
        })
        .collect()
}

/// The paths that make one matched promise good.
///
/// Hops are filtered to the promised relation: a chain of `calls` is why a
/// `calls` promise holds, and a path that reached the destination by some
/// other relation would be a different claim wearing this promise's name.
fn hops_for(
    provider: &ChartDelta,
    names: &[String],
    promise: &PlanEdgePromise,
    snapshot: &grid_wire::SnapshotId,
) -> Vec<grid_wire::Path> {
    let from = endpoints(names, &promise.src.0);
    let to = endpoints(names, &promise.dst.0);
    if from.is_empty() || to.is_empty() {
        return Vec::new();
    }
    let query = WalkQuery {
        from,
        to: Some(to),
        direction: Some(WalkQueryDirection::Out),
        relations: Some(vec![promise.relation]),
        at_or_above: None,
        fuel: Some(Fuel {
            max_hops: Some(MAX_HOPS),
            max_nodes: Some(MAX_NODES),
            max_ms: Some(MAX_MS),
        }),
        snapshot: None,
    };
    let mut paths = Vec::new();
    for segment in provider.head().segments() {
        // A segment that holds none of the endpoints answers nothing about
        // this promise; the walk engine would call the unresolved root an
        // error,
        // and across segments it is simply an absence.
        let Ok(result) = grid_walk::query(segment, &query, snapshot.clone()) else {
            continue;
        };
        paths.extend(result.paths.into_iter().flatten());
    }
    paths.sort_by(|a, b| {
        (a.nodes.len(), &a.nodes.first().map(|d| d.0.clone()))
            .cmp(&(b.nodes.len(), &b.nodes.first().map(|d| d.0.clone())))
    });
    paths.dedup();
    paths
}

/// Changed but not reached by the supplied trace — the reviewer's red flags.
///
/// No trace, no claim: coverage is empty rather than "everything is uncovered",
/// because an absent test run is not evidence that code never ran.
fn coverage(
    diff: &grid_wire::Diff,
    trace: Option<&grid_wire::TraceId>,
    root: &Path,
) -> Result<Vec<Datum>, Error> {
    let Some(trace_id) = trace else {
        return Ok(Vec::new());
    };
    let store = rpc::trace_dir(root)
        .and_then(grid_trace::Store::open)
        .map_err(|e| Error::Chart(chart_provider::Error::Config(e.to_string())))?;
    let stored = store
        .get(&trace_id.0)
        .map_err(|e| Error::Chart(chart_provider::Error::Config(e.to_string())))?
        .ok_or_else(|| Error::UnknownTrace(trace_id.0.clone()))?;
    let mut reached: Vec<String> = stored
        .hops
        .iter()
        .flat_map(|(caller, callee)| [caller.to_string(), callee.to_string()])
        .collect();
    reached.sort();
    reached.dedup();

    let mut out: Vec<Datum> = diff
        .defs
        .iter()
        .filter(|d| d.action != DefChangeAction::Removed)
        .filter_map(|d| d.after.clone())
        .filter(|datum| reached.binary_search(&datum.0).is_err())
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup();
    Ok(out)
}

/// Render a check as porcelain records (`docs/PORCELAIN.md`), sections in
/// gate-first order: drift (its suspects, covenant warnings and closing
/// `miss` counts), the compile report, the verdict — with, under `why`, a
/// `path` per fulfilling route, its `hop` records beneath it, and a `miss`
/// per changed-but-unexecuted definition — stale verdicts, compat; then the
/// ranked tail and the certificate. Every section renders exactly as its
/// own verb renders it: a second grammar for one judgment would be a second
/// judgment.
#[must_use]
pub fn porcelain(answer: &CheckResult, name: &crate::porcelain::NameOf, why: bool) -> String {
    let show = |d: &Datum| name(d).unwrap_or_else(|| format!("@{}", prefix(d)));
    let mut out = String::new();
    if let Some(drift) = answer.drift.as_ref() {
        out.push_str(&crate::porcelain::drift(drift));
    }
    if let Some(compile) = answer.compile.as_ref() {
        out.push_str(&crate::porcelain::compile(compile));
    }
    if let Some(verdict) = answer.verdict.as_ref() {
        out.push_str(&crate::porcelain::verdict(verdict, name));
    }
    if let Some(why) = answer.why.as_ref().filter(|_| why) {
        for item in &why.fulfilling {
            for path in &item.hops {
                let head = path.nodes.first().map_or_else(String::new, &show);
                record(
                    &mut out,
                    "path",
                    &[&head, &path.edges.len().to_string(), weakest(&path.edges)],
                );
                for edge in &path.edges {
                    record(
                        &mut out,
                        "hop",
                        &[
                            grid_gate::lint::relation_name(edge.relation),
                            &show(&edge.dst),
                            grade(edge.grade),
                        ],
                    );
                }
            }
        }
        for datum in &why.coverage {
            record(&mut out, "miss", &[&show(datum), "unexecuted"]);
        }
    }
    out.push_str(&crate::porcelain::stale(
        answer.stale.as_deref().unwrap_or_default(),
    ));
    out.push_str(&crate::porcelain::compat(
        answer.compat.as_deref().unwrap_or_default(),
    ));
    out.push_str(&crate::porcelain::weak(
        answer.weak.as_deref().unwrap_or_default(),
    ));
    out.push_str(&crate::porcelain::check_tail(answer));
    out
}

/// The weakest evidence on a path: a chain is only as good as its worst hop,
/// so that is what the `path` record reports.
fn weakest(edges: &[grid_wire::Edge]) -> &'static str {
    edges
        .iter()
        .map(|e| e.grade)
        .max_by_key(|z| rank(*z))
        .map_or("proven", grade)
}

const fn rank(z: Grade) -> u8 {
    match z {
        Grade::Proven => 0,
        Grade::Observed => 1,
        Grade::Modeled => 2,
        Grade::Heuristic => 3,
    }
}

const fn grade(z: Grade) -> &'static str {
    match z {
        Grade::Proven => "proven",
        Grade::Observed => "observed",
        Grade::Modeled => "modeled",
        Grade::Heuristic => "heuristic",
    }
}

fn prefix(datum: &Datum) -> String {
    datum.0.rsplit_once(':').map_or_else(
        || datum.0.clone(),
        |(_, hash)| hash[..8.min(hash.len())].to_string(),
    )
}

/// One record, in the shared grammar: tag padded to eight, columns separated
/// by two spaces.
fn record(out: &mut String, tag: &str, columns: &[&str]) {
    out.push_str(tag);
    for _ in 0..8usize.saturating_sub(tag.len()).max(2) {
        out.push(' ');
    }
    out.push_str(&columns.join("  "));
    out.push('\n');
}
