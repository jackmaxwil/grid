//! What a call can fail with, and what a failure carries.

use std::path::PathBuf;

use grid_wire::errors::WireError;
use serde_json::Value;

/// The id of one request on one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

/// A response line that is not a response to the outstanding request.
///
/// Every variant is a fault in what the daemon wrote, never in what the
/// caller asked; each one poisons the connection it arrived on. This is the
/// error type of [`crate::decode_line`], which is total over arbitrary bytes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protocol {
    /// The line has no terminating newline: the peer closed the socket inside
    /// it, or it reached the client's read cap. A partial line is never
    /// parsed: it may be a prefix of a valid response, and answering from a
    /// prefix would be answering a question the daemon did not finish.
    Truncated,
    /// The line is not UTF-8.
    NotUtf8,
    /// The line is not JSON.
    NotJson,
    /// The line is JSON but not an object.
    NotObject,
    /// `jsonrpc` is absent or not `"2.0"`; carries what was there.
    BadJsonrpc(Option<Value>),
    /// The object has no `id` key at all.
    MissingId,
    /// `id` is `null`: the daemon could not read the request line the client
    /// wrote, so the fault is on this side of the socket.
    NullId,
    /// `id` names a request other than the outstanding one. Responses are
    /// matched by id and never by arrival order, so this is a fault rather
    /// than an answer.
    ForeignId {
        /// The id the client is waiting on.
        expected: RequestId,
        /// The id the line carried.
        found: Value,
    },
    /// Both `result` and `error` are present.
    BothBodies,
    /// Neither `result` nor `error` is present.
    NoBody,
    /// `error` is not an object with an integer `code` and a string
    /// `message`.
    BadError,
    /// `grid.version` answered no usable `wire`: absent, not a string, or a
    /// string with no leading integer, so no major can be compared. Carries
    /// the string that was there, and the empty string when there was none.
    BadWireVersion(String),
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => f.write_str("response line has no terminating newline"),
            Self::NotUtf8 => f.write_str("response line is not UTF-8"),
            Self::NotJson => f.write_str("response line is not JSON"),
            Self::NotObject => f.write_str("response is not a JSON object"),
            Self::BadJsonrpc(found) => write!(f, "response jsonrpc is {found:?}, not \"2.0\""),
            Self::MissingId => f.write_str("response has no id"),
            Self::NullId => {
                f.write_str("response id is null: the daemon could not parse the request")
            }
            Self::ForeignId { expected, found } => {
                write!(
                    f,
                    "response id {found} is not the outstanding {}",
                    expected.0
                )
            }
            Self::BothBodies => f.write_str("response carries both result and error"),
            Self::NoBody => f.write_str("response carries neither result nor error"),
            Self::BadError => f.write_str("response error is not {code, message}"),
            Self::BadWireVersion(wire) => write!(f, "wire version {wire:?} has no leading major"),
        }
    }
}

impl std::error::Error for Protocol {}

/// A JSON-RPC error object the daemon answered with. The `code` is always
/// kept as the integer it arrived as; `wire` is the same code named, when
/// this build knows the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// The code as sent.
    pub code: i32,
    /// [`WireError::from_code`] of `code`: `None` for the JSON-RPC standard
    /// codes and for a code newer than this build.
    pub wire: Option<WireError>,
    /// The message as sent.
    pub message: String,
    /// The `data` member as sent, if any (the contract declares a shape per
    /// code; the daemon may omit it).
    pub data: Option<Value>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

/// Why a call did not return a result.
///
/// [`Error::Rpc`] is the daemon answering "no"; the connection is intact.
/// [`Error::Decode`] is an answer this build cannot read; the connection is
/// intact and [`crate::Client::call_raw`] reads the same answer untyped. Every
/// other variant that reaches a caller from a call leaves the connection
/// poisoned: later calls answer [`Error::Closed`], and the recovery is a new
/// [`crate::Client`].
#[non_exhaustive]
#[derive(Debug)]
pub enum Error {
    /// The socket failed; includes a read timeout set by
    /// [`crate::Client::set_read_timeout`], with the kind std reports.
    Io(std::io::Error),
    /// The daemon wrote something that is not a response to the outstanding
    /// request.
    Protocol(Protocol),
    /// The daemon answered with a JSON-RPC error.
    Rpc(RpcError),
    /// The daemon speaks a different wire major. Refused at connect, before
    /// any other call.
    Incompatible {
        /// The daemon's `wire` string.
        daemon: String,
        /// This build's [`grid_wire::WIRE_VERSION`].
        client: &'static str,
    },
    /// The result did not fit the typed shape this build was generated from.
    /// Both versions are named because the usual cause is a daemon one minor
    /// ahead, whose result grew a key `deny_unknown_fields` refuses.
    Decode {
        /// The wire method.
        method: &'static str,
        /// The daemon's `wire` string.
        daemon_wire: String,
        /// What serde said.
        source: serde_json::Error,
    },
    /// The params did not serialize.
    Encode {
        /// The wire method.
        method: &'static str,
        /// What serde said.
        source: serde_json::Error,
    },
    /// Nothing is listening on the socket path: no file, or a file nobody
    /// accepts on. The client never starts a daemon; the caller does.
    NoDaemon(PathBuf),
    /// The daemon closed the socket at a line boundary: it exited, or was
    /// started `--once`.
    Disconnected,
    /// An earlier fault poisoned this connection; open a new client.
    Closed,
    /// The per-connection id counter reached `u64::MAX`.
    IdsExhausted,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "socket: {e}"),
            Self::Protocol(p) => write!(f, "protocol: {p}"),
            Self::Rpc(e) => write!(f, "daemon refused: {e}"),
            Self::Incompatible { daemon, client } => {
                write!(
                    f,
                    "daemon wire {daemon} is not client wire {client}: majors differ"
                )
            }
            Self::Decode {
                method,
                daemon_wire,
                source,
            } => write!(
                f,
                "{method} result from daemon wire {daemon_wire} does not fit client wire {}: {source}",
                grid_wire::WIRE_VERSION
            ),
            Self::Encode { method, source } => write!(f, "{method} params: {source}"),
            Self::NoDaemon(path) => write!(
                f,
                "no daemon listening on {}; run `grid serve` in the repository root",
                path.display()
            ),
            Self::Disconnected => f.write_str("daemon closed the connection"),
            Self::Closed => {
                f.write_str("connection poisoned by an earlier fault; open a new client")
            }
            Self::IdsExhausted => f.write_str("request ids exhausted on this connection"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Protocol(p) => Some(p),
            Self::Rpc(e) => Some(e),
            Self::Decode { source, .. } | Self::Encode { source, .. } => Some(source),
            Self::Incompatible { .. }
            | Self::NoDaemon(_)
            | Self::Disconnected
            | Self::Closed
            | Self::IdsExhausted => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Protocol> for Error {
    fn from(p: Protocol) -> Self {
        Self::Protocol(p)
    }
}
