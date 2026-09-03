//! grid-client — a synchronous JSON-RPC 2.0 client for `grid serve` over its
//! unix socket, speaking wire v1 through the types in [`grid_wire`].
//!
//! One [`Client`] is one connection. [`Client::connect`] adopts a running
//! daemon, asks it `grid.version` before anything else and refuses a
//! different wire major; [`Client::call`] writes one line and reads one line,
//! matching the answer to the request by id. The typed surface is that one
//! generic function over [`grid_wire::methods::Method`], so this crate carries
//! no generated file and no table of its own.
//!
//! ## What is deliberately not here
//!
//! - **No spawn.** `grid serve` unlinks its socket path before binding, so a
//!   client that started a daemon against a live socket would silently steal
//!   it from the daemon already there (docs/TODOS.md T-60). The daemon's
//!   lifecycle is the caller's; a missing daemon is [`Error::NoDaemon`].
//! - **No pipelining.** The transport allows it and the daemon answers in
//!   order, but it buys round trips on a local socket against a serial
//!   daemon and drags in an out-of-order correlation path nothing exercises.
//!   What is kept is the property that would make it safe later: a response
//!   is matched to its request by id, never by arrival, and one that does
//!   not match is [`Protocol::ForeignId`], never attributed to the
//!   outstanding call.
//! - **No reconnect, no retry.** Three methods write (`grid.survey.run`,
//!   `grid.edit.apply`, `grid.cache.record`); a silent re-issue is a second
//!   write. After any socket or protocol fault the connection is poisoned and
//!   every later call is [`Error::Closed`]; the caller opens a new client,
//!   which is equivalent because the daemon keeps no per-connection state.
//! - **No cache.** The determinism key belongs to the caller.
//!
//! Concurrency is more clients: every call takes `&mut self`, and the daemon
//! serves connections one at a time, so a pool would buy nothing.
//!
//! ## Aground is a result
//!
//! A walk that runs out of fuel comes back as `Ok`: [`grid_wire::WalkResult`]
//! with [`grid_wire::WalkResultStatus::Aground`] and its `frontier`. No
//! [`Error`] variant can carry a walk, so the type system makes the
//! contract's rule visible — the caller issues a new walk from the frontier:
//!
//! ```no_run
//! use grid_client::Client;
//! use grid_wire::methods::QueryWalkParams;
//! use grid_wire::{WalkQuery, WalkResultStatus};
//!
//! # fn main() -> Result<(), grid_client::Error> {
//! let mut client = Client::connect(".grid/grid.sock")?;
//! let query: WalkQuery = serde_json::from_str(r#"{"from":[{"callsign":"Segment::parse"}]}"#)
//!     .map_err(|source| grid_client::Error::Encode { method: "grid.query.walk", source })?;
//! let walk = client.call(&QueryWalkParams { query: query.clone() })?;
//! if walk.status == WalkResultStatus::Aground {
//!     let next = WalkQuery {
//!         from: walk.frontier.unwrap_or_default().into_iter().map(|datum| grid_wire::Locator::Datum { datum }).collect(),
//!         ..query
//!     };
//!     let _more = client.call(&QueryWalkParams { query: next })?;
//! }
//! # Ok(())
//! # }
//! ```

mod error;
mod protocol;

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use grid_wire::methods::{Method, VersionParams};
use grid_wire::WIRE_VERSION;
use serde_json::{json, Value};

pub use error::{Error, Protocol, RequestId, RpcError};
pub use protocol::{decode_line, Body, Response};

/// One connection to one daemon.
#[derive(Debug)]
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: RequestId,
    daemon_wire: String,
    poisoned: bool,
}

/// Conservatism: one response line is read into memory whole, so an unbounded
/// read lets a peer on the socket path grow this process without limit. The cap
/// is an availability bound and not a framing limit the contract declares — it
/// declares none: the largest legitimate answer measured on this repository, a
/// `grid.scope` with bodies, is about 3 MiB, and a line that reaches the cap
/// arrives at the decoder with no newline and is refused as
/// [`Protocol::Truncated`] like any other unterminated line.
const MAX_LINE: u64 = 256 * 1024 * 1024;

/// Incident: a daemon that exited between two calls is seen as a reset or a
/// broken pipe on the write, or as EOF on the read, depending on who won the
/// race; all three are the same fact to a caller.
fn gone(e: std::io::Error) -> Error {
    match e.kind() {
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset => Error::Disconnected,
        _ => Error::Io(e),
    }
}

/// The digits before the first `.`, or `None` when there are none.
fn major(version: &str) -> Option<&str> {
    let head = version.split('.').next().unwrap_or(version);
    (!head.is_empty() && head.bytes().all(|b| b.is_ascii_digit())).then_some(head)
}

