//! C52: a response is matched to its request by id and never by arrival; a
//! truncated line, a foreign id, a double-bodied response and a foreign
//! major are each a typed refusal that poisons the connection; an unknown
//! error code keeps its integer and an aground walk is a result, not an
//! error. The daemon here is a fake: a listener on a thread answering from
//! a script, so every row of the fault table is one reply away.
// A test harness, not shipped code: outside a #[test] fn the workspace's
// panic deny does not exempt it, and an unmet fixture precondition should
// abort the run loudly rather than be threaded through a Result.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread::JoinHandle;

use grid_client::{Client, Error, Protocol, RequestId, RpcError};
use grid_wire::errors::WireError;
use grid_wire::methods::{QueryWalkParams, VersionParams};
use grid_wire::{WalkResultStatus, WIRE_VERSION};
use serde_json::{json, Value};

/// A daemon that answers from a script: one reply per request line, written
/// verbatim so a script controls the framing, then the socket is closed. An
/// empty reply reads the request and writes nothing, so the close lands after
/// the client's write and before its read — EOF at a line boundary, without
/// racing the close against the write. What it received comes back from the
/// join, so a test can assert what the client wrote and how many times.
struct Fake {
    dir: tempfile::TempDir,
    served: Option<JoinHandle<Vec<String>>>,
}

impl Fake {
    fn start(replies: Vec<Vec<u8>>) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let listener = UnixListener::bind(dir.path().join("fake.sock")).expect("bind");
        let served = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut out = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(stream);
            let mut received = Vec::new();
            for reply in replies {
                let mut line = String::new();
                if reader.read_line(&mut line).expect("read request") == 0 {
                    break;
                }
                received.push(line);
                out.write_all(&reply).expect("write reply");
                out.flush().expect("flush");
            }
            received
        });
        Self {
            dir,
            served: Some(served),
        }
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("fake.sock")
    }

    /// The request lines the fake read, once the client is gone.
    fn received(mut self) -> Vec<String> {
        self.served.take().unwrap().join().expect("fake thread")
    }
}

fn line(v: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(v).unwrap();
    bytes.push(b'\n');
    bytes
}

fn version(id: u64, wire: &str) -> Vec<u8> {
    line(&json!({"jsonrpc": "2.0", "id": id, "result": {"wire": wire, "engine": "fake"}}))
}

fn ok() -> Vec<u8> {
    version(1, WIRE_VERSION)
}

/// A connected client whose next call, id 2, gets `reply`.
fn after_handshake(reply: Vec<u8>) -> (Fake, Client) {
    let fake = Fake::start(vec![ok(), reply]);
    let client = Client::connect(fake.socket()).expect("handshake");
    (fake, client)
}

fn second_call_fails(reply: Vec<u8>) -> Error {
    let (_fake, mut client) = after_handshake(reply);
    client
        .call_raw("grid.version", &json!({}))
        .expect_err("the scripted reply is a fault")
}

fn protocol(reply: Vec<u8>) -> Protocol {
    match second_call_fails(reply) {
        Error::Protocol(p) => p,
        other => panic!("expected a protocol fault, got {other:?}"),
    }
}

// --- fault table, one row each -------------------------------------------

#[test]
fn eof_at_a_line_boundary_is_disconnected() {
    let fake = Fake::start(vec![ok(), Vec::new()]);
    let mut client = Client::connect(fake.socket()).unwrap();
    assert!(matches!(
        client.call_raw("grid.version", &json!({})),
        Err(Error::Disconnected)
    ));
    drop(client);
    assert_eq!(
        fake.received().len(),
        2,
        "the request was written; the answer was EOF"
    );
}

