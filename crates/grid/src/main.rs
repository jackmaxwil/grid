//! grid — CLI + daemon (single binary: survey, query, gate, serve).
//!
//! Stage 2: the verbs above, plus `grid survey`, `grid diff` and `grid
//! resolve`, over a daemon serving the real chart — `grid.snapshot.current`,
//! `grid.survey.run`, `grid.delta`, `grid.resolve`, `grid.plan.lint` and
//! `grid.gate.check`. Bare `grid` is the everyday verb: what changed, and does
//! it match the promise.
//!
//! Wave 6 adds the stage-1/2 agent verbs: `grid scope`, `grid todo` (a view
//! over the gate verdict, not a second delta engine), `grid edit apply` (the
//! CAS write path, D6's second writer), `grid uses` (proven inbound edges)
//! and `grid explain` (help generated from the wire contract and
//! `docs/ONTOLOGY.md` at build time).
//!
//! Wave 9 adds `grid check`: the gate verdict plus, under `--why`, the paths
//! that made each promise good, the surplus it never promised, and what
//! changed without being run. It holds — exit 3 — exactly when the gate holds.
//!
//! Two output modes, one vocabulary: porcelain records (`docs/PORCELAIN.md`)
//! by default, the wire result verbatim under `--json`. stdout carries
//! records; stderr carries hints and the first-run banner (`GRID_HINTS=0`
//! disables). Exit codes are uniform across verbs: 0 pass, 1 internal error,
//! 2 bad input, 3 divergence, 4 stale coordinate.

mod bisect;
mod chart_provider;
mod check;
mod compat;
mod compile;
mod covenant;
mod coverage;
mod drift;
mod edit;
mod explain;
mod history;
mod interfere;
mod plan;
mod porcelain;
mod rpc;
mod runner;
mod serve;
mod stale;
mod standing;
mod suspects;
mod testplan;
mod testrun;
mod watch;
mod weak;

use std::io::{BufRead as _, Write as _};
use std::process::ExitCode;

/// Exit codes, uniform across verbs (`docs/PORCELAIN.md`).
mod exit {
    pub const OK: u8 = 0;
    pub const INTERNAL: u8 = 1;
    pub const BAD_INPUT: u8 = 2;
    pub const DIVERGENCE: u8 = 3;
    pub const STALE: u8 = 4;
}

const PITCH: &str = "what your code promises vs what it does";
const WIRE_VERSION: &str = "1.1.0";
const ENGINE_VERSION: &str = concat!("grid ", env!("CARGO_PKG_VERSION"));

/// Teaching output. Machines read stdout only, so every hint goes to stderr,
/// and `GRID_HINTS=0` silences the channel entirely.
fn hint(msg: &str) {
    if std::env::var_os("GRID_HINTS").is_some_and(|v| v == "0") {
        return;
    }
    let _ = writeln!(std::io::stderr(), "{msg}");
}

fn banner() {
    hint(&format!(
        "grid — {PITCH}\n  grid              what changed, and does it match the promise\n  \
         grid uses X       who reaches X, across languages and processes\n  \
         grid explain Y    what any word, symbol, or id in this output means"
    ));
}

/// One line per verb — the banner sells three, this lists them all.
fn help() {
    hint(
        "\nverbs:\n  \
         survey                       chart the worktree; idempotent\n  \
         diff [base] [head]           what changed, in defs and edges\n  \
         drift [base] [head]          dependents of changed interfaces; exit 3 on suspects\n  \
         drift --tests                known tests whose recorded verdict no longer applies\n  \
         drift --compat               changed Deserialize types old bytes cannot read\n  \
         covenant <A> <B> --via <w>   declare a coupling no language establishes\n  \
         watch [base]                 signed drift deltas, one file event per stdin line\n  \
         interfere <other-worktree>   what another worktree's edits and yours collide over\n  \
         resolve <name|file|f:line>   where it is; a bare file lists its defs\n  \
         uses <name> [--relation r]   proven inbound edges\n  \
         scope <pattern> [--depth N]  context pack around an edit\n  \
         scope <p> --bodies N         the same, with the text and its horizon\n  \
         orphans [--under p] [--traced]  no proven caller (not the same as dead); --traced adds trace N\n  \
         hotspots [--under p]         complexity x churn, descending\n  \
         roots                        import name -> directory map\n  \
         log <name>                   a definition's history in identity space\n  \
         suspects <test> [--closure <since>]  likely culprits, ranked\n  \
         test [base] [--trace|--all]  run what the change can reach; replay the rest; exit 3 on red\n  \
         gate <plan> [head]           promise vs delta; exit 3 on hold\n  \
         plan lint <file>             validate a promise file\n  \
         todo <plan>                  the promise as a worklist\n  \
         check [base] [head] [--quick]  everything the change did that nobody checked; exit 3\n  \
         check --plan <p> [--why]     the same, plus the gate verdict and why it says so\n  \
         check --compile [-- args]    wrap cargo check; the callers it did not compile; exit 3\n  \
         cache lookup|record ...      test verdicts keyed by identity closure\n  \
         edit apply <file>            compare-and-swap edits by datum\n  \
         trace ingest|diff ...        runtime traces -> observed edges\n  \
         explain <anything>           define any word or id in grid output\n  \
         serve                        the same contract on a socket\n\n\
         every verb takes --json; hints live on stderr (GRID_HINTS=0)",
    );
}

