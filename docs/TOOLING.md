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
| `.config/nextest.toml` — `retries`, `slow-timeout`, per-binary `max-threads` | the suite's subprocess-spawning binaries do not oversubscribe the machine and hang | **adapt, minus the retries.** Grid's conformance and CLI suites spawn the `grid` binary, which is the same hazard yi capped at three threads; grid caps the same way, by binary *id* so a same-named in-process test in another crate is not swept in. `just test` uses nextest only when it is on `PATH`, so a fresh clone still runs the suite. `retries` is **0**, not yi's 1: a test that fails once and passes on retry is reported as passed, which is the failure `AGENTS.md` tells a reader to attribute ("a race, not your diff") rather than shrug at, and C11 is a claim about a race. Setting it to 0 immediately turned up one — a stale-bytecode race in `grid_test_runs_the_inventory_and_exits_three_on_a_red_test`, fixed in the same change. |
| `Cargo.toml` `[profile.dev] debug = "line-tables-only"` + `[profile.dev.package."*"] debug = false` | debuginfo for dependencies is a large fraction of link time and nobody reads a backtrace through serde | **adopt.** Grid links a dozen test binaries and several that spawn a child; the argument transfers unchanged. |
| `Cargo.toml` `[profile.dist]` — `opt-level = "z"`, fat LTO, `panic = "abort"`, `strip` | the shipping artifact, measured by a binary-size ratchet | **skip.** Grid has no release pipeline and no binary-size budget, so a second release profile would be a profile nothing measures. `[profile.release] lto = true, codegen-units = 1` stays as grid has it. |
| `resolver = "3"`, `edition = "2024"` | — | **skip.** Grid is `resolver = "2"` / edition 2021. An edition bump is a code change with its own diff, not a tooling change. (README's "edition 2024 workspace" is wrong; out of scope here.) |
| `python/` — four skill packages with `pyproject.toml` | packaging for the kernel-side Python | **adapt, to one package.** yi has no ruff or pytest configuration to port — there is none in the repo — and grid's guardrails stay `scripts/*.py` run by `python3` with the standard library only. What grid ships is one package, `clients/python` (distribution `grid-client`, import `grid_client`): the socket client for wire v1, half of it generated from `docs/wire-v1.openrpc.json` by the same `scripts/gen-wire.py` run that writes `crates/wire/src`, so one contract edit moves both clients or fails `check_wire_drift.py`. It is **stdlib-only and mechanically so**: `pyproject.toml` declares `dependencies = []`, there is no lockfile because there is nothing to lock, and `check_manifests.py` fails on the first dependency, extra or lockfile under `clients/` (C54). That is the Python spelling of grid's dependency law, which for Rust is a review rule over `Cargo.lock` — a diff nothing here produces. Its suite is `unittest` from the standard library, run by `just test` after the cargo suite. The one tool added is `mypy --strict clients/python`, a binary on `PATH` in the announced-skip shape `codespell` uses, because there is no compiler on that side of the wire. |

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
| `check_comments.py` | 3-line cap per comment run; pointer-only comments rejected; `Word:` prefixes restricted to `Incident:` / `Invariant:`; shrink-only volume ratchet outside `yi-types` | **skip.** Ported, then removed before merge. Every budget it carried sat at exactly the current measurement, so any added line of prose turned the gate red in a repository whose stated norm is the long explanatory comment; its 3-line "cap" was a grandfathered *count*, so deleting one 4-line run bought a 40-line one; its `--update` raised every budget including the two documented as permanently zero, which is the one ratchet in the set that could be relaxed; and a single added comment printed all 416 over-cap sites with no marker for the new one. What it uniquely caught — a private `Word:` dialect and pointer-only comments — had **never occurred**: both budgets read 0 on a 4,914-line comment corpus. Comment discipline stays a review matter and a paragraph in `AGENTS.md`. |
| `check_panic.py` | a grep for `.unwrap()`/`.expect(`/`panic!`/`todo!` with a zero budget | **skip.** Redundant once the clippy denies above are in the workspace lint table: the compiler sees through macros and `cfg`, a grep does not. yi runs both; grid runs the accurate one. |
| `check_filenames.py` | no `part_01.rs` / `foo_02.rs` — bisection is not decomposition | **adapt**, folded into `check_hygiene.py`. |
| `check_glob_reexport.py` | zero `pub use ...::*` and `use super::*` in `src/` | **adapt**, folded into `check_hygiene.py`. |
| `check_file_size.py` | 1,200-line ceiling, `src/` only | **adapt**, folded into `check_hygiene.py`. Three files predate the ceiling (`crates/grid/src/main.rs` 2097, `rpc.rs` 2060, `porcelain.rs` 1254, measured on this branch); they sit in `baselines/hygiene_debt.json` at their measured size and may only shrink, and `--update` refuses to raise a number. Everything else meets the ceiling outright. |
| `check_fn_size.py` | 150-line function ceiling, brace-counted | **adapt**, folded into `check_hygiene.py`. One function predates it (`serve.rs::dispatch`, 178) and sits in the same debt list. Four ~15-line scripts asking one question — is the source shaped sanely — become one file. |
| `check_env_surface.py` | every `YI_*` env var is a declared row; hard cap 40 | **skip.** Ported, then removed before merge. It scans `crates/*/src` only, and two of grid's four vars (`GRID_CORPUS`, `GRID_FAULT_ITERS`) are read from tests, the justfile and CI — invisible to it. Its third rule makes a declared-but-unfound row an error, so *declaring the half it cannot see fails the gate*: whichever way a developer went, the check gave a wrong answer. A hard cap of 16 guarding a surface of four is configuration for a value that never changes; the incident it inherits (jcode's 517 undocumented vars) is a different repository's. |
| `check_blob_size.py` | no committed file over 512 KB outside an allowlist | **adapt.** Grid's allowlist is `docs/research/*.pdf` — four papers the design argues from, deliberately vendored. |
| `check_deps_budget.py` | direct and transitive dependency counts, ratcheted | **skip.** Ported, then removed before merge: subsumed three times over. `deny.toml` names the crates that may never enter, sets `wildcards = "deny"` and `multiple-versions = "deny"`, and a package entering `Cargo.lock` is a line in the diff of every pull request. The budget produced a number — `10/10` direct, `64/64` transitive — that never says *which* crate arrived, and it cost a `cargo tree --workspace`, which is why it sat in the slow tier where the pre-commit hook could not reach it. |
| `check_schemas_lock.py` | every serialized shape in `yi-types` is normalized into a committed lock, so a schema edit is a reviewed diff | **adapt into `check_wire_drift.py`.** Grid's schema source of truth is not a crate but `docs/wire-v1.openrpc.json`, from which `scripts/gen-wire.py` generates `crates/wire/src`. The lock is therefore the generator itself: regenerate into a scratch copy, run `rustfmt` on it, and require the committed tree to be byte-identical. Same guarantee (a schema change is a reviewed diff, never a silent drift), one fewer artifact. |
| `check_commit_style.py` | one plain imperative sentence ≤ 72 chars, **no** `word:` prefix, no trailers, assistant co-author trailers banned outright; baseline edits may not ride beside code edits | **adapt, inverted.** Grid's convention is the opposite of yi's on two axes and grid's convention wins: grid uses conventional-commit subjects (`feat(watch): …`, `docs(plan): …`) and grid requires the `Co-Authored-By: Claude …` trailer yi bans. The machinery ports: no terminal period, no `WIP`/`fixup!`, git's own `Merge`/`Revert` exempt, `HEAD --not origin/main` as the range, and the baseline-rides-beside-code rule. yi's 72-character cap measures the *sentence* here rather than the whole line, because grid's `type(scope): ` prefix spends 10–16 characters before the sentence starts; the line itself gets a 100-character hard stop. Measured over grid's last 87 subjects the sentence median is 58 and p90 is 74. yi's `STANDARD_STARTS` device ports too, for yi's reason: the rule arrived on a lineage already ahead of `main`, so history before the commit that introduced it is not judged and the exclusion self-expires when the lineage lands. |
| `check_pr_metadata.py` (Row to Issue, landed yi PR 118) | a diff that adds a feature-ledger row or grows src past the free band names an open, sized, `area:`-labeled, milestoned issue in its PR body (`Closes #N`/`Refs #N`); a changelog row added by the same diff cites the same number; only CI can ask the forge, so `--selfcheck` over a fake transport is the tree's own check | **adapt.** The exemption is grid's own: a diff touching no charted source (`.rs`, `.py`, `.pyi` — the set `self-gate` already counts) is exempt, in place of yi's ledger-row-or-growth-band trigger, since grid has neither. The changelog-citation rule is **dropped**, not faked: grid has no changelog convention (no README/docs `## Changelog` heading, confirmed by grep). Lives at `scripts/check_pr_metadata.py`, not under `scripts/guardrails/` like the rest of this table — it never runs live outside CI (the `assurance` job in `.forgejo/workflows/grid-gate.yml`, guarded to `pull_request`), so `scripts/check-guardrails.sh` exercises only its `--selfcheck` flag. |
| `forge_tracking.py` + `.forgejo/workflows/tracking-hygiene.yml` (yi PRs 117, 119) | weekly: re-measure every open milestone's due date from closed-issue throughput (`--measure`), then fail on a stale date, an unsized/unlabeled/unmilestoned issue, or a pinned `Tracking` issue that is red or stale (`--hygiene`); size-day weights `S=1, M=3, L=7` match the `size:` labels directly | **adapt.** One repository per workflow file — `secrets.GITHUB_TOKEN` is scoped to the repo the job runs in — so this is grid's own copy, not a call into yi's. Grid's `size:`/`area:` label vocabulary already matches yi's, so `measure()` and `hygiene()` port unchanged. **Dropped**: `--comment-closed` (a postmerge `Closed by yi <version>, merge <sha>` comment keyed off `docs/ARCHITECTURE.md`'s version header, which grid has neither the header nor the postmerge job for) and with it `forgejo_pr_comment.upsert` and the close-keyword parser it served; `send` is inlined instead of imported, the shape `check_pr_metadata.py` already carries locally. `--selfcheck` ports over a fake transport, added to `scripts/check-guardrails.sh` next to `check_pr_metadata.py --selfcheck` since this too only runs live on a schedule. |
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
| `just install-skills` | copy `skills/` into the global agent root | **skip.** Ported as `just skills`, then removed: `cp -R skills/. ~/.claude/skills/` writes over a user's global skill root, and with the `har-*` copies gone (row below) there is nothing in `skills/` a person wants installed globally — the four that remain are grid's own and are read from the repository. |
| `completions/` (bash, fish, zsh) | shell completion for the binary | **skip here.** Grid has a verb-completions plan of its own (`docs/plans/2026-08-27-verb-completions.md`); shipping a hand-written completion file now would be a second source of truth for the verb list. |
| `vendor/seatbelt`, `vendor/rtk` | a vendored macOS sandbox policy and a vendored toolkit, licence files beside them | **skip.** Grid sandboxes nothing and vendors nothing. |

