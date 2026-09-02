//! `grid check --compile` — the callers a `cargo check` did not compile.
//!
//! `cargo check` compiles neither `#[cfg(test)]` code nor code behind a
//! feature it was not given, and it says nothing about either: a green
//! check over a moved signature is silent about the test module that calls
//! the old one. The chart is text, so it holds those callers, and every
//! definition carries the `cfg` predicate that conditions it (segment epoch
//! 4). The gap between "green check" and "red test" is then decidable from
//! the chart plus the flags the compile ran with.
//!
//! Two halves. [`wrap`] runs the compile and writes the **compile record**
//! (`.grid/compile.json`): what it was asked for — features after defaults,
//! `--all-features`, `--tests`, the target, and the target's cfg set as
//! rustc reports it. [`report`] evaluates the record against the changed set
//! `C` of `grid drift` (sig moved or unknown): every proven dependent whose
//! cfg the record decidably excludes is `uncompiled:<cfg>` and gates;
//! everything the evaluator cannot decide is a warning, because an unproven
//! exclusion does not hold.
//!
//! The record is the CLI's to write — grid spawns `cargo` the way it spawns
//! `git`, and a daemon does not compile things on request. `grid.check` over
//! the socket only reads it.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::{Command, Stdio};

use grid_chart::{Ordinal, Segment};
use grid_lang_rs::{eval_cfg, CfgEnv};
use grid_wire::{
    CompileRecord, CompileReport, DriftSuspect, DriftWarning, Grade, SnapshotId, SnapshotOrRev,
};

use crate::chart_provider::ChartDelta;
use crate::drift::{self, Changed};

/// Where the compile record lives: engine state, beside the chart.
pub const FILE: &str = ".grid/compile.json";

/// How the wrapped compile ended.
#[derive(Debug)]
pub enum Wrapped {
    /// `cargo check` succeeded; what it was asked for.
    Compiled(CompileRecord),
    /// `cargo check` failed with this exit code, passed through untouched —
    /// grid records nothing and says nothing on stdout, because the
    /// compiler's own diagnostics are the answer.
    Failed(u8),
}

/// Run `cargo check <args>` from the worktree root, its output on stderr,
/// and on success write the record.
///
/// # Errors
/// The spawn failure, or the record write failure, as text.
pub fn wrap(root: &Path, args: &[String], snapshot: SnapshotId) -> Result<Wrapped, String> {
    let out = Command::new("cargo")
        .arg("check")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cargo: {e}"))?;
    // Anything cargo put on stdout (`--message-format json`, say) is the
    // compiler's, not a grid record: stdout stays typed records only.
    eprint!("{}", String::from_utf8_lossy(&out.stdout));
    if !out.status.success() {
        return Ok(Wrapped::Failed(
            out.status
                .code()
                .and_then(|c| u8::try_from(c).ok())
                .unwrap_or(crate::exit::INTERNAL),
        ));
    }
    let record = record(root, args, snapshot);
    let text = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    let path = root.join(FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Wrapped::Compiled(record))
}

/// The record on disk, if a compile was ever wrapped here.
///
/// # Errors
/// A record that exists but does not parse: a broken record is not an
/// absent one, and reading it as absent would silently gate nothing.
pub fn load(root: &Path) -> Result<Option<CompileRecord>, String> {
    match std::fs::read(root.join(FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{FILE}: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{FILE}: {e}")),
    }
}

/// What the compile was asked for, read off its own arguments plus what
/// cargo and rustc say about the tree and the target.
fn record(root: &Path, args: &[String], snapshot: SnapshotId) -> CompileRecord {
    let mut features = Vec::new();
    let (mut all_features, mut no_default_features, mut tests) = (false, false, false);
    let mut target = None;
    let mut rest = args.iter().map(String::as_str);
    while let Some(arg) = rest.next() {
        let value = |rest: &mut dyn Iterator<Item = &str>, arg: &str, flag: &str| {
            arg.strip_prefix(flag)
                .and_then(|v| v.strip_prefix('='))
                .map(str::to_owned)
                .or_else(|| (arg == flag).then(|| rest.next().unwrap_or_default().to_owned()))
        };
        if arg == "--all-features" {
            all_features = true;
        } else if arg == "--no-default-features" {
            no_default_features = true;
        } else if arg == "--tests" || arg == "--all-targets" {
            tests = true;
        } else if let Some(list) = value(&mut rest, arg, "--features").or_else(|| {
            (arg == "-F")
                .then(|| rest.next().unwrap_or_default().to_owned())
                .or_else(|| arg.strip_prefix("-F").map(str::to_owned))
        }) {
            // `pkg/feat` names a package's feature; the record is flat.
            features.extend(
                list.split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|f| !f.is_empty())
                    .map(|f| f.rsplit('/').next().unwrap_or(f).to_owned()),
            );
        } else if let Some(t) = value(&mut rest, arg, "--target") {
            target = Some(t);
        }
    }
    if !no_default_features {
        features.push("default".to_owned());
    }
    let features = close_features(root, features);
    let host = rustc(root, &["-vV"])
        .lines()
        .find_map(|l| l.strip_prefix("host: "))
        .unwrap_or_default()
        .to_owned();
    let target = target.unwrap_or_else(|| host.clone());
    let cfgs: Vec<String> = if target.is_empty() {
        Vec::new()
    } else {
        rustc(root, &["--print", "cfg", "--target", &target])
            .lines()
            .map(str::to_owned)
            .collect()
    };
    CompileRecord {
        snapshot,
        features,
        all_features,
        no_default_features,
        tests,
        target,
        host,
        cfgs,
        recorded_at: grid_cache::now_rfc3339(),
    }
}

