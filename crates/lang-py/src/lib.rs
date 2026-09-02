//! grid-lang-py — the Python frontend: parse -> normalize -> identity.
//!
//! This crate owns everything that knows Python syntax: parsing, alpha-
//! renaming, canonical bytes, definition and reference extraction. What it
//! *emits* is the language-agnostic vocabulary of `grid-datum`; nothing above
//! this crate imports a Python parser. A second language arrives as a sibling
//! `grid-lang-*` crate and a dispatch arm in `grid-survey`, not as an edit
//! here.
mod canon;
mod extract;
mod refs;
mod sig;

pub use extract::definitions;
pub use refs::references;
pub use sig::sig_text;

use grid_datum::{Datum, Definition, FileRefs, ParseError};
use rustpython_parser::ast;
use rustpython_parser::ast::fold::Fold;
use rustpython_parser::{lexer, Mode, Parse, Tok};

/// A comment, lifted out of the source before hashing. Retained so that
/// "not identity" does not mean "lost".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    /// Byte offset in the source it was found at.
    pub offset: usize,
    /// The comment text, including its `#`.
    pub text: String,
}

/// The result of normalizing a module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalized {
    /// Identity of the module as a whole.
    pub datum: Datum,
    /// The canonical bytes the datum hashes — exposed because determinism is
    /// only checkable if the input to the hash is inspectable.
    pub canonical: Vec<u8>,
    /// Comments, in source order. Metadata, never hashed.
    pub comments: Vec<Comment>,
}

/// Both extractions off one parse — the survey's hot path, where parsing
/// every file twice was measured at half the whole extraction phase.
///
/// # Errors
/// Returns [`ParseError`] if `source` is not valid Python.
pub fn file_facts(source: &str) -> Result<(Vec<Definition>, FileRefs), ParseError> {
    let module =
        ast::ModModule::parse(source, "<datum>").map_err(|e| ParseError::new(e.to_string()))?;
    let defs = extract::definitions_of(&module.body, source);
    Ok((defs, refs::references_of(module.body)))
}

/// Parse -> alpha-rename locals -> erase ranges -> canonical bytes -> blake3.
///
/// # Errors
/// Returns [`ParseError`] if `source` is not valid Python.
pub fn normalize(source: &str) -> Result<Normalized, ParseError> {
    let module =
        ast::ModModule::parse(source, "<datum>").map_err(|e| ParseError::new(e.to_string()))?;
    let folded = canon::Renamer::new()
        .fold_mod(ast::Mod::Module(module))
        .unwrap_or_else(|e| match e {});
    let canonical = canon::canonical_bytes(&folded);
    Ok(Normalized {
        datum: Datum::of_canonical_bytes(&canonical),
        canonical,
        comments: comments(source),
    })
}

/// Comments in source order. The lexer sees them; the parser does not.
fn comments(source: &str) -> Vec<Comment> {
    lexer::lex(source, Mode::Module)
        .flatten()
        .filter_map(|(tok, range)| match tok {
            Tok::Comment(text) => Some(Comment {
                offset: usize::from(range.start()),
                text,
            }),
            _ => None,
        })
        .collect()
}
