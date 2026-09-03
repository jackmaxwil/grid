"""GENERATED from docs/wire-v1.openrpc.json by scripts/gen-wire.py - do not edit by hand."""

from __future__ import annotations

from typing import Literal, NewType, NotRequired, TypeAlias, TypedDict, Union


# Definition identity: blake3 of the normalized AST (alpha-renamed locals; formatting and comments
# excluded). The reference all coordinates are measured from.
Datum = NewType("Datum", str)


# Content-derived snapshot identity: blake3 of the canonical segment manifest the survey produced —
# never a minted id. A clean tree's snapshot is a pure function of (rev, segment-format epoch); a
# dirty tree folds in the worktree diff hash. Idempotent by construction: re-surveying an identical
# state returns the same id, and identical states dedupe to one snapshot. Porcelain displays @rev or
# @rev+tree, never this raw form. All segment writes are serialized through the daemon (single
# writer); the CLI never writes segments directly.
SnapshotId = NewType("SnapshotId", str)


# Git commit SHA, abbreviated or full.
GitRev = NewType("GitRev", str)


SnapshotOrRevSnapshot = TypedDict(
    "SnapshotOrRevSnapshot",
    {
        "snapshot": "SnapshotId",
    },
)


SnapshotOrRevGitRev = TypedDict(
    "SnapshotOrRevGitRev",
    {
        "git_rev": "GitRev",
    },
)


# A published snapshot, or a git rev the daemon will survey on demand.
SnapshotOrRev: TypeAlias = Union["SnapshotOrRevSnapshot", "SnapshotOrRevGitRev"]


# Language adapter that produced a definition.
Lang: TypeAlias = Literal["ts", "js", "py", "rs"]


PathStep = TypedDict(
    "PathStep",
    {
        # Structural role within the parent node, language-scoped: e.g. 'param', 'body', 'stmt',
        # 'call', 'arm', 'field'.
        "role": str,
        # Ordinal among siblings sharing the role.
        "index": int,
    },
)


# Interior coordinate: structural path inside a definition's AST, by role, never by byte offset.
# Empty array addresses the whole definition.
AstPath: TypeAlias = list["PathStep"]


# An exact position in code: a datum plus an optional interior path.
Position = TypedDict(
    "Position",
    {
        "datum": "Datum",
        "path": NotRequired["AstPath"],
    },
)


# Fully qualified human name, dot-separated, e.g. 'cortana.reaper.TagContract.validate'. Mutable: a
# rename rebinds the callsign, the datum does not change.
Callsign = NewType("Callsign", str)


# Glob over callsigns: '*' matches one segment, '**' matches any number, e.g. 'cortana.reaper.**'.
# Promise-space references use patterns because unwritten code has no datum.
CallsignPattern = NewType("CallsignPattern", str)


# Input convenience: callsign plus a datum-prefix disambiguator, e.g.
# 'cortana.reaper.TagContract.validate@9f2c'. The prefix must resolve uniquely within the snapshot
# (git short-SHA auto-lengthening). Display default is the bare callsign; results keep full datums —
# handles are for input and for renders where identity beyond the name matters.
Handle = NewType("Handle", str)


LocatorDatum = TypedDict(
    "LocatorDatum",
    {
        "datum": "Datum",
    },
)


LocatorPosition = TypedDict(
    "LocatorPosition",
    {
        "position": "Position",
    },
)


LocatorCallsign = TypedDict(
    "LocatorCallsign",
    {
        "callsign": "Callsign",
    },
)


LocatorHandle = TypedDict(
    "LocatorHandle",
    {
        "handle": "Handle",
    },
)


# Advisory: resolves via the current materialization; prefer datum or callsign.
LocatorFileLine = TypedDict(
    "LocatorFileLine",
    {
        "file": str,
        "line": int,
    },
)


# Any way of pointing at code. Exactly one variant.
Locator: TypeAlias = Union["LocatorDatum", "LocatorPosition", "LocatorCallsign", "LocatorHandle", "LocatorFileLine"]


DefRef = TypedDict(
    "DefRef",
    {
        "datum": "Datum",
        "callsign": NotRequired["Callsign"],
        "lang": NotRequired["Lang"],
        # One-line declaration as surveyed, e.g. 'def validate(self, tags: list[Tag]) -> Receipt'.
        # Lets grid.scope return signatures without bodies.
        "signature": NotRequired[str],
        # The conjunction of every #[cfg(...)] predicate conditioning the definition — on the item,
        # on each enclosing module, and on the cargo target directory — as written minus whitespace,
        # all(...)-joined when nested; a cfg_attr(...) is recorded whole. Empty when nothing
        # conditions it. Absent when the snapshot predates segment epoch 4 or the frontend has no
        # such notion (Python).
        "cfg": NotRequired[str],
        "materialization": NotRequired["DefRefMaterialization"],
    },
)


# Advisory only: where the definition currently happens to live. Files are projections; never store
# this as an address.
DefRefMaterialization = TypedDict(
    "DefRefMaterialization",
    {
        "file": str,
        "line": int,
    },
)


# Typed edge kinds of the chart. The call graph is the 'calls' + 'dispatches_to' view.
Relation: TypeAlias = Literal["calls", "constructs", "reads_field", "writes_field", "implements", "dispatches_to", "flows_into", "serializes_to", "deserializes_from", "registers_under", "sends_over", "links_against"]


# Evidence grade (after CATZOC), listed strongest first: proven = compiler-grade static; observed =
# seen in a trace at least once; modeled = registry/schema/route unification; heuristic = name or
# shape match. Filters use at_or_above semantics over this order.
Grade: TypeAlias = Literal["proven", "observed", "modeled", "heuristic"]


Edge = TypedDict(
    "Edge",
    {
        "src": "Datum",
        "relation": "Relation",
        "dst": "Datum",
        "grade": "Grade",
        # For modeled edges, the bridge that established the connection, e.g. 'route:/api/cards/:id'
        # or 'key:card.archived'.
        "via": NotRequired[str],
    },
)


# One edit edge in the patch DAG (the log): a definition's predecessor and successor identities.
Edit = TypedDict(
    "Edit",
    {
        "from": "Datum",
        "to": "Datum",
        "git_rev": NotRequired["GitRev"],
        "at": NotRequired[str],
    },
)


SnapshotStats = TypedDict(
    "SnapshotStats",
    {
        "defs": int,
        "edges": int,
        "langs": NotRequired[list["Lang"]],
    },
)


