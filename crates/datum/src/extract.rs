//! Definition extraction: a module's top-level and nested `def`/`class`
//! definitions, each with its own identity and raw-source span hash.
use rustpython_parser::ast::{self, EmptyRange, Ranged, Stmt};
use rustpython_parser::Parse;

use crate::canon::{self, Renamer};
use crate::{span_hash, Datum, ParseError};
use rustpython_parser::ast::fold::Fold;

/// What kind of definition a `Definition` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Function,
    Class,
}

/// One extracted definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub kind: DefKind,
    /// Dotted path from module root, e.g. `TagContract.validate`.
    pub callsign: String,
    /// `blake3(normalized AST)` — invariant under formatting, comments and
    /// local renaming.
    pub datum: Datum,
    /// `blake3(raw source bytes of the span)` — sensitive to exactly those.
    pub span_hash: String,
    /// Byte range of the definition in the source.
    pub span: std::ops::Range<usize>,
}

/// Extract every `def`/`class` definition in a Python source file, outermost
/// first, each nested definition following its parent.
///
/// # Errors
/// Returns `ParseError` if the source is not valid Python.
pub fn definitions(source: &str) -> Result<Vec<Definition>, ParseError> {
    let module = ast::ModModule::parse(source, "<datum>").map_err(|e| ParseError(e.to_string()))?;
    let mut out = Vec::new();
    walk(&module.body, source, "", &mut out);
    Ok(out)
}

fn walk(body: &[Stmt], source: &str, prefix: &str, out: &mut Vec<Definition>) {
    for stmt in body {
        let (kind, name, inner) = match stmt {
            Stmt::FunctionDef(f) => (DefKind::Function, f.name.as_str(), &f.body),
            Stmt::AsyncFunctionDef(f) => (DefKind::Function, f.name.as_str(), &f.body),
            Stmt::ClassDef(c) => (DefKind::Class, c.name.as_str(), &c.body),
            _ => continue,
        };
        let callsign = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}.{name}")
        };
        let span = usize::from(stmt.range().start())..usize::from(stmt.range().end());
        // A definition's identity is the identity of a module containing
        // exactly it: nesting and file position never reach the hash.
        let folded = Renamer::new()
            .fold_mod(ast::Mod::Module(ast::ModModule {
                range: EmptyRange::default(),
                body: vec![stmt.clone()],
                type_ignores: Vec::new(),
            }))
            .unwrap_or_else(|e| match e {});
        let text = source.get(span.clone()).unwrap_or_default();
        out.push(Definition {
            kind,
            callsign: callsign.clone(),
            datum: Datum::of_bytes(&canon::canonical_bytes(&folded)),
            span_hash: span_hash(text),
            span,
        });
        walk(inner, source, &callsign, out);
    }
}
