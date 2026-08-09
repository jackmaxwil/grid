# grid-xte

Single responsibility: spec gate: `Track` vs `MadeGood` -> `CrossTrackError`
verdicts.

Two spaces meet here. A `Track` is authored before code exists, so it names
code by callsign *pattern*; a `MadeGood` is a diff of surveyed reality, so it
names code by `Datum`. `DeltaProvider` is the seam between them: it supplies
the delta and names its data. Wave 3 ships a file-level provider in the `grid`
crate; Wave 5 replaces it with the chart and nothing in this crate changes.

Pure: no I/O, no clock, no chart. `verdict` is a function of its arguments.

- `pattern` — the glob language (`*` one segment, `**` any number) and the
  containment relation `lint` uses to call a plan impossible.
- `lint` — `track.lint`: everything checkable about a promise before code
  exists.
- `verdict` / `check` — `gate.check`: `hold` iff some itemized divergence
  exists.
