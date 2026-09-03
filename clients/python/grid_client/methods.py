"""GENERATED from docs/wire-v1.openrpc.json by scripts/gen-wire.py - do not edit by hand."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Literal, NotRequired, TypeAlias, TypedDict, Unpack, cast

from . import schemas


WIRE_VERSION = "1.1.0"


METHODS = (
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
)


WIRE_ERRORS: dict[int, str] = {
    -32002: "NoSnapshot",
    -32003: "UnknownSnapshot",
    -32004: "SurveyFailed",
    -32005: "UnsupportedLang",
    -32006: "InvalidPlan",
    -32001: "CorruptSegment",
    -32007: "UnknownPlan",
    -32008: "UnknownTrace",
}


VersionParams = TypedDict(
    "VersionParams",
    {},
)


VersionResult = TypedDict(
    "VersionResult",
    {
        # Semver of this contract, e.g. 0.1.0
        "wire": str,
        # grid binary version
        "engine": str,
        # Porcelain grammar semver; normative in docs/PORCELAIN.md
        "porcelain": NotRequired[str],
        "snapshot": NotRequired["schemas.SnapshotId"],
    },
)


SnapshotCurrentParams = TypedDict(
    "SnapshotCurrentParams",
    {},
)


SnapshotCurrentResult: TypeAlias = "schemas.Snapshot"


SurveyRunParams = TypedDict(
    "SurveyRunParams",
    {
        "git_rev": NotRequired["schemas.GitRev"],
        # Absolute path to a worktree to survey instead of the daemon's default repo
        "worktree": NotRequired[str],
    },
)


SurveyRunResult: TypeAlias = "schemas.Snapshot"


ResolveParams = TypedDict(
    "ResolveParams",
    {
        "locator": "schemas.Locator",
        "snapshot": NotRequired["schemas.SnapshotId"],
    },
)


ResolveResult: TypeAlias = list["schemas.DefRef"]


DeltaParams = TypedDict(
    "DeltaParams",
    {
        "base": "schemas.SnapshotOrRev",
        "head": "schemas.SnapshotOrRev",
    },
)


DeltaResult: TypeAlias = "schemas.Diff"


GateCheckParams = TypedDict(
    "GateCheckParams",
    {
        "plan": "schemas.Plan",
        "head": "schemas.SnapshotOrRev",
    },
)


GateCheckResult: TypeAlias = "schemas.GateVerdict"


QueryWalkParams = TypedDict(
    "QueryWalkParams",
    {
        "query": "schemas.WalkQuery",
    },
)


QueryWalkResult: TypeAlias = "schemas.WalkResult"


AnchorCarryParams = TypedDict(
    "AnchorCarryParams",
    {
        "anchors": list["schemas.Anchor"],
        "to": "schemas.SnapshotId",
    },
)


AnchorCarryResult: TypeAlias = list["schemas.CarriedAnchor"]


TraceIngestParams = TypedDict(
    "TraceIngestParams",
    {
        "trace": "schemas.Trace",
    },
)


TraceIngestResult = TypedDict(
    "TraceIngestResult",
    {
        "observed_edges": int,
        "resolved": int,
        "unresolved_symbols": list[str],
    },
)


CacheCheckParams = TypedDict(
    "CacheCheckParams",
    {
        "request": "schemas.CacheCheckRequest",
    },
)


CacheCheckResult: TypeAlias = "schemas.CacheCheckResult"


CacheRecordParams = TypedDict(
    "CacheRecordParams",
    {
        "kind": "CacheRecordKind",
        "env_hash": str,
        "snapshot": NotRequired["schemas.SnapshotId"],
        # Trace produced by the recorded run, if any; stored on each entry.
        "trace_id": NotRequired[str],
        "results": list["schemas.CacheResultEntry"],
    },
)


CacheRecordResult = TypedDict(
    "CacheRecordResult",
    {
        "recorded": int,
        # Entries grid refused to store, e.g. closure unresolved or fuel-exhausted at key-freeze
        # time. Rejection is the record-side face of one-directional soundness.
        "rejected": list["CacheRecordResultRejectedItem"],
        # Entries whose key already held the opposite verdict: same closure, same environment, a
        # different answer. The closure proves the charted code did not change, so the claim is
        # exactly "not the charted code" — a race is code, and grid does not say flake. Detected
        # from the last entry at record time; the new verdict is stored regardless.
        "nondeterministic": NotRequired[list["CacheRecordResultNondeterministicItem"]],
    },
)


CacheRecordKind: TypeAlias = Literal["typecheck", "lint", "test"]


CacheRecordResultRejectedItem = TypedDict(
    "CacheRecordResultRejectedItem",
    {
        "datum": "schemas.Datum",
        "reason": str,
    },
)


# The verdict the same key already held.
CacheRecordResultNondeterministicItemPrevious: TypeAlias = Literal["pass", "fail"]


CacheRecordResultNondeterministicItem = TypedDict(
    "CacheRecordResultNondeterministicItem",
    {
        "datum": "schemas.Datum",
        # The verdict the same key already held.
        "previous": "CacheRecordResultNondeterministicItemPrevious",
        "previous_at": str,
    },
)


ScopeParams = TypedDict(
    "ScopeParams",
    {
        "callsigns": list["schemas.CallsignPattern"],
        "depth": NotRequired[int],
        "snapshot": NotRequired["schemas.SnapshotOrRev"],
        # Pack body text for hops 0..=bodies. Naming either this or sigs turns the text tiers on,
        # and the walk runs at least max(bodies, sigs) hops.
        "bodies": NotRequired[int],
        # Pack sig text for hops (bodies+1)..=sigs. Raised to bodies when smaller: a body already
        # carries its own sig.
        "sigs": NotRequired[int],
        # Approximate token ceiling (bytes/4). Spent in whole hop layers, farthest first: a layer
        # that does not fit is dropped whole and the horizon stops at the previous one; a body layer
        # that does not fit is demoted to its sigs before it is dropped. Never a partial layer,
        # because a certificate over a partial layer is the lie complete_within exists to prevent.
        "budget": NotRequired[int],
        # Keep only definitions whose datum or sig moved since this snapshot — the delta map a
        # returning agent reads instead of the repo. Resolved against epochs the store already
        # retains; grid.scope never surveys.
        "since": NotRequired["schemas.SnapshotOrRev"],
    },
)


ScopeResult: TypeAlias = "schemas.ScopePack"


PlanLintParams = TypedDict(
    "PlanLintParams",
    {
        "plan": "schemas.Plan",
    },
)


PlanLintResult: TypeAlias = "schemas.LintResult"


PlanEstimateParams = TypedDict(
    "PlanEstimateParams",
    {
        "plan": "schemas.Plan",
        "base": NotRequired["schemas.SnapshotOrRev"],
    },
)


PlanEstimateResult: TypeAlias = "schemas.Estimate"


EditApplyParams = TypedDict(
    "EditApplyParams",
    {
        "edits": list["schemas.EditSpec"],
    },
)


EditApplyResult: TypeAlias = "schemas.EditResult"


TodoParams = TypedDict(
    "TodoParams",
    {
        "plan": "schemas.PlanId",
        "head": NotRequired["schemas.SnapshotOrRev"],
    },
)


TodoResult: TypeAlias = "schemas.TodoList"


CheckParams = TypedDict(
    "CheckParams",
    {
        "plan": NotRequired["schemas.PlanId"],
        "base": NotRequired["schemas.SnapshotOrRev"],
        "head": "schemas.SnapshotOrRev",
        "trace": NotRequired["schemas.TraceId"],
        "compile": NotRequired[bool],
        "quick": NotRequired[bool],
    },
)


CheckResult: TypeAlias = "schemas.CheckResult"


TraceDiffParams = TypedDict(
    "TraceDiffParams",
    {
        "green": "schemas.TraceId",
        "red": "schemas.TraceId",
    },
)


TraceDiffResult: TypeAlias = "schemas.TraceDiff"


SuspectsParams = TypedDict(
    "SuspectsParams",
    {
        "test": str,
        "trace": NotRequired["schemas.TraceId"],
        "base": NotRequired["schemas.SnapshotOrRev"],
        # Select the closure evidence mode and name the snapshot bodies are compared against.
        # Candidates are the test's transitive closure at head intersected with the base-side half
        # of the membership diff since this snapshot. A snapshot the retained chain never held is a
        # typed refusal, never an empty ranking; a closure that runs past its fuel refuses the whole
        # answer rather than serving a shorter one.
        "closure_since": NotRequired["schemas.SnapshotOrRev"],
    },
)


SuspectsResult: TypeAlias = "schemas.Suspects"


BisectParams = TypedDict(
    "BisectParams",
    {
        "good": "schemas.SnapshotOrRev",
        "bad": "schemas.SnapshotOrRev",
        "test": str,
    },
)


BisectResult: TypeAlias = "schemas.BisectResult"


DriftParams = TypedDict(
    "DriftParams",
    {
        "base": "schemas.SnapshotOrRev",
        "head": "schemas.SnapshotOrRev",
        "floor": NotRequired["schemas.Grade"],
        "plan": NotRequired["schemas.Plan"],
        # Also enumerate every known test and report the ones whose recorded verdict no longer
        # applies.
        "tests": NotRequired[bool],
        # Also check every changed type deriving Deserialize against the rules old bytes care about.
        # Heuristic unless a standing `persisted` invariant names the type, in which case the
        # finding gates at modeled.
        "compat": NotRequired[bool],
    },
)


DriftResult: TypeAlias = "schemas.DriftReport"


TestPlanParams = TypedDict(
    "TestPlanParams",
    {
        "base": NotRequired["schemas.SnapshotOrRev"],
        # The environment the run would happen in; a replayed verdict must have been recorded under
        # the same one.
        "env_hash": str,
        # The runner's own inventory. Grid never guesses what a test is.
        "tests": list["schemas.TestId"],
        # Absent or true: an unselected test's verdict is looked up and replayed. False: nothing is
        # looked up, and every unselected test is selected as not_recorded.
        "replay": NotRequired[bool],
    },
)


TestPlanResult: TypeAlias = "schemas.TestPlan"


InterfereParams = TypedDict(
    "InterfereParams",
    {
        # Filesystem path to the other worktree's root. Its store is at <other_root>/.grid and is
        # only ever read.
        "other_root": str,
        # The revision both sides are compared against. Defaults to the merge base of the two
        # worktrees' heads.
        "base": NotRequired["schemas.GitRev"],
    },
)


InterfereResult: TypeAlias = "schemas.InterfereReport"


class Methods:
    """One method per wire method; `call` is the transport a subclass supplies."""

    def call(self, method: str, params: Mapping[str, object]) -> object:
        raise NotImplementedError

    # Report wire and engine versions plus the current snapshot, if any.
    def version(self, **params: Unpack[VersionParams]) -> VersionResult:
        return cast("VersionResult", self.call("grid.version", params))

    # Return the currently published snapshot.
    def snapshot_current(self, **params: Unpack[SnapshotCurrentParams]) -> SnapshotCurrentResult:
        return cast("SnapshotCurrentResult", self.call("grid.snapshot.current", params))

    # Run a survey of the repo (or a specific rev/worktree) and publish a new snapshot. Incremental
    # by content hash; idempotent for an unchanged tree.
    def survey_run(self, **params: Unpack[SurveyRunParams]) -> SurveyRunResult:
        return cast("SurveyRunResult", self.call("grid.survey.run", params))

    # Resolve a locator (datum, position, callsign, or file:line) to definition references. Unknown
    # locators resolve to an empty list, not an error.
    def resolve(self, **params: Unpack[ResolveParams]) -> ResolveResult:
        return cast("ResolveResult", self.call("grid.resolve", params))

    # Compute the actual delta (Diff) between two snapshots or revs. Datum set-difference plus edge
    # changes with move detection.
    def delta(self, **params: Unpack[DeltaParams]) -> DeltaResult:
        return cast("DeltaResult", self.call("grid.delta", params))

    # Check a Plan (promised delta + expected tests + invariants) against the Diff at head. The
    # verdict feeds the Buildhost merge-gate pipeline.
    def gate_check(self, **params: Unpack[GateCheckParams]) -> GateCheckResult:
        return cast("GateCheckResult", self.call("grid.gate.check", params))

    # Run a walk walk: budgeted, evidence-filtered multi-hop traversal. Reachability sweep when 'to'
    # is absent; path-finding when present.
    def query_walk(self, **params: Unpack[QueryWalkParams]) -> QueryWalkResult:
        return cast("QueryWalkResult", self.call("grid.query.walk", params))

    # Carry anchors forward to a target snapshot along the patch DAG. Never errors per-anchor:
    # unresolvable anchors come back status 'orphaned'.
    def anchor_carry(self, **params: Unpack[AnchorCarryParams]) -> AnchorCarryResult:
        return cast("AnchorCarryResult", self.call("grid.anchor.carry", params))

    # Ingest a trace (runtime trace from a test run). Observations are resolved to datums where
    # possible; unresolved symbols are returned, not dropped silently.
    def trace_ingest(self, **params: Unpack[TraceIngestParams]) -> TraceIngestResult:
        return cast("TraceIngestResult", self.call("grid.trace.ingest", params))

    # Look up stored verification verdicts. Hits return the recorded verdict (pass or fail) for an
    # identical per-kind key; misses mean run it and record via grid.cache.record.
    def cache_check(self, **params: Unpack[CacheCheckParams]) -> CacheCheckResult:
        return cast("CacheCheckResult", self.call("grid.cache.check", params))

    # Record verification verdicts, populating the cache that grid.cache.check reads. Grid freezes
    # each entry's per-kind closure key at record time; definitions whose closure cannot be fully
    # established are rejected, not stored.
    def cache_record(self, **params: Unpack[CacheRecordParams]) -> CacheRecordResult:
        return cast("CacheRecordResult", self.call("grid.cache.record", params))

    # Context pack for a planned edit: signatures and handles (not bodies), edges in and out, in-
    # scope invariants, open anchors, churn. With bodies or sigs, the text itself as well — in whole
    # hop layers under a token budget, closed by a horizon naming the depth each tier is complete
    # to. Token cost proportional to blast radius.
    def scope(self, **params: Unpack[ScopeParams]) -> ScopeResult:
        return cast("ScopeResult", self.call("grid.scope", params))

    # Validate a plan before code exists: promised callsigns must resolve or plausibly pattern-
    # match; promised edges are checked against standing invariants. Pure validation — no survey.
    def plan_lint(self, **params: Unpack[PlanLintParams]) -> PlanLintResult:
        return cast("PlanLintResult", self.call("grid.plan.lint", params))

    # Blast radius as numbers: definitions in the promised delta's closure, tests in reverse
    # closure, predicted cache misses. Mechanical footing for effort estimation.
    def plan_estimate(self, **params: Unpack[PlanEstimateParams]) -> PlanEstimateResult:
        return cast("PlanEstimateResult", self.call("grid.plan.estimate", params))

    # The only write path besides survey. Coordinate-addressed edits with compare-and-swap:
    # base_datum must equal the current datum or the edit conflicts; absent base_datum means
    # creation (target must not exist) — grid's own config bootstrap uses this path, there is no
    # separate init write. Daemon-serialized.
    def edit_apply(self, **params: Unpack[EditApplyParams]) -> EditApplyResult:
        return cast("EditApplyResult", self.call("grid.edit.apply", params))

    # A view over grid.gate.check's promise accounting — not a second delta engine. The plan's
    # promises partitioned into done and pending, with a fraction over all promise kinds.
    def todo(self, **params: Unpack[TodoParams]) -> TodoResult:
        return cast("TodoResult", self.call("grid.todo", params))

    # Everything the change did that nobody checked, in one call: drift suspects, the compile
    # record's uncompiled callers, adherence to a plan when one is named, stale verdicts, covenant
    # fires and declared compat findings gate; weak tests, uncovered definitions, clone-of and
    # undeclared compat findings rank. Every section is the corresponding verb's own result, and a
    # section that refuses refuses the whole check.
    def check(self, **params: Unpack[CheckParams]) -> CheckResult:
        return cast("CheckResult", self.call("grid.check", params))

    # Two traces compared at call granularity; the first divergent hop is where behavior forked.
    # Requires trace retention.
    def trace_diff(self, **params: Unpack[TraceDiffParams]) -> TraceDiffResult:
        return cast("TraceDiffResult", self.call("grid.trace.diff", params))

    # Fault localization, in two evidence modes. Without closure_since: the failing test's observed
    # closure intersected with the recent delta, ranked by Ochiai — requires trace retention and a
    # populated verdict cache. With closure_since: the definitions in the test's static closure at
    # head whose bodies moved since that snapshot, ranked by hop distance — no run history required,
    # and heuristic, because a candidate is a definition that changed and is reachable, not a proven
    # cause.
    def suspects(self, **params: Unpack[SuspectsParams]) -> SuspectsResult:
        return cast("SuspectsResult", self.call("grid.suspects", params))

    # Bisect over the patch DAG, accelerated by the verdict cache: each probe re-runs only tests
    # whose closure changed. Requires a populated cache.
    def bisect(self, **params: Unpack[BisectParams]) -> BisectResult:
        return cast("BisectResult", self.call("grid.bisect", params))

    # Dependents of changed interfaces: definitions that depend on something whose sig moved, and
    # that did not themselves change. The set the compiler cannot enumerate across languages, re-
    # exports and processes.
    def drift(self, **params: Unpack[DriftParams]) -> DriftResult:
        return cast("DriftResult", self.call("grid.drift", params))

    # Test selection as data, no running: which of the runner's tests a change can reach, which
    # recorded verdicts still apply, and which changed definitions no test reaches. The CLI spawns
    # the runner and records verdicts through grid.cache.record; the daemon only answers the plan.
    def test_plan(self, **params: Unpack[TestPlanParams]) -> TestPlanResult:
        return cast("TestPlanResult", self.call("grid.test.plan", params))

    # Semantic overlap between two worktrees of one repository, against their common base: the
    # definitions both edited, and each side's drift suspects that land in the other's edits. What
    # git's merge cannot see — a moved signature here, a caller of it there, no file in common. The
    # other root's store is opened read-only and never written; a store that cannot be read, is
    # superseded, or does not hold the base is a refusal, never an answer of 'no interference'.
    def interfere(self, **params: Unpack[InterfereParams]) -> InterfereResult:
        return cast("InterfereResult", self.call("grid.interfere", params))
