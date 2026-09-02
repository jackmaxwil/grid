#!/usr/bin/env python3
"""Commit subjects and PR titles. The machinery is yi's check_commit_style.py; two of its
rules are inverted, because grid's convention is the opposite of yi's and grid's wins.

- yi allows exactly one subject prefix (`Ratchet: `) and fails every other `word:` form,
  conventional-commit dialect included. Grid *is* conventional commits: `type(scope): subject`,
  from a closed type list, and a bare sentence fails instead.
- yi bans assistant co-author trailers outright, after finding one on every commit of five open
  PRs. Grid requires the `Co-Authored-By:` line as its authorship record, so it is allowed —
  and it is the only trailer that is, beside the ones git writes itself.

Everything else ports unchanged, except that yi's 72-character cap measures the sentence
rather than the whole line here: no terminal period, no WIP/fixup!,
git's own Merge/Revert subjects exempt, `HEAD --not origin/main` as the range so landed history
is never retroactively judged, and yi's composition rule — a guardrail baseline edit lands in
its own commit, never beside the code that moved it, or the ratchet proves nothing.

`--selfcheck` runs the rules over fixtures. A gate whose own logic is untested passes for the
wrong reason, which is yi's habit and worth keeping."""
import argparse, os, re, subprocess, sys, pathlib
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from _common import ROOT, fail

# yi caps a subject at 72, which is git's own number for a bare sentence. Grid's subjects
# carry a `type(scope): ` prefix that spends 10-16 characters before the sentence starts, so
# the cap applies to the sentence and the whole line gets a looser hard stop. Measured over
# grid's last 87 subjects: median sentence 58, p90 74.
LIMIT = 72
HARD_LIMIT = 100
# yi's device, for yi's reason: these rules arrived on a lineage already ahead of main and
# were retroactively red on subjects written before them. History before this commit is not
# judged, and the exclusion self-expires once the lineage lands.
STANDARD_STARTS = "7879003"
TYPES = ("feat", "fix", "docs", "test", "perf", "refactor", "chore", "build", "ci", "revert")
CONVENTIONAL = re.compile(r"^(" + "|".join(TYPES) + r")(\([a-z0-9][a-z0-9._-]*\))?!?: (.+)$")
GIT_NATIVE = re.compile(r"^(Merge|Revert)\b")
SCRATCH = re.compile(r"^(WIP\b|wip\b|fixup!|squash!|amend!)")
NOT_IMPERATIVE = re.compile(
    r"^(Added|Adding|Fixed|Fixes|Fixing|Updated|Updating|Removed|Removing"
    r"|Changed|Changing|Created|Creating|Deleted|Deleting|Renamed|Renaming"
    r"|Moved|Moving|Refactored|Refactoring|Implemented|Implementing"
    r"|Bumped|Bumping|Wrote|Made)\b", re.I)
# git's own trailer parser, not a regex over the body: a commit whose prose names a trailer
# would trip any line-matching pattern, which is how yi's first gate commit failed its own gate.
FORMAT = "%H%x00%s%x00%(trailers:only,unfold,separator=%x0b)%x1e"
TRAILER_OK = re.compile(r"^(Co-Authored-By|Signed-off-by)$", re.I)
BASELINES = "scripts/guardrails/baselines/"


def subject_errors(subject):
    if GIT_NATIVE.match(subject):
        return []
    errs = []
    if len(subject) > HARD_LIMIT:
        errs.append(f"{len(subject)} chars > {HARD_LIMIT}")
    if SCRATCH.match(subject):
        errs.append("WIP/fixup!/squash! subject — squash it before pushing")
        return errs
    match = CONVENTIONAL.match(subject)
    if not match:
        errs.append(f"not `type(scope): subject` with type in {'|'.join(TYPES)}")
        return errs
    body = match.group(3)
    if len(body) > LIMIT:
        errs.append(f"the sentence is {len(body)} chars > {LIMIT}")
    if body.endswith("."):
        errs.append("ends in a period")
    if body[:1].isupper() and not body.split()[0].isupper():
        errs.append("the subject after the colon is lowercase unless it opens with an identifier")
    if NOT_IMPERATIVE.match(body):
        errs.append("past or progressive verb — write the imperative ('add', not 'added')")
    return errs


