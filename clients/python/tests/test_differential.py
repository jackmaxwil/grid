"""C54's differential half: the Python client answers what the CLI's `--json` prints.

The same five verbs `crates/grid/tests/client.rs` compares, asked the same way,
so the two clients are held to one answer rather than to each other. The CLI runs
every verb in-process through the same `rpc::*` functions the daemon dispatches
to and opens no socket, so what the comparison isolates is exactly what the SDK
adds: the daemon's framing, and this client's encode, id match and decode.
"""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import socket
import subprocess
import tempfile
import time
import unittest

from grid_client import Client
from grid_client.schemas import CallsignPattern, GitRev, SnapshotOrRevGitRev

ROOT = pathlib.Path(__file__).resolve().parents[3]
# The definition every comparison asks about. It sits in a subdirectory because
# `grid resolve` tells a `path:line` locator from a pinned name by the slash, so
# a top-level `a.py:1` would resolve as a callsign instead.
FILE = "src/a.py"
GIT_ENV = {
    "GIT_AUTHOR_NAME": "grid",
    "GIT_AUTHOR_EMAIL": "grid@example.invalid",
    "GIT_COMMITTER_NAME": "grid",
    "GIT_COMMITTER_EMAIL": "grid@example.invalid",
}


def grid_binary() -> str | None:
    """GRID_BIN, else the debug build, else whatever is on PATH."""
    named = os.environ.get("GRID_BIN")
    if named:
        return named
    built = ROOT / "target" / "debug" / "grid"
    return str(built) if built.is_file() else shutil.which("grid")


def without_stats(value: object) -> object:
    """The comparison surface contract convention 4 defines: the `stats` object is
    excluded, everywhere it appears, and nothing else is."""
    if isinstance(value, dict):
        return {k: without_stats(v) for k, v in value.items() if k != "stats"}
    if isinstance(value, list):
        return [without_stats(v) for v in value]
    return value


class Differential(unittest.TestCase):
    binary: str
    dir: str
    client: Client
    _tmp: tempfile.TemporaryDirectory[str]
    _daemon: subprocess.Popen[bytes]

    @classmethod
    def setUpClass(cls) -> None:
        binary = grid_binary()
        if binary is None:
            raise unittest.SkipTest("no grid binary: set GRID_BIN or run cargo build -p grid")
        cls.binary = binary
        cls._tmp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls._tmp.cleanup)
        cls.dir = cls._tmp.name
        cls._fixture()
        cls._daemon, socket_path = cls._serve()
        # Incident: registered the instant the daemon exists, because unittest runs
        # tearDownClass only when setUpClass finished. A raise from either line
        # below - a version mismatch, a survey the daemon refuses - otherwise left
        # `grid serve` alive holding the runner's inherited stdout, which hangs the
        # caller instead of failing it red.
        cls.addClassCleanup(cls._reap)
        cls.client = Client.connect(socket_path)
        cls.client.survey_run()

    @classmethod
    def _reap(cls) -> None:
        if hasattr(cls, "client"):
            cls.client.close()
        cls._daemon.kill()
        cls._daemon.wait()

    @classmethod
    def _fixture(cls) -> None:
        """A repo with two commits, so `diff` and `drift` have a real delta."""
        def git(*args: str) -> None:
            subprocess.run(["git", *args], cwd=cls.dir, env={**os.environ, **GIT_ENV},
                           check=True, capture_output=True)

        os.mkdir(os.path.join(cls.dir, "src"))
        git("init", "-q", "-b", "main")
        pathlib.Path(cls.dir, FILE).write_text("def a():\n    pass\n")
        git("add", "-A")
        git("commit", "-qm", "one")
        pathlib.Path(cls.dir, "src/b.py").write_text("def b():\n    a()\n")
        git("add", "-A")
        git("commit", "-qm", "two")

    @classmethod
    def _serve(cls) -> tuple[subprocess.Popen[bytes], str]:
        path = os.path.join(cls.dir, "grid.sock")
        daemon = subprocess.Popen([cls.binary, "serve", path], cwd=cls.dir,
                                  env={**os.environ, "GRID_HINTS": "0"},
                                  stderr=subprocess.DEVNULL)
        # Readiness is a connection that succeeds, not a file that exists: bind(2)
        # creates the socket file before listen(2) starts accepting.
        for _ in range(200):
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
                try:
                    probe.connect(path)
                    return daemon, path
                except OSError:
                    pass
            time.sleep(0.025)
        daemon.kill()
        daemon.wait()
        raise AssertionError(f"the daemon never accepted on {path}")

    def cli(self, *args: str) -> object:
        out = subprocess.run([self.binary, *args, "--json"], cwd=self.dir,
                             env={**os.environ, "GRID_HINTS": "0"},
                             capture_output=True, check=False)
        try:
            return json.loads(out.stdout)
        except ValueError as e:
            raise AssertionError(f"grid {args} --json: {e}\n{out.stdout!r}") from e

    def same(self, verb: list[str], answer: object) -> None:
        self.assertEqual(without_stats(self.cli(*verb)), without_stats(answer), f"grid {verb[0]}")

    def test_the_client_answers_what_the_cli_prints(self) -> None:
        self.same(["version"], self.client.version())

        defs = self.client.resolve(locator={"file": FILE, "line": 1})
        self.same(["resolve", f"{FILE}:1"], defs)

        # Read out of the answer above rather than guessed at: the differential is
        # about the two paths agreeing, not about how the Python frontend spells
        # a name.
        callsign = defs[0]["callsign"]

        base: SnapshotOrRevGitRev = {"git_rev": GitRev("HEAD~1")}
        head: SnapshotOrRevGitRev = {"git_rev": GitRev("HEAD")}
        self.same(["diff"], self.client.delta(base=base, head=head))
        self.same(["drift"], self.client.drift(base=base, head=head))
        self.same(["scope", callsign], self.client.scope(callsigns=[CallsignPattern(callsign)]))


if __name__ == "__main__":
    unittest.main()
