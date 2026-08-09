# grid fuzz targets

`cargo +nightly fuzz run <target>` (from `packages/grid`, via `just fuzz`).
A separate workspace: cargo-fuzz needs nightly and its own lock file.

| target | covers |
|---|---|
| `normalize` | datum normalizer: no panic, deterministic, on arbitrary input |
| `segment` | segment loader: no panic and no OOB on arbitrary bytes (C1) |

The `segment` corpus is seeded from real images: `cargo test -p grid-chart`
writes none, so seed it by hand with any `*.seg` a store produced — a valid
image is the neighbourhood the interesting mutations live in.