/// `grid roots` — the import-name -> directory map the callsigns are rooted
/// in. This is discovery output: the roots come from manifests, not the
/// chart, so it works before the first survey.
fn roots(_args: &[&str], json: bool) -> u8 {
    match grid_survey::files::crate_roots(root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(roots) => {
            let rows: Vec<(String, String)> = roots
                .entries()
                .map(|(dir, name)| {
                    let dir = if dir.is_empty() { "." } else { dir };
                    (name.to_owned(), dir.to_owned())
                })
                .collect();
            if json {
                return emit_json(&rows);
            }
            print!("{}", porcelain::roots(&rows));
            exit::OK
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let rest: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--json")
        .collect();

    // Engine state is settled before any verb runs: wire epoch 1 renamed the
    // retained-trace directory, and the one-time rename happens here so every
    // verb sees one store. Two candidate directories is an ambiguous state,
    // and an ambiguous state is bad input (exit 2), not a guess.
    if let Err(e) = rpc::trace_dir(root()) {
        hint(&format!("grid: {e}"));
        return ExitCode::from(exit::BAD_INPUT);
    }

    let code = match rest.split_first() {
        // Bare `grid` is the everyday verb: what changed (CLI plan §4). It
        // still teaches — the banner goes to stderr, so a machine reading
        // stdout gets records and a human gets both.
        None => {
            banner();
            diff(&[], json)
        }
        Some((&"help" | &"--help" | &"-h", _)) => {
            banner();
            help();
            exit::OK
        }
        Some((&"version", _)) => version(json),
        Some((&"survey", args)) => survey(args, json),
        Some((&"diff", args)) => diff(args, json),
        Some((&"drift", args)) => drift_verb(args, json),
        Some((&"covenant", args)) => covenant_verb(args, json),
        Some((&"watch", args)) => watch_verb(args, json),
        Some((&"resolve", args)) => resolve(args, json),
        Some((&"gate", args)) => gate(args, json),
        Some((&"plan", args)) => plan::main(args, json),
        Some((&"cache", args)) => cache(args, json),
        Some((&"scope", args)) => scope(args, json),
        Some((&"todo", args)) => todo(args, json),
        Some((&"edit", args)) => edit(args, json),
        Some((&"uses", args)) => uses(args, json),
        Some((&"orphans", args)) => orphans(args, json),
        Some((&"roots", args)) => roots(args, json),
        Some((&"log", args)) => log(args, json),
        Some((&"bisect", args)) => bisect_verb(args, json),
        Some((&"hotspots", args)) => hotspots(args, json),
        Some((&"check", args)) => check_verb(args, json),
        Some((&"explain", args)) => explain::main(args),
        Some((&"suspects", args)) => suspects(args, json),
        Some((&"interfere", args)) => interfere_verb(args, json),
        Some((&"test", args)) => testrun::main(args, json),
        Some((&"trace", args)) => trace(args, json),
        Some((&"serve", args)) => serve::main(args),
        Some((other, _)) => {
            hint(&format!(
                "grid: unknown verb `{other}`\nhint: `grid help` lists every verb"
            ));
            exit::BAD_INPUT
        }
    };
    ExitCode::from(code)
}

fn version(json: bool) -> u8 {
    let result = rpc::version(std::path::Path::new("."));
    if json {
        emit_json(&result)
    } else {
        // Not a porcelain record type (PORCELAIN.md lists none for version):
        // aligned key/value lines, stable enough to `awk '{print $2}'`.
        println!("engine     {}", result.engine);
        println!("wire       {}", result.wire);
        println!("porcelain  {}", porcelain::VERSION);
        exit::OK
    }
}

/// A `SnapshotOrRev` as typed: `snap:b3:...` is a snapshot id, anything else
/// is a git rev. Plan bases and gate heads both accept both.
fn snapshot_or_rev(typed: &str) -> grid_wire::SnapshotOrRev {
    if typed.starts_with("snap:") {
        grid_wire::SnapshotOrRev::Snapshot {
            snapshot: grid_wire::SnapshotId(typed.to_string()),
        }
    } else {
        grid_wire::SnapshotOrRev::GitRev {
            git_rev: grid_wire::GitRev(typed.to_string()),
        }
    }
}

/// `grid gate <plan-file|plan-id> [<head-rev>]` — check a plan against the
/// delta at head. Exit 3 on hold: the gate verdict *is* the exit code, so CI
/// needs no output parsing.
fn gate(args: &[&str], json: bool) -> u8 {
    let Some((spec, tail)) = args.split_first() else {
        hint("usage: grid gate <plan-file> [<head-rev>]");
        return exit::BAD_INPUT;
    };
    let plan = match plan::load(spec) {
        Ok(t) => t,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    let head = snapshot_or_rev(tail.first().unwrap_or(&"HEAD"));

    let params = grid_wire::methods::GateCheckParams { plan, head };
    match rpc::gate_check(&params, std::path::Path::new(".")) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(verdict) => {
            let code = if verdict.gate == grid_wire::GateVerdictGate::Hold {
                exit::DIVERGENCE
            } else {
                exit::OK
            };
            if json {
                // A serialization failure is internal (1) and outranks the
                // verdict; otherwise the verdict decides the exit code.
                return match emit_json(&verdict) {
                    exit::OK => code,
                    failure => failure,
                };
            }
            print!("{}", porcelain::verdict(&verdict, &namer()));
            for reason in verdict.reasons.iter().flatten() {
                hint(&format!("hold: {reason}"));
            }
            code
        }
    }
}

/// `grid cache lookup <kind> <env-hash> <datum>...` and
/// `grid cache record <kind> <env-hash> <datum>=<pass|fail>...`.
///
/// The CI shape: `lookup` prints `miss` records for what must run, `record`
/// stores the verdicts afterwards. Both call the same rpc functions the daemon
/// serves, so a CI wired through the CLI and one wired through the socket
/// cannot disagree.
fn cache(args: &[&str], json: bool) -> u8 {
    let root = std::path::Path::new(".");
    match args.split_first() {
        Some((&"lookup", rest)) => {
            let Some((kind, env, datums)) = cache_args(rest) else {
                hint("usage: grid cache lookup <lint|typecheck|test> <env-hash> <datum>...");
                return exit::BAD_INPUT;
            };
            let request = grid_wire::CacheCheckRequest {
                kind: match kind {
                    "lint" => grid_wire::CacheCheckRequestKind::Lint,
                    "typecheck" => grid_wire::CacheCheckRequestKind::Typecheck,
                    "test" => grid_wire::CacheCheckRequestKind::Test,
                    other => {
                        hint(&format!("grid: unknown cache kind `{other}`"));
                        return exit::BAD_INPUT;
                    }
                },
                env_hash: env.to_string(),
                snapshot: None,
                datums: datums
                    .iter()
                    .map(|d| grid_wire::Datum((*d).to_string()))
                    .collect(),
            };
            let params = grid_wire::methods::CacheCheckParams { request };
            match rpc::cache_check(&params, root) {
                Ok(result) => {
                    if json {
                        return emit_json(&result);
                    }
                    print!("{}", porcelain::cache_check(&result));
                    exit::OK
                }
                Err(e) => {
                    hint(&format!("grid: {e}"));
                    exit::BAD_INPUT
                }
            }
        }
        Some((&"record", rest)) => {
            let Some((trace_id, positional)) = trace_flag(rest) else {
                return exit::BAD_INPUT;
            };
            let Some((kind, env, entries)) = cache_args(&positional) else {
                hint("usage: grid cache record <lint|typecheck|test> <env-hash> <datum>=<pass|fail>... [--trace <id>]");
                return exit::BAD_INPUT;
            };
            let kind = match kind {
                "lint" => grid_wire::methods::CacheRecordKind::Lint,
                "typecheck" => grid_wire::methods::CacheRecordKind::Typecheck,
                "test" => grid_wire::methods::CacheRecordKind::Test,
                other => {
                    hint(&format!("grid: unknown cache kind `{other}`"));
                    return exit::BAD_INPUT;
                }
            };
            let mut results = Vec::with_capacity(entries.len());
            for entry in entries {
                let (datum, verdict) = entry.split_once('=').unwrap_or((entry, "pass"));
                results.push(grid_wire::CacheResultEntry {
                    datum: grid_wire::Datum(datum.to_string()),
                    verdict: match verdict {
                        "pass" => grid_wire::CacheResultEntryVerdict::Pass,
                        "fail" => grid_wire::CacheResultEntryVerdict::Fail,
                        other => {
                            hint(&format!(
                                "grid: verdict must be pass or fail, got `{other}`"
                            ));
                            return exit::BAD_INPUT;
                        }
                    },
                });
            }
            let params = grid_wire::methods::CacheRecordParams {
                kind,
                env_hash: env.to_string(),
                snapshot: None,
                trace_id,
                results,
            };
            match rpc::cache_record(&params, root) {
                Ok(result) => {
                    if json {
                        return emit_json(&result);
                    }
                    print!("{}", porcelain::cache_record(&result));
                    exit::OK
                }
                Err(e) => {
                    hint(&format!("grid: {e}"));
                    exit::BAD_INPUT
                }
            }
        }
        _ => {
            hint("usage: grid cache lookup|record <kind> <env-hash> <datum>...");
            exit::BAD_INPUT
        }
    }
}

/// `<kind> <env-hash> <rest>...`, or nothing if the shape is wrong.
/// Split a trailing `--trace <id>` off an argument list. `--trace` ties a
/// verdict to a retained trace, which is what lets `grid suspects` use it as
/// a spectrum. `None` prints its own hint.
fn trace_flag<'a>(args: &'a [&'a str]) -> Option<(Option<String>, Vec<&'a str>)> {
    let mut trace_id = None;
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--trace" {
            let Some(id) = it.next() else {
                hint("usage: grid cache record <kind> <env-hash> <datum>=<pass|fail>... [--trace <id>]");
                return None;
            };
            trace_id = Some((*id).to_owned());
        } else {
            positional.push(*arg);
        }
    }
    Some((trace_id, positional))
}

fn cache_args<'a>(args: &'a [&'a str]) -> Option<(&'a str, &'a str, &'a [&'a str])> {
    match args {
        [kind, env, rest @ ..] if !rest.is_empty() => Some((kind, env, rest)),
        _ => None,
    }
}

/// `--json` is the wire result verbatim — the same bytes the daemon returns.
fn emit_json<T: serde::Serialize>(value: &T) -> u8 {
    match serde_json::to_string(value) {
        Ok(s) => {
            println!("{s}");
            exit::OK
        }
        Err(e) => {
            hint(&format!("grid: cannot serialize result: {e}"));
            exit::INTERNAL
        }
    }
}

/// The chart to render names from, if the store opens.
///
/// Rendering degrades rather than fails: a diff or verdict is still worth
/// printing with `@prefix` coordinates when the store is missing, and a user
/// who cannot see the output cannot fix the store either.
fn namer() -> impl Fn(&grid_wire::Datum) -> Option<String> {
    let chart = chart_provider::ChartDelta::open(root()).ok();
    move |d: &grid_wire::Datum| {
        chart
            .as_ref()
            .and_then(|c| grid_gate::DeltaProvider::callsign(c, d))
            .map(|c| c.0)
    }
}

/// The repo root every verb works against. Grid is run from inside the tree it
/// charts, so this is `.` — named once rather than spelled at each call site.
fn root() -> &'static std::path::Path {
    std::path::Path::new(".")
}

