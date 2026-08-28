//! grid-lang-rs — the Rust frontend: parse -> canonicalize -> identity.
//!
//! Design and ceilings: `docs/LANG_RS.md`. The v1 datum is invariant under
//! formatting and comments (doc comments are erased from the token stream
//! before hashing — syn keeps them as `#[doc]` attributes, and unerased they
//! would move the datum on a doc edit) but NOT under local renames: renaming
//! a local moves the datum. That errs conservatively — a false "changed",
//! never a false "same" — and a scope-aware alpha renamer is the upgrade.
mod canon;
mod extract;
mod refs;

pub use extract::definitions;
pub use refs::references;

use grid_datum::{Definition, FileRefs, ParseError};

/// Both extractions off one parse — the survey's hot path, where parsing
/// every file twice was measured at half the whole extraction phase.
///
/// # Errors
/// Returns [`ParseError`] if `source` is not valid Rust.
pub fn file_facts(source: &str) -> Result<(Vec<Definition>, FileRefs), ParseError> {
    let file = syn::parse_file(source).map_err(|e| ParseError::new(e.to_string()))?;
    Ok((
        extract::definitions_of(&file, source),
        refs::references_of(&file),
    ))
}
