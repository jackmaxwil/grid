#!/usr/bin/env python3
"""crates/wire is generated; this proves the committed tree is still what generated it.

yi's equivalent is check_schemas_lock.py: every serialized shape is normalized into a
committed lock so a schema edit is a reviewed diff and never a silent drift. Grid's schema
source of truth is not a crate but docs/wire-v1.openrpc.json, from which scripts/gen-wire.py
writes crates/wire/src. The generator is therefore the lock, and the check is: regenerate,
format, compare. A hand edit to a generated file, or a contract change whose regeneration was
never run, fails here rather than shipping as a wire the daemon and its callers disagree about.

The generator writes in place, so the committed tree is copied out first and restored on every
exit path, including a failing one. `--selfcheck` proves the comparison can fail: a gate whose
own logic is untested passes for the wrong reason."""
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


def regenerate(scratch):
    """Run the generator over the working tree and hand back the formatted result."""
    run = subprocess.run([sys.executable, str(GEN)], cwd=ROOT, capture_output=True, text=True)
    if run.returncode != 0:
        return None, f"gen-wire.py exited {run.returncode}: {run.stderr.strip()}"
    # The committed sources are rustfmt's output, so the raw generator output is
    # normalized the same way before the comparison rather than diffed against it.
    fmt = subprocess.run(["rustfmt", "--edition", "2021", *[str(p) for p in OUT.rglob("*.rs")]],
                         cwd=ROOT, capture_output=True, text=True)
    if fmt.returncode != 0:
        return None, f"rustfmt on the generated sources exited {fmt.returncode}: {fmt.stderr.strip()}"
    shutil.copytree(OUT, scratch / "after")
    return scratch / "after", None


def check():
    with tempfile.TemporaryDirectory() as tmp:
        scratch = pathlib.Path(tmp)
        shutil.copytree(OUT, scratch / "before")
        try:
            after, err = regenerate(scratch)
            if err:
                return [err]
            drift = differing(scratch / "before", after)
        finally:
            shutil.rmtree(OUT)
            shutil.copytree(scratch / "before", OUT)
    return [f"crates/wire/src/{d}: not what gen-wire.py writes from {CONTRACT.name}"
            for d in drift] + (["regenerate deliberately: python3 scripts/gen-wire.py && cargo fmt --all"]
                               if drift else [])


def selfcheck():
    """Perturb the contract, prove the comparison notices, and put it back."""
    original = CONTRACT.read_bytes()
    try:
        CONTRACT.write_bytes(original.replace(b'"grid.version"', b'"grid.selfcheck"', 1))
        if not check():
            return ["--selfcheck: a renamed wire method produced no drift; the comparison is blind"]
    finally:
        CONTRACT.write_bytes(original)
    return check()


fail(selfcheck() if "--selfcheck" in sys.argv else check(), "wire_drift")