impl Client {
    /// Connect to the daemon listening on `socket` and check its wire major.
    ///
    /// The path is the caller's to resolve: an explicit choice, then
    /// `GRID_SOCKET`, then `<root>/.grid/grid.sock`. The daemon resolves its
    /// own default against *its* working directory, which this crate cannot
    /// know, so it defaults nothing.
    ///
    /// # Errors
    /// [`Error::NoDaemon`] when nothing accepts on `socket`; [`Error::Io`]
    /// for any other socket failure; [`Error::Incompatible`] when the
    /// daemon's wire major differs from [`grid_wire::WIRE_VERSION`]'s, before
    /// any other call is made; [`Error::Protocol`] when the `grid.version`
    /// answer carries no usable `wire`.
    ///
    /// ```no_run
    /// let client = grid_client::Client::connect(".grid/grid.sock")?;
    /// assert_eq!(client.daemon_wire().split('.').next(), Some("1"));
    /// # Ok::<(), grid_client::Error>(())
    /// ```
    pub fn connect(socket: impl AsRef<Path>) -> Result<Self, Error> {
        let socket = socket.as_ref();
        let stream = UnixStream::connect(socket).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                Error::NoDaemon(socket.to_path_buf())
            }
            _ => Error::Io(e),
        })?;
        let writer = stream.try_clone()?;
        let mut client = Self {
            reader: BufReader::new(stream),
            writer,
            next: RequestId(1),
            daemon_wire: String::new(),
            poisoned: false,
        };
        // Invariant: the handshake reads `wire` off the untyped answer. A daemon
        // one minor ahead whose `grid.version` result grew a key is precisely the
        // daemon this client must still reach, and `connect` is the one call a
        // caller cannot route around through [`Client::call_raw`], so decoding it
        // typed here would refuse the forward compatibility the rest of the crate
        // promises.
        let raw = client.call_raw(VersionParams::NAME, &json!({}))?;
        let wire = raw
            .get("wire")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let daemon = major(&wire).ok_or_else(|| Protocol::BadWireVersion(wire.clone()))?;
        if Some(daemon) != major(WIRE_VERSION) {
            return Err(Error::Incompatible {
                daemon: wire,
                client: WIRE_VERSION,
            });
        }
        client.daemon_wire = wire;
        Ok(client)
    }

    /// Call one wire method with typed params, decoding a typed result.
    ///
    /// # Errors
    /// [`Error::Rpc`] when the daemon answers with an error (the connection
    /// stays usable); [`Error::Decode`] when the result does not fit
    /// `M::Result` — a daemon one minor ahead whose result grew a key, which
    /// [`Client::call_raw`] reads untyped; [`Error::Encode`] when the params
    /// do not serialize; [`Error::Closed`] after an earlier fault;
    /// [`Error::IdsExhausted`] after `u64::MAX` calls; and the poisoning
    /// faults [`Error::Io`], [`Error::Disconnected`], [`Error::Protocol`].
    pub fn call<M: Method>(&mut self, params: &M) -> Result<M::Result, Error> {
        let params = serde_json::to_value(params).map_err(|source| Error::Encode {
            method: M::NAME,
            source,
        })?;
        let result = self.call_raw(M::NAME, &params)?;
        serde_json::from_value(result).map_err(|source| Error::Decode {
            method: M::NAME,
            daemon_wire: self.daemon_wire.clone(),
            source,
        })
    }

    /// Call one wire method by name with untyped params, returning the
    /// result as it arrived. The forward-compatible path: a result whose
    /// shape this build does not know is still an answer here.
    ///
    /// # Errors
    /// As [`Client::call`], minus [`Error::Decode`] and [`Error::Encode`].
    pub fn call_raw(&mut self, method: &str, params: &Value) -> Result<Value, Error> {
        if self.poisoned {
            return Err(Error::Closed);
        }
        let id = self.next;
        self.next = RequestId(id.0.checked_add(1).ok_or(Error::IdsExhausted)?);
        let outcome = self.exchange(id, method, params);
        // Invariant: the poisoning set is exactly the faults that leave the
        // stream at an unknown offset. A variant added to `Error` later is
        // non-poisoning until it is named here.
        if matches!(
            outcome,
            Err(Error::Io(_) | Error::Disconnected | Error::Protocol(_))
        ) {
            self.poisoned = true;
        }
        outcome
    }

    /// One line out, one line in, matched by id.
    fn exchange(&mut self, id: RequestId, method: &str, params: &Value) -> Result<Value, Error> {
        let request = json!({ "jsonrpc": "2.0", "id": id.0, "method": method, "params": params });
        let mut line = serde_json::to_vec(&request).map_err(|source| Error::Encode {
            method: "request",
            source,
        })?;
        line.push(b'\n');
        self.writer.write_all(&line).map_err(gone)?;
        self.writer.flush().map_err(gone)?;

        line.clear();
        if (&mut self.reader)
            .take(MAX_LINE)
            .read_until(b'\n', &mut line)
            .map_err(gone)?
            == 0
        {
            return Err(Error::Disconnected);
        }
        let response = decode_line(&line)?;
        if response.id.is_null() {
            return Err(Protocol::NullId.into());
        }
        if response.id.as_u64() != Some(id.0) {
            return Err(Protocol::ForeignId {
                expected: id,
                found: response.id,
            }
            .into());
        }
        match response.body {
            Body::Result(result) => Ok(result),
            Body::Error(error) => Err(Error::Rpc(error)),
        }
    }

    /// Bound every later read; `None` waits forever, the default, because
    /// `grid.check` and a cold `grid.survey.run` take seconds. A timeout
    /// surfaces from the next call as [`Error::Io`] with the kind std
    /// reports, and poisons the connection like any other socket fault.
    ///
    /// # Errors
    /// [`Error::Io`] if the socket refuses the option.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), Error> {
        Ok(self.reader.get_ref().set_read_timeout(timeout)?)
    }

    /// The daemon's wire version as `grid.version` reported it at connect.
    #[must_use]
    pub fn daemon_wire(&self) -> &str {
        &self.daemon_wire
    }
}
