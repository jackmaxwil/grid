//! grid-survey — extraction: repo -> defs + edges.
//!
//! A survey turns a working tree into a chart segment: every definition a
//! frontend can find, and every edge between them it can *prove*. Python and
//! Rust are the frontends at this wave; the next language lands as a
//! `grid-lang-*` crate, an extension in [`files::lang_of`], and a dispatch
//! arm in [`extract`] — nothing above those knows what language a file was.
//!
//! Two properties are load-bearing and everything here is arranged around
//! them:
//!
//! - **Determinism.** The same tree surveys to byte-identical segments, on any
//!   machine, in any order. Discovery order is erased by sorting, absolute
//!   paths never enter, and the segment builder treats its input as a set. The
//!   snapshot id is the hash of the result, so this is what makes an identical
//!   re-survey a no-op rather than a duplicate.
//! - **Incremental ≡ full.** Surveying a tree from cold and surveying it after
//!   editing one file must produce the *same* segment, not a similar one. The
//!   cache is keyed on file content hash and holds only per-file facts
//!   (definitions and unresolved references); everything cross-file is
//!   recomputed from those, so a stale edge cannot survive a change to either
//!   of its endpoints. That is why [`Cache`] stores references rather than
//!   edges — it is the difference between an optimization and a second,
//!   diverging implementation.
pub mod files;
pub mod index;
pub mod resolve;
mod resolve_rs;

use std::collections::HashMap;
use std::path::Path;

use grid_datum::{Definition, FileRefs, ParseError};
use grid_wire::Lang;

pub use files::SourceFile;
pub use index::{DefIndex, Symbol};
pub use resolve::ResolvedEdge;

/// Why a survey could not be produced.
#[derive(Debug)]
pub enum SurveyError {
    /// git could not be run, or answered with a failure.
    Git(String),
    /// The chart refused the graph the survey found.
    Chart(grid_chart::ChartError),
}

impl std::fmt::Display for SurveyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git(msg) => write!(f, "git: {msg}"),
            Self::Chart(e) => write!(f, "chart: {e}"),
        }
    }
}

impl std::error::Error for SurveyError {}

impl From<grid_chart::ChartError> for SurveyError {
    fn from(e: grid_chart::ChartError) -> Self {
        Self::Chart(e)
    }
}

/// The per-file facts extraction produces: what this file defines, and what
/// its definitions reach for by name.
///
/// Nothing here depends on any other file, which is precisely why it is safe
/// to cache on content hash. An edge does depend on other files, and so is
/// never cached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    /// Definitions found, outermost first.
    pub defs: Vec<Definition>,
    /// Imports and references, unresolved.
    pub refs: FileRefs,
}

/// Per-file extraction results, keyed by content hash.
///
/// Content-addressed, so it is correct across renames for free: moving a file
/// changes its module path but not its bytes, and the module path is applied
/// after the cache, not before it.
pub type Cache = HashMap<[u8; 32], FileFacts>;

/// What one survey found.
#[derive(Debug)]
pub struct Survey {
    /// The serialized segment image, ready to publish.
    pub image: Vec<u8>,
    /// Definitions interned.
    pub defs: u64,
    /// Edges proven.
    pub edges: u64,
    /// Files whose facts came from the cache rather than the parser. Zero on a
    /// cold survey; the incrementality measurement, reported rather than
    /// inferred from a timer.
    pub reread: u64,
    /// Languages that contributed at least one definition.
    pub langs: Vec<Lang>,
    /// Names defined in more than one place, and therefore not resolved
    /// against. Surfaced so a survey can say what it declined to prove.
    pub ambiguous: u64,
}

/// The definitions the frontend `lang` names finds in `source`. This is the
/// frontend seam: the one place a language is dispatched on. A language with
/// no frontend yields nothing — [`files::lang_of`] never hands one out, so
/// no file silently loses facts here.
///
/// # Errors
/// `ParseError` if the frontend's parser rejects the source.
pub fn definitions(lang: Lang, source: &str) -> Result<Vec<Definition>, ParseError> {
    match lang {
        Lang::Py => grid_lang_py::definitions(source),
        Lang::Rs => grid_lang_rs::definitions(source),
        Lang::Ts | Lang::Js => Ok(Vec::new()),
    }
}

