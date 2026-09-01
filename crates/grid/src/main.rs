//! grid — CLI + daemon (single binary: survey, query, gate, serve).
//!
//! Stage 2: the verbs above, plus `grid survey`, `grid diff` and `grid
//! resolve`, over a daemon serving the real chart — `grid.snapshot.current`,
//! `grid.survey.run`, `grid.delta`, `grid.resolve`, `grid.track.lint` and
//! `grid.gate.check`. Bare `grid` is the everyday verb: what changed, and does
//! it match the promise.
//!
//! Wave 6 adds the stage-1/2 agent verbs: `grid scope`, `grid todo` (a view
//! over the gate verdict, not a second delta engine), `grid edit apply` (the
//! CAS write path, D6's second writer), `grid uses` (proven inbound edges)
//! and `grid explain` (help generated from the wire contract and
//! `docs/ONTOLOGY.md` at build time).
//!
//! Wave 9 adds `grid review`: the gate verdict plus the paths that made each
//! promise good, the surplus it never promised, and what changed without
//! being run. It holds — exit 3 — exactly when the gate holds.
//!
//! Two output modes, one vocabulary: porcelain records (`docs/PORCELAIN.md`)
//! by default, the wire result verbatim under `--json`. stdout carries
//! records; stderr carries hints and the first-run banner (`GRID_HINTS=0`
//! disables). Exit codes are uniform across verbs: 0 pass, 1 internal error,
//! 2 bad input, 3 divergence, 4 stale coordinate.

mod bisect;
mod chart_provider;
mod covenant;
mod drift;
mod edit;
mod explain;
mod lcov;
mod legs;
mod porcelain;
mod review;
mod rpc;
mod serve;
mod stale;
mod standing;
mod suspects;
mod track;

use std::io::Write as _;
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
const WIRE_VERSION: &str = "0.1.0";
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
         bind <A> <B> --via <word>    declare a coupling no language establishes\n  \
         resolve <name|file|f:line>   where it is; a bare file lists its defs\n  \
         uses <name> [--relation r]   proven inbound edges\n  \
         scope <pattern> [--depth N]  context pack around an edit\n  \
         orphans [--under p]          no proven caller (not the same as dead)\n  \
         hotspots [--under p]         complexity x churn, descending\n  \
         roots                        import name -> directory map\n  \
         log <name>                   a definition's history in identity space\n  \
         bisect <test>                first failing patch in the chain\n  \
         suspects <test>              likely culprits, ranked\n  \
         gate <track> [head]          promise vs delta; exit 3 on hold\n  \
         track lint <file>            validate a promise file\n  \
         todo <track>                 the promise as a worklist\n  \
         review <track> [head]        gate verdict with its why\n  \
         cache check|record ...       test verdicts keyed by identity closure\n  \
         edit apply <file>            compare-and-swap edits by datum\n  \
         wake ingest|diff ...         runtime traces -> observed edges\n  \
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
        Some((&"bind", args)) => bind(args, json),
        Some((&"resolve", args)) => resolve(args, json),
        Some((&"gate", args)) => gate(args, json),
        Some((&"track", args)) => track::main(args, json),
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
        Some((&"review", args)) => review(args, json),
        Some((&"explain", args)) => explain::main(args),
        Some((&"suspects", args)) => suspects(args, json),
        Some((&"wake", args)) => wake(args, json),
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
/// is a git rev. Track bases and gate heads both accept both.
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

