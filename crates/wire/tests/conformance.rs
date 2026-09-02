//! D7 conformance: every example in wire-v1.openrpc.json round-trips through
//! the generated types byte-canonically; a mutated example fails.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{json, Map, Value};

fn contract() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/wire-v1.openrpc.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("read contract"))
        .expect("parse contract")
}

fn params_to_object(params: &[Value]) -> Value {
    params
        .iter()
        .map(|p| (p["name"].as_str().unwrap().to_string(), p["value"].clone()))
        .collect::<Map<String, Value>>()
        .into()
}

macro_rules! dispatch {
    ($method:expr, $params:ident, $result:ident, $($name:literal => $P:ty, $R:ty),* $(,)?) => {
        match $method {
            $(
                $name => {
                    let pv = params_to_object($params);
                    let typed: $P = serde_json::from_value(pv.clone())
                        .unwrap_or_else(|e| panic!("{} params must deserialize: {e}", $name));
                    let back = serde_json::to_value(&typed).unwrap();
                    assert_eq!(pv, back, "{} params must round-trip", $name);
                    let rv = $result["value"].clone();
                    let typed_r: $R = serde_json::from_value(rv.clone())
                        .unwrap_or_else(|e| panic!("{} result must deserialize: {e}", $name));
                    let back_r = serde_json::to_value(&typed_r).unwrap();
                    assert_eq!(rv, back_r, "{} result must round-trip", $name);
                }
            )*
            other => panic!("no typed dispatch for method {other}"),
        }
    };
}

fn roundtrip(method: &str, params: &[Value], result: &Value) {
    use grid_wire::methods::*;
    use grid_wire::*;
    dispatch!(method, params, result,
        "grid.version" => VersionParams, VersionResult,
        "grid.snapshot.current" => SnapshotCurrentParams, Snapshot,
        "grid.survey.run" => SurveyRunParams, Snapshot,
        "grid.resolve" => ResolveParams, Vec<DefRef>,
        "grid.delta" => DeltaParams, Diff,
        "grid.drift" => DriftParams, DriftReport,
        "grid.gate.check" => GateCheckParams, GateVerdict,
        "grid.query.walk" => QueryWalkParams, WalkResult,
        "grid.anchor.carry" => AnchorCarryParams, Vec<CarriedAnchor>,
        "grid.trace.ingest" => TraceIngestParams, TraceIngestResult,
        "grid.cache.check" => CacheCheckParams, CacheCheckResult,
        "grid.cache.record" => CacheRecordParams, CacheRecordResult,
        "grid.scope" => ScopeParams, ScopePack,
        "grid.plan.lint" => PlanLintParams, LintResult,
        "grid.plan.estimate" => PlanEstimateParams, Estimate,
        "grid.edit.apply" => EditApplyParams, EditResult,
        "grid.todo" => TodoParams, TodoList,
        "grid.check" => CheckParams, CheckResult,
        "grid.trace.diff" => TraceDiffParams, TraceDiff,
        "grid.suspects" => SuspectsParams, Suspects,
        "grid.bisect" => BisectParams, BisectResult,
        "grid.test.plan" => TestPlanParams, TestPlan,
        "grid.interfere" => InterfereParams, InterfereReport,
    );
}

#[test]
fn every_contract_example_roundtrips() {
    let c = contract();
    let methods = c["methods"].as_array().unwrap();
    assert_eq!(methods.len(), 23, "contract must have 23 methods");
    let mut example_count = 0;
    for m in methods {
        let name = m["name"].as_str().unwrap();
        let examples = m["examples"]
            .as_array()
            .unwrap_or_else(|| panic!("{name} must carry examples (D7)"));
        for ex in examples {
            let params: Vec<Value> = ex["params"].as_array().unwrap().clone();
            roundtrip(name, &params, &ex["result"]);
            example_count += 1;
        }
    }
    assert_eq!(
        example_count, 24,
        "one example per method, per the contract corpus"
    );
}

#[test]
fn mutated_example_fails() {
    let c = contract();
    let m = c["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "grid.cache.check")
        .unwrap();
    let mut v = m["examples"][0]["params"][0]["value"].clone();
    v.as_object_mut()
        .unwrap()
        .insert("bogus_key".into(), json!(1));
    assert!(
        serde_json::from_value::<grid_wire::CacheCheckRequest>(v).is_err(),
        "unknown key must be rejected (deny_unknown_fields)"
    );

    let mut v2 = m["examples"][0]["params"][0]["value"].clone();
    v2.as_object_mut().unwrap().remove("env_hash");
    assert!(
        serde_json::from_value::<grid_wire::CacheCheckRequest>(v2).is_err(),
        "missing required key must be rejected"
    );
}
