#!/usr/bin/env python3
"""The generated trees are generated; this proves the committed ones are still what wrote them.

yi's equivalent is check_schemas_lock.py: every serialized shape is normalized into a
committed lock so a schema edit is a reviewed diff and never a silent drift. Grid's schema
source of truth is not a crate but docs/wire-v1.openrpc.json, from which scripts/gen-wire.py
writes every directory in OUTS. The generator is therefore the lock, and the check is:
regenerate, format, compare. A hand edit to a generated file, or a contract change whose
regeneration was never run, fails here rather than shipping as a wire the daemon and its
callers disagree about.

This check writes nothing in the working tree. An earlier version regenerated over
crates/wire/src and restored it in a `finally`, and perturbed the contract in place the same
way, so a Ctrl-C, a CI step timeout or an OOM inside either window left the generated crate
deleted or the contract holding a renamed method. Everything now happens in a scratch copy,
including `--selfcheck`'s perturbation, which proves the comparison can fail: a gate whose own
logic is untested passes for the wrong reason."""
import filecmp, shutil, subprocess, sys, tempfile, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, fail

OUTS = [ROOT / "crates/wire/src", ROOT / "clients/python/grid_client"]
GEN = ROOT / "scripts/gen-wire.py"
CONTRACT = ROOT / "docs/wire-v1.openrpc.json"
# The --selfcheck perturbation: a wire method renamed in the contract. Held as bytes
# because both halves are used — one to perturb, one to find the committed files that
# spell the old name and must therefore drift.
RENAME = (b'"grid.version"', b'"grid.selfcheck"')


# Not generated and not committed: a `.pyc` written by any run that imported the
# client would otherwise read as drift on whichever side wrote it.
IGNORE = shutil.ignore_patterns("__pycache__")


def files(root):
    return {p.relative_to(root) for p in root.rglob("*")
            if p.is_file() and "__pycache__" not in p.parts}


def differing(a, b):
    """Relative paths under `a` and `b` that are missing on one side or differ in bytes."""
    left, right = files(a), files(b)
    out = sorted(str(p) for p in left ^ right)
    for rel in sorted(left & right):
        if not filecmp.cmp(a / rel, b / rel, shallow=False):
            out.append(str(rel))
    return out


def regenerate(root):
    """Run the generator over a copy of the tree and format its Rust output in place."""
    run = subprocess.run([sys.executable, str(root / GEN.relative_to(ROOT))],
                         cwd=root, capture_output=True, text=True)
    if run.returncode != 0:
        return f"gen-wire.py exited {run.returncode}: {run.stderr.strip()}"
    # The committed sources are rustfmt's output, so the raw generator output is
    # normalized the same way before the comparison rather than diffed against it.
    # Only the Rust half: there is no formatter for the other languages in the TCB,
    # and rustfmt with an empty file list reads stdin and hangs the gate forever.
    rs = [str(p) for out in OUTS for p in (root / out.relative_to(ROOT)).rglob("*.rs")]
    if not rs:
        return None
    fmt = subprocess.run(["rustfmt", "--edition", "2021", *rs],
                         cwd=root, capture_output=True, text=True)
    if fmt.returncode != 0:
        return f"rustfmt on the generated sources exited {fmt.returncode}: {fmt.stderr.strip()}"
    return None


def drifted(contract=None):
    """Regenerate into a scratch tree; return (paths relative to ROOT that differ, errors)."""
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp) / "root"
        # Everything gen-wire.py reads and writes, and nothing else: the contract, the
        # generator, and the output directories it overwrites.
        (root / "docs").mkdir(parents=True)
        (root / "scripts").mkdir(parents=True)
        (root / CONTRACT.relative_to(ROOT)).write_bytes(
            CONTRACT.read_bytes() if contract is None else contract)
        shutil.copy2(GEN, root / GEN.relative_to(ROOT))
        for out in OUTS:
            shutil.copytree(out, root / out.relative_to(ROOT),
                            ignore=IGNORE)
        err = regenerate(root)
        if err:
            return [], [err]
        return [f"{out.relative_to(ROOT)}/{d}"
                for out in OUTS
                for d in differing(out, root / out.relative_to(ROOT))], []


def check(contract=None):
    paths, errs = drifted(contract)
    return errs + [f"{p}: not what gen-wire.py writes from {CONTRACT.name}" for p in paths] + (
        ["regenerate deliberately: python3 scripts/gen-wire.py && cargo fmt --all"]
        if paths else [])


def selfcheck():
    """Rename a method in a scratch contract; every file that spells one must drift.

    Which files those are is read off the committed outputs rather than listed here, so an
    emitter added later for another language is covered the day it lands: if it writes the
    method names and then stops reading them from the contract, this fires.
    """
    want = {f"{out.relative_to(ROOT)}/{f.relative_to(out)}"
            for out in OUTS for f in out.rglob("*")
            if f.is_file() and RENAME[0] in f.read_bytes()}
    if not want:
        return ["--selfcheck: no generated file spells a method name, so a rename proves "
                "nothing; the perturbation is blind"]
    paths, errs = drifted(CONTRACT.read_bytes().replace(*RENAME, 1))
    if errs:
        return errs
    blind = sorted(want - set(paths))
    return [f"--selfcheck: {p} spells {RENAME[0].decode()} but a renamed method left it "
            "unchanged; the comparison is blind there" for p in blind] or check()


fail(selfcheck() if "--selfcheck" in sys.argv else check(), "wire_drift")