/// `grid gate <track-file|track-id> [<head-rev>]` — check a track against the
/// delta at head. Exit 3 on hold: the gate verdict *is* the exit code, so CI
/// needs no output parsing.
fn gate(args: &[&str], json: bool) -> u8 {
    let Some((spec, tail)) = args.split_first() else {
        hint("usage: grid gate <track-file> [<head-rev>]");
        return exit::BAD_INPUT;
    };
    let track = match track::load(spec) {
        Ok(t) => t,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::BAD_INPUT;
        }
    };
    let head = snapshot_or_rev(tail.first().unwrap_or(&"HEAD"));

    let params = grid_wire::methods::GateCheckParams { track, head };
    match rpc::gate_check(&params, std::path::Path::new(".")) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(verdict) => {
            let code = if verdict.gate == grid_wire::XteVerdictGate::Hold {
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

/// `grid cache check <kind> <env-hash> <datum>...` and
/// `grid cache record <kind> <env-hash> <datum>=<pass|fail>...`.
///
/// The CI shape: `check` prints `miss` records for what must run, `record`
/// stores the verdicts afterwards. Both call the same rpc functions the daemon
/// serves, so a CI wired through the CLI and one wired through the socket
/// cannot disagree.
fn cache(args: &[&str], json: bool) -> u8 {
    let root = std::path::Path::new(".");
    match args.split_first() {
        Some((&"check", rest)) => {
            let Some((kind, env, datums)) = cache_args(rest) else {
                hint("usage: grid cache check <lint|typecheck|test> <env-hash> <datum>...");
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
            let Some((wake_id, positional)) = wake_flag(rest) else {
                return exit::BAD_INPUT;
            };
            let Some((kind, env, entries)) = cache_args(&positional) else {
                hint("usage: grid cache record <lint|typecheck|test> <env-hash> <datum>=<pass|fail>... [--wake <id>]");
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
                wake_id,
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
            hint("usage: grid cache check|record <kind> <env-hash> <datum>...");
            exit::BAD_INPUT
        }
    }
}

/// `<kind> <env-hash> <rest>...`, or nothing if the shape is wrong.
/// Split a trailing `--wake <id>` off an argument list. `--wake` ties a
/// verdict to a retained wake, which is what lets `grid suspects` use it as
/// a spectrum. `None` prints its own hint.
fn wake_flag<'a>(args: &'a [&'a str]) -> Option<(Option<String>, Vec<&'a str>)> {
    let mut wake_id = None;
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--wake" {
            let Some(id) = it.next() else {
                hint("usage: grid cache record <kind> <env-hash> <datum>=<pass|fail>... [--wake <id>]");
                return None;
            };
            wake_id = Some((*id).to_owned());
        } else {
            positional.push(*arg);
        }
    }
    Some((wake_id, positional))
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
            .and_then(|c| grid_xte::DeltaProvider::callsign(c, d))
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
        Ok(made_good) => {
            if json {
                return emit_json(&made_good);
            }
            // Edges print in callsign space, so the renderer needs the chart
            // to name datums with.
            print!("{}", porcelain::diff(&made_good, &namer()));
            exit::OK
        }
    }
}

/// `grid drift [<base>] [<head>] [--floor <zoc>] [--track <file>]` — the
/// dependents of every changed interface that did not themselves change.
///
/// Exit 3 when suspects exist: an unaddressed dependent of a changed
/// interface is a divergence, and CI needs no output parsing to see it. Exit
/// 1 when an inbound walk ran past its fuel — the answer is refused whole,
/// because a truncated walk reports fewer suspects and looks cleaner for it.
fn drift_verb(args: &[&str], json: bool) -> u8 {
    let mut floor = None;
    let mut track = None;
    let mut tests = false;
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
                let Some(want) = zoc_named(spec) else {
                    hint(DRIFT_USAGE);
                    return exit::BAD_INPUT;
                };
                floor = Some(want);
                rest = tail;
            }
            "--track" => {
                let (Some(spec), Some(tail)) = (rest.first(), rest.get(1..)) else {
                    hint(DRIFT_USAGE);
                    return exit::BAD_INPUT;
                };
                match track::load(spec) {
                    Ok(t) => track = Some(t),
                    Err(e) => {
                        hint(&format!("grid: {e}"));
                        return exit::BAD_INPUT;
                    }
                }
                rest = tail;
            }
            "--tests" => tests = true,
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
        track,
        tests: tests.then_some(true),
    };
    match rpc::drift(&params, root()) {
        Err(
            e @ (drift::Error::BudgetRejected(_) | drift::Error::Wake(_) | drift::Error::Stale(_)),
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
            let code = if report.suspects.is_empty()
                && report.stale_tests.iter().flatten().next().is_none()
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
                     update them, or promise them in a track and re-run with --track",
                );
            }
            code
        }
    }
}

const DRIFT_USAGE: &str = "usage: grid drift [<base>] [<head>] \
     [--floor <proven|observed|modeled|heuristic>] [--track <file>] [--tests]";

/// The wire spelling of an evidence grade, for `--floor` arguments. The serde
/// names are the vocabulary, so the flag cannot drift from the contract.
fn zoc_named(name: &str) -> Option<grid_wire::Zoc> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