# What the survey declined to bind, split by why. Absent from a snapshot nobody re-surveyed in this
# run, so an unknown count is never reported as zero. external, method, internal and macros add up
# to the survey's declined total.
SnapshotDeclined = TypedDict(
    "SnapshotDeclined",
    {
        # Names that were never this tree's to hold — std/core/alloc or another crate in Rust, a
        # top-level module the tree does not root in Python.
        "external": int,
        # Calls through a receiver the file could not type, or typed and the rule refused. Counted,
        # never guessed.
        "method": int,
        # Names this tree could have held and the resolver could not bind: glob-supplied, ambiguous,
        # an unresolved re-export. The hole.
        "internal": int,
        # Of method: the calls whose receiver was typed to a charted type and the rule refused with
        # a reason — a trait candidate, a generic impl, no inherent method, a wrapper. A sub-count
        # of method, not a fourth population; absent from a snapshot surveyed before it was counted.
        "refused": NotRequired[int],
        # Macro invocations inside definitions, each counted once. A macro's tokens are not a place
        # grid can bind names, so nothing inside one is resolved or guessed; the invocation is
        # counted so an argument list grid did not read never looks like an absent one. The fourth
        # population beside external, method and internal; absent from a snapshot surveyed before it
        # was counted.
        "macros": NotRequired[int],
    },
)


Snapshot = TypedDict(
    "Snapshot",
    {
        "snapshot_id": "SnapshotId",
        "git_rev": "GitRev",
        "created_at": str,
        # Present iff the surveyed tree was dirty: blake3 of the canonical worktree diff vs git_rev.
        # Porcelain renders @rev+<prefix>.
        "worktree_diff_hash": NotRequired[str],
        "stats": "SnapshotStats",
        # Repo-relative paths of tracked files a frontend claims whose parser rejected them. Charted
        # as nothing — never as a file with nothing in it — and named, so their absence is never
        # read as a deletion. Absent from a snapshot nobody re-surveyed in this run; empty when the
        # survey read every file.
        "unparseable": NotRequired[list[str]],
        # Repo-relative paths of tracked files a frontend claims that are not UTF-8 or could not be
        # read at all. Charted as nothing and named, exactly as unparseable files are. Absent from a
        # snapshot nobody re-surveyed in this run.
        "unreadable": NotRequired[list[str]],
        # What the survey declined to bind, split by why. Absent from a snapshot nobody re-surveyed
        # in this run, so an unknown count is never reported as zero. external, method, internal and
        # macros add up to the survey's declined total.
        "declined": NotRequired["SnapshotDeclined"],
    },
)


DefChangeAction: TypeAlias = Literal["added", "removed", "modified", "moved"]


# added: 'after' only. removed: 'before' only. modified: both (this pair is a Edit).
DefChange = TypedDict(
    "DefChange",
    {
        "action": "DefChangeAction",
        "callsign": "Callsign",
        "before": NotRequired["Datum"],
        "after": NotRequired["Datum"],
        # For action 'moved': the callsign the datum answered to before. A rename rebinds the
        # callsign; the datum does not change.
        "moved_from": NotRequired["Callsign"],
        # Whether the interface identity changed, when both snapshots carry sigs. Absent means at
        # least one side predates segment epoch 3, and a consumer treats unknown as changed.
        "sig_changed": NotRequired[bool],
    },
)


EdgeChangeAction: TypeAlias = Literal["added", "removed", "moved"]


# Present iff action is 'moved': the edge's previous endpoints. A move is not a remove+add.
EdgeChangeMovedFrom = TypedDict(
    "EdgeChangeMovedFrom",
    {
        "src": "Datum",
        "dst": "Datum",
    },
)


EdgeChange = TypedDict(
    "EdgeChange",
    {
        "action": "EdgeChangeAction",
        "edge": "Edge",
        # Present iff action is 'moved': the edge's previous endpoints. A move is not a remove+add.
        "moved_from": NotRequired["EdgeChangeMovedFrom"],
    },
)


# The actual delta, datum-space. Deterministically ordered: defs by callsign, edges by (relation,
# src, dst).
Diff = TypedDict(
    "Diff",
    {
        "base": "SnapshotId",
        "head": "SnapshotId",
        "defs": list["DefChange"],
        "edges": list["EdgeChange"],
    },
)


PlanDefPromiseAction: TypeAlias = Literal["add", "remove", "modify", "move"]


PlanDefPromise = TypedDict(
    "PlanDefPromise",
    {
        "action": "PlanDefPromiseAction",
        "callsign": "CallsignPattern",
        # A promise's claim about whether callers can see the change, checked against
        # DefChange.sig_changed on a modify or move promise (rejected on add/remove by plan.lint,
        # exit 2). true: made good only by a sig_changed of true or absent (unknown is treated as
        # moved, conservative). false: made good only by sig_changed == false; a sig move under this
        # promise is unaccounted with reason 'interface'. Absent: today's behavior, either makes it
        # good.
        "interface": NotRequired[bool],
    },
)


PlanEdgePromiseAction: TypeAlias = Literal["add", "remove", "move"]


PlanEdgePromise = TypedDict(
    "PlanEdgePromise",
    {
        "action": "PlanEdgePromiseAction",
        "src": "CallsignPattern",
        "relation": "Relation",
        "dst": "CallsignPattern",
    },
)


# The promised delta, name-space (patterns), authored by the planner before code exists.
PlanDelta = TypedDict(
    "PlanDelta",
    {
        "defs": list["PlanDefPromise"],
        "edges": list["PlanEdgePromise"],
    },
)


ExpectedTestExpect: TypeAlias = Literal["pass", "fail", "skip"]


ExpectedTest = TypedDict(
    "ExpectedTest",
    {
        # Test node id as the runner reports it, e.g.
        # 'tests/test_tag_contract.py::test_retry_window'.
        "test": str,
        "expect": "ExpectedTestExpect",
        # True if the plan promises this test does not exist at base.
        "new": NotRequired[bool],
    },
)


# Edge shape for invariants. Omitted relation means any. at_or_above defaults to 'heuristic' (i.e.
# any evidence).
EdgeRule = TypedDict(
    "EdgeRule",
    {
        "src": "CallsignPattern",
        "relation": NotRequired["Relation"],
        "dst": "CallsignPattern",
        "at_or_above": NotRequired["Grade"],
    },
)


InvariantRuleForbidEdge = TypedDict(
    "InvariantRuleForbidEdge",
    {
        "forbid_edge": "EdgeRule",
    },
)


InvariantRuleRequireEdge = TypedDict(
    "InvariantRuleRequireEdge",
    {
        "require_edge": "EdgeRule",
    },
)


