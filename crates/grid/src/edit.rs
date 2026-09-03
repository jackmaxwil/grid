//! `grid edit apply` — the CAS write path, and one of the two writers the
//! engine allows (D6: survey writes segments, `edit.apply` writes source;
//! there is no third).
//!
//! An edit is coordinate-addressed: it targets `name` or `name@prefix`, and
//! `base_datum` is the compare-and-swap value — the identity the agent read.
//! If the definition moved since, the edit is a typed `conflict`, never a
//! splice against the wrong bytes. Absent `base_datum` the edit is a
//! *creation*: the target is a repo-relative path that must not exist (this
//! is also grid's own config bootstrap — init has no write privilege).
//!
//! Formatting is not grid's business: `new_text` lands verbatim and the
//! repo's own formatter owns style (CLI plan §8, resolved by deferral).
//!
//! A request validates in full before anything is written: every target
//! must resolve, every base must match, and the post-edit parse must
//! succeed — so a bad edit refuses the whole request rather than landing a
//! prefix of it and leaving the tree between states.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::path::Path;

use grid_wire::methods::{EditApplyParams, EditApplyResult};
use grid_wire::{Datum, EditResultConflictsItem};

use crate::chart_provider::suffix_matches;

/// Why a request was refused. Every variant is bad input (exit 2 on the
/// CLI, `-32602` on the socket): a stale read is *not* an error — it is a
/// conflict, reported in the result — so nothing here is the CAS path.
#[derive(Debug)]
pub enum Error {
    /// No definition answers to the target.
    UnknownTarget(String),
    /// The target suffix names more than one definition.
    AmbiguousTarget(String, Vec<String>),
    /// A creation target already exists (as a file or a definition).
    TargetExists(String),
    /// The target escapes the repo or is not a relative path.
    BadPath(String),
    /// The tree with the edits applied does not parse.
    BrokenResult(String),
    /// Two edits in one request claim overlapping spans.
    Overlap(String),
    /// The tree could not be listed or a file could not be read or written.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTarget(t) => write!(f, "no definition answers to `{t}`"),
            Self::AmbiguousTarget(t, candidates) => write!(
                f,
                "ambiguous — `{t}` could be any of: {}",
                candidates.join(", ")
            ),
            Self::TargetExists(t) => {
                write!(
                    f,
                    "`{t}` already exists — modifying it is an edit with a base_datum"
                )
            }
            Self::BadPath(t) => write!(f, "`{t}` is not a repo-relative path"),
            Self::BrokenResult(why) => write!(f, "the edit would not parse: {why}"),
            Self::Overlap(t) => write!(f, "two edits claim the same definition `{t}`"),
            Self::Io(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for Error {}

/// What one applied edit did, in porcelain terms. The wire result carries
/// identities only; the CLI renders names and transitions, so the engine
/// reports them alongside. The daemon encodes only [`Outcome::wire`] — the
/// socket speaks the contract, the CLI speaks records, and they cannot
/// disagree because both come from this one value.
#[derive(Debug)]
pub struct Outcome {
    /// The wire result: applied datums and conflicts.
    pub wire: EditApplyResult,
    /// Per-edit detail for the porcelain renderer, in request order.
    pub edits: Vec<EditDetail>,
}

/// One landed edit, as a record row waiting to happen.
#[derive(Debug)]
pub enum EditDetail {
    /// A definition replaced: callsign, old identity, new identity.
    Modified {
        /// The absolute callsign after the edit (a rename rebinds it).
        callsign: String,
        /// The base the CAS matched.
        was: Datum,
        /// The identity the replacement parses to.
        now: Datum,
    },
    /// A definition that did not exist before, inside a created file.
    CreatedDef {
        /// The new definition's absolute callsign.
        callsign: String,
        /// Its identity.
        now: Datum,
    },
    /// A file created that defines nothing (a config bootstrap, a data
    /// file): the path is the whole story.
    CreatedFile {
        /// The repo-relative path created.
        path: String,
    },
}

/// A definition in the working tree, with everything a splice needs.
struct Located {
    /// Repo-relative file path, slash-separated.
    file: String,
    /// Byte span of the definition in that file.
    span: Range<usize>,
    /// Absolute dotted name (`pkg.mod.Class.method`).
    callsign: String,
    /// Current content identity, wire form.
    datum: String,
}

/// The definitions this request could be addressing, and the crate roots the
/// write path needs.
///
/// The chart already holds every definition's callsign and the file it
/// materializes in, so it can say which files could answer a target — and
/// only those are read and parsed. What the chart must not be trusted for is
/// the *datum*, which is as old as the last survey: the candidate files are
/// re-parsed here so the compare-and-swap runs against the bytes on disk.
/// That is what keeps C15 true of a file edited since the survey — its
/// definitions have moved, the fresh parse says so, and the edit conflicts.
///
/// Conservatism: a definition the chart has never seen — written since the
/// last survey — is not a candidate, so a target naming one is refused as
/// unknown instead of resolved. Refusing is the safe direction and the
/// recovery is the survey the rest of grid already asks for. A store holding
/// no snapshot falls back to reading the whole tree, which is what every one
/// of these requests used to do.
fn locate(
    params: &EditApplyParams,
    root: &Path,
) -> Result<(Vec<Located>, grid_survey::files::CrateRoots), Error> {
    let Some(files) = candidate_files(params, root) else {
        return locate_all(root);
    };
    let roots = grid_survey::files::crate_roots(root).map_err(|e| Error::Io(e.to_string()))?;
    let mut out = Vec::new();
    for file in files {
        let (Some(lang), Ok(source)) = (
            grid_survey::files::lang_of(&file),
            std::fs::read_to_string(root.join(&file)),
        ) else {
            continue;
        };
        let module = grid_survey::files::module_of(lang, &file, &roots);
        for def in grid_survey::extract(lang, &source).defs {
            out.push(Located {
                span: whole_lines(&source, def.span),
                file: file.clone(),
                callsign: format!("{module}.{}", def.callsign),
                datum: def.datum.as_str().to_string(),
            });
        }
    }
    Ok((out, roots))
}

/// The files the live chart says could hold a definition this request names,
/// in path order — or `None` when there is no chart to ask.
fn candidate_files(
    params: &EditApplyParams,
    root: &Path,
) -> Option<std::collections::BTreeSet<String>> {
    let chart = crate::chart_provider::ChartDelta::open(root).ok()?;
    chart.current()?;
    let mut files = std::collections::BTreeSet::new();
    for spec in &params.edits {
        let (name, _) = split_target(&spec.target);
        let locator = grid_wire::Locator::Callsign {
            callsign: grid_wire::Callsign(name.to_owned()),
        };
        for hit in chart.resolve(&locator) {
            if let Some(at) = hit.materialization {
                files.insert(at.file);
            }
        }
    }
    Some(files)
}

/// Every definition in the tree, in `git ls-files` order then span order.
fn locate_all(root: &Path) -> Result<(Vec<Located>, grid_survey::files::CrateRoots), Error> {
    let (sources, roots) =
        grid_survey::files::sources(root).map_err(|e| Error::Io(e.to_string()))?;
    let mut out = Vec::new();
    for file in &sources {
        let module = grid_survey::files::module_of(file.lang, &file.path, &roots);
        for def in grid_survey::extract(file.lang, &file.source).defs {
            out.push(Located {
                file: file.path.clone(),
                span: whole_lines(&file.source, def.span),
                callsign: format!("{module}.{}", def.callsign),
                datum: def.datum.as_str().to_string(),
            });
        }
    }
    Ok((out, roots))
}

/// Split `name@prefix` into its parts; a bare `name` has no prefix, and a
/// `@` that introduces no hex is part of the name, not a handle.
fn split_target(target: &str) -> (&str, Option<&str>) {
    match target.rsplit_once('@') {
        Some((name, prefix))
            if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            (name, Some(prefix))
        }
        _ => (target, None),
    }
}

/// The one definition `target` names, by unique callsign suffix — the same
/// input rule `resolve` and the porcelain display policy name. The `@prefix`
/// is *not* matched here: it is a second CAS token, checked with the base so
/// a stale prefix surfaces as a conflict, not as "no such definition".
fn resolve<'a>(target: &str, defs: &'a [Located]) -> Result<&'a Located, Error> {
    let (name, _) = split_target(target);
    let candidates: Vec<&Located> = defs
        .iter()
        .filter(|d| suffix_matches(&d.callsign, name))
        .collect();
    match candidates.as_slice() {
        [one] => Ok(one),
        [] => Err(Error::UnknownTarget(target.to_string())),
        many => Err(Error::AmbiguousTarget(
            target.to_string(),
            many.iter().map(|d| d.callsign.clone()).collect(),
        )),
    }
}

