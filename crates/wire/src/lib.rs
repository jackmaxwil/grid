//! grid-wire - versioned external schemas; the only public vocabulary.
//!
//! GENERATED from docs/wire-v0.openrpc.json by scripts/gen-wire.py - do not
//! edit by hand. `deny_unknown_fields` everywhere: unknown keys are a wire
//! error, never silently dropped.

pub mod errors;
pub mod methods;

use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Datum(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GitRev(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SnapshotOrRev {
    Snapshot { snapshot: SnapshotId },
    GitRev { git_rev: GitRev },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    #[serde(rename = "ts")]
    Ts,
    #[serde(rename = "js")]
    Js,
    #[serde(rename = "py")]
    Py,
    #[serde(rename = "rs")]
    Rs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathStep {
    pub role: String,
    pub index: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AstPath(pub Vec<PathStep>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fix {
    pub datum: Datum,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<AstPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Callsign(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallsignPattern(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Handle(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Locator {
    Datum { datum: Datum },
    Fix { fix: Fix },
    Callsign { callsign: Callsign },
    Handle { handle: Handle },
    FileLine { file: String, line: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefRef {
    pub datum: Datum,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callsign: Option<Callsign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<Lang>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<DefRefMaterialization>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefRefMaterialization {
    pub file: String,
    pub line: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Relation {
    #[serde(rename = "calls")]
    Calls,
    #[serde(rename = "constructs")]
    Constructs,
    #[serde(rename = "reads_field")]
    ReadsField,
    #[serde(rename = "writes_field")]
    WritesField,
    #[serde(rename = "implements")]
    Implements,
    #[serde(rename = "dispatches_to")]
    DispatchesTo,
    #[serde(rename = "flows_into")]
    FlowsInto,
    #[serde(rename = "serializes_to")]
    SerializesTo,
    #[serde(rename = "deserializes_from")]
    DeserializesFrom,
    #[serde(rename = "registers_under")]
    RegistersUnder,
    #[serde(rename = "sends_over")]
    SendsOver,
    #[serde(rename = "links_against")]
    LinksAgainst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Zoc {
    #[serde(rename = "proven")]
    Proven,
    #[serde(rename = "observed")]
    Observed,
    #[serde(rename = "modeled")]
    Modeled,
    #[serde(rename = "heuristic")]
    Heuristic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub src: Datum,
    pub relation: Relation,
    pub dst: Datum,
    pub zoc: Zoc,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Leg {
    pub from: Datum,
    pub to: Datum,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_rev: Option<GitRev>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub snapshot_id: SnapshotId,
    pub git_rev: GitRev,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_diff_hash: Option<String>,
    pub stats: SnapshotStats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotStats {
    pub defs: u64,
    pub edges: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub langs: Option<Vec<Lang>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefChange {
    pub action: DefChangeAction,
    pub callsign: Callsign,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Datum>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Datum>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<Callsign>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig_changed: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefChangeAction {
    #[serde(rename = "added")]
    Added,
    #[serde(rename = "removed")]
    Removed,
    #[serde(rename = "modified")]
    Modified,
    #[serde(rename = "moved")]
    Moved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeChange {
    pub action: EdgeChangeAction,
    pub edge: Edge,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<EdgeChangeMovedFrom>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeChangeAction {
    #[serde(rename = "added")]
    Added,
    #[serde(rename = "removed")]
    Removed,
    #[serde(rename = "moved")]
    Moved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeChangeMovedFrom {
    pub src: Datum,
    pub dst: Datum,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MadeGood {
    pub base: SnapshotId,
    pub head: SnapshotId,
    pub defs: Vec<DefChange>,
    pub edges: Vec<EdgeChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackDefPromise {
    pub action: TrackDefPromiseAction,
    pub callsign: CallsignPattern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackDefPromiseAction {
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "remove")]
    Remove,
    #[serde(rename = "modify")]
    Modify,
    #[serde(rename = "move")]
    Move,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackEdgePromise {
    pub action: TrackEdgePromiseAction,
    pub src: CallsignPattern,
    pub relation: Relation,
    pub dst: CallsignPattern,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackEdgePromiseAction {
    #[serde(rename = "add")]
    Add,
    #[serde(rename = "remove")]
    Remove,
    #[serde(rename = "move")]
    Move,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackDelta {
    pub defs: Vec<TrackDefPromise>,
    pub edges: Vec<TrackEdgePromise>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedTest {
    pub test: String,
    pub expect: ExpectedTestExpect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpectedTestExpect {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "skip")]
    Skip,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeRule {
    pub src: CallsignPattern,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<Relation>,
    pub dst: CallsignPattern,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_or_above: Option<Zoc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invariant {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub rule: InvariantRule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InvariantRule {
    ForbidEdge { forbid_edge: EdgeRule },
    RequireEdge { require_edge: EdgeRule },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub track_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_rev: Option<String>,
    pub base: SnapshotOrRev,
    pub delta: TrackDelta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests: Option<Vec<ExpectedTest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariants: Option<Vec<Invariant>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerate: Option<Vec<EdgeRule>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestComparison {
    pub test: String,
    pub expected: TestComparisonExpected,
    pub actual: TestComparisonActual,
    pub ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestComparisonExpected {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "skip")]
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestComparisonActual {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "skip")]
    Skip,
    #[serde(rename = "missing")]
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvariantViolation {
    pub id: String,
    pub witness: Path,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XteVerdict {
    pub gate: XteVerdictGate,
    pub track_id: String,
    pub head: SnapshotId,
    pub matched: Vec<TrackEdgePromise>,
    pub missing: Vec<TrackEdgePromise>,
    pub surplus: Vec<EdgeChange>,
    pub tolerated: Vec<EdgeChange>,
    pub defs_unaccounted: Vec<DefChange>,
    pub tests: Vec<TestComparison>,
    pub invariant_violations: Vec<InvariantViolation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasons: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XteVerdictGate {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "hold")]
    Hold,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fuel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hops: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_nodes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeadlineQuery {
    pub from: Vec<Locator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Vec<Locator>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<LeadlineQueryDirection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relations: Option<Vec<Relation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_or_above: Option<Zoc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel: Option<Fuel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeadlineQueryDirection {
    #[serde(rename = "out")]
    Out,
    #[serde(rename = "in")]
    In,
    #[serde(rename = "both")]
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Path {
    pub nodes: Vec<Datum>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeadlineResult {
    pub status: LeadlineResultStatus,
    pub snapshot: SnapshotId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<Path>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reached: Option<Vec<DefRef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontier: Option<Vec<Datum>>,
    pub stats: LeadlineResultStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeadlineResultStatus {
    #[serde(rename = "complete")]
    Complete,
    #[serde(rename = "aground")]
    Aground,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeadlineResultStats {
    pub visited: u64,
    pub hops: u64,
    pub ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete_within: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub anchor_id: String,
    pub fix: Fix,
    pub kind: AnchorKind,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnchorKind {
    #[serde(rename = "annotation")]
    Annotation,
    #[serde(rename = "spec_ref")]
    SpecRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedAnchor {
    pub anchor_id: String,
    pub status: CarriedAnchorStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<Fix>,
    pub confidence: f64,
    pub via: Vec<Leg>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CarriedAnchorStatus {
    #[serde(rename = "exact")]
    Exact,
    #[serde(rename = "carried")]
    Carried,
    #[serde(rename = "orphaned")]
    Orphaned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeObservation {
    pub caller: WakeEndpoint,
    pub callee: WakeEndpoint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WakeEndpoint {
    Datum { datum: Datum },
    Symbol { symbol: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wake {
    pub wake_id: String,
    pub git_rev: GitRev,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    pub observations: Vec<WakeObservation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCheckRequest {
    pub kind: CacheCheckRequestKind,
    pub env_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotId>,
    pub datums: Vec<Datum>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheCheckRequestKind {
    #[serde(rename = "typecheck")]
    Typecheck,
    #[serde(rename = "lint")]
    Lint,
    #[serde(rename = "test")]
    Test,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheHit {
    pub datum: Datum,
    pub verdict: CacheHitVerdict,
    pub recorded_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheHitVerdict {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheResultEntry {
    pub datum: Datum,
    pub verdict: CacheResultEntryVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheResultEntryVerdict {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCheckResult {
    pub snapshot: SnapshotId,
    pub hits: Vec<CacheHit>,
    pub misses: Vec<Datum>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TrackId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WakeId(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopePack {
    pub snapshot: SnapshotId,
    pub defs: Vec<DefRef>,
    pub in_edges: Vec<Edge>,
    pub out_edges: Vec<Edge>,
    pub invariants: Vec<Invariant>,
    pub anchors: Vec<Anchor>,
    pub churn: Vec<ScopePackChurnItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopePackChurnItem {
    pub datum: Datum,
    pub legs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LintFinding {
    pub promise: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LintResult {
    pub errors: Vec<LintFinding>,
    pub warnings: Vec<LintFinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Estimate {
    pub defs_in_closure: u64,
    pub tests_in_reverse_closure: Vec<String>,
    pub predicted_cache_misses: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditSpec {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_datum: Option<Datum>,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditResult {
    pub applied: Vec<Datum>,
    pub conflicts: Vec<EditResultConflictsItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditResultConflictsItem {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub was: Option<Datum>,
    pub now: Datum,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoList {
    pub track_id: TrackId,
    pub defs_done: Vec<TrackDefPromise>,
    pub defs_pending: Vec<TrackDefPromise>,
    pub edges_done: Vec<TrackEdgePromise>,
    pub edges_pending: Vec<TrackEdgePromise>,
    pub tests_done: Vec<String>,
    pub tests_pending: Vec<String>,
    pub fraction: TodoListFraction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoListFraction {
    pub num: u64,
    pub den: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPack {
    pub verdict: XteVerdict,
    pub fulfilling: Vec<ReviewPackFulfillingItem>,
    pub surplus: Vec<EdgeChange>,
    pub coverage: Vec<Datum>,
    pub open_anchors: Vec<Anchor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPackFulfillingItem {
    pub promise: TrackEdgePromise,
    pub hops: Vec<Path>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeDiff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_divergence: Option<WakeDiffFirstDivergence>,
    pub added: Vec<Edge>,
    pub removed: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeDiffFirstDivergence {
    pub caller: Datum,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub green_callee: Option<Datum>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red_callee: Option<Datum>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suspects {
    pub ranked: Vec<SuspectsRankedItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuspectsRankedItem {
    pub datum: Datum,
    pub score: f64,
    pub via: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Covenant {
    pub a: CallsignPattern,
    pub b: CallsignPattern,
    pub via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeakTest {
    pub test: String,
    pub def: DefRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaleTest {
    pub test: String,
    pub reason: StaleTestReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaleTestReason {
    #[serde(rename = "closure-changed")]
    ClosureChanged,
    #[serde(rename = "closure-rejected")]
    ClosureRejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriftWarning {
    pub subject: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriftSuspect {
    pub dependent: DefRef,
    pub changed: DefRef,
    pub relation: Relation,
    pub zoc: Zoc,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig_before: Option<Datum>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig_after: Option<Datum>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriftUnchecked {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_refs: Option<u64>,
    pub budget_rejected: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unattributed_entries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weak_skipped: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriftReport {
    pub base: SnapshotId,
    pub head: SnapshotId,
    pub suspects: Vec<DriftSuspect>,
    pub unchecked: DriftUnchecked,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<DriftWarning>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_tests: Option<Vec<StaleTest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weak: Option<Vec<WeakTest>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BisectResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub culprit: Option<Datum>,
    pub steps: u64,
    pub cache_hits: u64,
}
