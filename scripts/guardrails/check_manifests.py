#!/usr/bin/env python3
"""Manifest law. Ported from yi's check_manifests.py, whose incidents were codex's
workspace lints silently skipped without a per-crate opt-in, its two grandfathered
folder/crate name mismatches, and jcode's 84 crates frozen at 0.1.0 under a moving root.

Grid adds the rule yi does not need. Cargo's [lints] table is all-or-nothing, so
crates/chart-io cannot say `workspace = true` and still relax unsafe_code for the mmap;
its hand-written tables must therefore be a superset of the workspace's at the same or a
stricter level, or every lint added to the workspace quietly misses the one unsafe crate."""
import sys, tomllib, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, fail

# Folder x/ is crate grid-x. crates/grid/ is the binary and keeps the bare name.
BARE = "grid"
# The one crate allowed a hand-written [lints] table, and the one lint it may relax.
LINTS_EXCEPTION = {"grid-chart-io": {("rust", "unsafe_code")}}
LEVELS = ["allow", "warn", "deny", "forbid"]


def levels(table):
    """{(tool, lint): level} from a [lints.<tool>] table, both the string and dict forms."""
    out = {}
    for tool, lints in table.items():
        if not isinstance(lints, dict):
            continue
        for lint, spec in lints.items():
            level = spec.get("level") if isinstance(spec, dict) else spec
            if isinstance(level, str):
                out[(tool, lint)] = level
    return out


root = tomllib.loads((ROOT / "Cargo.toml").read_text())
want = levels(root["workspace"].get("lints", {}))

errs = []
for m in sorted((ROOT / "crates").glob("*/Cargo.toml")):
    folder = m.parent.name
    t = tomllib.loads(m.read_text())
    pkg = t.get("package", {})
    name = pkg.get("name", "")
    expected = BARE if folder == BARE else f"{BARE}-{folder}"
    if name != expected:
        errs.append(f"{m}: crate {name!r} != {expected} (naming law: folder x -> grid-x)")
    for field in ("edition", "license"):
        if pkg.get(field) != {"workspace": True}:
            errs.append(f"{m}: {field} must be {field}.workspace = true")
    if not pkg.get("version"):
        errs.append(f"{m}: no version (path deps pin one, so it cannot be omitted)")

    lints = t.get("lints", {})
    if lints == {"workspace": True}:
        continue
    if name not in LINTS_EXCEPTION:
        errs.append(f"{m}: missing [lints] workspace = true (workspace lints are decorative without it)")
        continue
    have = levels(lints)
    for key, level in sorted(want.items()):
        if key in LINTS_EXCEPTION[name]:
            continue
        mine = have.get(key)
        if mine is None:
            errs.append(f"{m}: [lints.{key[0]}] does not restate workspace {key[1]} = {level!r}")
        elif LEVELS.index(mine) < LEVELS.index(level):
            errs.append(f"{m}: {key[1]} is {mine!r}, weaker than the workspace's {level!r}")

fail(errs, "manifests")
