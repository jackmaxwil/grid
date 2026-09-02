#!/usr/bin/env python3
"""Every GRID_* env var is a declared row; hard cap 16. Ported from yi's check_env_surface.py,
whose incident was jcode's 517 JCODE_* vars, 87% of them undocumented. An env var is a public
interface nobody reviews, which is why it costs a row here before it costs a line of code."""
import json, re, sys, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, BASE, src_files, fail

CAP = 16
declared = json.loads((BASE / "env_vars.json").read_text())
found = set()
for f in src_files():
    found |= set(re.findall(r"GRID_[A-Z_]+", f.read_text()))
errs = [f"undeclared env var {v} (add it to baselines/env_vars.json with what it does)"
        for v in sorted(found - set(declared))]
if len(declared) > CAP:
    errs.append(f"env surface {len(declared)} > hard cap {CAP}")
for v in sorted(set(declared) - found):
    errs.append(f"declared env var {v} is read nowhere in src (stale row = error)")
fail(errs, f"env_surface ({len(found)} used / {len(declared)} declared)")
