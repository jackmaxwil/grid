#!/usr/bin/env python3
"""crates/wire is generated; this proves the committed tree is still what generated it.

yi's equivalent is check_schemas_lock.py: every serialized shape is normalized into a
committed lock so a schema edit is a reviewed diff and never a silent drift. Grid's schema
source of truth is not a crate but docs/wire-v1.openrpc.json, from which scripts/gen-wire.py
writes crates/wire/src. The generator is therefore the lock, and the check is: regenerate,
format, compare. A hand edit to a generated file, or a contract change whose regeneration was
never run, fails here rather than shipping as a wire the daemon and its callers disagree about.

This check writes nothing in the working tree. An earlier version regenerated over
crates/wire/src and restored it in a `finally`, and perturbed the contract in place the same
way, so a Ctrl-C, a CI step timeout or an OOM inside either window left the generated crate
deleted or the contract holding a renamed method. Everything now happens in a scratch copy,
including `--selfcheck`'s perturbation, which proves the comparison can fail: a gate whose own
logic is untested passes for the wrong reason."""
import filecmp, shutil, subprocess, sys, tempfile, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, fail

OUT = ROOT / "crates/wire/src"
GEN = ROOT / "scripts/gen-wire.py"
CONTRACT = ROOT / "docs/wire-v1.openrpc.json"


def differing(a, b):
    """Relative paths under `a` and `b` that are missing on one side or differ in bytes."""
    left = {p.relative_to(a) for p in a.rglob("*") if p.is_file()}
    right = {p.relative_to(b) for p in b.rglob("*") if p.is_file()}
    out = sorted(str(p) for p in left ^ right)
    for rel in sorted(left & right):
        if not filecmp.cmp(a / rel, b / rel, shallow=False):
            out.append(str(rel))
    return out


def regenerate(root):
    """Run the generator over a copy of the tree and hand back its formatted output."""
    out = root / OUT.relative_to(ROOT)
    run = subprocess.run([sys.executable, str(root / GEN.relative_to(ROOT))],
                         cwd=root, capture_output=True, text=True)
    if run.returncode != 0:
        return None, f"gen-wire.py exited {run.returncode}: {run.stderr.strip()}"
    # The committed sources are rustfmt's output, so the raw generator output is
    # normalized the same way before the comparison rather than diffed against it.
    fmt = subprocess.run(["rustfmt", "--edition", "2021", *[str(p) for p in out.rglob("*.rs")]],
                         cwd=root, capture_output=True, text=True)
    if fmt.returncode != 0:
        return None, f"rustfmt on the generated sources exited {fmt.returncode}: {fmt.stderr.strip()}"
    return out, None


def check(contract=None):
    """Regenerate into a scratch tree and compare; `contract` overrides the contract bytes."""
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp) / "root"
        # Everything gen-wire.py reads and writes, and nothing else: the contract, the
        # generator, and the output directory it overwrites.
        (root / "docs").mkdir(parents=True)
        (root / "scripts").mkdir(parents=True)
        (root / CONTRACT.relative_to(ROOT)).write_bytes(
            CONTRACT.read_bytes() if contract is None else contract)
        shutil.copy2(GEN, root / GEN.relative_to(ROOT))
        shutil.copytree(OUT, root / OUT.relative_to(ROOT))
        after, err = regenerate(root)
        if err:
            return [err]
        drift = differing(OUT, after)
    return [f"crates/wire/src/{d}: not what gen-wire.py writes from {CONTRACT.name}"
            for d in drift] + (["regenerate deliberately: python3 scripts/gen-wire.py && cargo fmt --all"]
                               if drift else [])


def selfcheck():
    """Regenerate from a perturbed contract and prove the comparison notices."""
    perturbed = CONTRACT.read_bytes().replace(b'"grid.version"', b'"grid.selfcheck"', 1)
    if not check(perturbed):
        return ["--selfcheck: a renamed wire method produced no drift; the comparison is blind"]
    return check()


fail(selfcheck() if "--selfcheck" in sys.argv else check(), "wire_drift")
