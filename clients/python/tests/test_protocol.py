"""C54: the Python client refuses what the Rust client refuses, for the same reason.

The fault table is one test per row of the SDK plan's answer 4, and the rows are
the ones `crates/client/tests/protocol.rs` defends, so the two clients cannot
drift apart about what a fault means. Rows that are purely about reading a line
call `_decode_line` directly; rows about a connection — a null id, a foreign id,
EOF, poisoning, the handshake — go through a fake daemon that answers whatever
it was scripted to, verbatim.
"""

import json
import os
import socket
import tempfile
import threading
import unittest

from grid_client import Client, GridConnectionError, GridError, GridProtocolError, WIRE_VERSION
from grid_client.client import _decode_line


def line(**members: object) -> bytes:
    """One response line. Members are written exactly as given, so a test can
    omit `id` or carry both bodies."""
    return json.dumps(members).encode("utf-8") + b"\n"


def version_line(request_id: int = 1, wire: str = WIRE_VERSION, **extra: object) -> bytes:
    result: dict[str, object] = {"wire": wire, "engine": "grid 0.1.0", **extra}
    return line(jsonrpc="2.0", id=request_id, result=result)


class Fake:
    """A daemon on its own socket that replies once per request line, verbatim.

    It is the only way to script a response the real daemon would never send,
    which is most of the fault table. `received` is what the client actually
    wrote, so a test can assert a refusal happened *before* a second line.
    """

    def __init__(self, *replies: bytes) -> None:
        self._dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self._dir.name, "grid.sock")
        self._replies = replies
        self.received: list[bytes] = []
        self._listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._listener.bind(self.path)
        self._listener.listen(1)
        self._thread = threading.Thread(target=self._serve, daemon=True)
        self._thread.start()

    def _serve(self) -> None:
        try:
            conn, _ = self._listener.accept()
        except OSError:
            return
        with conn, conn.makefile("rb") as reader:
            for reply in self._replies:
                request = reader.readline()
                if not request:
                    return
                self.received.append(request)
                conn.sendall(reply)

    def close(self) -> None:
        self._listener.close()
        self._thread.join(timeout=5)
        self._dir.cleanup()


class Fault(unittest.TestCase):
    """Every malformed line is a GridProtocolError naming what was wrong, and
    nothing else escapes the decoder."""

    def test_a_line_with_no_newline_is_truncated_and_never_parsed(self) -> None:
        # A whole line with only its newline removed, and the fault named rather
        # than merely raised: the decoder drops a trailing byte to strip the frame,
        # so a line it declined to reject as truncated goes on to fail as not-JSON,
        # and a bare assertRaises cannot tell the guard working from the guard gone.
        whole = line(jsonrpc="2.0", id=1, result={"a": 1})
        with self.assertRaisesRegex(GridProtocolError, "truncated"):
            _decode_line(whole[:-1])

    def test_non_utf8_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(b"\xff\xfe\n")

    def test_non_json_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(b"not json\n")

    def test_a_non_object_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(b"[1, 2, 3]\n")

    def test_a_jsonrpc_other_than_2_0_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(line(jsonrpc="1.0", id=1, result={}))

    def test_a_missing_id_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(line(jsonrpc="2.0", result={}))

    def test_both_result_and_error_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(line(jsonrpc="2.0", id=1, result={}, error={"code": -1, "message": "x"}))

    def test_neither_result_nor_error_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(line(jsonrpc="2.0", id=1))

    def test_a_malformed_error_object_is_a_fault(self) -> None:
        with self.assertRaises(GridProtocolError):
            _decode_line(line(jsonrpc="2.0", id=1, error="boom"))
        with self.assertRaises(GridProtocolError):
            _decode_line(line(jsonrpc="2.0", id=1, error={"code": -1}))

    def test_an_error_code_outside_i32_is_a_fault_not_an_answer(self) -> None:
        # The Rust client types the code i32 and refuses a wider integer here.
        # Returning it as a GridError would leave this connection usable where the
        # other client poisons it, which is the disagreement the table forbids.
        with self.assertRaises(GridProtocolError):
            _decode_line(line(jsonrpc="2.0", id=1, error={"code": 99999999999, "message": "x"}))

    def test_a_lone_surrogate_is_a_fault(self) -> None:
        # serde_json refuses the escape outright; `json` decodes it into a str that
        # raises on any later encode, so a caller writing the answer out would get
        # a UnicodeEncodeError from under the decoder instead of a fault at it.
        with self.assertRaises(GridProtocolError):
            _decode_line(b'{"jsonrpc":"2.0","id":1,"result":"\\ud800"}\n')

    def test_a_surrogate_pair_is_an_ordinary_string(self) -> None:
        _, body = _decode_line(b'{"jsonrpc":"2.0","id":1,"result":"\\ud83d\\ude00"}\n')
        self.assertEqual(body, "\U0001f600")

    def test_a_nesting_overflow_is_a_fault_not_a_recursion_error(self) -> None:
        # `json` reports it as RecursionError, which is not a ValueError and would
        # escape a caller catching only the decoder's own faults.
        with self.assertRaises(GridProtocolError):
            _decode_line(b"[" * 20000 + b"\n")

    def test_an_unknown_error_code_keeps_the_integer_it_arrived_as(self) -> None:
        _, body = _decode_line(line(jsonrpc="2.0", id=1, error={"code": -32601, "message": "no"}))
        self.assertIsInstance(body, GridError)
        assert isinstance(body, GridError)
        self.assertEqual(body.code, -32601)
        self.assertIsNone(body.wire, "-32601 is not one of the contract's eight")

    def test_a_contract_error_code_is_named(self) -> None:
        _, body = _decode_line(
            line(jsonrpc="2.0", id=1, error={"code": -32003, "message": "gone", "data": {"x": 1}})
        )
        assert isinstance(body, GridError)
        self.assertEqual(body.wire, "UnknownSnapshot")
        self.assertEqual(body.data, {"x": 1})