InvariantRulePersisted = TypedDict(
    "InvariantRulePersisted",
    {
        # Types matching this pattern have bytes on disk written by an older version of themselves.
        # The human fact a static check cannot establish, and the one that promotes a Deserialize
        # compatibility finding from a warning to a gate at modeled.
        "persisted": "CallsignPattern",
    },
)


InvariantRule: TypeAlias = Union["InvariantRuleForbidEdge", "InvariantRuleRequireEdge", "InvariantRulePersisted"]


# A standing query checked at every gate, e.g. forbid_edge {src: 'cortana.boards.**', relation:
# 'calls', dst: 'cortana.persistence.**'} — board code may not call persistence directly. The
# pattern language has no negation; state rules in positive scope terms.
Invariant = TypedDict(
    "Invariant",
    {
        "id": str,
        "description": NotRequired[str],
        "rule": "InvariantRule",
    },
)


# The spec: promised delta + expected test outcomes + invariants + tolerances. Authored by the
# planner; checked by grid.gate.check.
Plan = TypedDict(
    "Plan",
    {
        "plan_id": str,
        "pr": NotRequired[int],
        # Plan revision this plan was authored against, e.g. 'v3'.
        "plan_rev": NotRequired[str],
        "base": "SnapshotOrRev",
        "delta": "PlanDelta",
        "tests": NotRequired[list["ExpectedTest"]],
        "invariants": NotRequired[list["Invariant"]],
        # Surplus edge changes matching any rule here do not hold the gate; they surface in
        # GateVerdict.tolerated. The pressure valve against gate fatigue from incidental helper
        # edges.
        "tolerate": NotRequired[list["EdgeRule"]],
    },
)


TestComparisonExpected: TypeAlias = Literal["pass", "fail", "skip"]


TestComparisonActual: TypeAlias = Literal["pass", "fail", "skip", "missing"]


TestComparison = TypedDict(
    "TestComparison",
    {
        "test": str,
        "expected": "TestComparisonExpected",
        "actual": "TestComparisonActual",
        "ok": bool,
    },
)


# The witness path is the concrete evidence chain that violates the invariant.
InvariantViolation = TypedDict(
    "InvariantViolation",
    {
        "id": str,
        "witness": "Path",
    },
)


GateVerdictGate: TypeAlias = Literal["pass", "hold"]


# Cross-plan error. Surplus is computed after tolerate filtering. gate is 'hold' iff missing,
# defs_missing, surplus, defs_unaccounted, a failed TestComparison, or an invariant violation is
# non-empty; 'pass' otherwise. The verdict itemizes divergence — it never says only 'hold'.
GateVerdict = TypedDict(
    "GateVerdict",
    {
        "gate": "GateVerdictGate",
        "plan_id": str,
        "head": "SnapshotId",
        # Promises fulfilled by the Diff.
        "matched": list["PlanEdgePromise"],
        # Promised but not made good.
        "missing": list["PlanEdgePromise"],
        # Made good, not promised, and not tolerated.
        "surplus": list["EdgeChange"],
        # Surplus changes matched by Plan.tolerate; reported for visibility, never a hold cause.
        "tolerated": list["EdgeChange"],
        # Definitions changed by the diff but covered by no def promise.
        "defs_unaccounted": list["DefChange"],
        "tests": list["TestComparison"],
        "invariant_violations": list["InvariantViolation"],
        # Human-readable one-liners, one per contributing hold cause; empty on pass.
        "reasons": NotRequired[list[str]],
        # Def promises no change in the delta made good — the definition side of `missing`, and a
        # hold cause. Optional: a verdict from an engine that predates this field omits it, which
        # reads as "not checked", never as "nothing was pending".
        "defs_missing": NotRequired[list["PlanDefPromise"]],
        # The subset of defs_unaccounted whose sig moved (sig_changed true or absent) and which
        # matches no def promise permitting that move (interface true or absent) — held for reason
        # 'interface'. Always a subset of defs_unaccounted; a member of both lists is held once, not
        # twice.
        "interface_unaccounted": NotRequired[list["DefChange"]],
    },
)


# Traversal budget. Every walk carries fuel; there are no unbounded queries.
Fuel = TypedDict(
    "Fuel",
    {
        "max_hops": NotRequired[int],
        "max_nodes": NotRequired[int],
        # Optional wall-clock guard, deliberately no default. Unlike hop/node budgets it is
        # nondeterministic: results truncated by max_ms are outside the determinism claim. Leave
        # unset for cacheable results.
        "max_ms": NotRequired[int],
    },
)


WalkQueryDirection: TypeAlias = Literal["out", "in", "both"]


WalkQuery = TypedDict(
    "WalkQuery",
    {
        "from": list["Locator"],
        # Present: find paths from 'from' to 'to'. Absent: reachability sweep from 'from'.
        "to": NotRequired[list["Locator"]],
        "direction": NotRequired["WalkQueryDirection"],
        # Restrict traversal to these edge kinds. Absent means all.
        "relations": NotRequired[list["Relation"]],
        "at_or_above": NotRequired["Grade"],
        "fuel": NotRequired["Fuel"],
        "snapshot": NotRequired["SnapshotId"],
    },
)


# edges[i] connects nodes[i] to nodes[i+1]; edges.length = nodes.length - 1. Every hop carries its
# grade — a path's overall confidence is its weakest hop.
Path = TypedDict(
    "Path",
    {
        "nodes": list["Datum"],
        "edges": list["Edge"],
    },
)


WalkResultStatus: TypeAlias = Literal["complete", "aground"]


WalkResult = TypedDict(
    "WalkResult",
    {
        "status": "WalkResultStatus",
        "snapshot": "SnapshotId",
        # Path-finding results (query had 'to').
        "paths": NotRequired[list["Path"]],
        # Sweep results (query had no 'to').
        "reached": NotRequired[list["DefRef"]],
        # Present iff status is 'aground': where the walk ran out of fuel. Resume by issuing a new
        # walk from the frontier.
        "frontier": NotRequired[list["Datum"]],
        "stats": "WalkStats",
    },
)


# Diagnostics, plus one claim. visited/hops/ms are diagnostics and are excluded from the determinism
# comparison surface; complete_within is not a diagnostic — it is a statement about the answer, is a
# pure function of the graph and the query, and belongs to the comparison surface.
WalkStats = TypedDict(
    "WalkStats",
    {
        "visited": int,
        "hops": int,
        "ms": int,
        # Depth through which the answer is complete: every definition reachable within this many
        # hops, under this query's relations and evidence floor, is in the result. On 'complete' it
        # is the effective hop bound. On 'aground' it is the last layer the walk fully expanded
        # before the budget refused; absent when even the roots' own layer did not finish, which
        # certifies nothing and says so.
        "complete_within": NotRequired[int],
    },
)


