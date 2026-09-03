"""The hand-written half: the socket, the framing, the id match and the faults.

Everything the Rust client refuses, this refuses the same way and for the same
reason, so the two clients cannot disagree about what a fault means. What is
deliberately absent is also the same: no spawn (`grid serve` unlinks its socket
path before binding, so starting one against a live socket steals it - see
docs/TODOS.md T-60), no pipelining, no reconnect and no retry (three methods
write), and no cache (the determinism key belongs to the caller).
"""

from __future__ import annotations

import json
import re
import socket
from collections.abc import Mapping
from pathlib import Path

from .methods import WIRE_ERRORS, WIRE_VERSION, Methods

# Conservatism: one response line is read into memory whole, so an unbounded read
# lets a peer on the socket path grow this process without limit. The cap is an
# availability bound and not a framing limit the contract declares - it declares
# none: the largest legitimate answer measured on this repository, a `grid.scope`
# with bodies, is about 3 MiB, and a line that reaches the cap arrives at the
# decoder with no newline and is refused as truncated like any other.
MAX_LINE = 256 * 1024 * 1024
# An escape into the surrogate block, which is the only way a lone surrogate can
# reach `json.loads`: raw surrogate bytes fail the UTF-8 decode a line earlier.
_SURROGATE_ESCAPE = re.compile(r"\\[uU][dD][89abAB]")


class GridError(Exception):
    """The daemon answered with a JSON-RPC error member.

    `code` is the integer as it arrived and is never turned into anything else:
    a ninth contract code, or any of the JSON-RPC codes the contract does not
    declare, leaves `wire` as None rather than failing a lookup. The connection
    stays usable - the daemon answered.
    """

    def __init__(self, code: int, message: str, data: object = None) -> None:
        super().__init__(f"{message} ({code})")
        self.code = code
        self.message = message
        self.data = data
        self.wire = WIRE_ERRORS.get(code)


class GridProtocolError(Exception):
    """A response line that is not a JSON-RPC 2.0 response to the outstanding
    request. Disjoint from `GridError` so `except GridError` cannot swallow a
    desynced socket."""


class GridConnectionError(Exception):
    """No daemon, a daemon that went away, a foreign wire major, or a connection
    an earlier fault poisoned."""


def _major(version: str) -> str | None:
    """The digits before the first `.`, or None when there are none. Majors are
    compared as strings so the client carries no semver parser."""
    head = version.split(".", 1)[0]
    return head if head.isascii() and head.isdigit() else None


def _rpc_error(error: object) -> GridError:
    if not isinstance(error, dict):
        raise GridProtocolError("the error member is not an object")
    code, message = error.get("code"), error.get("message")
    # `bool` is an `int` in Python, so a JSON `true` would otherwise pass for a code.
    if not isinstance(code, int) or isinstance(code, bool) or not isinstance(message, str):
        raise GridProtocolError("the error member is not {code: int, message: string}")
    # Invariant: the Rust client types the code i32 and refuses a wider integer as
    # a protocol fault. Returning it here instead would be the two clients
    # disagreeing about whether the connection is still usable, which is the one
    # thing the shared fault table exists to prevent.
    if not -2147483648 <= code <= 2147483647:
        raise GridProtocolError(f"the error code {code} does not fit i32")
    return GridError(code, message, error.get("data"))


def _decode_line(line: bytes) -> tuple[object, object]:
    """Decode one newline-terminated response line into `(id, body)`, where the
    body is the `result` member or a `GridError` carrying the `error` member.

    Total over arbitrary bytes: every malformed input is a `GridProtocolError`
    naming what was wrong, and nothing else escapes. The Rust fuzzer's corpus is
    replayed through here for exactly that claim (`tests/test_corpus.py`). Keys
    other than the four JSON-RPC 2.0 names are ignored, so an additive envelope
    change is not a fault.
    """
    if not line.endswith(b"\n"):
        raise GridProtocolError("truncated: a line with no newline is never parsed")
    try:
        text = line[:-1].decode("utf-8")
    except UnicodeDecodeError:
        raise GridProtocolError("not utf-8") from None
    try:
        value = json.loads(text)
    # Conservatism: `json` reports a nesting overflow as RecursionError and an
    # over-long integer literal as ValueError, and the caller of a decoder over
    # socket bytes should have to catch one exception type, not three.
    except (ValueError, RecursionError):
        raise GridProtocolError("not json") from None
    # Invariant: `json` decodes a lone surrogate escape into a str that raises on
    # any later encode, where serde_json refuses the line outright; the cheap scan
    # keeps the re-encode off every response that cannot contain one.
    if _SURROGATE_ESCAPE.search(text):
        try:
            json.dumps(value, ensure_ascii=False).encode("utf-8")
        except UnicodeEncodeError:
            raise GridProtocolError("not json") from None
    if not isinstance(value, dict):
        raise GridProtocolError(f"not an object: {type(value).__name__}")
    if value.get("jsonrpc") != "2.0":
        raise GridProtocolError(f"jsonrpc is {value.get('jsonrpc')!r}, not '2.0'")
    if "id" not in value:
        raise GridProtocolError("no id member")
    has_result, has_error = "result" in value, "error" in value
    if has_result and has_error:
        raise GridProtocolError("both result and error")
    if not has_result and not has_error:
        raise GridProtocolError("neither result nor error")
    if has_result:
        return value["id"], value["result"]
    return value["id"], _rpc_error(value["error"])