## CI

**One estate.** Grid's CI is `.forgejo/workflows/grid-gate.yml` and nothing
else. The port initially added `.github/workflows/` as a twin in yi's lane
shape; that directory is deleted. The `github` remote is a mirror whose push
URL is literally `no_push`, so those lanes could never run, and by the time
they were written the twins already disagreed with each other about which jobs
were PR-tier and how tools were installed — a second copy of the gate that
drifts is worse than no second copy. `.github/PULL_REQUEST_TEMPLATE.md` stays,
because Forgejo reads pull-request templates out of `.github/` too, so that one
file is live on the estate that runs.

yi's rule ports intact: a workflow is a thin caller of the justfile the git
hooks already run, never a second copy of the rules. Grid's Forgejo file had
become exactly that second copy (it ran `cargo clippy --workspace` without
`--all-targets` and no guardrails at all, so nothing added here would have run
in the CI grid actually uses), and it is now four thin lane jobs plus the ones
the justfile does not cover.

**The job names are a contract.** Branch protection on `main` requires six
exact status contexts — `assurance`, `build-test`, `chart-io-sanitizers`,
`deny`, `fault-injection`, `fuzz-build`, each `(pull_request)`. A job that is
renamed, or moved off the `pull_request` event, never reports its context and
no pull request can merge again. So `build-test` is the name of the job that
runs `just lane test` and `deny` keeps its own job even though the guardrails
lane runs `cargo deny check` as well; and the two heavy jobs
(`fault-injection`, `chart-io-sanitizers`) stay on `pull_request` rather than
moving to a postmerge tier. `fault-injection` is made cheaper *inside* the job
instead: `just soak` now defaults its iteration count rather than hard-coding
it, so a pull request runs 500 kill-9 rounds and the push to `main` runs the
full 10 000.