/// `grid survey [<rev>]` — survey the tree and publish a snapshot.
///
/// Idempotent: an unchanged tree produces the same snapshot id, so running it
/// twice costs a walk and publishes nothing new.
fn survey(args: &[&str], json: bool) -> u8 {
    let params = grid_wire::methods::SurveyRunParams {
        git_rev: args.first().map(|r| grid_wire::GitRev((*r).to_string())),
        worktree: None,
    };
    let started = std::time::Instant::now();
    match rpc::Chart::new(root()).survey_run(&params) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::INTERNAL
        }
        Ok((snapshot, ran)) => {
            if let Some(epoch) = ran.superseded {
                hint(&format!(
                    "grid: dropped a chart written by segment format epoch {epoch} — \
                     re-surveyed from cold, nothing was reinterpreted"
                ));
            }
            if json {
                return emit_json(&snapshot);
            }
            print!(
                "{}",
                porcelain::snapshot(&snapshot, Some((ran.reread, started.elapsed())))
            );
            exit::OK
        }
    }
}

/// `grid diff [<base>] [<head>]` — what actually changed, in the chart.
///
/// Defaults to `HEAD` against the working tree's snapshot, which is the
/// question "what have I changed" and the reason bare `grid` runs it.
fn diff(args: &[&str], json: bool) -> u8 {
    let rev = |s: &str| grid_wire::SnapshotOrRev::GitRev {
        git_rev: grid_wire::GitRev(s.to_string()),
    };
    let params = grid_wire::methods::DeltaParams {
        base: rev(args.first().copied().unwrap_or("HEAD~1")),
        head: rev(args.get(1).copied().unwrap_or("HEAD")),
    };
    match rpc::delta(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(diff) => {
            if json {
                return emit_json(&diff);
            }
            // Edges print in callsign space, so the renderer needs the chart
            // to name datums with.
            print!("{}", porcelain::diff(&diff, &namer()));
            exit::OK
        }
    }
}

/// `grid drift [<base>] [<head>] [--floor <grade>] [--plan <file>]` — the
/// dependents of every changed interface that did not themselves change.
///
/// Exit 3 when suspects exist: an unaddressed dependent of a changed
/// interface is a divergence, and CI needs no output parsing to see it. Exit
/// 1 when an inbound walk ran past its fuel — the answer is refused whole,
/// because a truncated walk reports fewer suspects and looks cleaner for it.
fn drift_verb(args: &[&str], json: bool) -> u8 {
    let mut floor = None;
    let mut plan = None;
    let mut tests = false;
    let mut compat = false;
    let mut positional = Vec::new();
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match *arg {
            "--floor" => {
                let (Some(spec), Some(tail)) = (rest.first(), rest.get(1..)) else {
                    hint(DRIFT_USAGE);
                    return exit::BAD_INPUT;
                };
                let Some(want) = grade_named(spec) else {
                    hint(DRIFT_USAGE);
                    return exit::BAD_INPUT;
                };
                floor = Some(want);
                rest = tail;
            }
            "--plan" => {
                let (Some(spec), Some(tail)) = (rest.first(), rest.get(1..)) else {
                    hint(DRIFT_USAGE);
                    return exit::BAD_INPUT;
                };
                match plan::load(spec) {
                    Ok(t) => plan = Some(t),
                    Err(e) => {
                        hint(&format!("grid: {e}"));
                        return exit::BAD_INPUT;
                    }
                }
                rest = tail;
            }
            "--tests" => tests = true,
            "--compat" => compat = true,
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{DRIFT_USAGE}"));
                return exit::BAD_INPUT;
            }
            other => positional.push(other),
        }
    }
    if positional.len() > 2 {
        hint(DRIFT_USAGE);
        return exit::BAD_INPUT;
    }
    let params = grid_wire::methods::DriftParams {
        base: snapshot_or_rev(positional.first().unwrap_or(&"HEAD~1")),
        head: snapshot_or_rev(positional.get(1).unwrap_or(&"HEAD")),
        floor,
        plan,
        tests: tests.then_some(true),
        compat: compat.then_some(true),
    };
    match rpc::drift(&params, root()) {
        Err(
            e @ (drift::Error::BudgetRejected(_) | drift::Error::Trace(_) | drift::Error::Stale(_)),
        ) => {
            hint(&format!("grid: {e}"));
            exit::INTERNAL
        }
        Err(e @ (drift::Error::Chart(_) | drift::Error::Covenants(_))) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(report) => {
            // A stale verdict is proven-grade staleness — the closure key is
            // identity math — so it diverges exactly as a suspect does.
            // A compat finding gates only where a `persisted` invariant
            // declared that old bytes exist: that declaration is the human
            // fact, and without it the finding is heuristic and ranks.
            let declared_compat = report.compat.iter().flatten().any(|f| f.declared);
            let code = if report.suspects.is_empty()
                && report.stale_tests.iter().flatten().next().is_none()
                && !declared_compat
            {
                exit::OK
            } else {
                exit::DIVERGENCE
            };
            if json {
                // A serialization failure is internal (1) and outranks the
                // verdict, exactly as `grid gate` treats it.
                return match emit_json(&report) {
                    exit::OK => code,
                    failure => failure,
                };
            }
            print!("{}", porcelain::drift(&report));
            if code == exit::DIVERGENCE {
                hint(
                    "grid: these depend on an interface that moved and were not touched — \
                     update them, or promise them in a plan and re-run with --plan",
                );
            }
            code
        }
    }
}

const DRIFT_USAGE: &str = "usage: grid drift [<base>] [<head>] \
     [--floor <proven|observed|modeled|heuristic>] [--plan <file>] [--tests] [--compat]";

/// The wire spelling of an evidence grade, for `--floor` arguments. The serde
/// names are the vocabulary, so the flag cannot drift from the contract.
fn grade_named(name: &str) -> Option<grid_wire::Grade> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

const COVENANT_USAGE: &str =
    "usage: grid covenant <A> <B> --via <word> [--note <text>] | grid covenant --list";

/// `grid covenant <A> <B> --via <word> [--note <text>]` — declare a coupling no
/// language's binding rules establish, so drift can see it.
///
/// Both patterns are resolved against the current snapshot and a pattern
/// matching nothing is refused: a declaration about code that does not exist
/// is a rule that will never fire, and writing it down silently is worse than
/// not writing it. Removing a covenant is editing the file — it is a
/// declaration, like a plan.
fn covenant_verb(args: &[&str], json: bool) -> u8 {
    if args.first() == Some(&"--list") {
        return match covenant::load(root()) {
            Err(e) => {
                hint(&format!("grid: {e}"));
                exit::BAD_INPUT
            }
            Ok(rows) => {
                if json {
                    return emit_json(&rows);
                }
                print!("{}", porcelain::covenants(&rows));
                exit::OK
            }
        };
    }
    let mut via = None;
    let mut note = None;
    let mut positional = Vec::new();
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        let mut flag = |slot: &mut Option<String>| match rest.split_first() {
            Some((value, tail)) => {
                *slot = Some((*value).to_owned());
                rest = tail;
                true
            }
            None => false,
        };
        match *arg {
            "--via" => {
                if !flag(&mut via) {
                    hint(COVENANT_USAGE);
                    return exit::BAD_INPUT;
                }
            }
            "--note" => {
                if !flag(&mut note) {
                    hint(COVENANT_USAGE);
                    return exit::BAD_INPUT;
                }
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{COVENANT_USAGE}"));
                return exit::BAD_INPUT;
            }
            other => positional.push(other),
        }
    }
    let ([a, b], Some(via)) = (positional.as_slice(), via) else {
        hint(COVENANT_USAGE);
        return exit::BAD_INPUT;
    };
    if via.is_empty() {
        hint(COVENANT_USAGE);
        return exit::BAD_INPUT;
    }
    for pattern in [*a, *b] {
        match rpc::matching(root(), pattern) {
            Err(e) => {
                hint(&format!("grid: {e}"));
                return exit::INTERNAL;
            }
            // The `resolve` contract, restated: a name that resolves to
            // nothing is bad input, and the hint lists the closest charted
            // callsigns — silence was read as "does not exist", which is the
            // one conclusion grid never licenses.
            Ok(0) => {
                hint(&miss(pattern));
                return exit::BAD_INPUT;
            }
            Ok(_) => {}
        }
    }
    let mut covenants = match covenant::load(root()) {
        Ok(rows) => rows,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    let fresh = grid_wire::Covenant {
        a: grid_wire::CallsignPattern((*a).to_owned()),
        b: grid_wire::CallsignPattern((*b).to_owned()),
        via,
        note,
    };
    // Declaring the same pair twice is one declaration, not two.
    if !covenants
        .iter()
        .any(|c| c.a == fresh.a && c.b == fresh.b && c.via == fresh.via)
    {
        covenants.push(fresh);
    }
    if let Err(e) = covenant::save(root(), &covenants) {
        hint(&format!("grid: {e}"));
        return exit::INTERNAL;
    }
    if json {
        return emit_json(&covenants);
    }
    print!("{}", porcelain::covenants(&covenants));
    exit::OK
}