fn rustc(root: &Path, args: &[&str]) -> String {
    Command::new("rustc")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// The features a set of names enables, closed over every workspace
/// package's `[features]` table — `default = ["a"]`, `a = ["b"]` enables
/// `b`. Sorted and deduplicated so the record is a function of the request.
///
/// ponytail: the tables are unioned across packages, so a feature name two
/// packages both define is one name here; per-package records are the
/// upgrade if that ever misreports a compile.
fn close_features(root: &Path, seeds: Vec<String>) -> Vec<String> {
    let table: HashMap<String, Vec<String>> = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| {
            let mut out: HashMap<String, Vec<String>> = HashMap::new();
            for package in v.get("packages")?.as_array()? {
                for (name, deps) in package.get("features")?.as_object()? {
                    out.entry(name.clone()).or_default().extend(
                        deps.as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|d| d.as_str())
                            // `dep:x` and `x/y` name dependencies, not
                            // this package's features.
                            .filter(|d| !d.starts_with("dep:") && !d.contains('/'))
                            .map(str::to_owned),
                    );
                }
            }
            Some(out)
        })
        .unwrap_or_default();
    let mut enabled: Vec<String> = Vec::new();
    let mut worklist = seeds;
    while let Some(f) = worklist.pop() {
        if enabled.contains(&f) {
            continue;
        }
        worklist.extend(table.get(&f).into_iter().flatten().cloned());
        enabled.push(f);
    }
    enabled.sort();
    enabled.dedup();
    enabled
}

/// What one dependent's cfg says under the record.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Compiled,
    Uncompiled(String),
    Unknown(String),
}

/// `None` is a snapshot that predates the cfg column, and it stays unknown:
/// an older image answers nothing, and reading nothing as "compiled" would
/// call every `#[cfg(test)]` caller in it compiled.
fn classify(cfg: Option<&str>, env: &CfgEnv) -> Verdict {
    match cfg {
        None => Verdict::Unknown("cfg unknown: pre-epoch-4 snapshot".to_owned()),
        Some("") => Verdict::Compiled,
        Some(cfg) => match eval_cfg(cfg, env) {
            Some(true) => Verdict::Compiled,
            Some(false) => Verdict::Uncompiled(cfg.to_owned()),
            None => Verdict::Unknown(format!("cfg unknown: {cfg}")),
        },
    }
}

