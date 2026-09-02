//! Interface identity for one Python definition: the bytes a *caller* can
//! observe without reading the body.
//!
//! Python's interface is wider than Rust's, because Python's call convention
//! is wider:
//!
//! - `def`: name, decorator list (a decorator rewrites the callable the
//!   caller actually gets), every parameter's *name* (keyword callers depend
//!   on it), its annotation, its `*`/`**`/positional-only/keyword-only
//!   marker, and its default's presence **and value** — a changed default
//!   silently changes every caller that omits the argument, which is exactly
//!   hidden drift. Plus the return annotation and PEP-695 type parameters.
//! - `class`: name, decorator list, base list and class keywords
//!   (`metaclass=`), the class-level annotated assignments that are its
//!   attribute contract, and `__init__`'s signature.
//!
//! Sub-expressions (annotations, defaults, bases) are rendered through the
//! datum's own range-erasing fold, so a sig is invariant under formatting and
//! comments for the same reason a datum is. The fold runs with an empty scope
//! stack, so nothing is alpha-renamed: parameter names *are* interface here,
//! which is the one place the sig must not follow the datum.
use std::fmt::Write as _;

use rustpython_parser::ast::fold::Foldable;
use rustpython_parser::ast::{self, Arg, ArgWithDefault, Ranged, Stmt};
use rustpython_parser::text_size::TextRange;
use rustpython_parser::Parse;

use crate::canon::Renamer;

/// The declared epoch. Sig bytes and datum bytes are different claims about
/// the same node, so they are domain-separated and neither can be mistaken
/// for the other.
const EPOCH: &str = "grid-lang-py-sig/v1\n";

/// Canonical interface bytes for one definition, or an empty interface for a
/// statement that is not one.
#[must_use]
pub(crate) fn sig_bytes(stmt: &Stmt) -> Vec<u8> {
    let mut out = String::from(EPOCH);
    match stmt {
        Stmt::FunctionDef(f) => def(
            &mut out,
            "def",
            f.name.as_str(),
            &f.decorator_list,
            &f.args,
            f.returns.as_deref(),
            &f.type_params,
        ),
        Stmt::AsyncFunctionDef(f) => def(
            &mut out,
            "async def",
            f.name.as_str(),
            &f.decorator_list,
            &f.args,
            f.returns.as_deref(),
            &f.type_params,
        ),
        Stmt::ClassDef(c) => class(&mut out, c),
        // Not a definition: no interface to state, and an empty one is a
        // truthful answer rather than a guess.
        _ => {}
    }
    out.into_bytes()
}

/// One expression as canonical text: range-erased so formatting cannot reach
/// it, `Debug`-printed so the encoding is the AST's own and cannot drift from
/// the datum's. `Debug` never emits a raw newline, so one expression is
/// always one line and the framing below stays unambiguous.
fn expr(node: &ast::Expr<TextRange>) -> String {
    // The fold is infallible (`Infallible` error type) and runs with no
    // scopes pushed, so no name is rewritten.
    node.clone().fold(&mut Renamer::new()).map_or_else(
        |e: std::convert::Infallible| match e {},
        |e| format!("{e:?}"),
    )
}

fn opt_expr(node: Option<&ast::Expr<TextRange>>) -> String {
    node.map_or_else(|| "-".to_owned(), expr)
}

/// A parameter row: marker, name, annotation, default. Four fixed columns so
/// moving an argument between kinds is a different line, not a coincidence.
fn param(out: &mut String, marker: &str, arg: &Arg<TextRange>, default: Option<&ast::Expr>) {
    let _ = writeln!(
        out,
        "p {marker} {} {} {}",
        arg.arg.as_str(),
        opt_expr(arg.annotation.as_deref()),
        opt_expr(default)
    );
}