Tools are installed explicitly, with `taiki-e/install-action` (prebuilt) on
every lane that needs one — `just` everywhere, `cargo-deny` and
`cargo-machete` on the guardrails lane beside `pipx install codespell` and
`pipx install 'mypy>=1.5'`, and `cargo-nextest` on the test lane.
`cargo-fuzz` is the exception: the same action was tried for it and
`fuzz-build` went red in 18 s while every other install-action step in that
run was green, and this Forgejo build serves no job *logs* over its API (the
section below says exactly what it does and does not serve), so the cause
could not be read back and the step went back to `cargo install cargo-fuzz
--locked` rather than guessing (`docs/TODOS.md` T-40). The installs are not
optional politeness: the guardrail aggregator *announces* a
missing optional tool rather than failing on it (so a fresh clone can make
`just check` green), which means CI has to guarantee the tools are there or the
checks quietly stop running.

`self-gate` shares the test lane's cache (`shared-key: test`) rather than
warming a fourth cold debug build of its own.

### Reading the forge's CI results from a session

Measured 2026-09-03 against `git.example.invalid`, which reports
`15.0.7+gitea-1.22.0`, driven by `tea` 0.15.1 (go-sdk v1.2.0). Written down
because "the API does not serve it" was believed more broadly here than the
evidence supports, and a session that gives up on the forge reads its own gate
from the web UI or not at all.