/// Evaluate the record against the changed set between `base` and `head`.
///
/// Dependents come from base's inbound proven edges — the old graph is the
/// one that knew them — and each one's cfg is read at head, the tree the
/// compile ran over. A dependent head no longer holds was not left
/// uncompiled; it was removed.
///
/// # Errors
/// [`drift::Error::Chart`] when an endpoint does not resolve;
/// [`drift::Error::BudgetRejected`] when an inbound walk ran past its fuel —
/// refused whole, never served short.
pub fn report(
    root: &Path,
    base: &SnapshotOrRev,
    head: &SnapshotOrRev,
    record: &CompileRecord,
) -> Result<CompileReport, drift::Error> {
    let chart = ChartDelta::open(root)?;
    let (base_epoch, head_epoch) = (chart.epoch_of(base)?, chart.epoch_of(head)?);
    let diff = grid_chart::diff(
        SnapshotId(base_epoch.snapshot_id().as_str().to_owned()),
        base_epoch.segments(),
        SnapshotId(head_epoch.snapshot_id().as_str().to_owned()),
        head_epoch.segments(),
    );
    let changed = drift::changed_set_of(&diff, base_epoch.segments(), head_epoch.segments());
    let env = CfgEnv {
        features: &record.features,
        all_features: record.all_features,
        tests: record.tests,
        cfgs: &record.cfgs,
    };
    let at_head = if changed.is_empty() {
        HashMap::new()
    } else {
        by_callsign(head_epoch.segments())
    };
    let none = HashMap::new();
    let mut uncompiled: BTreeMap<(String, String, u8), DriftSuspect> = BTreeMap::new();
    let mut unknown: BTreeMap<String, DriftWarning> = BTreeMap::new();
    let mut rejected = 0u32;
    for change in &changed {
        let Some(found) = drift::inbound_of(
            base_epoch.segments(),
            change,
            Grade::Proven,
            &none,
            drift::FUEL,
        ) else {
            rejected = rejected.saturating_add(1);
            continue;
        };
        for dep in found {
            let Some(callsign) = dep.callsign.as_deref() else {
                continue;
            };
            let Some(&(seg, ord)) = at_head.get(callsign) else {
                continue;
            };
            let Some(segment) = head_epoch.segments().get(seg) else {
                continue;
            };
            match classify(segment.cfg(ord), &env) {
                Verdict::Compiled => {}
                Verdict::Uncompiled(cfg) => {
                    let key = (
                        callsign.to_owned(),
                        change.callsign.clone(),
                        grid_chart::relation_tag(dep.relation),
                    );
                    uncompiled.entry(key).or_insert_with(|| {
                        suspect(&chart, segment, ord, callsign, &cfg, change, &dep)
                    });
                }
                Verdict::Unknown(message) => {
                    unknown
                        .entry(callsign.to_owned())
                        .or_insert_with(|| DriftWarning {
                            subject: callsign.to_owned(),
                            message,
                        });
                }
            }
        }
    }
    if rejected > 0 {
        return Err(drift::Error::BudgetRejected(rejected));
    }
    Ok(CompileReport {
        compiled_with: record.clone(),
        uncompiled: uncompiled.into_values().collect(),
        unknown: unknown.into_values().collect(),
    })
}

fn suspect<B: AsRef<[u8]>>(
    chart: &ChartDelta,
    segment: &Segment<B>,
    ord: Ordinal,
    callsign: &str,
    cfg: &str,
    change: &Changed,
    dep: &drift::Inbound,
) -> DriftSuspect {
    let datum = segment.datum(ord).unwrap_or(dep.datum);
    let mut dependent = drift::def_ref(chart, datum, Some(callsign));
    dependent.cfg = Some(cfg.to_owned());
    DriftSuspect {
        dependent,
        changed: drift::def_ref(chart, change.datum, Some(&change.callsign)),
        relation: dep.relation,
        grade: dep.grade,
        sig_before: change.sig_before.clone(),
        sig_after: change.sig_after.clone(),
        reason: Some(format!("uncompiled:{cfg}")),
    }
}

