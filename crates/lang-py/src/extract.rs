//! Definition extraction: a module's top-level and nested `def`/`class`
//! definitions, each with its own identity and raw-source span hash.
use rustpython_parser::ast::{self, EmptyRange, Ranged, Stmt};
use rustpython_parser::Parse;

use crate::canon::{self, Renamer};
use grid_datum::{span_hash, Datum, DefKind, Definition, ParseError};
use rustpython_parser::ast::fold::Fold;

/// Extract every `def`/`class` definition in a Python source file, outermost
/// first, each nested definition following its parent.
///
/// # Errors
/// Returns `ParseError` if the source is not valid Python.
pub fn definitions(source: &str) -> Result<Vec<Definition>, ParseError> {
    let module =
        ast::ModModule::parse(source, "<datum>").map_err(|e| ParseError::new(e.to_string()))?;
    Ok(definitions_of(&module.body, source))
}

pub(crate) fn definitions_of(body: &[Stmt], source: &str) -> Vec<Definition> {
    let mut out = Vec::new();
    walk(body, source, "", &mut out);
    out
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
            datum: Datum::of_canonical_bytes(&canon::canonical_bytes(&folded)),
            span_hash: span_hash(text),
            span,
        });
        walk(inner, source, &callsign, out);
    }
}
