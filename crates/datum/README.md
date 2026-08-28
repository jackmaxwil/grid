# grid-datum

Single responsibility: the identity vocabulary every frontend speaks. Pure,
no I/O, no parser.

A `Datum` answers "is this the same definition?" — `b3:` + blake3 of the
canonical bytes a frontend's normalizer produced. `span_hash` is the inverse
instrument: raw source bytes, sensitive to everything the datum ignores.
`Definition`, `Reference`, `Import`, `FileRefs` are the shapes a frontend
emits; a callsign is an opaque ordered string in the frontend's own spelling.

Frontends (`grid-lang-py` today) own parsing and normalization for one
language each. Dispatch by language lives in `grid-survey`, the one layer
that knows what a file is. This crate exists so that adding a language never
adds a parser to anything above a frontend.