AnchorKind: TypeAlias = Literal["annotation", "spec_ref"]


# A durable reference into code, owned by a consumer (e.g. an Artifacts review annotation). Grid
# stores no annotation payloads — only positions.
Anchor = TypedDict(
    "Anchor",
    {
        # Storage identity only; renders display the anchor's position handle, never the ULID.
        "anchor_id": str,
        "position": "Position",
        "kind": "AnchorKind",
        "created_at": str,
    },
)


# exact: datum unchanged. carried: followed edits to a successor. orphaned: no successor found;
# 'position' is absent.
CarriedAnchorStatus: TypeAlias = Literal["exact", "carried", "orphaned"]


CarriedAnchor = TypedDict(
    "CarriedAnchor",
    {
        "anchor_id": str,
        # exact: datum unchanged. carried: followed edits to a successor. orphaned: no successor
        # found; 'position' is absent.
        "status": "CarriedAnchorStatus",
        "position": NotRequired["Position"],
        "confidence": float,
        # The edits followed, in order; empty when exact.
        "via": list["Edit"],
    },
)


TraceObservation = TypedDict(
    "TraceObservation",
    {
        "caller": "TraceEndpoint",
        "callee": "TraceEndpoint",
        "count": NotRequired[int],
    },
)


TraceEndpointDatum = TypedDict(
    "TraceEndpointDatum",
    {
        "datum": "Datum",
    },
)


TraceEndpointSymbol = TypedDict(
    "TraceEndpointSymbol",
    {
        # Raw runtime symbol when the instrument cannot know datums, e.g.
        # 'cortana/reaper.py:TagContract.validate'. The daemon resolves it.
        "symbol": str,
    },
)


TraceEndpoint: TypeAlias = Union["TraceEndpointDatum", "TraceEndpointSymbol"]


# A runtime trace from a test run: the path actually traveled. Ingested observations become grade
# 'observed' edges.
Trace = TypedDict(
    "Trace",
    {
        # Storage identity only; renders display the contextual form command@rev/time, never the
        # ULID.
        "trace_id": str,
        "git_rev": "GitRev",
        # The test command that produced this trace.
        "command": NotRequired[str],
        "started_at": NotRequired[str],
        # The chart snapshot this trace's observations were mapped against, set by the coverage
        # ingestion paths (--lcov, --coverage-json). It pins the chart a later reader must grade the
        # trace against without resolving git_rev through git, which a shallow clone or a rewritten
        # history can no longer answer. Absent on a trace a runner emitted directly, where git_rev
        # is all there is.
        "snapshot": NotRequired["SnapshotId"],
        "observations": list["TraceObservation"],
    },
)


CacheCheckRequestKind: TypeAlias = Literal["typecheck", "lint", "test"]


# Cache lookup. Keys are per-kind, all scoped by (env_hash, snapshot). lint keys on span_hash —
# blake3 of the definition's raw source bytes at the snapshot — because comments (noqa, eslint-
# disable) are lint input that the normalized datum deliberately discards. typecheck keys on
# span_hash plus the closure over proven signature-bearing edges (type-ignore comments are self-
# input; dependency signatures live in their datums). test keys on the closure over ALL edges
# regardless of grade — closure traversal ignores at_or_above by design, since a missed dependency
# is a false hit. Soundness is one-directional: a miss for a cacheable definition is acceptable; a
# hit for a closure grid could not fully establish is a contract violation. What the chart cannot
# see — hermeticity, ambient state, monkeypatching — is warranted by the caller, not by grid
# (assurance claim C4).
CacheCheckRequest = TypedDict(
    "CacheCheckRequest",
    {
        "kind": "CacheCheckRequestKind",
        # Caller-computed hash of every verification input not reachable through the chart:
        # toolchain, flags, config files (tsconfig, lint rules), fixtures, golden files, migrations,
        # env vars. The caller warrants its completeness.
        "env_hash": str,
        # Chart snapshot the closures are computed against. Absent means the current published
        # snapshot; the resolved id is echoed in the result. Closures are a function of the chart,
        # so the chart is part of the key.
        "snapshot": NotRequired["SnapshotId"],
        "datums": list["Datum"],
    },
)


# The stored result. fail-hits are the valuable case: skip re-running a known-red verification.
CacheHitVerdict: TypeAlias = Literal["pass", "fail"]


CacheHit = TypedDict(
    "CacheHit",
    {
        "datum": "Datum",
        # The stored result. fail-hits are the valuable case: skip re-running a known-red
        # verification.
        "verdict": "CacheHitVerdict",
        "recorded_at": str,
        # Present when the recorded run also produced a trace.
        "trace_id": NotRequired[str],
        # Present when the hit was looked up for a named test (grid.test.plan): the test whose
        # verdict this is.
        "test": NotRequired["TestId"],
    },
)


CacheResultEntryVerdict: TypeAlias = Literal["pass", "fail"]


CacheResultEntry = TypedDict(
    "CacheResultEntry",
    {
        "datum": "Datum",
        "verdict": "CacheResultEntryVerdict",
    },
)


# {hit.datum} ∪ misses = request.datums, disjoint. Normative: any datum whose closure traversal
# exhausted fuel or touched an unresolved endpoint is a miss — never a hit.
CacheCheckResult = TypedDict(
    "CacheCheckResult",
    {
        # The snapshot the closures were computed against.
        "snapshot": "SnapshotId",
        "hits": list["CacheHit"],
        "misses": list["Datum"],
    },
)


# Plan identity, resolved against the daemon's plan store (.grid/plans).
PlanId = NewType("PlanId", str)


# Trace storage identity (ULID); renders display command@rev/time.
TraceId = NewType("TraceId", str)


ScopePackChurnItem = TypedDict(
    "ScopePackChurnItem",
    {
        "datum": "Datum",
        "edits": int,
    },
)


ScopePack = TypedDict(
    "ScopePack",
    {
        "snapshot": "SnapshotId",
        # Signatures ride on DefRef.signature; bodies are fetched separately and deliberately
        # absent.
        "defs": list["DefRef"],
        "in_edges": list["Edge"],
        "out_edges": list["Edge"],
        # Standing invariants whose scope patterns intersect the requested callsigns.
        "invariants": list["Invariant"],
        "anchors": list["Anchor"],
        # Edit-frequency signal: edit count per definition in scope.
        "churn": list["ScopePackChurnItem"],
        # Body text for every definition within horizon.bodies_complete_within hops, ordered by hop
        # then callsign. Absent unless the request asked for bodies or sigs.
        "bodies": NotRequired[list["PackText"]],
        # Sig text for every definition past the body tier and within horizon.sigs_complete_within
        # hops, ordered by hop then callsign. Absent unless the request asked for bodies or sigs.
        "sigs": NotRequired[list["PackText"]],
        # The depth each tier is complete to. Present exactly when bodies and sigs are: text without
        # a horizon is a bounded answer nobody can bound.
        "horizon": NotRequired["PackHorizon"],
    },
)


