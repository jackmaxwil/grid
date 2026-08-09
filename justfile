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

[group('build')]
bench *ARGS:
    cargo bench --workspace {{ARGS}}

[group('run')]
serve *ARGS:
    cargo run -p grid -- serve {{ARGS}}

[group('meta')]
deny:
    cargo deny check
