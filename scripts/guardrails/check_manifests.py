#!/usr/bin/env python3
"""Manifest law. Ported from yi's check_manifests.py, whose incidents were codex's
workspace lints silently skipped without a per-crate opt-in, its two grandfathered
folder/crate name mismatches, and jcode's 84 crates frozen at 0.1.0 under a moving root.

Grid adds the rule yi does not need. Cargo's [lints] table is all-or-nothing, so
crates/chart-io cannot say `workspace = true` and still relax unsafe_code for the mmap;
its hand-written tables must therefore be a superset of the workspace's at the same or a
stricter level, or every lint added to the workspace quietly misses the one unsafe crate.

The dependency law is a review rule for Rust, because every package entering Cargo.lock is
visible in the diff of every pull request. Under clients/ there is no lockfile to read, so
the law is mechanical instead: a client ships against its language's standard library and
nothing else, and the first dependency, extra or lockfile fails here."""
import re, sys, tomllib, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, fail

# Folder x/ is crate grid-x. crates/grid/ is the binary and keeps the bare name.
BARE = "grid"
# The naming law by analogy: the package under clients/<lang>/ is grid-client in that
# language's spelling (grid-client distribution, grid_client import).
CLIENTS = ROOT / "clients"
CLIENT_NAME = "grid-client"
# Every spelling of "something was installed here" that can appear under clients/.
# pyvenv.cfg is the venv itself, which self-ignores (its own .gitignore is one `*`
# line) and is therefore invisible to git status, check_blob_size.py and the
# "the gate wrote nothing" invariant alike: this check is the only thing that sees it.
INSTALLED = ("uv.lock", "poetry.lock", "pdm.lock", "requirements*.txt", "Pipfile",
             "Pipfile.lock", "pyvenv.cfg", "*.egg-info")
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

# clients/ is absent in a checkout that predates the SDK wave, and an absent client has no
# dependency to declare; only a present one is held to the law.
for m in sorted(CLIENTS.glob("*/pyproject.toml")):
    t = tomllib.loads(m.read_text())
    project = t.get("project", {})
    if project.get("name") != CLIENT_NAME:
        errs.append(f"{m}: project.name {project.get('name')!r} != {CLIENT_NAME!r}")
    if project.get("dependencies"):
        errs.append(f"{m}: dependencies must stay empty — the client's TCB is the standard library")
    for extra, deps in project.get("optional-dependencies", {}).items():
        if deps:
            errs.append(f"{m}: optional-dependencies.{extra} must stay empty — an extra is a dependency")
    # PEP 735 groups are what `uv sync` and `pip install --group` install from, and
    # build-system.requires is what pip fetches to build the package at all. Neither
    # is project.dependencies, and both are a package entering the client's TCB.
    for group, deps in t.get("dependency-groups", {}).items():
        if deps:
            errs.append(f"{m}: dependency-groups.{group} must stay empty — a dev group is a dependency")
    build = t.get("build-system", {})
    backend = build.get("build-backend", "").split(".")[0].lower().replace("_", "-")
    for req in build.get("requires", []):
        name = re.split(r"[<>=!~;\[ ]", req, maxsplit=1)[0].strip().lower().replace("_", "-")
        if name != backend:
            errs.append(f"{m}: build-system.requires {req!r} is not the {backend!r} backend — a build requirement is a package")

for pattern in INSTALLED:
    for found in sorted(CLIENTS.glob(f"**/{pattern}")):
        errs.append(f"{found}: {found.name} under clients/ means something is being installed")

fail(errs, "manifests")
