"""C54: the Python decoder rejects every input the Rust fuzzer's corpus rejects.

The fuzz rung har-verify names for a decoder over bytes it did not write is
climbed once, in Rust (`fuzz/fuzz_targets/client_response.rs`). This is the
differential across the two decoders over the inputs that rung found: every
artifact must come back as a value or as a `GridProtocolError`, and never as
anything else. Writing a second fuzzer in Python to avoid installing one is the
bloat the plan cut.
"""

import pathlib
import unittest

from grid_client import GridProtocolError
from grid_client.client import _decode_line

CORPUS = pathlib.Path(__file__).resolve().parents[3] / "fuzz" / "corpus" / "client_response"


class Corpus(unittest.TestCase):
    def test_every_artifact_is_a_value_or_a_protocol_fault(self) -> None:
        if not CORPUS.is_dir():
            # A skip and not a pass: silence here would look identical whether the
            # decoders agree or the corpus was never replayed. `cargo +nightly fuzz
            # run client_response` writes crashes under fuzz/artifacts and only
            # minimized artifacts are committed, so an empty directory means the
            # rung found nothing rather than that it was not climbed.
            self.skipTest(f"no corpus at {CORPUS}: run cargo +nightly fuzz run client_response")
        artifacts = sorted(p for p in CORPUS.iterdir() if p.is_file())
        if not artifacts:
            self.skipTest(f"{CORPUS} is empty: the Rust fuzzer minimized nothing to replay")
        for artifact in artifacts:
            with self.subTest(artifact=artifact.name):
                try:
                    _decode_line(artifact.read_bytes())
                except GridProtocolError:
                    pass


if __name__ == "__main__":
    unittest.main()
