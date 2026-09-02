# grid's verbs. `just check` is the gate; everything else is a lane of it or a deeper rung.
set positional-arguments := true

default: check

[group('build')]
build *ARGS:
    cargo build --workspace {{ARGS}}

[group('gate')]
fmt:
    cargo fmt --all

[group('gate')]
fmt-check:
    cargo fmt --all --check

[group('gate')]
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# nextest when it is on PATH, cargo when it is not, so a fresh clone still runs the suite.
# nextest pools every test from every binary and .config/nextest.toml caps the ones that
# spawn the grid binary; it cannot run doctests, so those stay a second invocation rather
# than quietly leaving the gate.
[group('gate')]
test *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v cargo-nextest >/dev/null && [ $# -eq 0 ]; then
      cargo nextest run --workspace
      cargo test --workspace --doc
    else
      cargo test --workspace "$@"
    fi

# The lanes `check` runs, named so CI can run them as separate jobs.
[group('gate')]
lint: fmt-check clippy

[group('gate')]
guardrails:
    bash scripts/check-guardrails.sh

# The build-free subset, which is what keeps pre-commit under a couple of seconds.
[group('gate')]
guardrails-fast:
    bash scripts/check-guardrails.sh --fast

[group('gate')]
check: lint guardrails test

# The assurance lane on its own: the claim table and its two grep-gates, plus the deeper
# verification rungs a claim's evidence column names. `verify` needs cargo-kani and `miri`
# a nightly toolchain, so this is a deliberate verb, never part of `check`.
[group('gate')]
har: guardrails miri verify

# One lane of the gate, carrying the invariant. `lint`, `guardrails` and `test` do not
# depend on each other, so CI runs them as parallel jobs and pays the longest rather than
# the sum; the invariant has to ride the lane rather than the whole gate, or a split run
# stops asserting it.
[group('gate')]
lane name:
    #!/usr/bin/env bash
    set -euo pipefail
    # Not "is the tree clean" — a run with unrelated WIP is normal. The invariant is that
    # the gate wrote nothing: crates/wire is generated and the baselines are generated, and
    # one left behind fails on the next machine.
    before=$(git status --porcelain)
    just {{name}}
    if [ "$before" != "$(git status --porcelain)" ]; then
      echo "the gate regenerated files — commit them:"
      git status --porcelain
      exit 1
    fi

# Tier 1, every commit: source-only checks, no build and no suite.
[group('gate')]
precommit: fmt-check clippy guardrails-fast

# Tier 2, every push: the full gate, and nothing the gate itself regenerated.
[group('gate')]
prepush: (lane "check")

# Point git at the versioned hooks (per-repo, shared by every worktree). Run by a person.
[group('meta')]
hooks:
    git config core.hooksPath .githooks
    @echo "hooks installed: .githooks"

# Every worktree with its branch and dirt, and the stash they all share.
[group('meta')]
worktrees:
    #!/usr/bin/env bash
    set -euo pipefail
    git worktree list --porcelain | sed -n 's/^worktree //p' | while read -r path; do
      branch=$(git -C "$path" symbolic-ref --quiet --short HEAD || echo "(detached)")
      count=$(git -C "$path" status --porcelain | wc -l | tr -d ' ')
      [ "$count" = 0 ] && state=clean || state="$count dirty"
      printf '%-34s %-12s %s\n' "$branch" "$state" "$path"
    done
    # The stash stack is one stack for every worktree, which is why `git stash pop` here
    # can take a concurrent session's work.
    echo "stash: $(git stash list | wc -l | tr -d ' ') entries, shared by all of the above"

# Regenerate crates/wire from the contract. check_wire_drift.py fails when this is owed.
[group('meta')]
wire:
    python3 scripts/gen-wire.py
    cargo fmt --all

[group('test')]
fuzz *ARGS:
    cargo +nightly fuzz run {{ARGS}}

[group('test')]
verify *ARGS:
    cargo kani {{ARGS}}

# Crash-safety soak: the PR-time loop is short, this is the 10k-iteration one.
[group('test')]
soak *ARGS:
    GRID_FAULT_ITERS=10000 cargo test -p grid-chart-io --release --test crash_safety -- --nocapture {{ARGS}}

# The one unsafe block, under the tools that can see through it.
[group('test')]
miri *ARGS:
    MIRIFLAGS=-Zmiri-strict-provenance cargo +nightly miri test -p grid-chart --test adversarial {{ARGS}}

[group('build')]
bench *ARGS:
    cargo bench --workspace {{ARGS}}

[group('run')]
serve *ARGS:
    cargo run -p grid -- serve {{ARGS}}

[group('meta')]
deny:
    cargo deny check
