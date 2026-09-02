//! Method param and result types, one per wire method. GENERATED.

use serde::{Deserialize, Serialize};

use super::{
    Anchor, CacheCheckRequest, CacheResultEntry, CallsignPattern, CarriedAnchor, Datum, DefRef,
    Diff, DriftReport, EditResult, EditSpec, Estimate, GateVerdict, GitRev, Grade, LintResult,
    Locator, Plan, PlanId, ScopePack, Snapshot, SnapshotId, SnapshotOrRev, Suspects, TestId,
    TestPlan, TodoList, Trace, TraceDiff, TraceId, WalkQuery, WalkResult,
};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCurrentParams {}

pub type SnapshotCurrentResult = Snapshot;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyRunParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_rev: Option<GitRev>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

pub type SurveyRunResult = Snapshot;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveParams {
    pub locator: Locator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotId>,
}

pub type ResolveResult = Vec<DefRef>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeltaParams {
    pub base: SnapshotOrRev,
    pub head: SnapshotOrRev,
}

pub type DeltaResult = Diff;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateCheckParams {
    pub plan: Plan,
    pub head: SnapshotOrRev,
}

pub type GateCheckResult = GateVerdict;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryWalkParams {
    pub query: WalkQuery,
}

pub type QueryWalkResult = WalkResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorCarryParams {
    pub anchors: Vec<Anchor>,
    pub to: SnapshotId,
}

pub type AnchorCarryResult = Vec<CarriedAnchor>;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheCheckParams {
    pub request: CacheCheckRequest,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanLintParams {
    pub plan: Plan,
}

pub type PlanLintResult = LintResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEstimateParams {
    pub plan: Plan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SnapshotOrRev>,
}

pub type PlanEstimateResult = Estimate;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditApplyParams {
    pub edits: Vec<EditSpec>,
}

pub type EditApplyResult = EditResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoParams {
    pub plan: PlanId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<SnapshotOrRev>,
}

pub type TodoResult = TodoList;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckParams {
    pub plan: PlanId,
    pub head: SnapshotOrRev,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceDiffParams {
    pub green: TraceId,
    pub red: TraceId,
}

pub type TraceDiffResult = TraceDiff;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BisectParams {
    pub good: SnapshotOrRev,
    pub bad: SnapshotOrRev,
    pub test: String,
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