`tea actions runs list` works and gives run-level status. **`tea actions runs
view <id>` and `tea actions runs logs <id>` do not**, and the failure is
neither a token nor a permission: both call
`/repos/{owner}/{repo}/actions/runs/{run_id}/jobs`, and this Forgejo does not
implement that route. Its own spec (`/swagger.v1.json`) carries exactly two
run endpoints — `GET /repos/{owner}/{repo}/actions/runs` and
`GET .../actions/runs/{run_id}` — with no `/jobs` child. `tea`'s go-sdk targets
a Gitea version that has one; the server is a Forgejo that does not. A 404
there is a client/server surface mismatch, not an answer about the run.

Per-job status **is** served, at a path `tea` has no command for:

```
tea api "/repos/apex/grid/actions/tasks?limit=60"
```

One record per job — `name`, `status`, `run_number`, `event`, `head_sha` — so
a run is read job by job by grouping on `run_number`. That is how the wave-12
merge was verified: run 78 (`push`, `aba4efb`) green on all eight jobs that
run there, against run 77 (`pull_request`, `8dba626`) green on nine with
`self-gate` red.

One trap that reading exposes, worth more than the endpoint itself. On run 78
`self-gate` and `title` are **skipped, not passed** — both are `if:
github.event_name == 'pull_request'`. Across the 30 runs the API returns,
every completed `pull_request` run has failed and every completed `push` run
has succeeded, and that pattern measures `self-gate`'s standing failure rather
than the health of `main`: the job most likely to be red does not execute on a
push. **A green push run is a weaker claim than a green pull-request run**, and
the two must not be quoted as if they were the same evidence.

