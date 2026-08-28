//! Definition extraction: a module's top-level and nested `def`/`class`
//! definitions, each with its own identity and raw-source span hash.
use rustpython_parser::ast::{self, EmptyRange, Ranged, Stmt, Visitor};
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
        // exactly it: nesting and file position never reach the hash. The
        // *shape* additionally erases the definition's own name — the
        // rename-join key; the datum keeps the name because the chart
        // interns datums uniquely and same-body defs must not collapse.
        let hash_of = |s: &Stmt| {
            let folded = Renamer::new()
                .fold_mod(ast::Mod::Module(ast::ModModule {
                    range: EmptyRange::default(),
                    body: vec![s.clone()],
                    type_ignores: Vec::new(),
                }))
                .unwrap_or_else(|e| match e {});
            Datum::of_canonical_bytes(&canon::canonical_bytes(&folded))
        };
        let mut shaped = stmt.clone();
        match &mut shaped {
            Stmt::FunctionDef(f) => f.name = ast::Identifier::new("__shape"),
            Stmt::AsyncFunctionDef(f) => f.name = ast::Identifier::new("__shape"),
            Stmt::ClassDef(c) => c.name = ast::Identifier::new("__shape"),
            _ => {}
        }
        let text = source.get(span.clone()).unwrap_or_default();
        out.push(Definition {
            kind,
            callsign: callsign.clone(),
            datum: hash_of(stmt),
            span_hash: span_hash(text),
            span,
            shape: hash_of(&shaped),
            complexity: complexity_of(stmt),
        });
        walk(inner, source, &callsign, out);
    }
}

/// Cyclomatic complexity: 1 + branch points. Branches: `if`/`elif`, loops,
/// `except` arms, `and`/`or` operands beyond the first, ternaries,
/// comprehension `if`s, and `match` cases beyond the first.
fn complexity_of(stmt: &Stmt) -> u16 {
    #[derive(Default)]
    struct Cx(u16);
    impl Cx {
        fn bump(&mut self, by: usize) {
            self.0 = self.0.saturating_add(u16::try_from(by).unwrap_or(u16::MAX));
        }
    }
    impl Visitor for Cx {
        fn visit_stmt_if(&mut self, node: ast::StmtIf) {
            self.bump(1);
            self.generic_visit_stmt_if(node);
        }
        fn visit_stmt_for(&mut self, node: ast::StmtFor) {
            self.bump(1);
            self.generic_visit_stmt_for(node);
        }
        fn visit_stmt_async_for(&mut self, node: ast::StmtAsyncFor) {
            self.bump(1);
            self.generic_visit_stmt_async_for(node);
        }
        fn visit_stmt_while(&mut self, node: ast::StmtWhile) {
            self.bump(1);
            self.generic_visit_stmt_while(node);
        }
        fn visit_excepthandler(&mut self, node: ast::ExceptHandler) {
            self.bump(1);
            self.generic_visit_excepthandler(node);
        }
        fn visit_expr_bool_op(&mut self, node: ast::ExprBoolOp) {
            self.bump(node.values.len().saturating_sub(1));
            self.generic_visit_expr_bool_op(node);
        }
        fn visit_expr_if_exp(&mut self, node: ast::ExprIfExp) {
            self.bump(1);
            self.generic_visit_expr_if_exp(node);
        }
        fn visit_stmt_match(&mut self, node: ast::StmtMatch) {
            self.bump(node.cases.len().saturating_sub(1));
            self.generic_visit_stmt_match(node);
        }
        fn visit_comprehension(&mut self, node: ast::Comprehension) {
            self.bump(node.ifs.len());
        }
    }
    let mut cx = Cx(1);
    cx.visit_stmt(stmt.clone());
    cx.0
}
