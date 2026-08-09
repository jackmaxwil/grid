# grid package verbs — forwarded from the repo root justfile.
set positional-arguments := true

[group('build')]
build *ARGS:
    cargo build --workspace {{ARGS}}

[group('test')]
test *ARGS:
    cargo test --workspace {{ARGS}}

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