def trailer_errors(trailers):
    errs = []
    for line in trailers.split("\x0b"):
        key = line.partition(":")[0].strip()
        if key and not TRAILER_OK.match(key):
            errs.append(f"unexpected trailer {key!r} (grid carries Co-Authored-By and nothing else)")
    return errs


def git(*args):
    return subprocess.run(("git", "-C", str(ROOT)) + args, capture_output=True, text=True, check=False)


def code_path(path):
    if path.startswith("crates/") and "/src/" in path:
        return True
    return path == "justfile" or (path.startswith("scripts/") and not path.startswith(BASELINES))


def composition_errors(sha):
    show = git("show", "--name-status", "--format=%P", sha)
    if show.returncode != 0:
        return [f"git show failed: {show.stderr.strip()}"]
    lines = show.stdout.splitlines()
    if lines and len(lines[0].split()) > 1:
        return []
    edits = [(l.split("\t")[0][:1], l.split("\t")[-1]) for l in lines[1:] if "\t" in l]
    baselines = sum(1 for status, f in edits if f.startswith(BASELINES) and status in "MD")
    code = sum(1 for _, f in edits if code_path(f))
    if baselines and code:
        return [f"{baselines} baseline edit(s) ride beside {code} code file(s); a baseline edit "
                "lands in its own commit (a new baseline may seed with its gate)"]
    return []


def upstream():
    for ref in ("origin/main", "main"):
        if git("rev-parse", "--verify", "--quiet", ref).returncode == 0:
            return ref
    return None


def selfcheck():
    good = ["feat(watch): signed-delta suspect maintenance",
            "chore(tooling): the lint wall, ported from yi",
            "fix: refuse a walk whose first layer did not finish",
            "Merge branch 'main' into topic"]
    bad = {"Add the drift verb": "not `type(scope): subject`",
           "feat(watch): added the suspect table": "past or progressive verb",
           "feat(watch): maintain the suspect table.": "ends in a period",
           "wip": "WIP/fixup!/squash!",
           "feat(x): " + "a" * (LIMIT + 1): f"sentence is {LIMIT + 1} chars > {LIMIT}"}
    errs = [f"--selfcheck: {s!r} should pass, got {subject_errors(s)}" for s in good if subject_errors(s)]
    for subject, want in bad.items():
        got = subject_errors(subject)
        if not any(want in e for e in got):
            errs.append(f"--selfcheck: {subject!r} should fail on {want!r}, got {got}")
    if trailer_errors("Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"):
        errs.append("--selfcheck: grid's own authorship trailer was rejected")
    if not trailer_errors("Reviewed-by: nobody"):
        errs.append("--selfcheck: an unknown trailer was accepted")
    return errs


ap = argparse.ArgumentParser()
ap.add_argument("--range", help="git revision range (default: HEAD --not origin/main)")
ap.add_argument("--title", help="check a PR title instead of a commit range")
ap.add_argument("--selfcheck", action="store_true", help="run the rules over fixtures")
args = ap.parse_args()

if args.selfcheck:
    fail(selfcheck(), "commit_style --selfcheck")
    sys.exit(0)

title = args.title or os.environ.get("PR_TITLE")
if title:
    fail([f"{title} — {e}" for e in subject_errors(title.strip())], "pr_title")
    sys.exit(0)

if args.range:
    span, hint = (args.range,), args.range
else:
    base = upstream()
    if base is None:
        print("FAIL commit_style")
        print("  no origin/main or main to compare against; a shallow clone cannot run this")
        print("  gate, and passing blind is what it exists to prevent")
        sys.exit(1)
    span, hint = ("HEAD", "--not", base), base
    if git("rev-parse", "--verify", "--quiet", STANDARD_STARTS + "^{commit}").returncode == 0:
        span += (STANDARD_STARTS + "^",)

log = git("log", f"--format={FORMAT}", *span)
if log.returncode != 0:
    print("FAIL commit_style")
    print(f"  git log failed: {log.stderr.strip()}")
    sys.exit(1)

errs, seen = [], 0
for record in log.stdout.split("\x1e"):
    if not record.strip():
        continue
    sha, subject, trailers = record.strip("\n").split("\x00", 2)
    seen += 1
    for e in subject_errors(subject) + trailer_errors(trailers) + composition_errors(sha):
        errs.append(f"{sha[:8]} {subject} — {e}")
if errs:
    errs.append(f"rewrite the messages: git rebase -i {hint}")
fail(errs, f"commit_style ({seen} commits)")
