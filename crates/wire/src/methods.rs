//! Method param and result types, one per wire method. GENERATED.

use super::*;
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionParams {

}

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
pub struct SnapshotCurrentParams {

}

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

pub type DeltaResult = MadeGood;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateCheckParams {
    pub track: Track,
    pub head: SnapshotOrRev,
}

pub type GateCheckResult = XteVerdict;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryWalkParams {
    pub query: LeadlineQuery,
}

pub type QueryWalkResult = LeadlineResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorCarryParams {
    pub anchors: Vec<Anchor>,
    pub to: SnapshotId,
}

pub type AnchorCarryResult = Vec<CarriedAnchor>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeIngestParams {
    pub wake: Wake,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeIngestResult {
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
    pub wake_id: Option<String>,
    pub results: Vec<CacheResultEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheRecordResult {
    pub recorded: u64,
    pub rejected: Vec<CacheRecordResultRejectedItem>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeParams {
    pub callsigns: Vec<CallsignPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<SnapshotOrRev>,
}

pub type ScopeResult = ScopePack;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackLintParams {
    pub track: Track,
}

pub type TrackLintResult = LintResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackEstimateParams {
    pub track: Track,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SnapshotOrRev>,
}

pub type TrackEstimateResult = Estimate;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditApplyParams {
    pub edits: Vec<EditSpec>,
}

pub type EditApplyResult = EditResult;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TodoParams {
    pub track: TrackId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<SnapshotOrRev>,
}

pub type TodoResult = TodoList;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPackParams {
    pub track: TrackId,
    pub head: SnapshotOrRev,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<WakeId>,
}

pub type ReviewPackResult = ReviewPack;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeDiffParams {
    pub green: WakeId,
    pub red: WakeId,
}

pub type WakeDiffResult = WakeDiff;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuspectsParams {
    pub test: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake: Option<WakeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<SnapshotOrRev>,
}

pub type SuspectsResult = Suspects;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BisectParams {
    pub good: SnapshotOrRev,
    pub bad: SnapshotOrRev,
    pub test: String,
}