class Connection(unittest.TestCase):
    """A response is matched to its request by id, and a fault poisons."""

    def connected(self, *replies: bytes) -> tuple[Fake, Client]:
        fake = Fake(version_line(), *replies)
        self.addCleanup(fake.close)
        client = Client.connect(fake.path)
        self.addCleanup(client.close)
        return fake, client

    def test_a_foreign_id_is_a_fault_not_an_answer(self) -> None:
        _, client = self.connected(line(jsonrpc="2.0", id=99, result={}))
        with self.assertRaises(GridProtocolError):
            client.call("grid.version", {})

    def test_a_null_id_is_the_daemon_rejecting_our_line(self) -> None:
        _, client = self.connected(line(jsonrpc="2.0", id=None, result={}))
        with self.assertRaises(GridProtocolError):
            client.call("grid.version", {})

    def test_eof_at_a_line_boundary_is_a_connection_fault(self) -> None:
        fake = Fake(version_line())
        self.addCleanup(fake.close)
        client = Client.connect(fake.path)
        self.addCleanup(client.close)
        with self.assertRaises(GridConnectionError):
            client.call("grid.version", {})

    def test_a_fault_poisons_the_connection_and_a_new_client_recovers(self) -> None:
        fake = Fake(version_line(), line(jsonrpc="2.0", id=99, result={}), version_line(3))
        self.addCleanup(fake.close)
        client = Client.connect(fake.path)
        self.addCleanup(client.close)
        with self.assertRaises(GridProtocolError):
            client.call("grid.version", {})
        # Not another protocol fault: the connection refuses without writing.
        with self.assertRaises(GridConnectionError):
            client.call("grid.version", {})
        self.assertEqual(len(fake.received), 2, "a poisoned client wrote a third line")

    def test_an_rpc_error_does_not_poison(self) -> None:
        _, client = self.connected(
            line(jsonrpc="2.0", id=2, error={"code": -32002, "message": "no snapshot"}),
            line(jsonrpc="2.0", id=3, result={"ok": True}),
        )
        with self.assertRaises(GridError) as caught:
            client.call("grid.snapshot.current", {})
        self.assertEqual(caught.exception.wire, "NoSnapshot")
        self.assertEqual(client.call("grid.version", {}), {"ok": True})

    def test_no_daemon_names_the_path(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            stale = os.path.join(tmp, "grid.sock")
            with self.assertRaises(GridConnectionError) as caught:
                Client.connect(stale)
            self.assertIn(stale, str(caught.exception))


class Handshake(unittest.TestCase):
    """`connect` asks `grid.version` before anything else and refuses a foreign major."""

    def test_a_foreign_major_is_refused_before_any_other_call(self) -> None:
        fake = Fake(version_line(wire="2.0.0"), version_line(2))
        self.addCleanup(fake.close)
        with self.assertRaises(GridConnectionError) as caught:
            Client.connect(fake.path)
        self.assertIn("2.0.0", str(caught.exception))
        self.assertIn(WIRE_VERSION, str(caught.exception))
        self.assertEqual(len(fake.received), 1, "the client wrote a second line after refusing")

    def test_a_wire_string_with_no_leading_integer_is_a_protocol_fault(self) -> None:
        fake = Fake(version_line(wire="x.1.0"))
        self.addCleanup(fake.close)
        with self.assertRaises(GridProtocolError):
            Client.connect(fake.path)

    def test_a_newer_minor_connects_and_its_extra_keys_survive(self) -> None:
        # `grown` is a key no generated shape declares, which is what a daemon one
        # minor ahead looks like. The Rust client reaches it through `call_raw`;
        # this one hands back the result member as it arrived, so the second reply
        # is what proves the key survived rather than the handshake alone.
        fake = Fake(version_line(wire="1.2.0"), version_line(2, wire="1.2.0", grown=True))
        self.addCleanup(fake.close)
        client = Client.connect(fake.path)
        self.addCleanup(client.close)
        self.assertEqual(client.daemon_wire, "1.2.0")
        self.assertEqual(client.call("grid.version", {}), {
            "wire": "1.2.0", "engine": "grid 0.1.0", "grown": True
        })


if __name__ == "__main__":
    unittest.main()