const WATCH_USAGE: &str = "usage: grid watch [<base>] [--floor <grade>] [--plan <file>]\n\
     reads changed paths on stdin, one per line; a blank line ends an event";

/// `grid watch [<base>]` — drift maintained as signed deltas.
///
/// Events arrive on stdin: one changed path per line, a blank line ending an
/// event. That is deliberately the whole event source — an OS watcher is a
/// dependency decision, and `fswatch . | grid watch` or
/// `git diff --name-only | grid watch` already reaches this.
///
/// Output is `delta` records, `+` for a suspect that appeared and `-` for one
/// retracted. A consumer holds a running sum; the sum after any prefix of
/// events is what a fresh `grid drift` would say.
fn watch_verb(args: &[&str], json: bool) -> u8 {
    let mut floor = None;
    let mut plan = None;
    let mut positional = Vec::new();
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match *arg {
            "--floor" => {
                let (Some(spec), Some(want), Some(tail)) = (
                    rest.first(),
                    rest.first().and_then(|s| grade_named(s)),
                    rest.get(1..),
                ) else {
                    hint(WATCH_USAGE);
                    return exit::BAD_INPUT;
                };
                let _ = spec;
                floor = Some(want);
                rest = tail;
            }
            "--plan" => {
                let (Some(spec), Some(tail)) = (rest.first(), rest.get(1..)) else {
                    hint(WATCH_USAGE);
                    return exit::BAD_INPUT;
                };
                match plan::load(spec) {
                    Ok(t) => plan = Some(t),
                    Err(e) => {
                        hint(&format!("grid: {e}"));
                        return exit::BAD_INPUT;
                    }
                }
                rest = tail;
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{WATCH_USAGE}"));
                return exit::BAD_INPUT;
            }
            other => positional.push(other),
        }
    }
    if positional.len() > 1 {
        hint(WATCH_USAGE);
        return exit::BAD_INPUT;
    }
    let base = snapshot_or_rev(positional.first().unwrap_or(&"HEAD"));
    let mut session = match watch::Session::open(
        root(),
        base,
        floor.unwrap_or(grid_wire::Grade::Observed),
        plan,
    ) {
        Ok(session) => session,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    hint("grid: watching — one changed path per line, blank line ends an event");

    let mut batch: Vec<String> = Vec::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            hint("grid: cannot read stdin");
            return exit::INTERNAL;
        };
        let path = line.trim();
        if !path.is_empty() {
            batch.push(path.to_owned());
            continue;
        }
        if batch.is_empty() {
            continue;
        }
        let code = emit_event(&mut session, &batch, json);
        batch.clear();
        if code != exit::OK {
            return code;
        }
    }
    // End of input is the end of the last event, so a caller that pipes a
    // file list and closes the pipe gets an answer rather than silence.
    let code = if batch.is_empty() {
        exit::OK
    } else {
        emit_event(&mut session, &batch, json)
    };
    let declined = session.declined();
    if code == exit::OK && declined > 0 && !json {
        print!("{}", porcelain::declined(&[format!("{declined} events")]));
    }
    code
}

/// One event's worth of output. Always exit 0 on success: watch mode reports
/// a stream, and a stream has no verdict — `grid drift` is the verb that
/// holds.
fn emit_event(session: &mut watch::Session, batch: &[String], json: bool) -> u8 {
    match session.event(batch) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::INTERNAL
        }
        Ok(emitted) => {
            for path in &emitted.declined {
                hint(&format!(
                    "grid: {path} does not parse — no deltas for this event, \
                     because a file grid cannot read is not a file with nothing in it"
                ));
            }
            if !json {
                // The refusal is part of the stream, not a note beside it.
                print!("{}", porcelain::declined(&emitted.declined));
            }
            if emitted.recomputed {
                hint(
                    "grid: event too large to maintain incrementally — recomputed the whole \
                     suspect set; the deltas are the same, the work was not",
                );
            }
            if json {
                return emit_json(&emitted.deltas);
            }
            print!("{}", porcelain::deltas(&emitted.deltas));
            // A stream is only a stream if it arrives: without this the
            // consumer sees nothing until the pipe closes.
            let _unflushed = std::io::stdout().flush();
            exit::OK
        }
    }
}

/// `grid resolve <name>` — the definitions a name refers to.
///
/// Ambiguity is not an error here: every candidate is printed, because the
/// list is what makes the next command typeable.
fn resolve(args: &[&str], json: bool) -> u8 {
    let Some(name) = args.first() else {
        hint("usage: grid resolve <callsign|name@prefix>");
        return exit::BAD_INPUT;
    };
    // `path:line` jumps from a location to the definitions containing it;
    // `@` may be a datum pin or a Rust trait-impl callsign; the Handle
    // locator tells them apart by whether the suffix is hex, so a pinned
    // name spells the same here as in `grid uses`.
    let file_line = name
        .rsplit_once(':')
        .and_then(|(file, line)| Some((file, line.parse::<u64>().ok()?)))
        .filter(|(file, _)| file.contains('/'));
    // A bare file path lists what the file contains — the "what's in
    // here" opener. `:line` narrows to the enclosing definition instead.
    let looks_like_file = file_line.is_none()
        && !name.contains('@')
        && (name.contains('/')
            || matches!(
                name.rsplit_once('.').map(|(_, e)| e),
                Some("py" | "pyi" | "rs")
            ));
    if looks_like_file {
        return match rpc::defs_in_file(root(), name) {
            Err(e) => {
                hint(&format!("grid: {e}"));
                exit::INTERNAL
            }
            Ok(defs) if defs.is_empty() => {
                hint(&format!(
                    "grid: no charted definitions in `{name}` — paths are repo-relative"
                ));
                exit::BAD_INPUT
            }
            Ok(defs) => {
                if json {
                    return emit_json(&defs);
                }
                print!("{}", porcelain::refs(&defs));
                exit::OK
            }
        };
    }
    let locator = if let Some((file, line)) = file_line {
        grid_wire::Locator::FileLine {
            file: file.to_owned(),
            line,
        }
    } else if name.contains('@') {
        grid_wire::Locator::Handle {
            handle: grid_wire::Handle((*name).to_string()),
        }
    } else {
        grid_wire::Locator::Callsign {
            callsign: grid_wire::Callsign((*name).to_string()),
        }
    };
    let params = grid_wire::methods::ResolveParams {
        locator,
        snapshot: None,
    };
    match rpc::resolve(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::INTERNAL
        }
        Ok(defs) => {
            if json {
                return emit_json(&defs);
            }
            if defs.len() > 1 {
                hint(&format!("grid: {name} names {} definitions", defs.len()));
            }
            // A miss used to be a silent exit 0, and agents read silence as
            // "does not exist". Bad input plus the near misses instead.
            if defs.is_empty() {
                hint(&miss(name));
                return exit::BAD_INPUT;
            }
            print!("{}", porcelain::refs(&defs));
            exit::OK
        }
    }
}

/// The refusal for a name that resolved to nothing: what was asked, and the
/// closest charted callsigns so a partial path or a typo self-corrects.
fn miss(name: &str) -> String {
    let mut msg = format!("grid: nothing is called `{name}`");
    if let Ok(near) = rpc::nearest(root(), name, 5) {
        if !near.is_empty() {
            msg.push_str(" — closest:");
            for callsign in near {
                msg.push_str("\n  ");
                msg.push_str(&callsign);
            }
        }
    }
    msg
}