PackText = TypedDict(
    "PackText",
    {
        "def": "DefRef",
        # Hops from the nearest root of the pack's walk; 0 is a definition a pattern named.
        "hop": int,
        # The definition's source lines for a body, the frontend's rendered signature for a sig.
        # Read from the worktree when the pack is built, so a file edited since the survey packs
        # what the file says now.
        "text": str,
    },
)


PackHorizon = TypedDict(
    "PackHorizon",
    {
        # Every definition within this many hops has its body in the pack. Absent when no body layer
        # fit the budget — not even the roots' — so the pack certifies no body depth at all.
        "bodies_complete_within": NotRequired[int],
        # Every definition within this many hops has at least its sig in the pack. Absent when not
        # even the roots' sigs fit.
        "sigs_complete_within": NotRequired[int],
        # Approximate token count of the packed text: bytes/4. Grid runs no tokenizer, so this is an
        # estimate and every rendering states it as one.
        "approx_tokens": int,
    },
)


LintFinding = TypedDict(
    "LintFinding",
    {
        # Canonical rendering of the offending promise.
        "promise": str,
        "reason": str,
    },
)


# Pure validation of a plan — no survey, no snapshot involved.
LintResult = TypedDict(
    "LintResult",
    {
        "errors": list["LintFinding"],
        "warnings": list["LintFinding"],
    },
)


Estimate = TypedDict(
    "Estimate",
    {
        "defs_in_closure": int,
        "tests_in_reverse_closure": list[str],
        "predicted_cache_misses": int,
    },
)


EditSpec = TypedDict(
    "EditSpec",
    {
        # Callsign, or callsign@prefix handle. Creation edits (no base_datum) use a bare callsign.
        "target": str,
        # CAS guard: present — the current datum must equal it, or the edit conflicts. Absent —
        # creation: the target must not exist. Grid's own config bootstrap is a creation edit; there
        # is no separate init write path.
        "base_datum": NotRequired["Datum"],
        "new_text": str,
    },
)


EditResultConflictsItem = TypedDict(
    "EditResultConflictsItem",
    {
        "target": str,
        # The stale base_datum; absent for creation conflicts (target already exists).
        "was": NotRequired["Datum"],
        "now": "Datum",
    },
)


# Conflicts are data, not errors: the caller re-reads and retries with fresh coordinates.
EditResult = TypedDict(
    "EditResult",
    {
        # New datums, in edit order.
        "applied": list["Datum"],
        "conflicts": list["EditResultConflictsItem"],
    },
)


# Over all promise kinds: defs + edges + tests.
TodoListFraction = TypedDict(
    "TodoListFraction",
    {
        "num": int,
        "den": int,
    },
)


TodoList = TypedDict(
    "TodoList",
    {
        "plan_id": "PlanId",
        "defs_done": list["PlanDefPromise"],
        "defs_pending": list["PlanDefPromise"],
        "edges_done": list["PlanEdgePromise"],
        "edges_pending": list["PlanEdgePromise"],
        "tests_done": list[str],
        "tests_pending": list[str],
        # Over all promise kinds: defs + edges + tests.
        "fraction": "TodoListFraction",
    },
)


# Everything the change did that nobody checked. Every section is the corresponding verb's own
# result — DriftReport, CompileReport, GateVerdict, StaleTest, WeakTest, CompatFinding — never a re-
# implementation; the exit is the maximum over the gating sections; a section that refuses (aground,
# unknown snapshot, superseded store) refuses the whole check with that reason.
CheckResult = TypedDict(
    "CheckResult",
    {
        # grid.gate.check's own verdict, present when a plan was named. Optional since wire 1.1.
        # Gates: hold is exit 3.
        "verdict": NotRequired["GateVerdict"],
        "why": NotRequired["CheckWhy"],
        # grid.drift's own report over base and head, byte for byte, with its stale_tests, weak and
        # compat lists lifted into the sections below (its unchecked counters stay). Suspects gate:
        # exit 3. Absent only when the check refused before drift answered — never a check with
        # drift silently missing.
        "drift": NotRequired["DriftReport"],
        # Present when .grid/compile.json exists and names the head snapshot this check surveyed:
        # the proven dependents of the changed set the recorded compile did not see. Uncompiled
        # gates: exit 3. Absent, with a warning, when there is no record or it predates the last
        # survey.
        "compile": NotRequired["CompileReport"],
        # grid.drift --tests's stale_tests. Proven-grade: gates, exit 3. Absent under quick.
        "stale": NotRequired[list["StaleTest"]],
        # grid.drift --tests's weak. Heuristic: ranks, never moves the exit. Absent under quick or
        # when no membership image exists.
        "weak": NotRequired[list["WeakTest"]],
        # Changed definitions no traced test reaches — grid.test.plan's uncovered, recomputed over
        # the retained traces as the inventory. Heuristic. Absent under quick or when no trace is
        # retained.
        "uncovered": NotRequired[list["DefRef"]],
        # Definitions added since base whose name-erased shape equals an existing definition's,
        # exact match only. Heuristic. Absent under quick.
        "clones": NotRequired[list["CloneFinding"]],
        # grid.drift --compat's compat findings. A declared one (a persisted invariant) gates at
        # modeled, exit 3; an undeclared one ranks. Absent under quick.
        "compat": NotRequired[list["CompatFinding"]],
        # Present when no gate fired and the check was not quick: the three facts a refactor claim
        # rests on.
        "refactor": NotRequired["RefactorCertificate"],
        # Reported rather than refused by the check itself: today, the compile record being absent
        # or older than the last survey (subject '-', message 'no compile record since @<head
        # prefix>'), which gates nothing on the compile section.
        "warnings": NotRequired[list["DriftWarning"]],
    },
)


# A definition added since base whose shape — the name-erased canonical hash — equals an existing
# definition's. Exact match only; near-duplicates are not claimed. Heuristic: warn <new>  clone-of
# <existing>, ranked, never a gate.
CloneFinding = TypedDict(
    "CloneFinding",
    {
        # The definition added since base.
        "def": "DefRef",
        # A definition head holds that was not added since base and whose name-erased shape is byte-
        # identical to the new one's.
        "existing": "DefRef",
    },
)


