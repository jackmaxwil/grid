//! The daemon: unix socket, newline-framed JSON-RPC 2.0.
//!
//! One request per line, one response per line — the framing the wire contract
//! names (`x-transport`). Connections are served one at a time: Wave 3 has no
//! shared mutable state to contend over, and a serial loop cannot interleave
//! writes. Concurrency arrives with the chart it would protect.
//!
//! Errors use the eight wire codes (`grid_wire::errors::WireError`) plus the
//! standard JSON-RPC set; an unknown method is -32601, never a crash.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use grid_wire::errors::WireError;
use serde_json::{json, Value};

use crate::{exit, hint, rpc};

/// Default socket location, relative to the repo root. `GRID_SOCKET`
/// overrides it (wire contract, `x-transport.path_env`).
const SOCKET: &str = ".grid/grid.sock";

fn default_socket() -> PathBuf {
    std::env::var_os("GRID_SOCKET").map_or_else(|| PathBuf::from(SOCKET), PathBuf::from)
}

/// `grid serve [<socket-path>] [--once]`.
pub fn main(args: &[&str]) -> u8 {
    let once = args.contains(&"--once");
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map_or_else(default_socket, PathBuf::from);

    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                hint(&format!("grid: cannot create {}: {e}", dir.display()));
                return exit::INTERNAL;
            }
        }
    }
    // A socket file left by a dead daemon would make bind fail forever.
    // Removing it is safe: bind below is what proves nobody is listening.
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            hint(&format!("grid: cannot bind {}: {e}", path.display()));
            return exit::INTERNAL;
        }
    };
    hint(&format!("grid: serving on {}", path.display()));

    let root = Path::new(".");
    // One chart for the daemon's life: the survey cache it carries is what
    // makes a warm survey warm, and it is per-process state by nature.
    let mut chart = rpc::Chart::new(root);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = serve_conn(stream, root, &mut chart) {
                    hint(&format!("grid: connection ended: {e}"));
                }
            }
            Err(e) => hint(&format!("grid: accept failed: {e}")),
        }
        if once {
            break;
        }
    }
    let _ = std::fs::remove_file(&path);
    exit::OK
}

fn serve_conn(stream: UnixStream, root: &Path, chart: &mut rpc::Chart) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    for line in BufReader::new(stream).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = handle(&line, root, chart);
        // A notification (no id) gets no response, per JSON-RPC 2.0.
        if let Some(response) = response {
            writeln!(out, "{response}")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// One request line in, one response line out (or none, for a notification).
#[must_use]
pub fn handle(line: &str, root: &Path, chart: &mut rpc::Chart) -> Option<String> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        // Parse error: id is null, because there is no id to read.
        Err(e) => return Some(error_line(&Value::Null, -32700, &e.to_string())),
    };
    let id = request.get("id").cloned();
    let response = dispatch(&request, root, chart);
    // Notifications are dispatched for their effect and answered with silence.
    let id = id?;
    Some(match response {
        Ok(result) => envelope(&id, json!({ "result": result })),
        Err((code, message)) => envelope(
            &id,
            json!({ "error": { "code": code, "message": message } }),
        ),
    })
}

fn envelope(id: &Value, body: Value) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("jsonrpc".into(), json!("2.0"));
    obj.insert("id".into(), id.clone());
    if let Value::Object(body) = body {
        obj.extend(body);
    }
    Value::Object(obj).to_string()
}

fn error_line(id: &Value, code: i32, message: &str) -> String {
    envelope(id, json!({ "error": { "code": code, "message": message } }))
}

/// Map a chart-provider failure onto the wire error code that describes it.
/// An unsurveyed rev and an unknown snapshot are the same class of thing to a
/// caller: the coordinate you named is not in the store.
fn chart_error(e: &crate::chart_provider::Error) -> (i32, String) {
    use crate::chart_provider::Error as E;
    let code = match e {
        E::NoSnapshotForRev(_) | E::Store(_) => WireError::UnknownSnapshot.code(),
        E::Survey(_) => WireError::SurveyFailed.code(),
        E::Config(_) => WireError::InvalidTrack.code(),
    };
    (code, e.to_string())
}

/// Route a request to its method. Errors are `(code, message)` pairs so the
/// transport layer decides framing and the method layer decides meaning.
fn dispatch(request: &Value, root: &Path, chart: &mut rpc::Chart) -> Result<Value, (i32, String)> {
    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err((-32600, "jsonrpc must be \"2.0\"".into()));
    }
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or((-32600, "missing method".into()))?;
    // Params are by-name across the whole contract; absent means empty.
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    let bad_params = |e: serde_json::Error| (-32602, e.to_string());
    let encode = |v: &dyn erased::Serialize| v.to_value();

    match method {
        "grid.version" => Ok(encode(&rpc::version(root))),
        "grid.track.lint" => {
            let params = serde_json::from_value(params).map_err(bad_params)?;
            let result =
                rpc::track_lint(&params, root).map_err(|e| (WireError::InvalidTrack.code(), e))?;
            Ok(encode(&result))
        }
        "grid.gate.check" => {
            let params = serde_json::from_value(params).map_err(bad_params)?;
            rpc::gate_check(&params, root)
                .map(|v| encode(&v))
                .map_err(|e| chart_error(&e))
        }
        "grid.snapshot.current" => rpc::snapshot_current(root)
            .map_err(|e| chart_error(&e))?
            // A repo that has never been surveyed has no current snapshot.
            // That is a state, not a protocol failure, but the method's result
            // is a Snapshot, so it is the contract's error rather than null.
            .map_or_else(
                || {
                    Err((
                        WireError::NoSnapshot.code(),
                        "no snapshot published: run `grid survey`".to_string(),
                    ))
                },
                |s| Ok(encode(&s)),
            ),
        "grid.survey.run" => {
            let params = serde_json::from_value(params).map_err(bad_params)?;
            // The daemon is the single writer (D6), and connections are served
            // one at a time, so this is the only place segments are written.
            chart
                .survey_run(&params)
                .map(|(s, _)| encode(&s))
                .map_err(|e| chart_error(&e))
        }
        "grid.delta" => {
            let params = serde_json::from_value(params).map_err(bad_params)?;
            rpc::delta(&params, root)
                .map(|d| encode(&d))
                .map_err(|e| chart_error(&e))
        }
        "grid.resolve" => {
            let params = serde_json::from_value(params).map_err(bad_params)?;
            rpc::resolve(&params, root)
                .map(|r| encode(&r))
                .map_err(|e| chart_error(&e))
        }
        other => Err((-32601, format!("no such method: {other}"))),
    }
}

/// Serializing a concrete result to `Value` without naming its type at every
/// call site. Serialization of a generated wire type cannot fail (no maps with
/// non-string keys, no custom impls), so the fallback is unreachable in
/// practice and null in principle.
mod erased {
    use serde_json::Value;

    pub trait Serialize {
        fn to_value(&self) -> Value;
    }

    impl<T: serde::Serialize> Serialize for T {
        fn to_value(&self) -> Value {
            serde_json::to_value(self).unwrap_or(Value::Null)
        }
    }
}
