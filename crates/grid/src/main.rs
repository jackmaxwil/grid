//! grid — CLI + daemon (single binary: survey, query, gate, serve).
//!
//! Wave 3 ships stage 1: `grid gate`, `grid track new|show|lint`, `grid
//! version`, and `grid serve` — a unix-socket, newline-framed JSON-RPC daemon
//! serving `grid.version`, `grid.track.lint` and `grid.gate.check`.
//!
//! Two output modes, one vocabulary: porcelain records (`docs/PORCELAIN.md`)
//! by default, the wire result verbatim under `--json`. stdout carries
//! records; stderr carries hints and the first-run banner (`GRID_HINTS=0`
//! disables). Exit codes are uniform across verbs: 0 pass, 1 internal error,
//! 2 bad input, 3 divergence, 4 stale coordinate.

mod porcelain;
mod provider;
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
        // Bare `grid` and `grid help` are the same teaching output: one
        // memorized fact (`grid`) has to be enough (CLI plan §1).
        None | Some((&"help" | &"--help" | &"-h", _)) => {
            banner();
            exit::OK
        }
        Some((&"version", _)) => version(json),
        Some((&"gate", args)) => gate(args, json),
        Some((&"track", args)) => track::main(args, json),
        Some((&"cache", args)) => cache(args, json),
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
    let result = rpc::version();
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
            print!("{}", porcelain::verdict(&verdict));
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