class Client(Methods):
    """One connection to one daemon.

    Built by `Client.connect`; the constructor takes an already-connected socket
    so the handshake can be one ordinary call through this class.
    """

    def __init__(self, sock: socket.socket) -> None:
        self._sock = sock
        self._reader = sock.makefile("rb")
        self._next = 1
        self._poisoned = False
        self.daemon_wire = ""

    @classmethod
    def connect(cls, socket_path: str | Path) -> Client:
        """Connect to the daemon listening on `socket_path` and check its wire major.

        The path is the caller's to resolve, as it is for the Rust client: the
        daemon resolves its own default against *its* working directory, which
        this package cannot know, so it defaults nothing.

        Raises `GridConnectionError` when nothing accepts, or when the daemon's
        wire major differs from `WIRE_VERSION`'s - before any second call is
        made; `GridProtocolError` when the reported version has no major.
        """
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.connect(str(socket_path))
        except (FileNotFoundError, ConnectionRefusedError) as e:
            sock.close()
            raise GridConnectionError(
                f"no daemon on {socket_path}: run grid serve in its root"
            ) from e
        except OSError as e:
            sock.close()
            raise GridConnectionError(f"{socket_path}: {e}") from e
        client = cls(sock)
        # The handshake reads `wire` off the answer as it arrived, as the Rust
        # client's does: a daemon one minor ahead whose result grew a key is
        # precisely the daemon this client must still reach, and an answer with no
        # usable `wire` is that client's BadWireVersion rather than a KeyError.
        wire = client.version().get("wire")
        daemon = _major(wire) if isinstance(wire, str) else None
        if daemon is None:
            client.close()
            raise GridProtocolError(f"grid.version reported wire {wire!r}, which has no major")
        if daemon != _major(WIRE_VERSION):
            client.close()
            raise GridConnectionError(
                f"the daemon speaks wire {wire}; this client speaks {WIRE_VERSION}"
            )
        client.daemon_wire = wire
        return client

    def call(self, method: str, params: Mapping[str, object]) -> object:
        """Call one wire method by name, returning its `result` member as it arrived.

        The generated methods on `Methods` are one typed call each through here.

        Raises `GridError` when the daemon answers with an error, which leaves
        the connection usable; `GridProtocolError` or `GridConnectionError`
        otherwise, which poison it.
        """
        if self._poisoned:
            raise GridConnectionError(
                "this connection was closed by an earlier fault; open a new client"
            )
        request_id = self._next
        self._next += 1
        try:
            return self._exchange(request_id, method, params)
        except GridError:
            raise
        # Invariant: everything but a GridError leaves the stream at an unknown
        # offset, and a client that wrote again would be writing a second time
        # into a conversation it has lost track of.
        except BaseException:
            self._poisoned = True
            raise

    def _exchange(self, request_id: int, method: str, params: Mapping[str, object]) -> object:
        """One line out, one line in, matched by id."""
        request = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": dict(params)}
        try:
            self._sock.sendall(json.dumps(request).encode("utf-8") + b"\n")
            raw = self._reader.readline(MAX_LINE)
        # Incident: a daemon that exited between two calls is seen as a reset or
        # a broken pipe on the write, or as EOF on the read, depending on who
        # won the race; all three are the same fact to a caller.
        except (BrokenPipeError, ConnectionResetError) as e:
            raise GridConnectionError(f"the daemon went away: {e}") from e
        except OSError as e:
            raise GridConnectionError(f"{method}: {e}") from e
        if not raw:
            raise GridConnectionError("the daemon closed the connection")
        answer_id, body = _decode_line(raw)
        if answer_id is None:
            raise GridProtocolError(
                "the daemon answered id null: it could not parse the line this client wrote"
            )
        if not isinstance(answer_id, int) or isinstance(answer_id, bool) or answer_id != request_id:
            raise GridProtocolError(
                f"the answer carries id {answer_id!r}, not the outstanding {request_id}"
            )
        if isinstance(body, GridError):
            raise body
        return body

    def close(self) -> None:
        """Close the socket. Idempotent, and every later call refuses."""
        self._poisoned = True
        self._reader.close()
        self._sock.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_exception: object) -> None:
        self.close()
