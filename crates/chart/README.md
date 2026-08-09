# grid-chart

Single responsibility: graph + definition store: immutable segments, CSR indexes.

A **segment** is an immutable byte image — interned datum ordinals, callsigns,
and edges in CSR form both directions — and a **manifest** names the live set.
The snapshot id is the hash of the canonical manifest (D5): never minted, so
re-surveying an identical state is idempotent by construction.

```rust
let image = grid_chart::build(&defs, &edges)?;      // deterministic bytes
let seg = grid_chart::Segment::parse(image)?;       // validated, or refused
for n in seg.out_edges(seg.ordinal(&datum).unwrap()) { /* contiguous scan */ }
```

`Segment::parse` is the trust boundary: it checks magic, epoch, section table,
checksum, CSR monotonicity, ordinal range, digest ordering, and UTF-8 *before*
any accessor exists. Everything after it is total — accessors answer `None`,
never panic, for any ordinal. That is what lets `chart-io` map hostile bytes.

No unsafe, no I/O. `chart-io` maps the bytes; this crate decides what they mean.
