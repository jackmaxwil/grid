#!/usr/bin/env python3
"""Source shape, src/ only. Four of yi's guardrails asking one question, in one file:

- filenames (yi check_filenames.py): no part_01.rs / foo_02.rs. Bisection is not
  decomposition; jcode's part_NN.rs splits are the incident.
- glob re-exports (yi check_glob_reexport.py): zero, not budgeted. 63% of jcode was glued
  into one namespace by `pub use ...::*` chains and its crate split removed no imports.
- file ceiling (yi check_file_size.py): 1,200 lines.
- function ceiling (yi check_fn_size.py): 150 lines, brace-counted rather than parsed —
  good enough for a ceiling.

Three files and one function predate the ceilings. They are recorded in
baselines/hygiene_debt.json at their measured size and may only shrink; everything else
meets the ceiling outright. `--update` rewrites the file and refuses to raise a number,
because a ratchet that can be relaxed is a comment."""
import json, re, sys, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, BASE, src_files, prod_lines, fail

FILE_CEILING = 1200
FN_CEILING = 150
DEBT = BASE / "hygiene_debt.json"
BISECTED = re.compile(r"(part_\d+|_\d\d)\.rs$")
GLOB = re.compile(r"^\s*(pub\s+use\s+.*::\*|use\s+super::\*)")
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:const\s+)?(?:unsafe\s+)?fn\s+(\w+)")


def fn_sizes(lines):
    sizes, index = [], 0
    while index < len(lines):
        match = FN.match(lines[index])
        if not match or "{" not in "".join(lines[index : index + 3]):
            index += 1
            continue
        name, start, depth, opened = match.group(1), index, 0, False
        while index < len(lines):
            stripped = re.sub(r'"(?:[^"\\]|\\.)*"', '""', lines[index])
            stripped = re.sub(r"//.*", "", stripped)
            depth += stripped.count("{") - stripped.count("}")
            if stripped.count("{"):
                opened = True
            index += 1
            if opened and depth <= 0:
                break
        sizes.append((name, start + 1, index - start))
    return sizes


debt = json.loads(DEBT.read_text())
errs, measured = [], {"files": {}, "fns": {}}
for f in src_files():
    rel = str(f.relative_to(ROOT))
    if BISECTED.search(f.name):
        errs.append(f"{rel}: bisected filename (a split happens at a seam, never by line count)")
    size = len(f.read_text().splitlines())
    ceiling = debt["files"].get(rel, FILE_CEILING)
    if rel in debt["files"]:
        measured["files"][rel] = size
    if size > ceiling:
        errs.append(f"{rel}: {size} lines > {ceiling}")
    prod = prod_lines(f)
    for i, line in enumerate(prod, 1):
        if GLOB.match(line):
            errs.append(f"{rel}:{i}: {line.strip()}")
    for name, line, fsize in fn_sizes(prod):
        key = f"{rel}::{name}"
        ceiling = debt["fns"].get(key, FN_CEILING)
        if key in debt["fns"]:
            measured["fns"][key] = fsize
        if fsize > ceiling:
            errs.append(f"{rel}:{line} fn {name}: {fsize} lines > {ceiling}")

if "--update" in sys.argv:
    raised = [k for group in ("files", "fns") for k, v in measured[group].items() if v > debt[group][k]]
    if raised or errs:
        print("FAIL hygiene --update: the debt list only shrinks")
        for k in raised:
            print(f"  {k} grew")
        for e in errs:
            print(f"  {e}")
        sys.exit(1)
    DEBT.write_text(json.dumps(measured, indent=2, sort_keys=True) + "\n")
    print(f"ok   hygiene (debt updated: {len(measured['files'])} files, {len(measured['fns'])} fns)")
    sys.exit(0)

gone = [k for group in ("files", "fns") for k in debt[group] if k not in measured[group]]
for k in gone:
    errs.append(f"{k}: paid off — drop it from the debt list (`--update`)")
fail(errs, f"hygiene ({len(debt['files'])} file, {len(debt['fns'])} fn debts)")