What is genuinely unavailable is **log text**: the API carries no log route at
all, for a run or a job, which is why `tea actions runs logs` could not help
even if it reached the right endpoint. T-40 stands unchanged — a step whose
failure is only explicable from its log still cannot be diagnosed from a
session, and the web UI is the only reader.

| yi job | grid |
|---|---|
| `pr.yml` `gate` — a `[lint, guardrails, test]` matrix over `just lane`, `fetch-depth: 0`, `rustup show`, `Swatinem/rust-cache@v2` keyed per lane | **adapt.** The three lanes are three named jobs rather than a matrix, because a matrix leg's status context is `gate (test)` and branch protection asks for `build-test`. `rustup show` installs whatever `rust-toolchain.toml` pins, so the version lives in one file rather than in every workflow; `Swatinem/rust-cache@v2` is keyed per lane. The rules stay in the justfile. |
| `pr.yml` `title` — CI reads `PR_TITLE` and judges it by the same script the local hook judges subjects with | **adopt**, guarded to `pull_request`. On a push `PR_TITLE` is empty, the script falls through to range mode, `HEAD --not origin/main` is empty, and it prints `ok commit_style (0 commits)` — a vacuous pass and a wasted runner. |
| `pr.yml` `size-report` — posts the measured LOC map to the PR over the Forgejo API | **skip.** No size ledger, no Forgejo token, and the growth ratchets it reports were skipped above. |
| `postmerge.yml` — the suite under the shipping profile with panic forced to unwind, plus the eval runner | **skip.** Grid's deep rungs (`just miri`, `just soak`, `cargo fuzz build`) are already PR-tier jobs that branch protection requires there, so a postmerge tier would be a second place for the same evidence. `just verify` (kani) runs nowhere in CI — see the C3 note below. |
| `.github/PULL_REQUEST_TEMPLATE.md` — cold-reader narrative, zero checkboxes, gate proof lives in the checks tab where it cannot be ticked by hand | **adapt**, and kept: Forgejo reads it from `.github/`. Grid's sections are grid's: summary, user outcomes, claims touched, evidence, wire changes, files edited, assurance notes. |

## Agent instructions and skills

| yi mechanism | grid |
|---|---|
| `.ruler/*.md` fragments compiled by `npx @intellectronica/ruler apply` into a gitignored `AGENTS.md` / `CLAUDE.md` / per-agent skill directories | **adapt, without ruler.** Grid tracks `AGENTS.md` as the single source and `CLAUDE.md` as a symlink to it. Ruler buys multi-agent fan-out grid does not need, and costs an untracked-generated-file rule that has its own failure mode. |
| `.ruler/000-identity` … `100-never` — identity, done-bar, architecture, style, guardrails, schema, deps, refs, testing, workflow, never | **adopt the shape**, section by section, with grid's content: the assurance case as the law, the gate list, the commit convention *with its deviations recorded*, the conservatism rule (a green test proves nothing until it has been seen red for the right reason), no new runtime dependencies, additive wire evolution only. `070-refs` (reading spans out of `ref/`) and `085-tui` are skipped — grid has neither. |
| `.ruler/skills/har*` — the ten `har-skills` `SKILL.md` files, vendored | **skip.** Ported as byte-identical copies, then removed before merge: 1,775 lines that nothing gated against their upstream, so the copies would drift silently while `.codespellrc` and this file both asserted byte-identity as a fact. `AGENTS.md` names `~/Development/har-skills/skills/` as where the doctrine lives and reads it from there. `skills/` keeps only grid's own four: `review`, `grid`, `grid-wire`, `grid-assurance`. |
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
seen red first: C47 with its check deleted, C5 with `check-unsafe-confined.sh`
deleted. Two evidence columns carried prose fragments rather than filenames
(`incremental≡full` on C2, `(cache)` on C4); those are now file lists, and
the statements already said what the fragments were reaching for.

