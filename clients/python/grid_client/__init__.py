"""grid-client - a synchronous JSON-RPC 2.0 client for `grid serve`, speaking
wire v1 over the daemon's unix socket with the standard library and nothing else.

One `Client` is one connection. `Client.connect` adopts a running daemon, asks
it `grid.version` before anything else and refuses a different wire major; one
call is one line written and one line read, matched to its request by id. The
typed surface is generated into `grid_client.methods` from the same contract the
Rust client's is, so the two cannot disagree about a shape or a method name.

Named re-exports, never a star import: `check_hygiene.py` keeps that budget at
zero here for the reason it keeps it at zero for Rust.
"""

from .client import Client, GridConnectionError, GridError, GridProtocolError
from .methods import METHODS, WIRE_ERRORS, WIRE_VERSION

__all__ = [
    "METHODS",
    "WIRE_ERRORS",
    "WIRE_VERSION",
    "Client",
    "GridConnectionError",
    "GridError",
    "GridProtocolError",
]