/// A validated modify, planned against one file.
struct Splice {
    /// Span in the file's current bytes.
    span: Range<usize>,
    /// Replacement text, landing verbatim.
    new_text: String,
    /// The identity the CAS matched, for the porcelain transition.
    was: String,
    /// Position in the request, so `applied` answers in request order.
    order: usize,
}

/// Apply the edits.
///
/// Conflicts are reported together and nothing is written for any of them:
/// the agent re-reads the `now` coordinates and retries the whole request,
/// which is the recovery the accept criterion walks through.
///
/// # Errors
/// [`Error`] for bad input: unresolvable or ambiguous targets, creations
/// that already exist, overlapping edits, results that do not parse. A
/// stale `base_datum` is *not* an error — it comes back as a conflict item.
pub fn apply(params: &EditApplyParams, root: &Path) -> Result<Outcome, Error> {
    let (defs, roots) = locate(params, root)?;
    let plan = plan(params, &defs, root)?;
    if !plan.conflicts.is_empty() {
        return Ok(Outcome {
            wire: EditApplyResult {
                applied: Vec::new(),
                conflicts: plan.conflicts,
            },
            edits: Vec::new(),
        });
    }
    build_and_write(plan, root, &roots)
}

/// A fully validated request: the splices per file, the creations, and
/// every CAS conflict found along the way.
struct Plan<'a> {
    splices: BTreeMap<String, Vec<Splice>>,
    creations: Vec<(&'a grid_wire::EditSpec, String, usize)>,
    conflicts: Vec<EditResultConflictsItem>,
}