# One done record over the check's own data when no gate fired: interfaces held or moved, covering
# tests replayed pass, changed definitions uncovered. It certifies those three facts and nothing
# more — the phrase behavior-preserving appears nowhere.
RefactorCertificate = TypedDict(
    "RefactorCertificate",
    {
        # Changed definitions whose interface identity moved or is unknown — grid.drift's changed
        # set. Zero prints as 'interfaces held'.
        "interfaces_moved": int,
        # Tests a retained trace shows covering the change whose verdict at head's closure key is a
        # recorded pass. Absent when no trace is retained.
        "replayed_pass": NotRequired[int],
        # Changed definitions no traced test reaches — the length of CheckResult.uncovered. Absent
        # when no trace is retained.
        "uncovered": NotRequired[int],
    },
)


# A definition with no proven inbound edge (grid orphans), with the retained-trace count beside it
# under --traced. A new struct rather than a column on DefRef: an orphan is a claim about edges, and
# DefRef makes none.
Orphan = TypedDict(
    "Orphan",
    {
        "def": "DefRef",
        # How many retained traces hit the definition, as caller or callee. Present under grid
        # orphans --traced; zero is the workspace-level unused-pub question rustc cannot ask, and
        # still a maybe for pub in a published crate.
        "traces": NotRequired[int],
    },
)


# The compile record: what one cargo check was asked for, written to .grid/compile.json by grid
# check --compile so a later check can re-evaluate cfg predicates against it.
CompileRecord = TypedDict(
    "CompileRecord",
    {
        # The head snapshot the compile ran over.
        "snapshot": "SnapshotId",
        # Every feature the compile enabled: those named on the command line plus each workspace
        # package's defaults, closed over the [features] table, unless --no-default-features.
        "features": list[str],
        # --all-features was given, so every feature = "…" predicate held.
        "all_features": bool,
        "no_default_features": bool,
        # --tests or --all-targets was given, so cfg(test) items and tests/ targets were compiled.
        "tests": bool,
        # The --target triple, else the host triple.
        "target": str,
        # The host triple from rustc -vV; empty when rustc could not be asked.
        "host": str,
        # The target's cfg set exactly as rustc --print cfg reports it (unix, target_os="macos", …).
        # Empty when rustc could not be asked, and then every target_* predicate is unknown rather
        # than false.
        "cfgs": list[str],
        # RFC 3339 UTC.
        "recorded_at": str,
    },
)


# What a compile did not compile: for every changed interface, the proven dependents sitting under a
# cfg the recorded cargo check excluded, and the ones nobody can decide.
CompileReport = TypedDict(
    "CompileReport",
    {
        "compiled_with": "CompileRecord",
        # Proven dependents of a changed interface whose cfg the record decidably excluded — the
        # compile never saw them. Each carries reason uncompiled:<cfg>. Gates: exit 3.
        "uncompiled": list["DriftSuspect"],
        # Dependents whose cfg the evaluator could not decide (cfg_attr, a custom key, a target fact
        # the record lacks) or whose snapshot predates the cfg column. Reported, never gated: an
        # unproven exclusion does not hold.
        "unknown": list["DriftWarning"],
    },
)


CheckWhyFulfillingItem = TypedDict(
    "CheckWhyFulfillingItem",
    {
        "promise": "PlanEdgePromise",
        "hops": list["Path"],
    },
)


# Why the verdict says what it says: a path per fulfilled promise, the surplus nobody promised, what
# changed without being run, and what is still open against this code.
CheckWhy = TypedDict(
    "CheckWhy",
    {
        "fulfilling": list["CheckWhyFulfillingItem"],
        "surplus": list["EdgeChange"],
        # Changed but not reached by the supplied trace — the reviewer's red flags.
        "coverage": list["Datum"],
        "open_anchors": list["Anchor"],
    },
)


# Absent when the traces are identical.
TraceDiffFirstDivergence = TypedDict(
    "TraceDiffFirstDivergence",
    {
        "caller": "Datum",
        "green_callee": NotRequired["Datum"],
        "red_callee": NotRequired["Datum"],
    },
)


TraceDiff = TypedDict(
    "TraceDiff",
    {
        # Absent when the traces are identical.
        "first_divergence": NotRequired["TraceDiffFirstDivergence"],
        # Observed in red, not green.
        "added": list["Edge"],
        # Observed in green, not red.
        "removed": list["Edge"],
    },
)


# What one `grid trace export <dir>` or `grid trace import <dir>` did, by trace id — the `--json`
# form of the verb's records. Ids rather than counts, because a refusal has to name what it refused.
# CLI-only: no wire method carries it, since the transport is a file copy between two paths on one
# machine.
TraceTransportResult = TypedDict(
    "TraceTransportResult",
    {
        # Trace ids (Trace.trace_id) whose file the transport wrote.
        "written": list[str],
        # Trace ids the destination already held, byte for byte — why running the transport twice
        # writes nothing the second time.
        "unchanged": list[str],
        # Trace ids the destination holds under different bytes: never overwritten. On import the
        # whole transport is refused when this is non-empty, so these are the ids to reconcile, not
        # the ones that were skipped.
        "refused": list[str],
    },
)


SuspectsRankedItem = TypedDict(
    "SuspectsRankedItem",
    {
        "datum": "Datum",
        # The Ochiai score, present only under Ochiai scoring. Absent in closure mode, where the
        # ranking is the hop distance and the porcelain prints the word `closure` in its place — a
        # number here would be read as confidence, and none was computed.
        "score": NotRequired[float],
        # Why this ranked: the intersection that implicated it.
        "via": str,
        # Hop distance from the test to this definition in the closure walked at head. Present only
        # in closure mode (closure_since), where the ranking IS the hop distance; absent under
        # Ochiai scoring, which ranks by spectra rather than by distance.
        "hops": NotRequired[int],
        # The name the chart asked about gives the datum, so the row can be handed to grid.resolve
        # or grid.explain as printed. Absent when that chart no longer holds the datum.
        "callsign": NotRequired["Callsign"],
        "materialization": NotRequired["DefRefMaterialization"],
    },
)


Suspects = TypedDict(
    "Suspects",
    {
        "ranked": list["SuspectsRankedItem"],
    },
)


# A declared, symmetric coupling between two callsign patterns that no language's binding rules
# establish — a serde boundary, a JSON contract, a Python dict a Rust struct deserializes. A human
# knows the pair; this is where it is written down. Declared coupling is the one modeled evidence
# that gates, because a human declared it.
Covenant = TypedDict(
    "Covenant",
    {
        "a": "CallsignPattern",
        "b": "CallsignPattern",
        # The boundary the coupling crosses: 'schema', 'wire', 'protocol' — free text, and the word
        # that appears in a drift record's reason.
        "via": str,
        "note": NotRequired[str],
    },
)