/// `grid scope <callsign-pattern>... [--depth N] [--bodies N] [--sigs M]
/// [--budget T] [--since S]` — the context pack around an intended edit: what
/// matches, what touches it, what rules reach it. Without `--bodies`/`--sigs`
/// the text is deliberately absent and the agent pulls it itself; with them
/// the pack carries bodies to one depth and signatures to a further one, under
/// a token budget spent in whole layers, closed by a horizon.
fn scope(args: &[&str], json: bool) -> u8 {
    const USAGE: &str =
        "usage: grid scope <pattern>... [--depth N] [--bodies N] [--sigs M] [--budget T] [--since <snapshot|rev>]";
    let mut patterns = Vec::new();
    let (mut depth, mut bodies, mut sigs, mut budget) = (None, None, None, None);
    let mut since = None;
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        let number = |slot: &mut Option<u64>| {
            let Some(n) = tail.first().and_then(|n| n.parse::<u64>().ok()) else {
                return false;
            };
            *slot = Some(n);
            true
        };
        let taken = match *arg {
            "--depth" => number(&mut depth),
            "--bodies" => number(&mut bodies),
            "--sigs" => number(&mut sigs),
            "--budget" => number(&mut budget),
            "--since" => {
                since = tail.first().map(|s| snapshot_or_rev(s));
                since.is_some()
            }
            _ => {
                patterns.push(grid_wire::CallsignPattern((*arg).to_string()));
                rest = tail;
                continue;
            }
        };
        if !taken {
            hint(USAGE);
            return exit::BAD_INPUT;
        }
        rest = &tail[1..];
    }
    if patterns.is_empty() {
        hint(USAGE);
        return exit::BAD_INPUT;
    }
    let tiers = bodies.is_some() || sigs.is_some();
    let params = grid_wire::methods::ScopeParams {
        callsigns: patterns,
        depth,
        snapshot: None,
        bodies,
        sigs,
        budget,
        since,
    };
    match rpc::scope(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(pack) => {
            if json {
                return emit_json(&pack);
            }
            if pack.horizon.is_some() {
                // Text mode: prose on stdout, closed by its own horizon line.
                print!("{}", porcelain::scope_document(&pack));
            } else {
                print!("{}", porcelain::scope(&pack, &namer()));
            }
            // The pack's horizon, said out loud. `scope` walks a fixed number
            // of layers and spends no fuel, so it is complete to exactly that
            // depth and to nothing beyond — a reader who cannot tell those
            // apart cannot use a bounded answer.
            let hops = if tiers {
                // Asking for text asks for the hops it lives at, so the walk
                // is at least as deep as the outermost tier.
                depth
                    .unwrap_or(DEFAULT_SCOPE_DEPTH)
                    .max(sigs.unwrap_or(3).max(bodies.unwrap_or(1)))
            } else {
                depth.unwrap_or(DEFAULT_SCOPE_DEPTH)
            };
            hint(&format!(
                "grid: scope complete through {hops} hop{} — nothing further was walked",
                if hops == 1 { "" } else { "s" }
            ));
            exit::OK
        }
    }
}

/// The depth `grid scope` walks when none is named. Stated here rather than
/// only in the daemon, because the horizon banner has to name the real one.
const DEFAULT_SCOPE_DEPTH: u64 = 0;

/// `grid todo <plan> [<head-rev|snap:id>]` — the gate verdict as a
/// worklist. Always exit 0 when it answers: remaining work is a report, not
/// a failure — the gate is the verb that holds.
fn todo(args: &[&str], json: bool) -> u8 {
    let Some((id, tail)) = args.split_first() else {
        hint("usage: grid todo <plan> [<head-rev>]");
        return exit::BAD_INPUT;
    };
    let head = tail.first().map(|h| snapshot_or_rev(h));
    let params = grid_wire::methods::TodoParams {
        plan: grid_wire::PlanId((*id).to_string()),
        head,
    };
    match rpc::todo(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(list) => {
            if json {
                return emit_json(&list);
            }
            print!("{}", porcelain::todo(&list));
            exit::OK
        }
    }
}

/// `grid edit apply <file|->` — the CAS write path. Edits are JSON
/// (`EditApplyParams`) from a file or stdin; conflicts are `conflict`
/// records and exit 4, and the recovery is re-resolving the `now`
/// coordinates and retrying the whole request.
fn edit(args: &[&str], json: bool) -> u8 {
    let Some((&"apply", rest)) = args.split_first() else {
        hint("usage: grid edit apply <edits.json|->");
        return exit::BAD_INPUT;
    };
    let Some(source) = rest.first() else {
        hint("usage: grid edit apply <edits.json|->");
        return exit::BAD_INPUT;
    };
    let text = if *source == "-" {
        use std::io::Read as _;
        let mut buf = String::new();
        if std::io::stdin().read_to_string(&mut buf).is_err() {
            hint("grid: cannot read edits from stdin");
            return exit::BAD_INPUT;
        }
        buf
    } else {
        match std::fs::read_to_string(source) {
            Ok(text) => text,
            Err(e) => {
                hint(&format!("grid: cannot read {source}: {e}"));
                return exit::BAD_INPUT;
            }
        }
    };
    let params: grid_wire::methods::EditApplyParams = match serde_json::from_str(&text) {
        Ok(params) => params,
        Err(e) => {
            hint(&format!("grid: edits are not valid EditApplyParams: {e}"));
            return exit::BAD_INPUT;
        }
    };
    match edit::apply(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(outcome) => {
            record_edit_legs(&outcome);
            let stale = !outcome.wire.conflicts.is_empty();
            if json {
                let failure = emit_json(&outcome.wire);
                return if failure == exit::OK && stale {
                    exit::STALE
                } else {
                    failure
                };
            }
            print!("{}", porcelain::edit_applied(&outcome.edits));
            print!("{}", porcelain::edit_conflicts(&outcome.wire.conflicts));
            for conflict in &outcome.wire.conflicts {
                // The recovery is the conflict itself: `now` is the current
                // identity, so a retry with it as the base cannot go stale.
                hint(&format!(
                    "hint: {} moved — retry with base_datum {}",
                    conflict.target, conflict.now.0
                ));
            }
            if stale {
                exit::STALE
            } else {
                exit::OK
            }
        }
    }
}

/// `grid uses <name>` — who reaches X: the proven inbound edges of the one
/// definition the name resolves to. Ambiguity is a list of pasteable
/// commands, per the CLI plan's error doctrine.
fn uses(args: &[&str], json: bool) -> u8 {
    let mut relation = None;
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--relation" {
            let Some(want) = it.next().and_then(|w| relation_named(w)) else {
                hint("usage: grid uses <name> --relation <calls|constructs|reads_field|writes_field|implements|imports>");
                return exit::BAD_INPUT;
            };
            relation = Some(want);
        } else {
            positional.push(*arg);
        }
    }
    let Some(name) = positional.first() else {
        hint("usage: grid uses <name|name@prefix> [--relation <r>]");
        return exit::BAD_INPUT;
    };
    let locator = if name.contains('@') {
        grid_wire::Locator::Handle {
            handle: grid_wire::Handle((*name).to_string()),
        }
    } else {
        grid_wire::Locator::Callsign {
            callsign: grid_wire::Callsign((*name).to_string()),
        }
    };
    let params = grid_wire::methods::ResolveParams {
        locator,
        snapshot: None,
    };
    match rpc::uses(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok((defs, mut edges)) => {
            if let Some(want) = relation {
                edges.retain(|e| e.relation == want);
            }
            match defs.as_slice() {
                [] => {
                    hint(&miss(name));
                    return exit::BAD_INPUT;
                }
                [_, _, ..] => {
                    let mut msg = "ambiguous — did you mean:".to_string();
                    for def in &defs {
                        if let Some(callsign) = &def.callsign {
                            msg.push_str("\n  grid uses ");
                            msg.push_str(&callsign.0);
                        }
                    }
                    hint(&msg);
                    return exit::BAD_INPUT;
                }
                [_] => {}
            }
            if json {
                return emit_json(&edges);
            }
            if edges.is_empty() {
                hint("grid: no proven callers — not the same as unused; `grid orphans` lists these honestly");
            }
            print!("{}", porcelain::uses(&edges, &namer()));
            exit::OK
        }
    }
}

