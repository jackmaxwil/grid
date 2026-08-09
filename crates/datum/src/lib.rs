//! grid-datum — identity: parse -> normalize -> blake3. Pure, no I/O.
//!
//! A [`Datum`] is what code *is*, independent of how it is written: reformat
//! it, rename its locals, rewrite its comments, and the datum is unchanged.
//! Change a name it calls, a literal, or its structure, and the datum changes.
//! It is the reference every other coordinate is measured from (wire schema
//! `Datum`, `^b3:[0-9a-f]{64}$`).
//!
//! [`span_hash`] is the opposite instrument: the raw source bytes, sensitive
//! to everything the datum ignores. The verdict cache needs both — lint keys
//! on the span, typecheck and test key on identity.
mod canon;
mod extract;

pub use extract::{definitions, DefKind, Definition};

use rustpython_parser::ast;
use rustpython_parser::ast::fold::Fold;
use rustpython_parser::{lexer, Mode, Parse, Tok};

/// Content identity: `b3:` + 64 lowercase hex of the canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Datum(String);

impl Datum {
    /// Hash canonical bytes into a datum. The only constructor: a datum that
    /// did not come from normalization is not a datum.
    #[must_use]
    pub(crate) fn of_bytes(bytes: &[u8]) -> Self {
        Self(format!("b3:{}", blake3::hash(bytes).to_hex()))
    }

    /// The wire form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Datum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

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

/// Source that is not valid Python. Grid declines to hash it; it never guesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub(crate) String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse -> alpha-rename locals -> erase ranges -> canonical bytes -> blake3.
///
/// # Errors
/// Returns [`ParseError`] if `source` is not valid Python.
pub fn normalize(source: &str) -> Result<Normalized, ParseError> {
    let module = ast::ModModule::parse(source, "<datum>").map_err(|e| ParseError(e.to_string()))?;
    let folded = canon::Renamer::new()
        .fold_mod(ast::Mod::Module(module))
        .unwrap_or_else(|e| match e {});
    let canonical = canon::canonical_bytes(&folded);
    Ok(Normalized {
        datum: Datum::of_bytes(&canonical),
        canonical,
        comments: comments(source),
    })
}

/// `blake3` of raw source bytes: sensitive to formatting and comments by
/// design. The lint-kind cache key, where formatting *is* the subject.
#[must_use]
pub fn span_hash(source: &str) -> String {
    format!("b3:{}", blake3::hash(source.as_bytes()).to_hex())
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