# A test that covered a definition, kept passing while that definition's body changed, and was not
# itself touched — so it proved nothing about the change. Heuristic: a ranking, never a gate.
WeakTest = TypedDict(
    "WeakTest",
    {
        "test": str,
        # The definition whose behaviour moved while this test kept passing, as it was at base.
        "def": "DefRef",
    },
)


# closure-changed: the recorded verdict keys to a closure this snapshot no longer has. closure-
# rejected: the closure could not be established at all, which is a miss and therefore never a hit.
StaleTestReason: TypeAlias = Literal["closure-changed", "closure-rejected"]


# A test whose recorded verdict no longer applies. Staleness is identity math over the closure, not
# a guess, so it gates.
StaleTest = TypedDict(
    "StaleTest",
    {
        "test": str,
        # closure-changed: the recorded verdict keys to a closure this snapshot no longer has.
        # closure-rejected: the closure could not be established at all, which is a miss and
        # therefore never a hit.
        "reason": "StaleTestReason",
    },
)


# One signed change to the suspect set. Watch mode emits these rather than a current list, so a
# consumer never has to diff lists to see what changed, and the algebra of the stream is checkable
# by summation.
DriftDelta = TypedDict(
    "DriftDelta",
    {
        "suspect": "DriftSuspect",
        # +1 the suspect appeared, -1 it was retracted. A retraction is emitted when an edit
        # resolves a suspect — the dependent was updated, a covenant was satisfied, the interface
        # was reverted. Consumers hold a running sum, and the sum after any prefix of events equals
        # a fresh grid drift.
        "weight": int,
    },
)


DriftWarning = TypedDict(
    "DriftWarning",
    {
        # The callsign or pattern the warning is about.
        "subject": str,
        "message": str,
    },
)


DriftSuspect = TypedDict(
    "DriftSuspect",
    {
        # The definition that depends on the changed one and did not itself change.
        "dependent": "DefRef",
        # The definition whose interface moved, as it was at base.
        "changed": "DefRef",
        "relation": "Relation",
        # Evidence grade of the edge that made this a suspect.
        "grade": "Grade",
        # The changed definition's interface identity at base, when the snapshot carries one.
        "sig_before": NotRequired["Datum"],
        # Its interface identity at head, when the snapshot carries one. Absent on both sides means
        # the sig was unknown and the change was treated as visible.
        "sig_after": NotRequired["Datum"],
        # Why this edge implicated the dependent beyond the edge itself: 'stale-trace' for a decayed
        # observation, 'covenant:<via>' for a declared coupling.
        "reason": NotRequired[str],
    },
)


# A changed type deriving Deserialize whose new shape cannot read what the old one wrote. Within one
# repository the compiler moves both ends of a serde boundary together; this is the case it cannot
# see.
CompatFinding = TypedDict(
    "CompatFinding",
    {
        # The type whose deserializing shape moved, as head holds it.
        "def": "DefRef",
        # Which rule old bytes trip on: 'required field added', 'field renamed without alias', 'type
        # changed', 'variant removed', 'field removed under deny_unknown_fields', or 'representation
        # changed'.
        "rule": str,
        # True when a standing `persisted` invariant matches this type — a human said old bytes
        # exist, which is what makes the finding modeled evidence and a gate. False is heuristic:
        # reported, ranked, and never moves the exit code.
        "declared": bool,
    },
)


# What drift could not check. A drift report claiming no suspects where it could not see callers is
# dishonest silence, so the counts ride with every answer.
DriftUnchecked = TypedDict(
    "DriftUnchecked",
    {
        # References the head survey declined to bind, so no edge exists for them and no dependent
        # could be found through them. Absent means head was a published snapshot nobody re-surveyed
        # in this run, so the count is unknown — never reported as zero.
        "unresolved_refs": NotRequired[int],
        # Of unresolved_refs: names that were never this tree's to hold — std/core/alloc or another
        # crate in Rust, a top-level module the tree does not root in Python. They cost a claim
        # about this repo nothing. Absent whenever unresolved_refs is.
        "unresolved_external": NotRequired[int],
        # Of unresolved_refs: calls through a receiver whose type would have to be known. Never
        # resolved and never guessed, because types are modeled and a survey emits none. Absent
        # whenever unresolved_refs is.
        "unresolved_method": NotRequired[int],
        # Of unresolved_refs: names this tree could have held and the resolver could not bind —
        # glob-supplied, ambiguous, an unresolved re-export. The population that is an actual hole
        # in what drift claims. Absent whenever unresolved_refs is.
        "unresolved_internal": NotRequired[int],
        # Of unresolved_refs: macro invocations inside definitions, one each — argument lists the
        # survey never read, so a caller inside one is a dependent drift cannot see. Absent whenever
        # unresolved_refs is, and from a report computed before macros were counted.
        "unresolved_macros": NotRequired[int],
        # Definitions whose inbound walk ran past its budget. The walk is rejected whole, never
        # truncated: a truncated walk is indistinguishable from a smaller one.
        "budget_rejected": int,
        # Recorded verdicts no retained trace claims, so no test owns them and no test-staleness
        # check covered them. Present when the report was asked for tests.
        "unattributed_entries": NotRequired[int],
        # Weak-test checks declined because their coverage came from a decayed trace. A heuristic
        # built on a demoted heuristic is noise, so grid declines it and says how often.
        "weak_skipped": NotRequired[int],
        # Changed types whose Deserialize derive the source does not spell out — behind a cfg_attr,
        # produced by a macro, or carrying a serde attribute this reader cannot parse — plus any
        # whose source at either snapshot could not be read. Skipped and counted, never declared
        # compatible. Present when the report was asked for compat.
        "compat_unchecked": NotRequired[int],
    },
)


DriftReport = TypedDict(
    "DriftReport",
    {
        "base": "SnapshotId",
        "head": "SnapshotId",
        "suspects": list["DriftSuspect"],
        "unchecked": "DriftUnchecked",
        # Reported rather than refused: a declared covenant whose pattern matches nothing at drift
        # time, and anything else drift wants a human to see without holding on it.
        "warnings": NotRequired[list["DriftWarning"]],
        # Present when the report was asked for tests: every known test whose recorded verdict no
        # longer applies.
        "stale_tests": NotRequired[list["StaleTest"]],
        # Present when the report was asked for tests: tests that covered a changed definition,
        # stayed green, and were never touched. Ranked, never gated.
        "weak": NotRequired[list["WeakTest"]],
        # Present when the report was asked for compat: changed types deriving Deserialize whose new
        # shape cannot read old bytes. Step 9's `grid check` embeds the same findings; drift carries
        # them so the check is exercisable now.
        "compat": NotRequired[list["CompatFinding"]],
    },
)


