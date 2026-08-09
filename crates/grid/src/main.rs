//! grid — CLI + daemon (single binary: survey, query, gate, serve).
//!
//! Stage 2: the verbs above, plus `grid survey`, `grid diff` and `grid
//! resolve`, over a daemon serving the real chart — `grid.snapshot.current`,
//! `grid.survey.run`, `grid.delta`, `grid.resolve`, `grid.track.lint` and
//! `grid.gate.check`. Bare `grid` is the everyday verb: what changed, and does
//! it match the promise.
//!
//! Two output modes, one vocabulary: porcelain records (`docs/PORCELAIN.md`)
//! by default, the wire result verbatim under `--json`. stdout carries
//! records; stderr carries hints and the first-run banner (`GRID_HINTS=0`
//! disables). Exit codes are uniform across verbs: 0 pass, 1 internal error,
//! 2 bad input, 3 divergence, 4 stale coordinate.

mod chart_provider;
mod porcelain;
mod rpc;
mod serve;
mod standing;
mod track;

use std::io::Write as _;
use std::process::ExitCode;

/// Exit codes, uniform across verbs (`docs/PORCELAIN.md`).
mod exit {
    pub const OK: u8 = 0;
    pub const INTERNAL: u8 = 1;
    pub const BAD_INPUT: u8 = 2;
    pub const DIVERGENCE: u8 = 3;
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
            exit::OK
        }
        Some((&"version", _)) => version(json),
        Some((&"survey", args)) => survey(args, json),
        Some((&"diff", args)) => diff(args, json),
        Some((&"resolve", args)) => resolve(args, json),
        Some((&"gate", args)) => gate(args, json),
        Some((&"track", args)) => track::main(args, json),
        Some((&"cache", args)) => cache(args, json),
        Some((&"wake", args)) => wake(args, json),
        Some((&"serve", args)) => serve::main(args),
        Some((other, _)) => {
            hint(&format!(
                "grid: unknown verb `{other}`\nhint: run `grid` for the three commands"
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
    let head = grid_wire::SnapshotOrRev::GitRev {
        git_rev: grid_wire::GitRev((*tail.first().unwrap_or(&"HEAD")).to_string()),
    };

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
            let Some((kind, env, entries)) = cache_args(rest) else {
                hint("usage: grid cache record <lint|typecheck|test> <env-hash> <datum>=<pass|fail>...");
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
                wake_id: None,
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
        Ok((snapshot, reread)) => {
            if json {
                return emit_json(&snapshot);
            }
            print!(
                "{}",
                porcelain::snapshot(&snapshot, Some((reread, started.elapsed())))
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

/// `grid resolve <name>` — the definitions a name refers to.
///
/// Ambiguity is not an error here: every candidate is printed, because the
/// list is what makes the next command typeable.
fn resolve(args: &[&str], json: bool) -> u8 {
    let Some(name) = args.first() else {
        hint("usage: grid resolve <callsign>");
        return exit::BAD_INPUT;
    };
    let params = grid_wire::methods::ResolveParams {
        locator: grid_wire::Locator::Callsign {
            callsign: grid_wire::Callsign((*name).to_string()),
        },
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
            // An empty result is exit 0 with no records: "nothing is called
            // that" is an answer (PORCELAIN.md).
            print!("{}", porcelain::refs(&defs));
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
