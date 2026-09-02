# Grid

Grid charts every definition in a repository with a content-derived identity
and every relationship it can prove, then answers questions about them: where
a definition lives, who calls it, what changed since a snapshot, whether a
diff matches what a plan promised. One Rust binary, twelve crates, no server
and no model.

Authority order: `docs/ASSURANCE.md` is the law — the claim-to-evidence table
the whole design is accountable to. `docs/ONTOLOGY.md` is the vocabulary,
`docs/PORCELAIN.md` the output contract, `docs/wire-v1.openrpc.json` the wire.
Code follows the docs; changing a settled claim means editing its row first,
in the same change. `docs/TOOLING.md` records where this repository's tooling
came from and why each piece is shaped the way it is.

`docs/plans/` holds the wave and step plans. Work inside the current wave
(`docs/CURRENT_WAVE`); do not scaffold ahead of it.

Mandatory skills, at session start and in any case before writing Rust:
`har`, `har-verify` and `har-supply`. The HAR doctrine lives at
`~/Development/har-skills/skills/` and is read from there, never vendored into
this repository — a copy is a fork that nothing diffs. `har-unsafe` governs
`crates/chart-io`, `har-threat` the segment parser, and the rest load by their
stated triggers. `skills/` here holds only grid's own: `review`, `grid`,
`grid-wire`, `grid-assurance`.

# Declaring work ready

"Done" means all of the following, in order. A passing test suite is
necessary, not sufficient.

1. The build succeeds and the focused tests for the changed path pass.
2. `just check` is green — and every gate is judged by its exit code, never by
   piped output: `cargo test | grep ...` reports grep's exit, not the tests'.
   `just check` runs fmt, clippy, the guardrails *and* the suite, so
   `guardrails: all green` inside a grep is not the gate: read `$?`.
3. For behavior changes, run the real binary against a real tree — the repo
   itself is the corpus grid always has. `--json` gives the wire result
   verbatim, which is what a caller sees.
4. Report failures verbatim; never paraphrase an error you have not fixed.

Misdiagnosis defaults: a gate that turns red after your change was broken by
your change — fix the code, never the baseline. Ratchet growth is intentional
only as `--update` in its own commit. Attribute a red gate before editing
anything: a test that fails inside the full suite and passes standalone is a
race, not your diff.

# Architecture

Twelve crates under `crates/`, all in the default build. Naming law: folder
`x/` is crate `grid-x`, and `crates/grid/` is the binary — enforced by
`scripts/guardrails/check_manifests.py`.

Dependency direction is an allowlist in `scripts/guardrails/boundaries.toml`:
unknown crate = error, stale entry = error, undeclared internal dep = error.
Standing rules:

- `grid-wire` is the schema wall and it is **generated**. Every serialized
  shape lives there, written by `scripts/gen-wire.py` from
  `docs/wire-v1.openrpc.json` into `crates/wire/src/lib.rs` and `methods.rs`.
  Never hand-edit either; edit the contract and run `just wire`.
  `check_wire_drift.py` fails when that is owed. (`errors.rs` beside them is
  hand-written and the generator does not touch it, so the check cannot see a
  change to it either — C44 says as much.)
- `grid-datum` is the parser-free identity vocabulary every frontend emits, so
  a `grid-lang-*` crate depends on it and never the reverse. Adding a language
  adds a `grid-lang-*` crate, never a parser above one.
- `grid-chart` is the crate an adversarial segment image reaches, and C1's
  argument rests on it staying a two-dependency TCB (`blake3`, `grid-wire`).
- `grid-chart-io` is the only crate that may contain `unsafe`, and the only
  one that may relax the workspace `unsafe_code` forbid — `check-unsafe-confined.sh`
  enforces both halves. Because Cargo's `[lints]` table is all-or-nothing, its
  hand-written lint tables must stay a superset of the workspace's;
  `check_manifests.py` fails when they fall behind.
- There are two writers to the store and no more: `survey` and `edit.apply`
  (C5).

# Style: HAR compliance

The `har*` skills at `~/Development/har-skills/skills/` are normative for
every Rust line. Enforced highlights:

- `#![forbid(unsafe_code)]` workspace-wide, relaxed in exactly one crate.
  Panic budget zero: clippy denies `unwrap`/`expect`/`panic`/`todo`/
  `unimplemented` and `clippy.toml` exempts tests, where an assertion is the
  point.
- `clippy::all` and `clippy::pedantic` are denied, which is stricter than yi's
  wall and stays that way. Never lower a level to get a build green.
- Newtypes for every id and ordinal; checked or saturating arithmetic on all
  budget, fuel and offset math. Typed errors at crate boundaries; wire-facing
  enums stay decodable.
- A comment earns its line only by naming what the code cannot: the incident
  that created a constant or a guard, an invariant the type system cannot
  express, the assurance claim the code is evidence for, or what the code does
  when it cannot decide. Restating a signature or the next three statements is
  none of these. A pointer-only comment (`// see drift.rs`) says nothing a
  reader cannot get from the code. The grant vocabulary is closed:
  `Incident:`, `Invariant:`, `Claim:`, `Conservatism:`, `Property:`,
  `Differential:`, `Deterministic:`. This is a review rule, deliberately: a
  budget on comment lines taxes every commit that adds a line of prose, and
  grid's norm is the long explanatory comment.
- A Rust item named in a doc comment is an intra-doc link —
  ``[`Segment::parse`]``, not bare backticks. `rustdoc::broken_intra_doc_links`
  is denied and `cargo doc --document-private-items` runs in the guardrails,
  so a rename that misses its comment fails the build. Bare backticks mean
  not-an-item: a parameter, a wire name, a porcelain column.