/// Phase 1 — validate everything, plan the writes, touch nothing.
fn plan<'a>(params: &'a EditApplyParams, defs: &[Located], root: &Path) -> Result<Plan<'a>, Error> {
    let mut plan = Plan {
        splices: BTreeMap::new(),
        creations: Vec::new(),
        conflicts: Vec::new(),
    };
    for (order, spec) in params.edits.iter().enumerate() {
        if let Some(base) = &spec.base_datum {
            let found = resolve(&spec.target, defs)?;
            let (_, prefix) = split_target(&spec.target);
            let prefix_holds = prefix.is_none_or(|p| {
                found
                    .datum
                    .strip_prefix("b3:")
                    .is_some_and(|hex| hex.starts_with(p))
            });
            if found.datum != base.0 || !prefix_holds {
                plan.conflicts.push(EditResultConflictsItem {
                    target: spec.target.clone(),
                    was: Some(base.clone()),
                    now: Datum(found.datum.clone()),
                });
                continue;
            }
            let planned = plan.splices.entry(found.file.clone()).or_default();
            if planned.iter().any(|s| spans_touch(&s.span, &found.span)) {
                return Err(Error::Overlap(spec.target.clone()));
            }
            planned.push(Splice {
                span: found.span.clone(),
                new_text: spec.new_text.clone(),
                was: found.datum.clone(),
                order,
            });
            continue;
        }
        // No base: a creation. The target is a repo-relative path that
        // must not exist — not as a file, not as a definition.
        let path = &spec.target;
        if path.starts_with('/') || path.split('/').any(|seg| seg == "..") || escapes(root, path) {
            return Err(Error::BadPath(path.clone()));
        }
        if root.join(path).exists() || resolve(path, defs).is_ok() {
            return Err(Error::TargetExists(path.clone()));
        }
        plan.creations.push((spec, path.clone(), order));
    }
    Ok(plan)
}

/// True when `path`, resolved through symlinks, would land outside `root`.
///
/// The lexical check above sees a leading `/` and a `..` segment; it cannot
/// see a symlink. `link/x.rs`, where `link` points at another directory, is
/// neither absolute nor climbing, and writing it leaves the repository — and
/// through `grid serve` that is a write primitive outside the tree for any
/// local socket client.
///
/// A creation's own file does not exist yet, so the deepest ancestor that
/// does exist is the one that can be canonicalized: if that ancestor is
/// outside the repository, everything under it is too. A root that cannot be
/// canonicalized, or a path with no existing ancestor at all, is refused
/// rather than guessed at.
fn escapes(root: &Path, path: &str) -> bool {
    let Ok(root) = root.canonicalize() else {
        return true;
    };
    let mut candidate = root.join(path);
    loop {
        if let Ok(real) = candidate.canonicalize() {
            return !real.starts_with(&root);
        }
        match candidate.parent() {
            Some(parent) if parent != candidate => candidate = parent.to_path_buf(),
            _ => return true,
        }
    }
}

