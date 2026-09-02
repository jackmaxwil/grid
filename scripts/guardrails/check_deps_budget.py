#!/usr/bin/env python3
"""Dependency counts, ratcheted. Ported from yi's check_deps_budget.py.

ASSURANCE.md's standing rule is that no new runtime dependency enters and every one that is
already here has a justification line. deny.toml names the crates that may never enter; this
counts the ones nobody thought to ban. Runtime graph only — dev-dependencies never ship, and
counting them would price a proptest harness like a parser."""
import json, subprocess, sys, tomllib, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, BASE, fail

budget = json.loads((BASE / "deps_budget.json").read_text())
internal = lambda d: d == "grid" or d.startswith("grid-")
direct = set()
for m in (ROOT / "crates").glob("*/Cargo.toml"):
    t = tomllib.loads(m.read_text())
    for table in ("dependencies", "build-dependencies"):
        direct |= {d for d in t.get(table, {}) if not internal(d)}
out = subprocess.run(["cargo", "tree", "-e", "normal,build", "--workspace", "--prefix", "none"],
                     cwd=ROOT, capture_output=True, text=True, check=True).stdout
trans = {l.split()[0] for l in out.splitlines() if l.strip()}
trans = {c for c in trans if c and not internal(c)}
errs = []
if len(direct) > budget["direct"]:
    errs.append(f"direct deps {len(direct)} > {budget['direct']}: {sorted(direct)}")
if len(trans) > budget["transitive"]:
    errs.append(f"transitive deps {len(trans)} > {budget['transitive']} "
                "(every one is a line in docs/ASSURANCE.md before it is a line in a manifest)")
if "--update" in sys.argv and not errs:
    (BASE / "deps_budget.json").write_text(
        json.dumps({"direct": len(direct), "transitive": len(trans)}, indent=2) + "\n")
    print(f"deps {budget} -> direct {len(direct)}, transitive {len(trans)}")
    sys.exit(0)
fail(errs, f"deps ({len(direct)}/{budget['direct']} direct, {len(trans)}/{budget['transitive']} transitive)")
