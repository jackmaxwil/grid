# Repository tooling: what grid took from yi, and what it did not

`~/Development/yi` is a Rust + Python monorepo with a mature assurance
harness: a pinned toolchain, a lint wall, twenty-odd guardrail scripts behind
one aggregator, three tiers of local CI, and agent instructions that are
themselves versioned. Grid and yi share an author, a doctrine (*High Assurance
Rust*), and a shape (a flat `crates/` workspace, one binary, `cargo deny`), so
most of yi's harness transfers. Some of it is yi's domain and does not.

This file is the contract for that transfer: every mechanism yi has, what it
enforces, and grid's verdict. **Adopt** means the same file with names
changed. **Adapt** means the same idea with a stated difference. **Skip**
means grid does not take it, with the reason on one line.

Where a yi setting is weaker than a rule grid already has, grid's rule wins
and the row says so.

## Root configuration

| yi file | what it enforces | grid |
|---|---|---|
| `Cargo.toml` `[workspace.lints.clippy]` — `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` = deny | the panic budget, at compile time | **adopt.** Zero hits in grid `src/` today (build scripts, examples, `tests/` and `crates/chart-io/src/bin/crash-writer.rs` excepted). |
| `Cargo.toml` `[workspace.lints.clippy]` — `await_holding_lock`, `await_holding_invalid_type` | a `std` guard held across `.await` | **skip.** Grid has no async runtime; both lints are dead weight in a synchronous workspace. |
| `Cargo.toml` `[workspace.lints.clippy]` — `disallowed_methods` = deny | the `clippy.toml` denylist below | **skip.** Grid's analogue of yi's confinement rules — one unsafe crate, two writers — is already enforced by `check-unsafe-confined.sh` and ASSURANCE C5, which read the tree rather than the call graph. `Command::new` appears in nine grid files by design (git is the file-discovery and blob-reading interface), so a repo-wide ban would be nine scoped allows and no information. |
| `Cargo.toml` `[workspace.lints.rustdoc]` — `broken_intra_doc_links` = deny, `private_intra_doc_links` = allow | a rename that misses its doc comment fails the build; grid publishes nothing, so a public doc naming a private item is normal | **adopt**, and `cargo doc --workspace --no-deps --document-private-items` joins the gate so the lint is actually run. Fired on three links in the tree; fixed in the same commit. |
| grid `[workspace.lints.clippy]` — `all` + `pedantic` = deny | every clippy lint in two large groups | **grid's rule, kept.** yi denies neither group. Grid is stricter here and stays stricter; `missing_errors_doc`, `missing_panics_doc` and `must_use_candidate` are pedantic members, so grid already has them and no separate row is needed. |
| grid `[workspace.lints.rust]` — `unsafe_code` = forbid | one unsafe crate, and only one | **grid's rule, kept.** yi states the same posture in prose (`#![forbid(unsafe_code)]` per crate) but does not put it in the workspace lint table. |
| `clippy.toml` — `allow-unwrap-in-tests`, `allow-expect-in-tests` | the panic denies do not fire on assertions | **adopt**, plus `allow-panic-in-tests` for the same reason. Grid has no `clippy.toml` today; the file exists to make the new denies survivable. It only reaches `#[test]` functions and `#[cfg(test)]` items, though — a helper in an integration-test file is neither. yi's answer is doctrinal (its tests return `Result<(), Box<dyn Error>>`); grid's 32 integration-test roots and one example instead carry a one-line `#![allow(…)]` naming the reason, because rewriting every fixture helper is a code change, not a tooling one. `crates/grid/build.rs` carries the same allow: HAR grants build scripts, tests and benches the exemption and nothing else. |
| `clippy.toml` — `await-holding-invalid-types`, `[[disallowed-methods]]` | see the two skip rows above | **skip**, same reasons. |
| `rustfmt.toml` — `style_edition = "2024"` | formatting pinned to the edition, not to whatever rustfmt defaults to next | **adapt:** grid is an edition-2021 workspace, so it pins `style_edition = "2021"`. Same intent (formatting does not drift when the toolchain moves), zero diff on the current tree. `style_edition = "2024"` would reorder imports across every file; that is an edition migration, not a tooling change. |
| `rust-toolchain.toml` — `channel = "1.94.0"`, `components = ["clippy", "rustfmt"]` | one compiler for everyone | **already present and identical.** Grid pinned the same version independently; `cargo 1.94.0` builds the tree. No change. |
| `deny.toml` `[graph]` — `targets = [...]`, `exclude-dev = true` | the ban graph is the graph that ships, on the platforms grid builds for | **adopt.** This is what makes the row below possible. |
| `deny.toml` `[bans] multiple-versions` | two versions of one crate in the shipped graph | **adapt, stricter:** grid was `warn`, becomes `deny` with `skip = [{ crate = "syn" }]` — the one remaining duplicate, syn 2 + syn 3, is proc-macro build deps with zero runtime bytes, exactly yi's carve-out. With `exclude-dev` the rest (getrandom ×3, rand ×2) are proptest's and never ship. |
| `deny.toml` `[bans] deny = [...]` — reqwest, hyper, clap, anyhow, git2, once_cell, … | a named crate cannot enter, ever | **adapt.** Grid's list is grid's: the crates a code-charting tool is tempted by and must refuse (`clap`, `anyhow`, `regex`, `walkdir`, `notify`, `git2`/`gix`, `rayon`, `tokio`, `reqwest`, `openssl-sys`, `native-tls`, `lazy_static`, `once_cell`). ASSURANCE.md already argues each refusal in prose — this makes it mechanical. |
| `deny.toml` `[advisories] yanked = "deny"`, `[sources] unknown-*` | supply chain | **already present.** No change. |
| `.codespellrc` | typos in prose and comments are a gate, not a habit | **adapt.** Grid's ignore list is grid's domain vocabulary (`crate`, `ba`, `nd`, `datum`…), skips `target`, `Cargo.lock`, `docs/research`, `.grid`. |
| `.gitguardian.yaml` | scanners skip the deliberately key-shaped fixtures | **skip.** Grid has no secret-redaction feature and no planted-key fixtures; there is nothing to exempt. |
| `.config/nextest.toml` — `retries`, `slow-timeout`, per-binary `max-threads` | the suite's subprocess-spawning binaries do not oversubscribe the machine and hang | **adapt.** Grid's conformance and CLI suites spawn the `grid` binary, which is the same hazard yi capped at three threads. Grid caps the same way, and `just test` uses nextest only when it is on `PATH` so a fresh clone still runs the suite. |
| `Cargo.toml` `[profile.dev] debug = "line-tables-only"` + `[profile.dev.package."*"] debug = false` | debuginfo for dependencies is a large fraction of link time and nobody reads a backtrace through serde | **adopt.** Grid links a dozen test binaries and several that spawn a child; the argument transfers unchanged. |
| `Cargo.toml` `[profile.dist]` — `opt-level = "z"`, fat LTO, `panic = "abort"`, `strip` | the shipping artifact, measured by a binary-size ratchet | **skip.** Grid has no release pipeline and no binary-size budget, so a second release profile would be a profile nothing measures. `[profile.release] lto = true, codegen-units = 1` stays as grid has it. |
| `resolver = "3"`, `edition = "2024"` | — | **skip.** Grid is `resolver = "2"` / edition 2021. An edition bump is a code change with its own diff, not a tooling change. (README's "edition 2024 workspace" is wrong; out of scope here.) |
| `python/` — four skill packages with `pyproject.toml` | packaging for the kernel-side Python | **skip.** yi has no ruff, mypy or pytest configuration to port — there is none in the repo. Grid's Python is `scripts/*.py`, run by `python3` with the standard library only, same as yi's own guardrails. |

## Guardrails

yi runs `scripts/guardrails/check_guardrails.sh`, an aggregator that runs
every check, counts failures, and exits non-zero once. Grid adopts the shape
(`scripts/check-guardrails.sh`, `--fast` for the build-free subset) and a
subset of the checks. Everything below lives under `scripts/guardrails/` in
grid too, with `_common.py` ported as-is.

| yi check | what it enforces | grid |
|---|---|---|
| `check_guardrails.sh` | one entrypoint, one exit code, `--fast` for the pre-commit tier | **adopt** as `scripts/check-guardrails.sh`. |
| `_common.py` | `ROOT`, `src_files()`, `prod_lines()` (everything before `#[cfg(test)]`), `fail()` | **adopt** unchanged except the root. |
| `check_manifests.py` | folder `x/` ⇒ crate `yi-x`; `version`/`edition`/`license`/`rust-version` are `workspace = true`; `[lints] workspace = true` (workspace lints are decorative without it); deps are `{ workspace = true }` | **adapt.** Grid keeps the naming law (`x/` ⇒ `grid-x`, with `crates/grid` the binary as the declared exception) and the workspace-field law, and adds the rule yi does not need: `crates/chart-io` cannot say `[lints] workspace = true` (it must relax `unsafe_code` for the mmap), so the check requires its hand-written lint table to be a **superset** of the workspace's at the same or a stricter level. That hole is real today — chart-io silently would not inherit any lint added below. Grid does **not** adopt the centralized-`[workspace.dependencies]` rule: grid declares versions per crate, and moving them is a different change. |
| `check_boundaries.py` + `boundaries.toml` | crate dependency direction as an allowlist; unknown crate = error, stale entry = error, undeclared internal dep = error | **adopt.** Grid's graph is layered (`wire` and `datum` at the bottom, `grid` at the top) and the layering is load-bearing for the assurance case — `chart` must stay a two-dependency TCB. |
| `check_comments.py` | 3-line cap per comment run; pointer-only comments rejected; `Word:` prefixes restricted to `Incident:` / `Invariant:`; shrink-only volume ratchet outside `yi-types` | **adapt.** The pointer-only rule and the closed tag vocabulary are adopted at zero — grid's ids are `C1`…`C43` claim rows, `D`-rows and `Wave N`, and the tree already had zero pointer-only comments. The vocabulary is grid's own, read off the tree: yi's `Incident:` and `Invariant:` plus `Claim:`, `Conservatism:`, `Property:`, `Differential:` and `Deterministic:`, all of which were already in use as grants. Three comments opened with a `Word:` that was prose rather than a grant and were reworded. The 3-line cap is a **shrink-only ratchet rather than a hard cap**: 399 runs in grid exceed it, and grid's prose norm is a longer explanatory comment that yi's is not — rewriting them is a code change with its own review, not a tooling one. Volume outside `crates/wire` (generated) is ratcheted as yi does it. |
| `check_panic.py` | a grep for `.unwrap()`/`.expect(`/`panic!`/`todo!` with a zero budget | **skip.** Redundant once the clippy denies above are in the workspace lint table: the compiler sees through macros and `cfg`, a grep does not. yi runs both; grid runs the accurate one. |
| `check_filenames.py` | no `part_01.rs` / `foo_02.rs` — bisection is not decomposition | **adapt**, folded into `check_hygiene.py`. |
| `check_glob_reexport.py` | zero `pub use ...::*` and `use super::*` in `src/` | **adapt**, folded into `check_hygiene.py`. |
| `check_file_size.py` | 1,200-line ceiling, `src/` only | **adapt**, folded into `check_hygiene.py`. Three files predate the ceiling (`crates/grid/src/main.rs` 2088, `rpc.rs` 1964, `porcelain.rs` 1241); they sit in `baselines/hygiene_debt.json` at their measured size and may only shrink, and `--update` refuses to raise a number. Everything else meets the ceiling outright. |
| `check_fn_size.py` | 150-line function ceiling, brace-counted | **adapt**, folded into `check_hygiene.py`. One function predates it (`serve.rs::dispatch`, 178) and sits in the same debt list. Four ~15-line scripts asking one question — is the source shaped sanely — become one file. |
| `check_env_surface.py` | every `YI_*` env var is a declared row; hard cap 40 | **adapt** as a `GRID_*` declaration list. Grid has four (`GRID_CORPUS`, `GRID_FAULT_ITERS`, `GRID_HINTS`, `GRID_SOCKET`); the cap is 16. |
| `check_blob_size.py` | no committed file over 512 KB outside an allowlist | **adapt.** Grid's allowlist is `docs/research/*.pdf` — four papers the design argues from, deliberately vendored. |
| `check_deps_budget.py` | direct and transitive dependency counts, ratcheted | **adopt.** ASSURANCE.md's standing rule is "no new runtime dependency"; a counted budget is that rule in a form CI can check. |
| `check_schemas_lock.py` | every serialized shape in `yi-types` is normalized into a committed lock, so a schema edit is a reviewed diff | **adapt into `check_wire_drift.py`.** Grid's schema source of truth is not a crate but `docs/wire-v1.openrpc.json`, from which `scripts/gen-wire.py` generates `crates/wire/src`. The lock is therefore the generator itself: regenerate into a scratch copy, run `rustfmt` on it, and require the committed tree to be byte-identical. Same guarantee (a schema change is a reviewed diff, never a silent drift), one fewer artifact. |
| `check_commit_style.py` | one plain imperative sentence ≤ 72 chars, **no** `word:` prefix, no trailers, assistant co-author trailers banned outright; baseline edits may not ride beside code edits | **adapt, inverted.** Grid's convention is the opposite of yi's on two axes and grid's convention wins: grid uses conventional-commit subjects (`feat(watch): …`, `docs(plan): …`) and grid requires the `Co-Authored-By: Claude …` trailer yi bans. The machinery ports: no terminal period, no `WIP`/`fixup!`, git's own `Merge`/`Revert` exempt, `HEAD --not origin/main` as the range, and the baseline-rides-beside-code rule. yi's 72-character cap measures the *sentence* here rather than the whole line, because grid's `type(scope): ` prefix spends 10–16 characters before the sentence starts; the line itself gets a 100-character hard stop. Measured over grid's last 87 subjects the sentence median is 58 and p90 is 74. yi's `STANDARD_STARTS` device ports too, for yi's reason: the rule arrived on a lineage already ahead of `main`, so history before the commit that introduced it is not judged and the exclusion self-expires when the lineage lands. |
| `check_duplication.py` | zero duplicated 15-line normalized windows in `src/` | **skip.** Grid's per-language frontends (`lang-py`, `lang-rs`) implement the same visitor contract over two different ASTs; the parallel structure is the point, and a window hash cannot tell it from copy-paste. Revisit if a real duplicate is ever found by hand. |
| `check_crate_size.py`, `check_test_size.py`, `check_growth.py`, `check_binary_size.py`, `check_startup.py` | LOC and artifact ratchets, priced per version against a changelog row | **skip.** These are one mechanism: a size economy with `--update` commits, a growth memo, and a per-version changelog to argue against. Grid has no changelog and no size ledger; ratchets without the ritual that justifies moving them become noise people `--update` reflexively. `check_hygiene.py`'s hard ceilings cover the failure mode that actually matters (a file nobody can read). |
| `check_test_tiers.py` | every `#[ignore]` reads one exact string, so a paid test cannot enrol itself in a gate | **skip.** Grid has no paid tests and no `#[ignore]` tiers; its slow lanes are separate justfile recipes (`soak`, `fuzz`, `verify`), selected by name. |
| `check_behavior.py`, `check_prompt_examples.py`, `check_request_budget.py` | recorded model cassettes, prompt fixtures, provider request budgets | **skip.** yi's domain: there is no model in grid's loop. |
| `evals/selftest.py`, `skills/yi/session-mining/extract.py --selfcheck`, `scripts/pr_body.py --selfcheck`, `scripts/forgejo_pr_comment.py --selfcheck` | scripts that are themselves gated by a `--selfcheck` flag | **skip** (nothing to port) but **the pattern is adopted**: `check_wire_drift.py` and `check_commit_style.py` both carry `--selfcheck`, because a gate whose own logic is untested passes for the wrong reason. |
| `cargo doc --workspace --no-deps --document-private-items` in the aggregator | the rustdoc lint above is actually run | **adopt.** |
| `cargo machete` | declared-but-unused dependencies | **adopt** where installed, announced-skip where not — the same optional-tool shape yi uses for codespell. It flagged `rustpython-ast` in `grid-lang-py`, which is reached through `rustpython_parser::ast` and exists to turn on the `fold` and `visitor` features; recorded in that manifest's `[package.metadata.cargo-machete]` with the reason. |
| `cargo deny check` in the aggregator | the supply chain, once per gate run | **adopt.** |

## Local CI and hooks

| yi mechanism | what it enforces | grid |
|---|---|---|
| `just check` = `lint` + `guardrails` + `test` | one verb before claiming done | **adopt.** Grid's justfile has no `check`, `fmt`, `fmt-check` or `clippy` recipe at all today, despite the README claiming `just check` exists. |
| `just lane <name>` | one lane of the gate, carrying the invariant that **the gate regenerated nothing** | **adopt.** Grid needs it more than yi: `crates/wire` is generated, so "the gate wrote nothing" is a real assertion here. |
| `just precommit` (fast tier) / `just prepush` (full tier) | a hook slower than two seconds is a hook people pass `--no-verify` to | **adopt.** |
| `scripts/hooks/pre-commit` — refuses a direct commit on `main`, unsets `GIT_DIR` and friends | git exports its repo context to every child, so a test that shells out to git operates on the wrong repo | **adopt** verbatim into `.githooks/`. The `GIT_DIR` unset is a paid-for incident and grid's own tests shell out to git constantly. |
| `scripts/hooks/pre-push` — refuses deleting or non-fast-forwarding `main`, runs `just prepush`, plus the release gate on a version tag | | **adapt:** the `main` protections and `just prepush` port; the release-tag arm goes with the release pipeline. |
| `just install-hooks` (`git config core.hooksPath`) | hooks are installed by a person, never by a build | **adopt** as `just hooks`. Never run automatically. |
| `just worktrees` | every worktree, its branch, its dirt, and the shared stash stack | **adopt.** Grid development runs in `.claude/worktrees/*`, which is exactly the situation this exists for. |
| `just prerelease` / `package` / `package-musl` / `publish` / `scripts/package.sh` / `sign.sh` / `smoke.sh` / `install.sh` / `check_elf.py` / `zigcc.sh` / `zigar.sh` | build, sign, smoke-test and upload a release tarball | **skip.** Grid has no release process, no signing key and no artifact host. `cargo install --path crates/grid` is the whole distribution story. |
| `just journeys`, `just tui-proof`, `scripts/tui_pty.py`, `scripts/proof/` | TUI verification under a real PTY, GIF proof artifacts for PRs | **skip.** Grid has no TUI. |
| `just postmerge-evals`, `evals/` | a task-eval harness for an LLM agent over fixtures | **skip.** yi's domain. |
| `just install-skills` | copy `skills/` into the global agent root | **adapt** as `just skills`. |
| `completions/` (bash, fish, zsh) | shell completion for the binary | **skip here.** Grid has a verb-completions plan of its own (`docs/plans/2026-08-27-verb-completions.md`); shipping a hand-written completion file now would be a second source of truth for the verb list. |
| `vendor/seatbelt`, `vendor/rtk` | a vendored macOS sandbox policy and a vendored toolkit, licence files beside them | **skip.** Grid sandboxes nothing and vendors nothing. |

## CI

yi's CI lives in `.forgejo/workflows/`; its GitHub equivalents were dropped
when the estate moved. Grid gains `.github/workflows/` in yi's lane shape, and
`.forgejo/workflows/grid-gate.yml` is rewritten to call the same recipes —
yi's rule is that a workflow is a thin caller of the justfile the git hooks
already run and never a second copy of the rules, and grid's Forgejo file had
become exactly that second copy (it ran `cargo clippy --workspace` without
`--all-targets` and no guardrails at all, so the checks added above would
never have run in the CI grid actually uses).

| yi job | grid |
|---|---|
| `pr.yml` `gate` — a `[lint, guardrails, test]` matrix over `just lane`, `fetch-depth: 0`, `rustup show`, `Swatinem/rust-cache@v2` keyed per lane | **adopt.** Action versions pinned exactly as yi pins them (`actions/checkout@v4`, `Swatinem/rust-cache@v2`, and grid's existing `dtolnay/rust-toolchain` pinned to the toolchain file's version). The rules stay in the justfile; the workflow only says which lane on which runner. |
| `pr.yml` `title` — CI reads `PR_TITLE` and judges it by the same script the local hook judges subjects with | **adopt.** The title exists nowhere in the tree, so only CI can check it. |
| `pr.yml` `size-report` — posts the measured LOC map to the PR over the Forgejo API | **skip.** No size ledger, no Forgejo token, and the growth ratchets it reports were skipped above. |
| `postmerge.yml` — the suite under the shipping profile with panic forced to unwind, plus the eval runner | **adapt.** Grid's postmerge lane runs the deep rungs its justfile already has and PR time cannot afford: `just miri`, `just verify` (kani), `just soak`, and a `cargo fuzz build` of every target. These exist in `.forgejo/grid-gate.yml` today as PR-time jobs; the GitHub file moves the slow ones behind the merge and keeps `fuzz-build` at PR time, because a fuzz target that stops compiling is a claim with no evidence behind it. |
| `.github/PULL_REQUEST_TEMPLATE.md` — cold-reader narrative, zero checkboxes, gate proof lives in the checks tab where it cannot be ticked by hand | **adapt.** Grid's sections are grid's: summary, user outcomes, claims touched, evidence, wire changes, files edited, assurance notes. |

## Agent instructions and skills

| yi mechanism | grid |
|---|---|
| `.ruler/*.md` fragments compiled by `npx @intellectronica/ruler apply` into a gitignored `AGENTS.md` / `CLAUDE.md` / per-agent skill directories | **adapt, without ruler.** Grid tracks `AGENTS.md` as the single source and `CLAUDE.md` as a symlink to it. Ruler buys multi-agent fan-out grid does not need, and costs an untracked-generated-file rule that has its own failure mode. |
| `.ruler/000-identity` … `100-never` — identity, done-bar, architecture, style, guardrails, schema, deps, refs, testing, workflow, never | **adopt the shape**, section by section, with grid's content: the assurance case as the law, the gate list, the commit convention *with its deviations recorded*, the conservatism rule (a green test proves nothing until it has been seen red for the right reason), no new runtime dependencies, additive wire evolution only. `070-refs` (reading spans out of `ref/`) and `085-tui` are skipped — grid has neither. |
| `.ruler/skills/har*` — the ten `har-skills` `SKILL.md` files, vendored | **adopt.** Copied byte-identically from `~/Development/har-skills` (the upstream both repos install from), frontmatter unchanged, so they stay diffable against it — which is why two of their Rust Reference spellings are allowlisted in `.codespellrc` rather than corrected. `har`, `har-verify` and `har-supply` are the base three grid's `AGENTS.md` declares normative; `har-unsafe` governs the one unsafe block and `har-threat` the segment parser. |
| `skills/yi/review` — verify finished work against its acceptance criteria with a cold-context reviewer | **adopt** as `skills/review`, paths and check commands changed to grid's. |
| `skills/yi/grid` — how to query a repo's code chart with the `grid` binary | **adopt** as `skills/grid`. It documents grid's own CLI and belongs in grid's repo, not yi's. |
| `skills/yi/session-mining` — mine yi session JSONL, with planted-secret redaction fixtures | **skip.** yi's domain, and Python. |
| `.ruler/skills/yi-port` — port a cited span out of `ref/` under the Appendix A discipline | **skip** (no `ref/`), but its grid analogue is written fresh: `skills/grid-wire`, the contract-first discipline for changing the wire. |
| `.ruler/skills/yi-tui-verify` | **skip.** No TUI. Its grid analogue is `skills/grid-assurance`: how to add or discharge a claim row so `check-assurance-links.sh` stays green. |

## HAR compliance

yi enforces HAR through the lint wall, the guardrails, and `030-style.md`
declaring the `har*` skills normative for every Rust line. Grid takes the
same three, plus one grid-specific piece yi has no equivalent for:
`docs/ASSURANCE.md` is a claim-to-evidence table with a script behind it, and
`just har` runs the assurance lane (`check-assurance-links.sh`,
`check-unsafe-confined.sh`, and the verification rungs a claim's evidence
column names) as one verb.

`scripts/check-assurance-links.sh` is grid's own and predates this port, and
porting yi's habits found two holes in it. Its wave-parsing case chain was
three overlapping arms rather than one exclusive one, and the third arm also
matched the `N+` form and then failed its own equality test — so **every
standing claim skipped itself before the evidence was ever looked for**: C5,
C10, and all thirty `10+` rows. And a row naming four evidence files passed if
any one of them existed, so a claim could lose its actual evidence and stay
green behind a surviving sibling. Both are fixed here, and both fixes were
seen red first: C44 with its check deleted, C5 with `check-unsafe-confined.sh`
deleted. Two evidence columns carried prose fragments rather than filenames
(`incremental≡full` on C2, `(cache)` on C4); those are now file lists, and
the statements already said what the fragments were reaching for.

`docs/ASSURANCE.md` gains **C44**: every file under `crates/wire/src` is
byte-identical to what `gen-wire.py` writes and `rustfmt` normalizes. That is
the claim `check_wire_drift.py` is the evidence for, and it makes the
generated-code rule part of the assurance case rather than a habit.

Dependency rule, unchanged and restated: no new **runtime** crate dependency.
`Cargo.lock` must not gain a package for tooling. Every tool this port adds —
`codespell`, `cargo-nextest`, `cargo-machete`, `cargo-fuzz`, `kani` — is a
binary on `PATH`, checked for and announced-skipped when absent, exactly as yi
treats codespell. None of them is a `dev-dependency`, so the lockfile does not
move: `git diff` on `Cargo.lock` is empty across the whole port, and
`docs/ASSURANCE.md`'s dependency section says so in its own words.