/// Every callsign a live set names, to its segment index and ordinal —
/// one pass, so each dependent is a hash lookup rather than a scan.
fn by_callsign<B: AsRef<[u8]>>(segments: &[Segment<B>]) -> HashMap<String, (usize, Ordinal)> {
    let mut out = HashMap::new();
    for (i, segment) in segments.iter().enumerate() {
        for ord in 0..segment.def_count() {
            let ord = Ordinal(ord);
            if let Some(callsign) = segment.callsign(ord) {
                out.insert(callsign.to_owned(), (i, ord));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{classify, report, Verdict};
    use grid_chart::{build, DatumId, Def, DefFacts, Edge, Ordinal, Segment};
    use grid_lang_rs::CfgEnv;
    use grid_wire::{CompileRecord, GitRev, SnapshotId, SnapshotOrRev};

    fn record() -> CompileRecord {
        CompileRecord {
            snapshot: SnapshotId("snap:test".into()),
            features: vec!["default".into()],
            all_features: false,
            no_default_features: false,
            tests: false,
            target: "aarch64-apple-darwin".into(),
            host: "aarch64-apple-darwin".into(),
            cfgs: vec!["unix".into()],
            recorded_at: "1970-01-01T00:00:00Z".into(),
        }
    }

    /// The epoch rule, at the classifier: an image with no cfg column is
    /// unknown, never compiled.
    #[test]
    fn a_pre_epoch_4_answer_is_unknown_not_compiled() {
        let env = CfgEnv {
            features: &[],
            all_features: true,
            tests: true,
            cfgs: &[],
        };
        assert_eq!(
            classify(None, &env),
            Verdict::Unknown("cfg unknown: pre-epoch-4 snapshot".into())
        );
        assert_eq!(classify(Some(""), &env), Verdict::Compiled);
        assert_eq!(classify(Some("test"), &env), Verdict::Compiled);
        assert_eq!(
            classify(Some("kani"), &env),
            Verdict::Unknown("cfg unknown: kani".into())
        );
    }

    /// A segment's defs and edges as the epoch-4 builder takes them, with
    /// no cfg: the way to re-publish an older image's graph under the
    /// current epoch.
    fn rebuilt<B: AsRef<[u8]>>(fixture: &Segment<B>) -> (Vec<Def>, Vec<Edge>) {
        let defs: Vec<Def> = (0..fixture.def_count())
            .map(Ordinal)
            .map(|o| {
                let loc = fixture.location(o);
                Def {
                    datum: fixture.datum(o).expect("datum"),
                    callsign: fixture.callsign(o).expect("callsign").to_owned(),
                    lang: fixture.lang(o),
                    facts: Some(DefFacts {
                        path: loc.as_ref().map_or("", |l| l.path).to_owned(),
                        line: loc.as_ref().map_or(0, |l| l.line),
                        line_end: loc.as_ref().map_or(0, |l| l.line_end),
                        span: loc.as_ref().map_or((0, 0), |l| (l.span.start, l.span.end)),
                        shape: fixture.shape(o).unwrap_or_default(),
                        complexity: fixture.complexity(o).unwrap_or(1),
                        sig: fixture.sig(o).unwrap_or_default(),
                        cfg: String::new(),
                    }),
                }
            })
            .collect();
        let mut edges: Vec<Edge> = Vec::new();
        for o in (0..fixture.def_count()).map(Ordinal) {
            for e in fixture.out_edges(o) {
                edges.push(Edge {
                    src: fixture.datum(o).expect("src"),
                    dst: fixture.datum(e.other).expect("dst"),
                    relation: e.relation,
                    grade: e.grade,
                });
            }
        }
        (defs, edges)
    }

    /// The epoch-3 fixture is real: the epoch-3 builder's own bytes over
    /// this repository. Rebuilt through the epoch-4 builder (cfg-less) as
    /// base with one callee's identity moved, and served as head as it is,
    /// every proven caller of that callee is a warning — the fixture cannot
    /// say what conditioned them, and nothing is declared compiled.
    #[test]
    fn a_check_against_an_epoch3_head_warns_for_every_caller() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../chart/tests/fixtures/epoch3.seg"
        ))
        .expect("fixture");
        let fixture = Segment::parse(bytes.as_slice()).expect("epoch 3 parses");
        assert_eq!(
            fixture.cfg(Ordinal(0)),
            None,
            "epoch 3 carries no cfg column"
        );
        // The callee: the first definition with a proven inbound `calls`.
        let callee = (0..fixture.def_count())
            .map(Ordinal)
            .find(|&o| {
                fixture
                    .in_edges(o)
                    .any(|e| e.relation == grid_wire::Relation::Calls)
            })
            .expect("a called definition");
        let callers: Vec<String> = fixture
            .in_edges(callee)
            .filter(|e| e.relation == grid_wire::Relation::Calls)
            .filter_map(|e| fixture.callsign(e.other).map(str::to_owned))
            .collect();
        assert!(!callers.is_empty());
        let (mut defs, mut edges) = rebuilt(&fixture);
        // Base: the callee under an older identity and interface, with the
        // same callers pointing at it.
        let real = fixture.datum(callee).expect("callee datum");
        let old = DatumId([0xAB; 32]);
        for d in &mut defs {
            if d.datum == real {
                d.datum = old;
                if let Some(f) = d.facts.as_mut() {
                    f.sig = [0xCD; 32];
                }
            }
        }
        for e in &mut edges {
            if e.src == real {
                e.src = old;
            }
            if e.dst == real {
                e.dst = old;
            }
        }
        let base = build(&defs, &edges).expect("base image");

        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = grid_chart_io::Store::open(crate::chart_provider::store_dir(dir.path()))
            .expect("store");
        store.publish("base", None, &[base]).expect("publish base");
        store.publish("head", None, &[bytes]).expect("publish head");
        let rev = |r: &str| SnapshotOrRev::GitRev {
            git_rev: GitRev(r.to_owned()),
        };
        let rec = CompileRecord {
            all_features: true,
            tests: true,
            ..record()
        };
        let out = report(dir.path(), &rev("base"), &rev("head"), &rec).expect("report");
        assert!(
            out.uncompiled.is_empty(),
            "nothing is declared uncompiled: {:?}",
            out.uncompiled
        );
        let warned: Vec<&str> = out.unknown.iter().map(|w| w.subject.as_str()).collect();
        for caller in &callers {
            assert!(
                warned.contains(&caller.as_str()),
                "{caller} warned: {warned:?}"
            );
        }
        assert!(out
            .unknown
            .iter()
            .all(|w| w.message == "cfg unknown: pre-epoch-4 snapshot"));
    }
}