/// `grid trace ingest --lcov|--coverage-json <report> --test <name>` —
/// coverage as a reach set: every executed line inside a charted definition
/// becomes one observation from the named test.
///
/// One report is one test. The per-runner recipes that produce one report
/// per test are in the README; the orchestration that runs them is
/// `grid test --trace`, not this handler.
fn trace_coverage(flag: &str, rest: &[&str], json: bool, root: &std::path::Path) -> u8 {
    let (Some(path), Some(&"--test"), Some(test)) = (rest.get(1), rest.get(2), rest.get(3)) else {
        hint(&format!(
            "usage: grid trace ingest {flag} <report|-> --test <name>"
        ));
        return exit::BAD_INPUT;
    };
    let Some(report) = read_arg(path) else {
        return exit::BAD_INPUT;
    };
    let mapping = if flag == "--lcov" {
        coverage::trace_of_lcov(root, &report, test)
    } else {
        coverage::trace_of_coverage_json(root, &report, test)
    };
    let mapped = match mapping {
        Ok(mapped) => mapped,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    if mapped.unmapped_lines > 0 || mapped.unknown_files > 0 {
        hint(&format!(
            "grid: {} executed lines outside any definition, {} files not in the chart — omitted, not guessed",
            mapped.unmapped_lines, mapped.unknown_files
        ));
    }
    let total = mapped.trace.observations.len() as u64;
    let params = grid_wire::methods::TraceIngestParams {
        trace: mapped.trace,
    };
    match rpc::trace_ingest(&params, root) {
        Ok(result) => {
            if json {
                return emit_json(&result);
            }
            print!("{}", porcelain::trace_ingest(&result, total));
            exit::OK
        }
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
    }
}

/// A file argument, `-` meaning stdin. `None` prints its own hint.
fn read_arg(path: &str) -> Option<String> {
    if path == "-" {
        use std::io::Read as _;
        let mut buf = String::new();
        match std::io::stdin().read_to_string(&mut buf) {
            Ok(_) => Some(buf),
            Err(e) => {
                hint(&format!("grid: cannot read stdin: {e}"));
                None
            }
        }
    } else {
        match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(e) => {
                hint(&format!("grid: cannot read {path}: {e}"));
                None
            }
        }
    }
}

/// Every landed modification becomes a edit in the patch log. Best effort:
/// an unwritable log costs history, never the edit that already landed.
fn record_edit_legs(outcome: &edit::Outcome) {
    let root = root();
    let rev = chart_provider::resolve_rev(root, "HEAD");
    let rows: Vec<grid_wire::Edit> = outcome
        .edits
        .iter()
        .filter_map(|detail| match detail {
            edit::EditDetail::Modified { was, now, .. } => Some(grid_wire::Edit {
                from: was.clone(),
                to: now.clone(),
                git_rev: Some(grid_wire::GitRev(rev.clone())),
                at: None,
            }),
            _ => None,
        })
        .collect();
    if let Err(e) = history::append(root, &rows) {
        hint(&format!("grid: patch log not updated: {e}"));
    }
}

/// `grid log <name>` — a datum's history in identity space: every edit that
/// chains to it, newest first. A rename does not break the chain, because
/// edits chain datums, not names.
fn log(args: &[&str], json: bool) -> u8 {
    let Some(name) = args.first() else {
        hint("usage: grid log <name|name@prefix>");
        return exit::BAD_INPUT;
    };
    let params = grid_wire::methods::ResolveParams {
        locator: grid_wire::Locator::Callsign {
            callsign: grid_wire::Callsign((*name).to_string()),
        },
        snapshot: None,
    };
    let defs = match rpc::resolve(&params, root()) {
        Ok(defs) => defs,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::INTERNAL;
        }
    };
    let Some(def) = defs.first() else {
        hint(&miss(name));
        return exit::BAD_INPUT;
    };
    let rows = history::chain(root(), &def.datum.0);
    if json {
        return emit_json(&rows);
    }
    if rows.is_empty() {
        hint(&format!(
            "grid: no recorded history for `{name}` — edits accrue from `grid edit apply` and surveys"
        ));
    }
    print!("{}", porcelain::edits(&rows));
    exit::OK
}

/// `grid hotspots [--limit N] [--under <path>]` — complexity × churn,
/// descending: what to refactor first. No thresholds, no gate, no
/// exit-code opinion — clippy
/// owns lints; grid ranks, and the header names the evidence used.
fn hotspots(args: &[&str], json: bool) -> u8 {
    let mut limit = 20usize;
    let mut under = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--limit" {
            let Some(n) = it.next().and_then(|n| n.parse().ok()) else {
                hint("usage: grid hotspots [--limit N] [--under <path>]");
                return exit::BAD_INPUT;
            };
            limit = n;
        }
        if *arg == "--under" {
            let Some(path) = it.next() else {
                hint("usage: grid hotspots [--limit N] [--under <path>]");
                return exit::BAD_INPUT;
            };
            under = Some(*path);
        }
    }
    match rpc::hotspots(root(), limit, under) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok((rows, source)) => {
            if json {
                return emit_json(&rows);
            }
            match source {
                rpc::ChurnSource::GitFiles => hint(
                    "grid: churn seeded from git file history (file-level) — grid edits accrue from edit apply and surveys",
                ),
                rpc::ChurnSource::None => hint(
                    "grid: no history yet — ranking by complexity alone",
                ),
                rpc::ChurnSource::Edits => {}
            }
            print!("{}", porcelain::hotspots(&rows));
            exit::OK
        }
    }
}

/// `grid bisect <test> [<good> <bad>]` — first failing patch in the
/// retained chain, by the outcomes recorded against datum versions.
fn bisect_verb(args: &[&str], json: bool) -> u8 {
    let Some(test) = args.first() else {
        hint("usage: grid bisect <test> [<good> <bad>]");
        return exit::BAD_INPUT;
    };
    let params = grid_wire::methods::BisectParams {
        good: snapshot_or_rev(args.get(1).unwrap_or(&"HEAD~1")),
        bad: snapshot_or_rev(args.get(2).unwrap_or(&"HEAD")),
        test: (*test).to_owned(),
    };
    match bisect::run(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(result) => {
            if json {
                return emit_json(&result);
            }
            if let Some(culprit) = &result.culprit {
                println!(
                    "culprit  {}  steps {}  cache_hits {}",
                    culprit.0, result.steps, result.cache_hits
                );
            }
            exit::OK
        }
    }
}

/// The wire spelling of a relation, for `--relation` arguments. The serde
/// names are the vocabulary, so the flag cannot drift from the contract.
fn relation_named(name: &str) -> Option<grid_wire::Relation> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

const ORPHANS_USAGE: &str =
    "usage: grid orphans [--lang <py|rs|ts|js>] [--under <path>] [--traced]";

/// `grid orphans [--lang <py|rs|ts|js>] [--under <path>] [--traced]` —
/// definitions with no proven inbound edge. The header states the evidence
/// honestly: *no proven caller* — public API, tests, and macro-reached code
/// all look like this, and none of them is dead. `--traced` adds how many
/// retained traces hit each one; `trace 0` is the workspace-level unused
/// question, and still a maybe.
fn orphans(args: &[&str], json: bool) -> u8 {
    let mut lang = None;
    let mut under = None;
    let mut traced = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--lang" {
            let Some(want) = it.next().and_then(|w| {
                serde_json::from_value(serde_json::Value::String((*w).to_owned())).ok()
            }) else {
                hint(ORPHANS_USAGE);
                return exit::BAD_INPUT;
            };
            lang = Some(want);
        }
        if *arg == "--under" {
            let Some(path) = it.next() else {
                hint(ORPHANS_USAGE);
                return exit::BAD_INPUT;
            };
            under = Some(*path);
        }
        if *arg == "--traced" {
            traced = true;
        }
    }
    let rows = if traced {
        rpc::orphans_traced(root(), lang, under).map(|rows| {
            if json {
                return emit_json(&rows);
            }
            print!("{}", porcelain::orphans(&rows));
            exit::OK
        })
    } else {
        rpc::orphans(root(), lang, under).map(|rows| {
            if json {
                return emit_json(&rows);
            }
            print!("{}", porcelain::refs_as(&rows, "orphan"));
            exit::OK
        })
    };
    match rows {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(code) => {
            if !json {
                hint("grid: no proven caller — not the same as dead (public API, tests, macro-reached code all match)");
            }
            code
        }
    }
}

const CHECK_USAGE: &str = "usage: grid check [<base>] [<head>] [--plan <plan>] [--quick] [--why] [--trace <id>] [--compile [-- <cargo check args>]]\n       grid check --compile [-- <cargo check args>]";