- Fight for every line. The size ratchets are ceilings, not targets.

# Guardrails

Run `just check` before claiming any task done; quote failures verbatim.

- Ratchets only shrink. Intentional growth is `--update`, in its own commit; a
  baseline edit in the same commit as a code edit fails
  `check_commit_style.py`. Order is fixed: land the code commit red on the
  baselines, then `--update` and commit the baselines alone.
- A guardrail can fail on a file that is not yours — `blob_size` fires on any
  untracked blob in the tree. Report it; do not allowlist or delete another
  session's artifact.
- Budgets that start at zero stay at zero: glob re-exports, bisected
  filenames.
- `just guardrails-fast` is the build-free subset the pre-commit hook runs;
  `just guardrails` is the whole thing. `just hooks` installs the hooks, and
  nothing else ever does.

# Wire and format stability

- Every serialized shape lives in `grid-wire`, generated from the contract.
  Additive evolution only: new fields optional or defaulted, never renamed,
  never repurposed. `deny_unknown_fields` is deliberate there — an unknown key
  is a wire error, not a silent drop.
- The segment format is the other compatibility surface. `Segment::parse`
  keeps reading every epoch ever shipped, answering columns an older image
  does not declare with `None` and never a fabricated value, pinned by a
  committed fixture per epoch. A manifest from a past epoch is dropped and
  named (`Store::superseded`); the prescribed recovery is a re-survey.
- Normalization is a declared epoch: canonical bytes are prefixed
  `grid-datum/v1`, so a parser upgrade that moves normalization is a bump,
  never a silent re-hash.
- A fixture is never deleted or edited, only added beside.

# Dependencies

**No new runtime crate dependency.** Every package in `Cargo.lock` has a line
in `docs/ASSURANCE.md` saying why it is in the TCB, `deny.toml` names the
crates that may never enter — including `wildcards` and a second version of
any crate — and a package entering `Cargo.lock` is visible in the diff of
every pull request. Adding one edits, in the same commit: the ASSURANCE
justification and `deny.toml` where relevant. Dev
tooling that is a binary on `PATH` — `codespell`, `cargo-nextest`,
`cargo-machete`, `cargo-deny`, `cargo-fuzz`, `kani` — is not a dependency and
does not move the lockfile; that is the only kind of tool this repository
adds.

`default-features = false` where a default pulls something the crate does not
need; every such narrowing carries its reason in the manifest.

# Testing doctrine

Every test defends one concrete, externally observable contract. Name the
failure mode a consumer would see if it regressed; if you cannot name one, do
not add the test.

- **A green test proves nothing until it has been seen red for the right
  reason.** A regression test runs against the unfixed code before the fix is
  claimed: temporarily revert the fix, watch it fail, restore. A new gate is
  proven the same way — disable the gate, not the test, and watch the test
  fail for the gate's own reason. `grid drift --tests` exists because tests
  can be green and stale at once, which is the same failure this rule catches
  by hand.
- External ground truth over self-confirmation: a round-trip proves
  reversibility, not correctness.
- Assert exact bytes and ordering only where a consumer parses the exact bytes
  (segment images, porcelain records, wire JSON); otherwise assert semantic
  content.
- Never source-grep: a test that reads source text and asserts on it tests how
  code looks, not what it does.
- Climb har-verify's ladder to match the risk and stop: proptest for an
  algebraic property, `cargo fuzz` for anything that parses untrusted bytes,
  MIRI for the one unsafe crate, kani where absence of a panic over all inputs
  is the claim.

# Workflow

- A commit subject is a conventional-commit sentence: `type(scope): subject`,
  type from `feat|fix|docs|test|perf|refactor|chore|build|ci|revert`, the
  sentence imperative, lowercase, no terminal period, at most 72 characters
  after the colon and 100 for the whole line. A PR title is judged by the same
  rule, by the same script, in CI, and the `commit-msg` hook judges the subject
  as you write it rather than one commit later.
- Deviations from a settled decision are recorded, not silently taken: the
  claim row, the plan, or `docs/TOOLING.md`, in the same change as the code.
  A plan that landed differently than written says so at the end of it.
- The only trailer is `Co-Authored-By:`. Grid keeps it as its authorship
  record — this is a deliberate inversion of yi's rule, and `TOOLING.md` says
  so.
- A commit message containing backticks or `$(` goes through `git commit -F -`
  with a quoted heredoc, never `-m`: zsh command-substitutes inside double
  quotes and mangles the message.
- Never `git add -A` in a shared tree: it swallows another session's
  uncommitted files. Stage the paths the change touched, by name, and check
  the commit's own file list afterwards.
- `just worktrees` lists every worktree, its branch, its dirt, and the fact
  that the stash stack is shared by all of them.

# Never

- Never hand-edit `crates/wire/src/lib.rs` or `methods.rs` — edit
  `docs/wire-v1.openrpc.json` and run `just wire`.
- Never add a dependency that is not already in `docs/ASSURANCE.md`.
- Never edit a guardrail baseline in the same commit as code.
- Never commit while a gate is red; never commit or push unasked.
- Never modify a committed fixture; add a new one beside it.
- Never write `unsafe` outside `crates/chart-io`, and never relax the
  `unsafe_code` lint in another crate's manifest.
- Never lower a lint level, allowlist a blob, or `--update` a ratchet to make
  a gate pass — fix the code the gate is talking about.
- Never name a Rust item in bare backticks inside a doc comment: it is an
  intra-doc link, or it is not an item.
- Never scaffold ahead of the current wave.