const BIND_USAGE: &str = "usage: grid bind <A> <B> --via <word> [--note <text>] | grid bind --list";

/// `grid bind <A> <B> --via <word> [--note <text>]` — declare a coupling no
/// language's binding rules establish, so drift can see it.
///
/// Both patterns are resolved against the current snapshot and a pattern
/// matching nothing is refused: a declaration about code that does not exist
/// is a rule that will never fire, and writing it down silently is worse than
/// not writing it. Removing a covenant is editing the file — it is a
/// declaration, like a track.
fn bind(args: &[&str], json: bool) -> u8 {
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
                    hint(BIND_USAGE);
                    return exit::BAD_INPUT;
                }
            }
            "--note" => {
                if !flag(&mut note) {
                    hint(BIND_USAGE);
                    return exit::BAD_INPUT;
                }
            }
            other if other.starts_with("--") => {
                hint(&format!("grid: unknown flag `{other}`\n{BIND_USAGE}"));
                return exit::BAD_INPUT;
            }
            other => positional.push(other),
        }
    }
    let ([a, b], Some(via)) = (positional.as_slice(), via) else {
        hint(BIND_USAGE);
        return exit::BAD_INPUT;
    };
    if via.is_empty() {
        hint(BIND_USAGE);
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

/// `grid scope <callsign-pattern>... [--depth N]` — the context pack around
/// an intended edit: what matches, what touches it, what rules reach it.
/// Bodies are deliberately absent; the agent pulls those itself.
fn scope(args: &[&str], json: bool) -> u8 {
    let mut patterns = Vec::new();
    let mut depth = None;
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        if *arg == "--depth" {
            let Some(n) = tail.first().and_then(|n| n.parse::<u64>().ok()) else {
                hint("usage: grid scope <pattern>... [--depth N]");
                return exit::BAD_INPUT;
            };
            depth = Some(n);
            rest = &tail[1..];
            continue;
        }
        patterns.push(grid_wire::CallsignPattern((*arg).to_string()));
        rest = tail;
    }
    if patterns.is_empty() {
        hint("usage: grid scope <pattern>... [--depth N]");
        return exit::BAD_INPUT;
    }
    let params = grid_wire::methods::ScopeParams {
        callsigns: patterns,
        depth,
        snapshot: None,
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
            print!("{}", porcelain::scope(&pack, &namer()));
            exit::OK
        }
    }
}