/// `grid check [<base>] [<head>] [--plan <plan>] [--quick] [--why]` —
/// everything the change did that nobody checked: drift suspects, the
/// compile record's uncompiled callers, adherence when a plan is named,
/// stale verdicts, covenant fires and declared compat findings gate (exit
/// 3); weak tests, uncovered definitions, clone-of and undeclared compat
/// findings rank. When nothing gates, one `done  refactor` record states the
/// three facts a refactor claim rests on. `--why` adds, under a plan, a path
/// per fulfilling promise and what changed without being run. `--quick` is
/// drift plus the compile record and nothing that reads a trace or the
/// verdict log — the `PostToolUse` form.
///
/// A section that refuses refuses the whole check with its reason: exit 1
/// when grid could not stand behind an answer (aground, an unreadable
/// store), 2 when the input was wrong. Never a report with a section quietly
/// missing.
fn check_verb(args: &[&str], json: bool) -> u8 {
    let mut plan = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut trace = None;
    let mut why = false;
    let mut quick = false;
    let mut compile = None;
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match *arg {
            "--why" => why = true,
            "--quick" => quick = true,
            "--compile" => compile = Some(Vec::new()),
            "--" => {
                let Some(cargo_args) = compile.as_mut() else {
                    hint(&format!(
                        "grid: `--` carries cargo check args and needs --compile\n{CHECK_USAGE}"
                    ));
                    return exit::BAD_INPUT;
                };
                cargo_args.extend(rest.iter().map(|a| (*a).to_owned()));
                break;
            }
            "--trace" | "--plan" => {
                let Some((value, tail)) = rest.split_first() else {
                    hint(CHECK_USAGE);
                    return exit::BAD_INPUT;
                };
                if *arg == "--plan" {
                    plan = Some(grid_wire::PlanId((*value).to_string()));
                } else {
                    trace = Some(grid_wire::TraceId((*value).to_string()));
                }
                rest = tail;
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{CHECK_USAGE}"));
                return exit::BAD_INPUT;
            }
            other if positional.len() < 2 => positional.push(other),
            other => {
                hint(&format!(
                    "grid: unexpected argument `{other}`\n{CHECK_USAGE}"
                ));
                return exit::BAD_INPUT;
            }
        }
    }
    if trace.is_some() && plan.is_none() {
        hint(&format!(
            "grid: --trace names the run a plan's coverage is read from and needs --plan\n{CHECK_USAGE}"
        ));
        return exit::BAD_INPUT;
    }
    // `--compile` wraps the compile and writes the record first; with a
    // plan the check then reads it, without one the record's own report is
    // the whole answer — the mode use case 3 runs, unchanged.
    let mut compiled = None;
    if let Some(cargo_args) = compile {
        if !positional.is_empty() {
            hint(&format!(
                "grid: --compile checks the worktree against {COMPILE_BASE}; endpoints do not apply\n{CHECK_USAGE}"
            ));
            return exit::BAD_INPUT;
        }
        match compile_wrap(&cargo_args) {
            Ok(record) => compiled = Some(record),
            Err(code) => return code,
        }
    }
    if plan.is_none() {
        if let Some(record) = compiled.as_ref() {
            return compile_report(record, json);
        }
    }
    let params = grid_wire::methods::CheckParams {
        plan,
        base: positional.first().map(|b| snapshot_or_rev(b)),
        head: positional
            .get(1)
            .map_or_else(|| snapshot_or_rev("HEAD"), |h| snapshot_or_rev(h)),
        trace,
        compile: compiled.is_some().then_some(true),
        quick: quick.then_some(true),
    };
    match check::answer(&params, root()) {
        // Grid could not stand behind a section: a walk over fuel, a store
        // that would not open. Internal, and never a report without it.
        Err(
            e @ (check::Error::Refused(_)
            | check::Error::Drift(
                drift::Error::BudgetRejected(_) | drift::Error::Trace(_) | drift::Error::Stale(_),
            )),
        ) => {
            hint(&format!("grid: {e}"));
            exit::INTERNAL
        }
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(pack) => check_emit(&pack, why, json),
    }
}

/// Print a check's answer and compute its exit: 3 exactly when a gating
/// section fired, the same predicate that withholds the refactor certificate.
fn check_emit(pack: &grid_wire::CheckResult, why: bool, json: bool) -> u8 {
    let code = if check::gated(pack) {
        exit::DIVERGENCE
    } else {
        exit::OK
    };
    if json {
        // A serialization failure is internal (1) and outranks the
        // verdict, exactly as `grid gate` treats it.
        return match emit_json(pack) {
            exit::OK => code,
            failure => failure,
        };
    }
    print!("{}", check::porcelain(pack, &namer(), why));
    if code == exit::DIVERGENCE {
        hint("grid: something this change did was not checked — the records above say what; `grid explain <word>` says why it gates");
    }
    code
}

/// The base a bare `grid check --compile` measures the worktree against:
/// `grid drift`'s default, so the changed set is the one drift would name.
const COMPILE_BASE: &str = "HEAD~1";

/// Run the compile, survey both endpoints, write the record. `Err` is the
/// exit code to leave with: cargo's own when the compile failed (grid then
/// prints nothing on stdout), `0` with a hint when there is no compile to
/// wrap, `1` when the survey or the record write failed.
fn compile_wrap(cargo_args: &[String]) -> Result<grid_wire::CompileRecord, u8> {
    if !root().join("Cargo.toml").exists() {
        hint("grid: nothing to compile — no Cargo.toml in this tree, and a Python tree has no compile to wrap");
        return Err(exit::OK);
    }
    let started = std::time::Instant::now();
    let mut chart = rpc::Chart::new(root());
    let head = chart
        .survey_run(&grid_wire::methods::SurveyRunParams {
            git_rev: None,
            worktree: None,
        })
        .and_then(|(snapshot, _)| {
            // Retained, not made live: the worktree just surveyed stays what
            // every later verb answers about.
            chart.ensure(&snapshot_or_rev(COMPILE_BASE))?;
            Ok(snapshot.snapshot_id)
        });
    let snapshot = match head {
        Ok(id) => id,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return Err(exit::INTERNAL);
        }
    };
    let surveyed = started.elapsed();
    let compiling = std::time::Instant::now();
    let wrapped = match compile::wrap(root(), cargo_args, snapshot) {
        Ok(w) => w,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return Err(exit::INTERNAL);
        }
    };
    match wrapped {
        compile::Wrapped::Failed(code) => Err(code),
        compile::Wrapped::Compiled(record) => {
            hint(&format!(
                "grid: recorded {} — survey {}ms, cargo check and record {}ms",
                compile::FILE,
                surveyed.as_millis(),
                compiling.elapsed().as_millis()
            ));
            Ok(record)
        }
    }
}

/// The compile report on its own: drift's default endpoints, the worktree
/// against [`COMPILE_BASE`]. Exit 3 when the compile provably skipped a
/// proven dependent of a changed interface.
fn compile_report(record: &grid_wire::CompileRecord, json: bool) -> u8 {
    let started = std::time::Instant::now();
    // Head is the snapshot the record names — the one just surveyed — so
    // resolving it costs no git spawn; base is the rev it is measured from.
    let head = grid_wire::SnapshotOrRev::Snapshot {
        snapshot: record.snapshot.clone(),
    };
    let report = compile::report(root(), &snapshot_or_rev(COMPILE_BASE), &head, record);
    match report {
        Err(e @ drift::Error::BudgetRejected(_)) => {
            hint(&format!("grid: {e}"));
            exit::INTERNAL
        }
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(report) => {
            hint(&format!(
                "grid: compile check {}ms over cargo check",
                started.elapsed().as_millis()
            ));
            let code = if report.uncompiled.is_empty() {
                exit::OK
            } else {
                exit::DIVERGENCE
            };
            if json {
                return match emit_json(&report) {
                    exit::OK => code,
                    failure => failure,
                };
            }
            print!("{}", porcelain::compile(&report));
            if code == exit::DIVERGENCE {
                hint("grid: these proven callers sit under a cfg the compile excluded — re-run with `-- --tests` or the feature, or fix them");
            }
            code
        }
    }
}

const SUSPECTS_USAGE: &str =
    "usage: grid suspects <test> [<trace>] [<base>] [--closure <snapshot|rev>]";

const INTERFERE_USAGE: &str = "usage: grid interfere <other-root> [--base <rev>]";

