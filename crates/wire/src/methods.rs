//! Method param and result types, one per wire method. GENERATED.

use serde::{Deserialize, Serialize};

use super::{
    Anchor, CacheCheckRequest, CacheResultEntry, CallsignPattern, CarriedAnchor, Datum, DefRef,
    Diff, DriftReport, EditResult, EditSpec, Estimate, GateVerdict, GitRev, Grade, InterfereReport,
    LintResult, Locator, Plan, PlanId, ScopePack, Snapshot, SnapshotId, SnapshotOrRev, Suspects,
    TestId, TestPlan, TodoList, Trace, TraceDiff, TraceId, WalkQuery, WalkResult,
};

/// One wire method: the `method` string it is sent under, and the type its
/// `result` member decodes into.
///
/// The params struct is the implementor, so a caller names a method by the
/// value it already has to build.
pub trait Method: Serialize {
    /// The `method` string a request carries.
    const NAME: &'static str;
    /// What the `result` member decodes into.
    type Result: serde::de::DeserializeOwned;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionParams {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionResult {
    pub wire: String,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub porcelain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotId>,
}

impl Method for VersionParams {
    const NAME: &'static str = "grid.version";
    type Result = VersionResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCurrentParams {}

pub type SnapshotCurrentResult = Snapshot;

impl Method for SnapshotCurrentParams {
    const NAME: &'static str = "grid.snapshot.current";
    type Result = SnapshotCurrentResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyRunParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_rev: Option<GitRev>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

pub type SurveyRunResult = Snapshot;

impl Method for SurveyRunParams {
    const NAME: &'static str = "grid.survey.run";
    type Result = SurveyRunResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveParams {
    pub locator: Locator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotId>,
}

pub type ResolveResult = Vec<DefRef>;

impl Method for ResolveParams {
    const NAME: &'static str = "grid.resolve";
    type Result = ResolveResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaParams {
    pub base: SnapshotOrRev,
    pub head: SnapshotOrRev,
}

pub type DeltaResult = Diff;

impl Method for DeltaParams {
    const NAME: &'static str = "grid.delta";
    type Result = DeltaResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateCheckParams {
    pub plan: Plan,
    pub head: SnapshotOrRev,
}

pub type GateCheckResult = GateVerdict;

impl Method for GateCheckParams {
    const NAME: &'static str = "grid.gate.check";
    type Result = GateCheckResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryWalkParams {
    pub query: WalkQuery,
}

pub type QueryWalkResult = WalkResult;

impl Method for QueryWalkParams {
    const NAME: &'static str = "grid.query.walk";
    type Result = QueryWalkResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorCarryParams {
    pub anchors: Vec<Anchor>,
    pub to: SnapshotId,
}

pub type AnchorCarryResult = Vec<CarriedAnchor>;

impl Method for AnchorCarryParams {
    const NAME: &'static str = "grid.anchor.carry";
    type Result = AnchorCarryResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceIngestParams {
    pub trace: Trace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceIngestResult {
    pub observed_edges: u64,
    pub resolved: u64,
    pub unresolved_symbols: Vec<String>,
}

impl Method for TraceIngestParams {
    const NAME: &'static str = "grid.trace.ingest";
    type Result = TraceIngestResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCheckParams {
    pub request: CacheCheckRequest,
}

impl Method for CacheCheckParams {
    const NAME: &'static str = "grid.cache.check";
    type Result = super::CacheCheckResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRecordParams {
    pub kind: CacheRecordKind,
    pub env_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    pub results: Vec<CacheResultEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRecordResult {
    pub recorded: u64,
    pub rejected: Vec<CacheRecordResultRejectedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nondeterministic: Option<Vec<CacheRecordResultNondeterministicItem>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheRecordKind {
    #[serde(rename = "typecheck")]
    Typecheck,
    #[serde(rename = "lint")]
    Lint,
    #[serde(rename = "test")]
    Test,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRecordResultRejectedItem {
    pub datum: Datum,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheRecordResultNondeterministicItemPrevious {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRecordResultNondeterministicItem {
    pub datum: Datum,
    pub previous: CacheRecordResultNondeterministicItemPrevious,
    pub previous_at: String,
}

impl Method for CacheRecordParams {
    const NAME: &'static str = "grid.cache.record";
    type Result = CacheRecordResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeParams {
    pub callsigns: Vec<CallsignPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotOrRev>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bodies: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<SnapshotOrRev>,
}

pub type ScopeResult = ScopePack;

impl Method for ScopeParams {
    const NAME: &'static str = "grid.scope";
    type Result = ScopeResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanLintParams {
    pub plan: Plan,
}

pub type PlanLintResult = LintResult;

impl Method for PlanLintParams {
    const NAME: &'static str = "grid.plan.lint";
    type Result = PlanLintResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEstimateParams {
    pub plan: Plan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SnapshotOrRev>,
}

pub type PlanEstimateResult = Estimate;

impl Method for PlanEstimateParams {
    const NAME: &'static str = "grid.plan.estimate";
    type Result = PlanEstimateResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditApplyParams {
    pub edits: Vec<EditSpec>,
}

pub type EditApplyResult = EditResult;

impl Method for EditApplyParams {
    const NAME: &'static str = "grid.edit.apply";
    type Result = EditApplyResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoParams {
    pub plan: PlanId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<SnapshotOrRev>,
}

pub type TodoResult = TodoList;

impl Method for TodoParams {
    const NAME: &'static str = "grid.todo";
    type Result = TodoResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<PlanId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SnapshotOrRev>,
    pub head: SnapshotOrRev,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick: Option<bool>,
}

impl Method for CheckParams {
    const NAME: &'static str = "grid.check";
    type Result = super::CheckResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceDiffParams {
    pub green: TraceId,
    pub red: TraceId,
}

pub type TraceDiffResult = TraceDiff;

impl Method for TraceDiffParams {
    const NAME: &'static str = "grid.trace.diff";
    type Result = TraceDiffResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuspectsParams {
    pub test: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SnapshotOrRev>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure_since: Option<SnapshotOrRev>,
}

pub type SuspectsResult = Suspects;

impl Method for SuspectsParams {
    const NAME: &'static str = "grid.suspects";
    type Result = SuspectsResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BisectParams {
    pub good: SnapshotOrRev,
    pub bad: SnapshotOrRev,
    pub test: String,
}

impl Method for BisectParams {
    const NAME: &'static str = "grid.bisect";
    type Result = super::BisectResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriftParams {
    pub base: SnapshotOrRev,
    pub head: SnapshotOrRev,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<Grade>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Plan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<bool>,
}

pub type DriftResult = DriftReport;

impl Method for DriftParams {
    const NAME: &'static str = "grid.drift";
    type Result = DriftResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestPlanParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SnapshotOrRev>,
    pub env_hash: String,
    pub tests: Vec<TestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<bool>,
}

pub type TestPlanResult = TestPlan;

impl Method for TestPlanParams {
    const NAME: &'static str = "grid.test.plan";
    type Result = TestPlanResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfereParams {
    pub other_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<GitRev>,
}

pub type InterfereResult = InterfereReport;

impl Method for InterfereParams {
    const NAME: &'static str = "grid.interfere";
    type Result = InterfereResult;
}

/// Every method the contract declares, in contract order.
pub const METHODS: [&str; 23] = [
    "grid.version",
    "grid.snapshot.current",
    "grid.survey.run",
    "grid.resolve",
    "grid.delta",
    "grid.gate.check",
    "grid.query.walk",
    "grid.anchor.carry",
    "grid.trace.ingest",
    "grid.cache.check",
    "grid.cache.record",
    "grid.scope",
    "grid.plan.lint",
    "grid.plan.estimate",
    "grid.edit.apply",
    "grid.todo",
    "grid.check",
    "grid.trace.diff",
    "grid.suspects",
    "grid.bisect",
    "grid.drift",
    "grid.test.plan",
    "grid.interfere",
];
