//! grid-survey — extraction: repo -> defs + edges.
//!
//! A survey turns a working tree into a chart segment: every definition a
//! frontend can find, and every edge between them it can *prove*. Python is
//! the only frontend at this wave; a second language lands as a
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

use std::collections::HashMap;
use std::path::Path;

use grid_datum::{Definition, FileRefs};
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

/// Extract one file's facts with the frontend `lang` names. Pure: the same
/// text is the same facts.
///
/// This is the frontend seam: the one place a language is dispatched on. A
/// file the parser rejects yields no facts rather than failing the survey —
/// a repo with one syntactically broken file still has a chart, and the
/// alternative is a tool that cannot run on the tree it is meant to fix.
#[must_use]
pub fn extract(lang: Lang, source: &str) -> FileFacts {
    match lang {
        Lang::Py => FileFacts {
            defs: grid_lang_py::definitions(source).unwrap_or_default(),
            refs: grid_lang_py::references(source).unwrap_or_default(),
        },
        // No frontend claims these yet; [`files::lang_of`] never hands them
        // out, so no file silently loses facts here.
        Lang::Ts | Lang::Js | Lang::Rs => FileFacts {
            defs: Vec::new(),
            refs: FileRefs::default(),
        },
    }
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
    run_sources(&files::sources(root)?, cache)
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
    run_sources(&files::sources_at(root, rev)?, cache)
}

fn run_sources(sources: &[SourceFile], cache: &mut Cache) -> Result<Survey, SurveyError> {
    // Phase 1 — per-file, cacheable, order-independent.
    let mut reread = 0;
    let mut per_file = Vec::with_capacity(sources.len());
    for file in sources {
        let facts = if let Some(hit) = cache.get(&file.content_hash) {
            reread += 1;
            hit.clone()
        } else {
            let facts = extract(file.lang, &file.source);
            cache.insert(file.content_hash, facts.clone());
            facts
        };
        per_file.push((file, facts));
    }

    // Phase 2 — index the whole tree. Edges cannot be resolved before every
    // definition is known, which is the reason for two phases and the reason
    // the cache cannot hold edges.
    let mut index = DefIndex::default();
    for (file, facts) in &per_file {
        index.insert_file(&files::module_of(&file.path), file.lang, &facts.defs);
    }

    // Phase 3 — resolve names to datums.
    let mut edges = Vec::new();
    for (file, facts) in &per_file {
        edges.extend(resolve::resolve(
            &index,
            &files::module_of(&file.path),
            &facts.refs.imports,
            &facts.refs.refs,
        ));
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
