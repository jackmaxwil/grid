# grid-chart-io

Single responsibility: THE ONLY unsafe crate: mmap + segment loading, fully validating.

One `unsafe` block exists in grid, and it is in this file: `Mmap::map`. Its
obligation — the mapped file must not mutate or truncate under the mapping —
is discharged structurally, not by convention:

- segments are written to a temp name, fsynced, and published by atomic rename;
- names are content-addressed (`<blake3>.seg`), so a rewrite is a byte-identical
  no-op and in practice never happens;
- nothing in grid ever opens a published segment for writing.

Publication is the same discipline one level up: the manifest is written to
`manifest.tmp` and renamed, with the directory fsynced, so `kill -9` at any
instant leaves either the old live set or the new one. `Store::open` re-verifies
every segment's checksum against the manifest before it is used.

Readers hold an `Arc<Epoch>` from `Store::load()` and never block; publication
swaps a pointer (`arc-swap`), so a reader mid-query cannot observe a torn state.
`Store::publish` takes `&mut self` — decision D6 (two writers, total) expressed
in the type system.