/// `grid todo <track> [<head-rev|snap:id>]` — the gate verdict as a
/// worklist. Always exit 0 when it answers: remaining work is a report, not
/// a failure — the gate is the verb that holds.
fn todo(args: &[&str], json: bool) -> u8 {
    let Some((id, tail)) = args.split_first() else {
        hint("usage: grid todo <track> [<head-rev>]");
        return exit::BAD_INPUT;
    };
    let head = tail.first().map(|h| snapshot_or_rev(h));
    let params = grid_wire::methods::TodoParams {
        track: grid_wire::TrackId((*id).to_string()),
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

/// `grid wake ingest --lcov <report> --test <name>` — coverage as a reach
/// set: every executed line inside a charted definition becomes one
/// observation from the named test.
fn wake_lcov(rest: &[&str], json: bool, root: &std::path::Path) -> u8 {
    let (Some(path), Some(&"--test"), Some(test)) = (rest.get(1), rest.get(2), rest.get(3)) else {
        hint("usage: grid wake ingest --lcov <report|-> --test <name>");
        return exit::BAD_INPUT;
    };
    let Some(report) = read_arg(path) else {
        return exit::BAD_INPUT;
    };
    let mapped = match lcov::wake_of(root, &report, test) {
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
    let total = mapped.wake.observations.len() as u64;
    let params = grid_wire::methods::WakeIngestParams { wake: mapped.wake };
    match rpc::wake_ingest(&params, root) {
        Ok(result) => {
            if json {
                return emit_json(&result);
            }
            print!("{}", porcelain::wake_ingest(&result, total));
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

/// Every landed modification becomes a leg in the patch log. Best effort:
/// an unwritable log costs history, never the edit that already landed.
fn record_edit_legs(outcome: &edit::Outcome) {
    let root = root();
    let rev = chart_provider::resolve_rev(root, "HEAD");
    let rows: Vec<grid_wire::Leg> = outcome
        .edits
        .iter()
        .filter_map(|detail| match detail {
            edit::EditDetail::Modified { was, now, .. } => Some(grid_wire::Leg {
                from: was.clone(),
                to: now.clone(),
                git_rev: Some(grid_wire::GitRev(rev.clone())),
                at: None,
            }),
            _ => None,
        })
        .collect();
    if let Err(e) = legs::append(root, &rows) {
        hint(&format!("grid: patch log not updated: {e}"));
    }
}

/// `grid log <name>` — a datum's history in identity space: every leg that
/// chains to it, newest first. A rename does not break the chain, because
/// legs chain datums, not names.
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
    let rows = legs::history(root(), &def.datum.0);
    if json {
        return emit_json(&rows);
    }
    if rows.is_empty() {
        hint(&format!(
            "grid: no recorded history for `{name}` — legs accrue from `grid edit apply` and surveys"
        ));
    }
    print!("{}", porcelain::legs(&rows));
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
                    "grid: churn seeded from git file history (file-level) — grid legs accrue from edits and surveys",
                ),
                rpc::ChurnSource::None => hint(
                    "grid: no history yet — ranking by complexity alone",
                ),
                rpc::ChurnSource::Legs => {}
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

/// `grid orphans [--lang <py|rs|ts|js>] [--under <path>]` — definitions
/// with no proven inbound edge. The header states the evidence honestly:
/// *no proven
/// caller* — public API, tests, and macro-reached code all look like this,
/// and none of them is dead.
fn orphans(args: &[&str], json: bool) -> u8 {
    let mut lang = None;
    let mut under = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if *arg == "--lang" {
            let Some(want) = it.next().and_then(|w| {
                serde_json::from_value(serde_json::Value::String((*w).to_owned())).ok()
            }) else {
                hint("usage: grid orphans [--lang <py|rs|ts|js>] [--under <path>]");
                return exit::BAD_INPUT;
            };
            lang = Some(want);
        }
        if *arg == "--under" {
            let Some(path) = it.next() else {
                hint("usage: grid orphans [--lang <py|rs|ts|js>] [--under <path>]");
                return exit::BAD_INPUT;
            };
            under = Some(*path);
        }
    }
    match rpc::orphans(root(), lang, under) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(rows) => {
            if json {
                return emit_json(&rows);
            }
            hint("grid: no proven caller — not the same as dead (public API, tests, macro-reached code all match)");
            print!("{}", porcelain::refs_as(&rows, "orphan"));
            exit::OK
        }
    }
}

/// `grid review <track> [<head-rev|snap:id>] [--wake <id>]` — the reviewer's
/// pack: the gate verdict, then *why* — a path per fulfilling promise, the
/// surplus it did not promise, and what changed without being run.
///
/// The verdict is the gate's, so the exit code is the gate's too: hold is 3,
/// divergence. A reviewer and CI reading the same pack cannot disagree about
/// whether the promise was kept.
fn review(args: &[&str], json: bool) -> u8 {
    let mut track = None;
    let mut head = None;
    let mut wake = None;
    let mut rest = args;
    while let Some((arg, tail)) = rest.split_first() {
        rest = tail;
        if *arg == "--wake" {
            let Some((id, tail)) = rest.split_first() else {
                hint("usage: grid review <track> [<head-rev>] [--wake <id>]");
                return exit::BAD_INPUT;
            };
            wake = Some(grid_wire::WakeId((*id).to_string()));
            rest = tail;
        } else if track.is_none() {
            track = Some(grid_wire::TrackId((*arg).to_string()));
        } else if head.is_none() {
            head = Some(snapshot_or_rev(arg));
        } else {
            hint(&format!("grid: unexpected argument `{arg}`"));
            return exit::BAD_INPUT;
        }
    }
    let Some(track) = track else {
        hint("usage: grid review <track> [<head-rev>] [--wake <id>]");
        return exit::BAD_INPUT;
    };
    let params = grid_wire::methods::ReviewPackParams {
        track,
        head: head.unwrap_or_else(|| snapshot_or_rev("HEAD")),
        wake,
    };
    match review::pack(&params, root()) {
        Err(e) => {
            hint(&format!("grid: {e}"));
            exit::BAD_INPUT
        }
        Ok(pack) => {
            let code = if pack.verdict.gate == grid_wire::XteVerdictGate::Hold {
                exit::DIVERGENCE
            } else {
                exit::OK
            };
            if json {
                // A serialization failure is internal (1) and outranks the
                // verdict, exactly as `grid gate` treats it.
                return match emit_json(&pack) {
                    exit::OK => code,
                    failure => failure,
                };
            }
            print!("{}", review::porcelain(&pack, &namer()));
            code
        }
    }
}

/// `grid suspects <test> [<wake>] [<base>]` — rank changed definitions seen
/// in retained spectra for the named test.
fn suspects(args: &[&str], json: bool) -> u8 {
    let Some(test) = args.first() else {
        hint("usage: grid suspects <test> [<wake>] [<base>]");
        return exit::BAD_INPUT;
    };
    if args.len() > 3 {
        hint("usage: grid suspects <test> [<wake>] [<base>]");
        return exit::BAD_INPUT;
    }
    let params = grid_wire::methods::SuspectsParams {
        test: (*test).to_owned(),
        wake: args
            .get(1)
            .map(|wake| grid_wire::WakeId((*wake).to_owned())),
        base: args.get(2).map(|base| snapshot_or_rev(base)),
    };
    match suspects::rank(&params, root()) {
        Err(error) => {
            hint(&format!("grid: {error}"));
            match error {
                suspects::Error::UnknownWake(_) => exit::STALE,
                suspects::Error::InsufficientHistory(_) => exit::BAD_INPUT,
                suspects::Error::Chart(_) => exit::INTERNAL,
            }
        }
        Ok(result) => {
            if json {
                return emit_json(&result);
            }
            for item in result.ranked {
                println!("hit     {} {:.6} {}", item.datum.0, item.score, item.via);
            }
            exit::OK
        }
    }
}

/// `grid wake ingest <wake.json|->` and `grid wake diff <green> <red>`.
///
/// The CI shape: a test runner emits a wake JSON, `ingest` resolves and
/// retains it, and `diff` compares two retained runs — exit 3 when behavior
/// forked, because a forked trace is a divergence. Both call the same rpc
/// functions the daemon serves, so CLI and socket cannot disagree.
fn wake(args: &[&str], json: bool) -> u8 {
    let root = std::path::Path::new(".");
    match args.split_first() {
        Some((&"ingest", rest)) if rest.first() == Some(&"--lcov") => wake_lcov(rest, json, root),
        Some((&"ingest", rest)) => {
            let Some(path) = rest.first() else {
                hint("usage: grid wake ingest <wake.json|->");
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
            let w: grid_wire::Wake = match serde_json::from_str(&text) {
                Ok(w) => w,
                Err(e) => {
                    hint(&format!("grid: wake JSON: {e}"));
                    return exit::BAD_INPUT;
                }
            };
            let total = w.observations.len() as u64;
            let params = grid_wire::methods::WakeIngestParams { wake: w };
            match rpc::wake_ingest(&params, root) {
                Ok(result) => {
                    if json {
                        return emit_json(&result);
                    }
                    print!("{}", porcelain::wake_ingest(&result, total));
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
                hint("usage: grid wake diff <green-wake-id> <red-wake-id>");
                return exit::BAD_INPUT;
            };
            let params = grid_wire::methods::WakeDiffParams {
                green: grid_wire::WakeId((*green).to_string()),
                red: grid_wire::WakeId((*red).to_string()),
            };
            match rpc::wake_diff(&params, root) {
                Ok(diff) => {
                    if json {
                        return emit_json(&diff);
                    }
                    print!("{}", porcelain::wake_diff(&diff));
                    let forked = diff.first_divergence.is_some()
                        || !diff.added.is_empty()
                        || !diff.removed.is_empty();
                    if forked {
                        exit::DIVERGENCE
                    } else {
                        exit::OK
                    }
                }
                Err(rpc::WakeDiffError::UnknownWake(e)) => {
                    hint(&format!("grid: {e}"));
                    exit::BAD_INPUT
                }
                Err(rpc::WakeDiffError::Store(e)) => {
                    hint(&format!("grid: {e}"));
                    exit::INTERNAL
                }
            }
        }
        _ => {
            hint("usage: grid wake ingest|diff ...");
            exit::BAD_INPUT
        }
    }
}