/// `grid interfere <other-root> [--base <rev>]` — the semantic overlap
/// between two worktrees of one repository.
///
/// Exit 3 when a finding gates, 0 when there is none, and 2 whenever either
/// store could not be read — silence about a chart grid could not open would
/// read as "no interference", which is the opposite claim.
fn interfere_verb(args: &[&str], json: bool) -> u8 {
    let mut base = None;
    let mut positional = Vec::new();
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match *arg {
            "--base" => {
                let (Some(rev), Some(tail)) = (rest.first(), rest.get(1..)) else {
                    hint(INTERFERE_USAGE);
                    return exit::BAD_INPUT;
                };
                base = Some((*rev).to_owned());
                rest = tail;
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{INTERFERE_USAGE}"));
                return exit::BAD_INPUT;
            }
            other => positional.push(other),
        }
    }
    let ([other], true) = (positional.as_slice(), positional.len() == 1) else {
        hint(INTERFERE_USAGE);
        return exit::BAD_INPUT;
    };
    let params = grid_wire::methods::InterfereParams {
        other_root: (*other).to_owned(),
        base: base.map(grid_wire::GitRev),
    };
    match rpc::interfere(&params, root()) {
        Err(error) => {
            hint(&format!("grid: {error}"));
            match error {
                interfere::Error::Drift(_) => exit::INTERNAL,
                interfere::Error::Unreadable(_) | interfere::Error::Git(_) => exit::BAD_INPUT,
            }
        }
        Ok(report) => {
            let code = if report.suspects.is_empty() {
                exit::OK
            } else {
                exit::DIVERGENCE
            };
            if json {
                return match emit_json(&report) {
                    exit::OK => code,
                    failure => failure,
                };
            }
            print!("{}", porcelain::interfere(&report));
            if code == exit::DIVERGENCE {
                hint(
                    "grid: the other worktree edited something these depend on — \
                     rebase or talk before either of you merges",
                );
            }
            code
        }
    }
}

/// `grid suspects <test> [<trace>] [<base>] [--closure <since>]` — rank
/// changed definitions for the named test, in one of two evidence modes.
///
/// Without `--closure`, plain Ochiai over retained spectra, which refuses
/// without history. With it, the definitions the test's closure at head can
/// reach whose bodies moved since that snapshot, ranked by hop distance —
/// heuristic, so exit `0` on any answer, and no history required.
fn suspects(args: &[&str], json: bool) -> u8 {
    let mut closure = None;
    let mut positional = Vec::new();
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        match *arg {
            "--closure" => {
                let (Some(spec), Some(tail)) = (rest.first(), rest.get(1..)) else {
                    hint(SUSPECTS_USAGE);
                    return exit::BAD_INPUT;
                };
                closure = Some(snapshot_or_rev(spec));
                rest = tail;
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{SUSPECTS_USAGE}"));
                return exit::BAD_INPUT;
            }
            other => positional.push(other),
        }
    }
    let (Some(test), true) = (positional.first(), positional.len() <= 3) else {
        hint(SUSPECTS_USAGE);
        return exit::BAD_INPUT;
    };
    let params = grid_wire::methods::SuspectsParams {
        test: (*test).to_owned(),
        trace: positional
            .get(1)
            .map(|trace| grid_wire::TraceId((*trace).to_owned())),
        base: positional.get(2).map(|base| snapshot_or_rev(base)),
        closure_since: closure.clone(),
    };
    match closure {
        Some(since) => suspects_closure(&params, &since, json),
        None => match suspects::rank(&params, root()) {
            Err(error) => suspects_refused(&error),
            Ok(result) => {
                if json {
                    return emit_json(&result);
                }
                print!("{}", porcelain::suspects(&result));
                exit::OK
            }
        },
    }
}

/// The closure evidence mode's output: the ranking, the fork when two
/// retained traces straddle `since`, and the closing `miss` that says how
/// much of the closure the chart could not follow.
fn suspects_closure(
    params: &grid_wire::methods::SuspectsParams,
    since: &grid_wire::SnapshotOrRev,
    json: bool,
) -> u8 {
    match suspects::rank_closure(params, since, root()) {
        Err(error) => suspects_refused(&error),
        Ok(found) => {
            if json {
                return emit_json(&found.suspects);
            }
            print!("{}", porcelain::suspects(&found.suspects));
            if let Some(diff) = &found.fork {
                print!("{}", porcelain::fork(diff));
            }
            // Unknown, never zero: the declined split is a property of a
            // survey *run* and no survey ran here — this verb answers from
            // the published snapshot, which is what keeps it inside its
            // millisecond budget.
            println!("miss    ?  references declined uncounted (no survey ran in this call)");
            exit::OK
        }
    }
}

/// One refusal table for both modes.
fn suspects_refused(error: &suspects::Error) -> u8 {
    hint(&format!("grid: {error}"));
    match error {
        suspects::Error::UnknownTrace(_) => exit::STALE,
        suspects::Error::InsufficientHistory(_)
        | suspects::Error::UnknownTest(_)
        | suspects::Error::UnknownSince(_) => exit::BAD_INPUT,
        suspects::Error::ClosureRejected(_) | suspects::Error::Chart(_) => exit::INTERNAL,
    }
}

/// `grid trace ingest <trace.json|->` and `grid trace diff <green> <red>`.
///
/// The CI shape: a test runner emits a trace JSON, `ingest` resolves and
/// retains it, and `diff` compares two retained runs — exit 3 when behavior
/// forked, because a forked trace is a divergence. Both call the same rpc
/// functions the daemon serves, so CLI and socket cannot disagree.
fn trace(args: &[&str], json: bool) -> u8 {
    let root = std::path::Path::new(".");
    match args.split_first() {
        Some((&"ingest", rest)) if matches!(rest.first(), Some(&"--lcov" | &"--coverage-json")) => {
            trace_coverage(rest[0], rest, json, root)
        }
        Some((&"ingest", rest)) => {
            let Some(path) = rest.first() else {
                hint("usage: grid trace ingest <trace.json|->");
                return exit::BAD_INPUT;
            };
            let text = if *path == "-" {
                use std::io::Read as _;
                let mut buf = String::new();
                match std::io::stdin().read_to_string(&mut buf) {
                    Ok(_) => buf,
                    Err(e) => {
                        hint(&format!("grid: cannot read stdin: {e}"));
                        return exit::INTERNAL;
                    }
                }
            } else {
                match std::fs::read_to_string(path) {
                    Ok(text) => text,
                    Err(e) => {
                        hint(&format!("grid: cannot read {path}: {e}"));
                        return exit::BAD_INPUT;
                    }
                }
            };
            let w: grid_wire::Trace = match serde_json::from_str(&text) {
                Ok(w) => w,
                Err(e) => {
                    hint(&format!("grid: trace JSON: {e}"));
                    return exit::BAD_INPUT;
                }
            };
            let total = w.observations.len() as u64;
            let params = grid_wire::methods::TraceIngestParams { trace: w };
            match rpc::trace_ingest(&params, root) {
                Ok(result) => {
                    if json {
                        return emit_json(&result);
                    }
                    print!("{}", porcelain::trace_ingest(&result, total));
                    exit::OK
                }
                Err(e) => {
                    hint(&format!("grid: {e}"));
                    exit::INTERNAL
                }
            }
        }
        Some((&"diff", rest)) => {
            let (Some(green), Some(red)) = (rest.first(), rest.get(1)) else {
                hint("usage: grid trace diff <green-trace-id> <red-trace-id>");
                return exit::BAD_INPUT;
            };
            let params = grid_wire::methods::TraceDiffParams {
                green: grid_wire::TraceId((*green).to_string()),
                red: grid_wire::TraceId((*red).to_string()),
            };
            match rpc::trace_diff(&params, root) {
                Ok(diff) => {
                    if json {
                        return emit_json(&diff);
                    }
                    print!("{}", porcelain::trace_diff(&diff));
                    let forked = diff.first_divergence.is_some()
                        || !diff.added.is_empty()
                        || !diff.removed.is_empty();
                    if forked {
                        exit::DIVERGENCE
                    } else {
                        exit::OK
                    }
                }
                Err(rpc::TraceDiffError::UnknownTrace(e)) => {
                    hint(&format!("grid: {e}"));
                    exit::BAD_INPUT
                }
                Err(rpc::TraceDiffError::Store(e)) => {
                    hint(&format!("grid: {e}"));
                    exit::INTERNAL
                }
            }
        }
        _ => {
            hint("usage: grid trace ingest|diff ...");
            exit::BAD_INPUT
        }
    }
}