# Two worktrees of one repository, compared against their common base.
InterfereReport = TypedDict(
    "InterfereReport",
    {
        # The revision both worktrees are measured from: the merge base of their heads, or the one
        # the caller named.
        "base": "GitRev",
        # The live snapshot of the store this call ran in.
        "mine": "SnapshotId",
        # The live snapshot of the other root's store, as its last survey left it. Grid never
        # surveys the other root, so this is as fresh as that survey and no fresher.
        "theirs": "SnapshotId",
        # Definitions whose datum moved on both sides since base, named as base held them. A textual
        # conflict git will also find, listed for completeness; it does not gate.
        "both_touched": list["DefRef"],
        # One side's drift suspects that land in the other side's edits: reason 'interfere:<other-
        # root>'. The semantic overlap a merge cannot see, carrying the grade of the edge that found
        # it.
        "suspects": list["DriftSuspect"],
        # What neither side could check, summed over the two drift walks.
        "unchecked": "DriftUnchecked",
    },
)


BisectResult = TypedDict(
    "BisectResult",
    {
        # Absent when the range is clean or the culprit is outside surveyed code.
        "culprit": NotRequired["Datum"],
        "steps": int,
        "cache_hits": int,
    },
)


# A test as the runner names it, never as grid guesses it: nextest's `<binary-id> <name>`
# (`pkg::bin/x mod::tests::t`), pytest's node id (`tests/test_x.py::TestY::test_z`). Grid maps it to
# a callsign by the frontend's spelling and resolves it by the grid.resolve rule; a test the chart
# cannot resolve is always run.
TestId = NewType("TestId", str)


# reached: the inbound walk from the changed set reached the test's definition (or the test itself
# changed). no_trace: no retained trace still describes code base holds, so the static closure alone
# may not excuse a skip. unresolved: the chart cannot name the test, so nothing can be said about
# it. aground: the walk ran out of fuel, so every test is selected. not_recorded: unreached and
# traced, but no verdict for this closure in this environment is on record to replay. unwatched: a
# definition in the changed set is covered by no retained trace at all, so nobody watched which
# tests reach it — a trace that does not list it is then no evidence of absence, and no test replays
# across it.
TestSelectionWhy: TypeAlias = Literal["reached", "no_trace", "unresolved", "aground", "not_recorded", "unwatched"]


# Why the chart cannot name the test; present only when why is unresolved. ambiguous: the name
# resolves to more than one definition, and two definitions are not one closure. module_charted: the
# module the name sits in is charted and the name itself is not — the survey saw the module and
# could not see this definition in it. Named for the observation, which is the same in every
# language; what produced it is not: on Rust a macro invocation (proptest! and its kin) defines the
# test and a survey charts items, never macro output; on Python a decorator or a generated attribute
# (parameterized.expand, globals()[name] = fn) or a definition the frontend declined. not_charted:
# nothing under that module is in the chart at all.
TestSelectionUnresolved: TypeAlias = Literal["ambiguous", "module_charted", "not_charted"]


# One test the runner must run, and the reason. Every reason is a refusal to skip.
TestSelection = TypedDict(
    "TestSelection",
    {
        "test": "TestId",
        # The test's definition at head. Absent when the test is unresolved.
        "datum": NotRequired["Datum"],
        # reached: the inbound walk from the changed set reached the test's definition (or the test
        # itself changed). no_trace: no retained trace still describes code base holds, so the
        # static closure alone may not excuse a skip. unresolved: the chart cannot name the test, so
        # nothing can be said about it. aground: the walk ran out of fuel, so every test is
        # selected. not_recorded: unreached and traced, but no verdict for this closure in this
        # environment is on record to replay. unwatched: a definition in the changed set is covered
        # by no retained trace at all, so nobody watched which tests reach it — a trace that does
        # not list it is then no evidence of absence, and no test replays across it.
        "why": "TestSelectionWhy",
        # Why the chart cannot name the test; present only when why is unresolved. ambiguous: the
        # name resolves to more than one definition, and two definitions are not one closure.
        # module_charted: the module the name sits in is charted and the name itself is not — the
        # survey saw the module and could not see this definition in it. Named for the observation,
        # which is the same in every language; what produced it is not: on Rust a macro invocation
        # (proptest! and its kin) defines the test and a survey charts items, never macro output; on
        # Python a decorator or a generated attribute (parameterized.expand, globals()[name] = fn)
        # or a definition the frontend declined. not_charted: nothing under that module is in the
        # chart at all.
        "unresolved": NotRequired["TestSelectionUnresolved"],
    },
)


# Selection as data: one inbound walk from the datum-level changed set over every grade, fuel
# grid_cache::FUEL. A walk that runs aground selects every test rather than a shorter list; a test
# with no live trace is selected regardless of the walk.
TestPlan = TypedDict(
    "TestPlan",
    {
        "base": "SnapshotId",
        # The worktree survey the plan was computed against.
        "head": "SnapshotId",
        # The run set: selected ∪ replayed = the inventory, disjoint.
        "selected": list["TestSelection"],
        # Unselected tests whose recorded verdict, keyed by the closure at head and the given
        # environment, is on record. Each hit names its test.
        "replayed": list["CacheHit"],
        # Changed definitions the walk connected to no test. Heuristic: the chart is known
        # incomplete, so this ranks and never gates.
        "uncovered": list["DefRef"],
        "unchecked": "DriftUnchecked",
    },
)


TestOutcomeVerdict: TypeAlias = Literal["pass", "fail"]


TestOutcome = TypedDict(
    "TestOutcome",
    {
        "test": "TestId",
        "verdict": "TestOutcomeVerdict",
        # Present when the run was traced and the trace was ingested.
        "trace_id": NotRequired[str],
    },
)


# What `grid test --json` emits. No method answers it: the run is the CLI's, the way a survey's git
# spawn is; the plan inside it is grid.test.plan's answer verbatim.
TestRun = TypedDict(
    "TestRun",
    {
        # blake3 over toolchain version, lockfile bytes, runner args, target triple, and whether the
        # run was traced — stated so a verdict can be re-keyed later.
        "env_hash": str,
        "plan": "TestPlan",
        # What the runner said about each selected test it reported on.
        "ran": list["TestOutcome"],
        # nondeterministic and uncovered findings. Reported, never gated.
        "warnings": list["DriftWarning"],
    },
)
