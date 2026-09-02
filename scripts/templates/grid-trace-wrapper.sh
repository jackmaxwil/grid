#!/bin/sh
# One LLVM profile per nextest test — a template, not something grid installs.
#
# `grid test --trace` runs `cargo llvm-cov nextest` once per selected test,
# because a single profile cannot be attributed back to the tests that shared
# a process. nextest already runs each test in its own process and tells that
# process which test it is, so a wrapper can give each one its own profile
# and make the whole refresh one nextest run.
#
# Install (grid does none of this for you):
#   1. copy this file into your repository — `scripts/grid-trace-wrapper.sh`
#      is what the template config beside it names — and `chmod +x` it;
#   2. copy `nextest-trace.toml` to `.config/nextest.toml`, merging it with
#      whatever is already there;
#   3. `cargo llvm-cov nextest --no-report` writes one `.profraw` per test.
#
# The file name is the test id `grid test` speaks — `<binary-id> <name>` —
# with `%`, ` `, `/` and `:` percent-encoded, so an id survives a file name
# and comes back out of one. `grid test --trace` notices these files and says
# so; turning them into one trace per test without a second export is
# `docs/TODOS.md` T-45.
set -eu

dir="${GRID_LLVM_PROFILE_DIR:-${NEXTEST_WORKSPACE_ROOT:-.}/target/grid-profiles}"
mkdir -p "$dir"

name=$(printf '%s %s' "${NEXTEST_BINARY_ID:-unknown}" "${NEXTEST_TEST_NAME:-unknown}" |
	sed 's/%/%25/g; s/ /%20/g; s|/|%2F|g; s/:/%3A/g')

LLVM_PROFILE_FILE="$dir/$name.profraw"
export LLVM_PROFILE_FILE

exec "$@"