/// Phases 2 and 3 — build every post-image in memory, prove it parses, then
/// write. A file with no planned splice is never opened for writing, so a
/// refused request is byte-identical to no request.
fn build_and_write(
    plan: Plan<'_>,
    root: &Path,
    roots: &grid_survey::files::CrateRoots,
) -> Result<Outcome, Error> {
    let mut writes: Vec<(String, String)> = Vec::new();
    let mut applied: Vec<(usize, Datum)> = Vec::new();
    let mut details: Vec<(usize, EditDetail)> = Vec::new();
    for (file, mut list) in plan.splices {
        list.sort_by(|a, b| a.span.start.cmp(&b.span.start));
        let original = std::fs::read_to_string(root.join(&file))
            .map_err(|e| Error::Io(format!("cannot read {file}: {e}")))?;
        // Rebuild by concatenation: untouched slices plus the new texts.
        // Positions in the final image fall out of the build, no arithmetic
        // on spans needed beyond the splices themselves.
        let mut image = String::with_capacity(original.len());
        let mut cursor = 0;
        let mut new_starts = Vec::with_capacity(list.len());
        for splice in &list {
            image.push_str(&original[cursor..splice.span.start]);
            new_starts.push(image.len());
            image.push_str(&splice.new_text);
            cursor = splice.span.end;
        }
        image.push_str(&original[cursor..]);

        let lang = grid_survey::files::lang_of(&file)
            .ok_or_else(|| Error::BrokenResult(format!("{file}: no frontend claims this file")))?;
        let module = grid_survey::files::module_of(lang, &file, roots);
        let new_defs = grid_survey::definitions(lang, &image)
            .map_err(|e| Error::BrokenResult(format!("{file}: {e}")))?;
        for (splice, start) in list.iter().zip(&new_starts) {
            // The replacement def begins inside the region the new text
            // occupies (leading decorators or blank lines shift it off the
            // splice point itself).
            let region = *start..*start + splice.new_text.len();
            let Some(new_def) = new_defs.iter().find(|d| region.contains(&d.span.start)) else {
                return Err(Error::BrokenResult(format!(
                    "{file}: the replacement for the definition at byte {} defines nothing",
                    splice.span.start
                )));
            };
            let now = Datum(new_def.datum.as_str().to_string());
            details.push((
                splice.order,
                EditDetail::Modified {
                    callsign: format!("{module}.{}", new_def.callsign),
                    was: Datum(splice.was.clone()),
                    now: now.clone(),
                },
            ));
            applied.push((splice.order, now));
        }
        writes.push((file, image));
    }
    for (spec, path, order) in plan.creations {
        // A creation's applied identities are whatever the new file defines
        // — for a config bootstrap that is nothing, which is the honest list.
        let mut defined = false;
        if let Some(lang) = grid_survey::files::lang_of(&path) {
            let module = grid_survey::files::module_of(lang, &path, roots);
            for def in grid_survey::definitions(lang, &spec.new_text)
                .map_err(|e| Error::BrokenResult(format!("{path}: {e}")))?
            {
                defined = true;
                let now = Datum(def.datum.as_str().to_string());
                details.push((
                    order,
                    EditDetail::CreatedDef {
                        callsign: format!("{module}.{}", def.callsign),
                        now: now.clone(),
                    },
                ));
                applied.push((order, now));
            }
        }
        if !defined {
            details.push((order, EditDetail::CreatedFile { path: path.clone() }));
        }
        writes.push((path, spec.new_text.clone()));
    }

    // Phase 3 — write.
    for (file, image) in &writes {
        let full = root.join(file);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Io(format!("cannot create {}: {e}", parent.display())))?;
        }
        std::fs::write(&full, image).map_err(|e| Error::Io(format!("cannot write {file}: {e}")))?;
    }

    applied.sort_by_key(|(order, _)| *order);
    details.sort_by_key(|(order, _)| *order);
    Ok(Outcome {
        wire: EditApplyResult {
            applied: applied.into_iter().map(|(_, datum)| datum).collect(),
            conflicts: Vec::new(),
        },
        edits: details.into_iter().map(|(_, detail)| detail).collect(),
    })
}

/// The span extended through the end of its last line. The parser's range
/// stops at the last statement, before the newline; a splice that kept that
/// convention would grow a blank line per edit, so the write path claims the
/// trailing newline as part of the definition's territory.
fn whole_lines(source: &str, span: Range<usize>) -> Range<usize> {
    let rest = &source[span.end..];
    let extra = rest.strip_prefix('\r').map_or(0, |_| 1)
        + usize::from(rest.trim_start_matches('\r').starts_with('\n'));
    span.start..span.end + extra
}

/// Spans that share a byte. Nested definitions nest spans, so a parent and
/// its child always "touch" — editing both in one request is ambiguous about
/// which text lands, and refusing is the honest answer.
fn spans_touch(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}