#[test]
fn eof_inside_a_line_is_truncated_and_never_parsed() {
    let mut partial = line(&json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
    partial.pop();
    assert_eq!(protocol(partial), Protocol::Truncated);
}

#[test]
fn a_foreign_id_is_a_fault_not_an_answer() {
    let reply = line(&json!({"jsonrpc": "2.0", "id": 99, "result": {}}));
    assert_eq!(
        protocol(reply),
        Protocol::ForeignId {
            expected: RequestId(2),
            found: json!(99)
        }
    );
}

#[test]
fn a_null_id_is_the_daemon_rejecting_our_line() {
    let reply = line(&json!({
        "jsonrpc": "2.0", "id": null,
        "error": {"code": -32700, "message": "parse error"}
    }));
    assert_eq!(protocol(reply), Protocol::NullId);
}

#[test]
fn both_result_and_error_is_a_fault() {
    let reply = line(&json!({
        "jsonrpc": "2.0", "id": 2, "result": {},
        "error": {"code": -32603, "message": "and also no"}
    }));
    assert_eq!(protocol(reply), Protocol::BothBodies);
}

#[test]
fn neither_result_nor_error_is_a_fault() {
    let reply = line(&json!({"jsonrpc": "2.0", "id": 2}));
    assert_eq!(protocol(reply), Protocol::NoBody);
}

#[test]
fn a_jsonrpc_other_than_2_0_is_a_fault() {
    let reply = line(&json!({"jsonrpc": "1.0", "id": 2, "result": {}}));
    assert_eq!(protocol(reply), Protocol::BadJsonrpc(Some(json!("1.0"))));
}

#[test]
fn a_non_object_is_a_fault() {
    assert_eq!(protocol(line(&json!([1, 2]))), Protocol::NotObject);
}

#[test]
fn non_utf8_is_a_fault() {
    assert_eq!(protocol(vec![0xff, 0xfe, b'\n']), Protocol::NotUtf8);
}

#[test]
fn non_json_is_a_fault() {
    assert_eq!(protocol(b"{{{\n".to_vec()), Protocol::NotJson);
}

#[test]
fn a_missing_id_is_a_fault() {
    let reply = line(&json!({"jsonrpc": "2.0", "result": {}}));
    assert_eq!(protocol(reply), Protocol::MissingId);
}

#[test]
fn a_malformed_error_object_is_a_fault() {
    let reply = line(&json!({"jsonrpc": "2.0", "id": 2, "error": "boom"}));
    assert_eq!(protocol(reply), Protocol::BadError);
}

#[test]
fn an_error_code_outside_i32_is_a_fault_not_an_answer() {
    // The typed code is an i32, so a wider integer has no code to keep. Both
    // clients call this a protocol fault rather than the daemon's own error:
    // one refusing and the other returning would be the two of them
    // disagreeing about whether the connection is still usable.
    let reply = line(&json!({
        "jsonrpc": "2.0", "id": 2,
        "error": {"code": 99_999_999_999_i64, "message": "x"}
    }));
    assert_eq!(protocol(reply), Protocol::BadError);
}

#[test]
fn a_fault_poisons_the_connection_and_a_new_client_recovers() {
    let reply = line(&json!({"jsonrpc": "2.0", "id": 99, "result": {}}));
    let (fake, mut client) = after_handshake(reply);
    assert!(matches!(
        client.call_raw("grid.version", &json!({})),
        Err(Error::Protocol(Protocol::ForeignId { .. }))
    ));
    assert!(matches!(
        client.call_raw("grid.version", &json!({})),
        Err(Error::Closed)
    ));
    assert!(matches!(client.call(&VersionParams {}), Err(Error::Closed)));
    drop(client);
    // Nothing was written after the fault: the fake read exactly two lines.
    assert_eq!(fake.received().len(), 2);

    let fresh = Fake::start(vec![ok()]);
    assert!(Client::connect(fresh.socket()).is_ok());
}

// --- handshake ------------------------------------------------------------

#[test]
fn a_foreign_major_is_refused_before_any_other_call() {
    let fake = Fake::start(vec![version(1, "2.0.0"), ok()]);
    match Client::connect(fake.socket()) {
        Err(Error::Incompatible { daemon, client }) => {
            assert_eq!(daemon, "2.0.0");
            assert_eq!(client, WIRE_VERSION);
        }
        other => panic!("expected Incompatible, got {other:?}"),
    }
    let received = fake.received();
    assert_eq!(received.len(), 1, "exactly one line: the version handshake");
    let request: Value = serde_json::from_str(&received[0]).unwrap();
    assert_eq!(request["method"], "grid.version");
    assert_eq!(request["id"], 1);
}

#[test]
fn a_grown_result_from_a_newer_minor_is_decode_naming_both_versions() {
    let grown = line(&json!({
        "jsonrpc": "2.0", "id": 2,
        "result": {"wire": "1.2.0", "engine": "fake", "grown": true}
    }));
    let fake = Fake::start(vec![version(1, "1.2.0"), grown.clone()]);
    let mut client = Client::connect(fake.socket()).expect("same major connects");
    assert_eq!(client.daemon_wire(), "1.2.0");
    match client.call(&VersionParams {}) {
        Err(Error::Decode {
            method,
            daemon_wire,
            ..
        }) => {
            assert_eq!(method, "grid.version");
            assert_eq!(daemon_wire, "1.2.0");
        }
        other => panic!("expected Decode, got {other:?}"),
    }
    // Decode is not a poisoning fault: the line was consumed in sync, and
    // the untyped path reads what the typed one could not.
    drop(fake);
    let fake = Fake::start(vec![version(1, "1.2.0"), grown]);
    let mut client = Client::connect(fake.socket()).unwrap();
    let raw = client.call_raw("grid.version", &json!({})).unwrap();
    assert_eq!(raw["grown"], true);
}

#[test]
fn a_grown_handshake_result_still_connects() {
    // `connect` is the one call a caller cannot route around through
    // `call_raw`, so a same-major daemon whose `grid.version` result grew a
    // key must still be reachable: refusing here would make the whole client
    // unusable against the minor bump the wire law explicitly allows.
    let grown = line(&json!({
        "jsonrpc": "2.0", "id": 1,
        "result": {"wire": "1.2.0", "engine": "fake", "uptime_ms": 5}
    }));
    let fake = Fake::start(vec![grown]);
    let client = Client::connect(fake.socket()).expect("a grown version result still connects");
    assert_eq!(client.daemon_wire(), "1.2.0");
}

#[test]
fn a_wire_string_with_no_leading_integer_is_a_protocol_fault() {
    let fake = Fake::start(vec![version(1, "x.1.0")]);
    assert!(matches!(
        Client::connect(fake.socket()),
        Err(Error::Protocol(Protocol::BadWireVersion(w))) if w == "x.1.0"
    ));
}

// --- non-errors -----------------------------------------------------------

#[test]
fn an_aground_walk_is_ok() {
    let aground = line(&json!({
        "jsonrpc": "2.0", "id": 2,
        "result": {
            "status": "aground",
            "snapshot": "snap:b3:77e177e177e177e177e177e177e177e177e177e177e177e177e177e177e177e1",
            "frontier": ["b3:4da74da74da74da74da74da74da74da74da74da74da74da74da74da74da74da7"],
            "stats": {"visited": 5000, "hops": 2, "ms": 3, "complete_within": 2}
        }
    }));
    let (_fake, mut client) = after_handshake(aground);
    let query = serde_json::from_value(json!({"from": [{"callsign": "a.b"}]})).unwrap();
    let walk = client
        .call(&QueryWalkParams { query })
        .expect("aground is a result");
    assert_eq!(walk.status, WalkResultStatus::Aground);
    assert_eq!(walk.frontier.map(|f| f.len()), Some(1));
}

#[test]
fn an_rpc_error_keeps_its_integer_and_does_not_poison() {
    let known = line(&json!({
        "jsonrpc": "2.0", "id": 2,
        "error": {"code": -32003, "message": "unknown or expired snapshot",
                  "data": {"snapshot_id": "snap:b3:00"}}
    }));
    let unknown = line(&json!({
        "jsonrpc": "2.0", "id": 3,
        "error": {"code": -32601, "message": "no such method: grid.nope"}
    }));
    let fake = Fake::start(vec![ok(), known, unknown, version(4, WIRE_VERSION)]);
    let mut client = Client::connect(fake.socket()).unwrap();
    match client.call_raw("grid.resolve", &json!({})) {
        Err(Error::Rpc(RpcError {
            code,
            wire,
            message,
            data,
        })) => {
            assert_eq!(code, -32003);
            assert_eq!(wire, Some(WireError::UnknownSnapshot));
            assert_eq!(message, "unknown or expired snapshot");
            assert_eq!(data, Some(json!({"snapshot_id": "snap:b3:00"})));
        }
        other => panic!("expected Rpc, got {other:?}"),
    }
    match client.call_raw("grid.nope", &json!({})) {
        Err(Error::Rpc(e)) => {
            assert_eq!((e.code, e.wire), (-32601, None));
        }
        other => panic!("expected Rpc, got {other:?}"),
    }
    // Still in sync: id 4 gets the fourth reply.
    assert!(client.call(&VersionParams {}).is_ok());
}

#[test]
fn no_daemon_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.sock");
    assert!(matches!(Client::connect(&path), Err(Error::NoDaemon(p)) if p == path));
    // A socket file nobody accepts on is the same refusal.
    let stale = dir.path().join("stale.sock");
    drop(UnixListener::bind(&stale).unwrap());
    assert!(matches!(Client::connect(&stale), Err(Error::NoDaemon(p)) if p == stale));
}
