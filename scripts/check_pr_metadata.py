#!/usr/bin/env python3
"""PR body, judged from CI where the forge is reachable. Ported from yi's
scripts/guardrails/check_pr_metadata.py, trimmed to grid's own rule.

A diff that touches charted source — `.rs`, `.py`, `.pyi`, the same extensions
self-gate in .forgejo/workflows/grid-gate.yml counts to decide whether a plan
is owed — is work, and work has an issue. Such a PR names it in its body:
`Closes #N` finishing it, `Refs #N` as part of it. `#N` must be open, carry
exactly one `size:` label, at least one `area:` label, and a milestone. Every
other failure prints the `tea` command that fixes it.

Grid has no changelog convention (no README/docs `## Changelog` section, no
per-release table), so yi's rule that a changelog row must cite the same
issue number has no equivalent here and is dropped rather than faked.

This gate never runs offline: it asks the forge questions a machine without
one cannot answer, and a gate that guesses is worse than no gate. The tree's
own check of it is `--selfcheck`, over a fake transport that never leaves the
process.
"""
import json
import os
import pathlib
import re
import sys
import urllib.error
import urllib.request

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "guardrails"))
from _common import ROOT, fail  # noqa: E402

# Same extensions self-gate's `grep -cE '\.(rs|py|pyi)$'` counts (grid-gate.yml).
CHARTED = re.compile(r"\.(rs|py|pyi)$")
# `Closes apex/grid#4` is the same citation as `Closes #4`; `Closes other/repo#4`
# is a citation of someone else's register and does not count as one here.
CITE = re.compile(r"\b(closes|refs)\s+(?:([\w.-]+/[\w.-]+))?#(\d+)\b", re.I)


def send(method, url, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Authorization": f"token {os.environ['FORGEJO_TOKEN']}",
            "Accept": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req) as response:
            return json.load(response)
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return None
        raise


def git(*args):
    import subprocess

    return subprocess.run(
        ("git", "-C", str(ROOT)) + args, capture_output=True, text=True, check=False
    )


def base_commit():
    for ref in ("origin/main", "main"):
        if git("rev-parse", "--verify", "--quiet", ref).returncode == 0:
            found = git("merge-base", ref, "HEAD")
            if found.returncode == 0:
                return found.stdout.strip()
    return None


def touches_charted_source(base):
    """Whether this branch's diff against `base` touches any .rs/.py/.pyi file."""
    out = git("diff", "--name-only", base, "HEAD")
    return any(CHARTED.search(path) for path in out.stdout.splitlines())


def cited(repo, body):
    """The issue numbers this body claims, in order, deduplicated."""
    out = []
    for _, named, number in CITE.findall(body or ""):
        if named and named != repo:
            continue
        if int(number) not in out:
            out.append(int(number))
    return out


def issue_problems(transport, api, repo, number):
    issue = transport("GET", f"{api}/repos/{repo}/issues/{number}")
    if not issue:
        return [
            f"#{number} is not an issue on {repo}",
            "  tea issues ls          # cite one that exists, or open it",
        ]
    errs = []
    if issue.get("state") != "open":
        errs += [
            f"#{number} is {issue.get('state')}; a merge cannot close it again",
            f"  tea issues reopen {number}   # or cite the issue this work is really under",
        ]
    labels = [label.get("name", "") for label in issue.get("labels") or []]
    sizes = sorted(name for name in labels if name.startswith("size:"))
    areas = [name for name in labels if name.startswith("area:")]
    if not sizes:
        errs += [
            f"#{number} carries no `size:` label; exactly one is mandatory",
            f"  tea issues edit {number} --add-labels size:M",
        ]
    elif len(sizes) > 1:
        errs += [
            f"#{number} carries {len(sizes)} size labels ({', '.join(sizes)}); exactly one is",
            f"  tea issues edit {number} --remove-labels {','.join(sizes[1:])}",
        ]
    if not areas:
        errs += [
            f"#{number} carries no `area:` label",
            f"  tea issues edit {number} --add-labels area:runtime",
        ]
    if not issue.get("milestone"):
        errs += [
            f"#{number} has no milestone, and a milestone is what a due date is computed over",
            f'  tea issues edit {number} --milestone "<name>"',
        ]
    return errs


