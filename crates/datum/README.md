# grid-datum

Single responsibility: identity: parse -> normalize -> blake3. Pure, no I/O.

A `Datum` answers "is this the same definition?" Reformat the code, rewrite its
comments, rename its locals: same datum. Change what it calls, what it returns,
or any name another file can see: different datum.

```
normalize(source) -> { datum, canonical, comments }
definitions(source) -> [{ callsign, datum, span_hash, span, kind }]
span_hash(source) -> b3:...      // the inverse: sensitive to everything
```

Not renamed, because they are interface: module- and class-level names, globals
(`global x`), attributes, and every name the definition does not bind itself.

The canonical bytes carry a `grid-datum/v1` prefix. Changing normalization
changes every datum in the world, so that prefix is a declared epoch, bumped
deliberately or not at all.
