# grid fuzz targets

`cargo +nightly fuzz run <target>` (from `packages/grid`, via `just fuzz`).
A separate workspace: cargo-fuzz needs nightly and its own lock file.

| target | covers |
|---|---|
| `normalize` | datum normalizer: no panic, deterministic, on arbitrary input |