fn params(out: &mut String, args: &ast::Arguments<TextRange>) {
    let with_default = |out: &mut String, marker: &str, a: &ArgWithDefault<TextRange>| {
        param(out, marker, &a.def, a.default.as_deref());
    };
    for a in &args.posonlyargs {
        with_default(out, "posonly", a);
    }
    for a in &args.args {
        with_default(out, "pos", a);
    }
    if let Some(a) = &args.vararg {
        param(out, "star", a, None);
    } else if !args.kwonlyargs.is_empty() {
        // A bare `*` separator is not a parameter but *is* interface: it is
        // what makes everything after it keyword-only.
        out.push_str("p starsep - - -\n");
    }
    for a in &args.kwonlyargs {
        with_default(out, "kwonly", a);
    }
    if let Some(a) = &args.kwarg {
        param(out, "starstar", a, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn def(
    out: &mut String,
    keyword: &str,
    name: &str,
    decorators: &[ast::Expr<TextRange>],
    args: &ast::Arguments<TextRange>,
    returns: Option<&ast::Expr<TextRange>>,
    type_params: &[ast::TypeParam<TextRange>],
) {
    let _ = writeln!(out, "{keyword} {name}");
    for d in decorators {
        let _ = writeln!(out, "@ {}", expr(d));
    }
    for t in type_params {
        let _ = writeln!(out, "tp {t:?}");
    }
    params(out, args);
    let _ = writeln!(out, "-> {}", opt_expr(returns));
}

fn class(out: &mut String, c: &ast::StmtClassDef<TextRange>) {
    let _ = writeln!(out, "class {}", c.name.as_str());
    for d in &c.decorator_list {
        let _ = writeln!(out, "@ {}", expr(d));
    }
    for t in &c.type_params {
        let _ = writeln!(out, "tp {t:?}");
    }
    for b in &c.bases {
        let _ = writeln!(out, "base {}", expr(b));
    }
    for k in &c.keywords {
        let _ = writeln!(
            out,
            "kw {} {}",
            k.arg.as_ref().map_or("-", ast::Identifier::as_str),
            expr(&k.value)
        );
    }
    for stmt in &c.body {
        match stmt {
            // A class-level annotated assignment is the attribute contract:
            // name, type, and whether it carries a default a caller may omit.
            Stmt::AnnAssign(a) => {
                let _ = writeln!(
                    out,
                    "field {} {} {}",
                    expr(&a.target),
                    expr(&a.annotation),
                    opt_expr(a.value.as_deref())
                );
            }
            // `__init__` is how a caller constructs the class, so its own sig
            // is part of the class's. Other methods have their own
            // definitions and their own sigs; folding them in here would make
            // every method edit look like a class-interface change.
            Stmt::FunctionDef(f) if f.name.as_str() == "__init__" => {
                out.push_str("init ");
                def(
                    out,
                    "def",
                    f.name.as_str(),
                    &f.decorator_list,
                    &f.args,
                    f.returns.as_deref(),
                    &f.type_params,
                );
            }
            Stmt::AsyncFunctionDef(f) if f.name.as_str() == "__init__" => {
                out.push_str("init ");
                def(
                    out,
                    "async def",
                    f.name.as_str(),
                    &f.decorator_list,
                    &f.args,
                    f.returns.as_deref(),
                    &f.type_params,
                );
            }
            _ => {}
        }
    }
}

/// The header of one definition's *own* source: the `def`/`class` line (and
/// the continuation lines a multi-line signature spans), up to the colon that
/// opens the body — decorators included when the parser puts them inside the
/// statement's range.
///
/// Pure — same text in, same text out — and the input is the definition's
/// source as the worktree holds it, which is what `grid scope --sigs` packs.
/// Python's interface is names, annotations and defaults (see this module's
/// rule), and the header is exactly where the source spells them; a statement
/// that is not a definition answers `None` rather than guessing.
#[must_use]
pub fn sig_text(source: &str) -> Option<String> {
    // A method's source arrives indented off its class. Python rejects a
    // leading indent at module level, so the first line's indent comes off
    // every line before the parse; the body's relative indent survives.
    let source = dedent(source);
    let module = ast::ModModule::parse(&source, "<sig>").ok()?;
    let stmt = module.body.first()?;
    let body = match stmt {
        Stmt::FunctionDef(f) => &f.body,
        Stmt::AsyncFunctionDef(f) => &f.body,
        Stmt::ClassDef(c) => &c.body,
        _ => return None,
    };
    let start = usize::from(stmt.range().start());
    let end = usize::from(body.first()?.range().start());
    Some(source.get(start..end)?.trim_end().to_owned())
}

/// Strip the first line's leading whitespace from every line that carries it.
fn dedent(source: &str) -> String {
    let indent = source.len() - source.trim_start_matches([' ', '\t']).len();
    let Some(prefix) = source.get(..indent).filter(|p| !p.is_empty()) else {
        return source.to_owned();
    };
    source
        .lines()
        .map(|line| line.strip_prefix(prefix).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}
