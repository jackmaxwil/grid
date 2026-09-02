#!/usr/bin/env python3
"""No committed or untracked file over 512 KB outside the allowlist. Ported from yi's
check_blob_size.py (itself from codex's blob-size-policy). It fires on untracked blobs too,
which is the point: a stray artifact in the tree is reported, never allowlisted away."""
import sys, subprocess, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, BASE, fail

LIMIT = 512_000
allow = {l.strip() for l in (BASE / "blob_allowlist.txt").read_text().splitlines()
         if l.strip() and not l.startswith("#")}
out = subprocess.run(["git", "ls-files", "-co", "--exclude-standard"], cwd=ROOT,
                     capture_output=True, text=True, check=True).stdout.split("\n")
errs = []
for rel in filter(None, out):
    p = ROOT / rel
    if rel not in allow and p.is_file() and p.stat().st_size > LIMIT:
        errs.append(f"{rel}: {p.stat().st_size} bytes > {LIMIT} (allowlist: baselines/blob_allowlist.txt)")
fail(errs, "blob_size")