/// The imports and references the frontend `lang` names finds in `source`.
///
/// # Errors
/// `ParseError` if the frontend's parser rejects the source.
pub fn references(lang: Lang, source: &str) -> Result<FileRefs, ParseError> {
    match lang {
        Lang::Py => grid_lang_py::references(source),
        Lang::Rs => grid_lang_rs::references(source),
        Lang::Ts | Lang::Js => Ok(FileRefs::default()),
    }
}

/// Extract one file's facts. Pure: the same text is the same facts.
///
/// A file the parser rejects yields no facts rather than failing the survey —
/// a repo with one syntactically broken file still has a chart, and the
/// alternative is a tool that cannot run on the tree it is meant to fix.
#[must_use]
pub fn extract(lang: Lang, source: &str) -> FileFacts {
    // One parse serves both extractions; parsing twice was measured at half
    // the whole extraction phase on this repo.
    let facts = match lang {
        Lang::Py => grid_lang_py::file_facts(source),
        Lang::Rs => grid_lang_rs::file_facts(source),
        Lang::Ts | Lang::Js => Ok((Vec::new(), FileRefs::default())),
    };
    let (defs, refs) = facts.unwrap_or_default();
    FileFacts { defs, refs }
}

/// Survey a working tree into a segment image, reusing `cache` where the file
/// content hash says nothing changed.
///
/// Pass an empty cache for a cold survey. The cache is updated in place, so
/// the caller keeps it between surveys and the second survey of an unchanged
/// tree parses nothing at all.
///
/// # Errors
/// [`SurveyError::Git`] if the tree cannot be listed, [`SurveyError::Chart`]
/// if the graph exceeds what a segment can address.
pub fn run(root: &Path, cache: &mut Cache) -> Result<Survey, SurveyError> {
    let (sources, roots) = files::sources(root)?;
    run_sources(&sources, &roots, cache)
}

/// Survey the tree as it was at `rev`.
///
/// The cache is shared with [`run`] and stays correct across both, because its
/// key is file content and a blob has the same content wherever it is read
/// from — so a file unchanged between the rev and the worktree is parsed once.
///
/// # Errors
/// [`SurveyError::Git`] if the rev does not resolve, [`SurveyError::Chart`] if
/// the graph exceeds what a segment can address.
pub fn run_at(root: &Path, rev: &str, cache: &mut Cache) -> Result<Survey, SurveyError> {
    let (sources, roots) = files::sources_at(root, rev)?;
    run_sources(&sources, &roots, cache)
}