def body_problems(transport, api, repo, body, src_touched):
    """The whole body rule, pure but for `transport`."""
    if not src_touched:
        return []

    numbers = cited(repo, body)
    if not numbers:
        return [
            "the PR body names no issue, and it touches charted source (.rs, .py, .pyi)",
            "  add `Closes #N` (this PR finishes it) or `Refs #N` (this PR is part of it)",
            "  tea issues ls          # it is probably already open",
            '  tea issues create --title "<imperative sentence>" \\',
            '      --labels size:M,area:runtime --milestone "<name>"',
        ]
    errs = []
    for number in numbers:
        errs += issue_problems(transport, api, repo, number)
    return errs


def selfcheck():
    """Every branch, against a transport that never leaves the process."""

    def forge(issues):
        seen = []

        def transport(method, url, payload=None):
            seen.append((method, url))
            return issues.get(int(url.rsplit("/", 1)[-1]))

        return transport, seen

    api, repo = "https://git.example/api/v1", "apex/grid"
    good = {
        "state": "open",
        "labels": [{"name": "size:M"}, {"name": "area:runtime"}],
        "milestone": {"title": "Tracking"},
    }

    # --- the citation reader
    assert cited(repo, "Closes #4 and refs #5, Refs #4") == [4, 5]
    assert cited(repo, "closes apex/grid#7") == [7]
    assert cited(repo, "Closes other/repo#7") == [], "another register is not this one"
    assert cited(repo, "see issue 7 (#7)") == [], "a bare number is not a citation"
    assert cited(repo, None) == []

    # --- exempt: no charted source touched. Silent, and the forge is not asked.
    transport, seen = forge({})
    assert body_problems(transport, api, repo, "", False) == []
    assert body_problems(transport, api, repo, "no citation either", False) == []
    assert seen == [], seen

    # --- required and uncited
    transport, seen = forge({})
    errs = body_problems(transport, api, repo, "no citation here", True)
    assert "touches charted source" in errs[0], errs
    assert any("tea issues create" in e for e in errs), errs
    assert seen == [], "an uncited body has nothing to ask the forge about"
    # A citation of another repository's register does not satisfy this one.
    errs = body_problems(transport, api, repo, "Closes other/repo#3", True)
    assert errs and "names no issue" in errs[0], errs

    # --- cited and clean
    transport, seen = forge({4: good})
    assert body_problems(transport, api, repo, "Closes #4", True) == []
    assert seen == [("GET", f"{api}/repos/{repo}/issues/4")], seen
    assert body_problems(transport, api, repo, "Refs #4", True) == []

    # --- each issue defect, each with the tea command that fixes it
    def only(issue, body="Closes #4"):
        transport, _ = forge({4: issue})
        return body_problems(transport, api, repo, body, True)

    errs = only(None)
    assert "not an issue" in errs[0] and "tea issues ls" in errs[1], errs
    errs = only(dict(good, state="closed"))
    assert "is closed" in errs[0] and "tea issues reopen 4" in errs[1], errs
    errs = only(dict(good, labels=[{"name": "area:runtime"}]))
    assert "no `size:` label" in errs[0] and "--add-labels size:M" in errs[1], errs
    errs = only(dict(good, labels=[{"name": "size:S"}, {"name": "size:L"}, {"name": "area:tui"}]))
    assert "2 size labels" in errs[0] and "--remove-labels size:S" in errs[1], errs
    errs = only(dict(good, labels=[{"name": "size:M"}]))
    assert "no `area:` label" in errs[0], errs
    errs = only(dict(good, milestone=None))
    assert "no milestone" in errs[0] and "--milestone" in errs[1], errs
    # Every cited number is judged, not just the first.
    transport, seen = forge({4: good, 5: dict(good, milestone=None)})
    errs = body_problems(transport, api, repo, "Refs #4, closes #5", True)
    assert len(errs) == 2 and errs[0].startswith("#5 has no milestone"), errs

    print("ok   pr_metadata selfcheck")


def main(argv):
    if "--selfcheck" in argv:
        selfcheck()
        return 0
    base = base_commit()
    if base is None:
        fail(["no origin/main or main to diff against; this gate cannot pass blind"], "pr_metadata")
    errs = body_problems(
        send,
        os.environ["FORGEJO_API_URL"],
        os.environ["FORGEJO_REPOSITORY"],
        os.environ.get("PR_BODY", ""),
        touches_charted_source(base),
    )
    fail(errs, "pr_metadata")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