`docs/ASSURANCE.md` gains **C47**: `crates/wire/src/lib.rs` and `methods.rs`
are byte-identical to what `gen-wire.py` writes and `rustfmt` normalizes. That
is the claim `check_wire_drift.py` is the evidence for, and it makes the
generated-code rule part of the assurance case rather than a habit. The row
names the two generated files rather than the directory, because `errors.rs`
sits beside them and is hand-written: the generator never writes it, so the
check cannot see a change to it, and a claim should not describe a reach it
does not have.

C1's evidence token is `ci:chart-io-sanitizers` and **C3 has no CI token at
all**. `check-assurance-links.sh` used to skip every `ci:*` token before it
tested anything, so C3's `ci:kani` named a job that lived only in
`.github/workflows/` — on the estate that never runs. The check now resolves a
`ci:<job>` token against `.forgejo/workflows/`, and both rows were seen red
before they were made honest. The Kani harnesses are real and `just verify`
runs them; no CI job anywhere ever has. That job is a row in `docs/TODOS.md`,
not a claim here.

Dependency rule, unchanged and restated: no new **runtime** crate dependency.
`Cargo.lock` must not gain a package for tooling. Every tool this port adds —
`codespell`, `cargo-nextest`, `cargo-machete`, `cargo-fuzz`, `kani` — is a
binary on `PATH`, checked for and announced-skipped when absent, exactly as yi
treats codespell — and *announced* means the gate stays green, which the
aggregator now actually implements rather than only documenting. None of them
is a `dev-dependency`, so the lockfile does not move: `git diff` on
`Cargo.lock` is empty across the whole port, and `docs/ASSURANCE.md`'s
dependency section says so in its own words.

## What the hooks do and do not gate

`.githooks/pre-commit` runs `just precommit` over the **checkout**, not the
index. A partial `git add` is therefore gated against changes that are not
being committed: an unstaged fix can make a broken commit pass, and unrelated
work in progress can block a clean one. This is inherited from yi and is left
as it is deliberately — a stash-based hook has a worse failure mode in exactly
this repository's setup, where every worktree shares one stash stack (which is
what `just worktrees` prints and warns about). Pushing is where the tree and
the history agree again: `just prepush` runs the whole gate against what is
about to leave the machine.

`.githooks/commit-msg` judges the subject line as it is written. The rest of
`check_commit_style.py` — the trailer rule and the composition rule — needs a
commit to exist and so stays in the guardrail suite, one commit behind by
nature.

## Measured, on this branch

Apple M-series, warm `target/`, after the deletions and fixes above.

| what | measured |
|---|---|
| `just precommit`, nothing changed | 2.5 s |
| `just precommit` after touching `crates/chart/src/lib.rs` (rebuilds five downstream crates) | 4.2 s — fmt-check 0.33 s, clippy 3.5 s, `guardrails-fast` 1.5 s |
| `scripts/check-guardrails.sh --fast` alone | 1.48 s median of five (1.39–1.64) |
| `scripts/check-guardrails.sh`, whole | 3.8 s |
| `cargo nextest run --workspace` | 524 tests, ~27–47 s |
| comment lines in `crates/*/src` | 5,186 across 64 files |
| hygiene debt | `main.rs` 2097, `rpc.rs` 2060, `porcelain.rs` 1254, `serve.rs::dispatch` 178 |

The header of `check-guardrails.sh` claims the fast tier keeps the hook "under
a couple of seconds"; `guardrails-fast` alone meets that and `just precommit`
does not, because clippy dominates it. The honest statement is that the hook
costs about four seconds on a change that touches a widely-depended-on crate,
and that the ceiling is cargo's, not the guardrails'. `guardrails-fast` grows
slowly with branch length — `check_commit_style.py` runs one `git show` per
commit ahead of `origin/main` — which is why the number moved from 1.27 s at
eight commits to 1.48 s at eleven.