/// Phase 1 — per-file facts, cacheable, order-independent.
/// Order-independence is what makes cache misses safe to extract on scoped
/// threads: each miss is pure over its own source, and results land back by
/// index. Returns the facts in source order plus the cache-hit count.
fn extract_all<'s>(
    sources: &'s [SourceFile],
    cache: &mut Cache,
) -> (Vec<(&'s SourceFile, FileFacts)>, u64) {
    let mut reread = 0;
    let mut slots: Vec<Option<FileFacts>> = Vec::with_capacity(sources.len());
    let mut misses = Vec::new();
    for (i, file) in sources.iter().enumerate() {
        if let Some(hit) = cache.get(&file.content_hash) {
            reread += 1;
            slots.push(Some(hit.clone()));
        } else {
            slots.push(None);
            misses.push(i);
        }
    }
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let extracted: Vec<(usize, FileFacts)> = std::thread::scope(|scope| {
        let chunk = misses.len().div_ceil(threads.max(1)).max(1);
        let handles: Vec<_> = misses
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&i| (i, extract(sources[i].lang, &sources[i].source)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            // A panicked worker loses its chunk here; the sequential refill
            // below re-extracts anything still empty, so the segment bytes
            // cannot depend on whether a thread died.
            .filter_map(|h| h.join().ok())
            .flatten()
            .collect()
    });
    for (i, facts) in extracted {
        if let Some(file) = sources.get(i) {
            cache.insert(file.content_hash, facts.clone());
        }
        if let Some(slot) = slots.get_mut(i) {
            *slot = Some(facts);
        }
    }
    let mut per_file = Vec::with_capacity(sources.len());
    for (file, slot) in sources.iter().zip(slots) {
        let facts = slot.unwrap_or_else(|| {
            let facts = extract(file.lang, &file.source);
            cache.insert(file.content_hash, facts.clone());
            facts
        });
        per_file.push((file, facts));
    }
    (per_file, reread)
}

fn run_sources(
    sources: &[SourceFile],
    roots: &files::CrateRoots,
    cache: &mut Cache,
) -> Result<Survey, SurveyError> {
    let (per_file, reread) = extract_all(sources, cache);

    // Phase 2 — index the whole tree. Edges cannot be resolved before every
    // definition is known, which is the reason for two phases and the reason
    // the cache cannot hold edges.
    let mut index = DefIndex::default();
    for (file, facts) in &per_file {
        index.insert_file(
            &files::module_of(file.lang, &file.path, roots),
            file.lang,
            &facts.defs,
        );
    }
    // Aliases go in after every definition, so a real name always wins over
    // a re-export of the same spelling. A reexport inside a function body
    // is unreachable from other files and stays out of the map.
    for (file, facts) in &per_file {
        let module = files::module_of(file.lang, &file.path, roots);
        for rx in &facts.refs.reexports {
            if rx.scope.is_empty() {
                index.insert_alias(format!("{module}.{}", rx.alias), &module, &rx.target);
            }
        }
    }

    // Phase 3 — resolve names to datums, under each file's own language
    // rules: what "this name binds to" means is per-language, so the
    // resolver dispatches exactly where extraction does.
    let mut edges = Vec::new();
    for (file, facts) in &per_file {
        let module = files::module_of(file.lang, &file.path, roots);
        edges.extend(match file.lang {
            Lang::Py => resolve::resolve(&index, &module, &facts.refs.imports, &facts.refs.refs),
            Lang::Rs => resolve_rs::resolve(&index, &module, &facts.refs.imports, &facts.refs.refs),
            Lang::Ts | Lang::Js => Vec::new(),
        });
    }

    let chart_defs: Vec<grid_chart::Def> = index
        .symbols()
        .map(|s| grid_chart::Def {
            datum: grid_chart::DatumId(s.datum),
            callsign: s.absolute.clone(),
            lang: Some(s.lang),
        })
        .collect();
    let chart_edges: Vec<grid_chart::Edge> = edges
        .iter()
        .map(|e| grid_chart::Edge {
            src: grid_chart::DatumId(e.src),
            dst: grid_chart::DatumId(e.dst),
            relation: e.relation,
            zoc: e.zoc,
        })
        .collect();

    let image = grid_chart::build(&chart_defs, &chart_edges)?;
    // Counts are read back off the built segment rather than off the inputs:
    // the builder dedupes, so its numbers are the true ones.
    let segment = grid_chart::Segment::parse(image.as_slice())?;
    let (defs, edge_count) = (
        u64::from(segment.def_count()),
        u64::from(segment.edge_count()),
    );
    let mut langs: Vec<Lang> = per_file
        .iter()
        .filter(|(_, f)| !f.defs.is_empty())
        .map(|(file, _)| file.lang)
        .collect();
    langs.sort_by_key(|l| format!("{l:?}"));
    langs.dedup_by_key(|l| format!("{l:?}"));

    Ok(Survey {
        image,
        defs,
        edges: edge_count,
        reread,
        langs,
        ambiguous: index.ambiguous_count() as u64,
    })
}
