//! Reference extraction: the raw material of edges.
//!
//! A [`Definition`](grid_datum::Definition) says what code *is*; a
//! [`Reference`] says what it *reaches for*. Extraction stops at the name:
//! this module knows
//! that `TagContract.validate` calls something spelled `emit`, and refuses to
//! guess which `emit` that is. Binding a name to a datum needs the whole
//! surveyed tree, which is `grid-survey`'s job, not this crate's — so the two
//! halves of an edge are found in the two places that can actually prove them.
//!
//! Traversal is the generated `Visitor`, not a hand-written match: only the
//! nodes that bind a scope or name a target are overridden, so an expression
//! form this module has never heard of still gets walked into. Five of the
//! generated `generic_visit_*` bodies are empty stubs that do not descend
//! (`comprehension`, `arguments`, `arg`, `keyword`, `withitem`, `match_case`);
//! they are filled in below, because a call hiding under one of them is a real
//! edge and silently dropping it is the failure mode this crate exists to
//! avoid.
use rustpython_parser::ast::{self, Visitor};
use rustpython_parser::Parse;

use grid_datum::{FileRefs, Import, ParseError, RefKind, Reference};

/// Extract the imports and references of a Python source file.
///
/// # Errors
/// Returns `ParseError` if the source is not valid Python.
pub fn references(source: &str) -> Result<FileRefs, ParseError> {
    let module =
        ast::ModModule::parse(source, "<datum>").map_err(|e| ParseError::new(e.to_string()))?;
    Ok(references_of(module.body))
}

pub(crate) fn references_of(body: Vec<rustpython_parser::ast::Stmt>) -> FileRefs {
    let mut scan = Scan::default();
    for stmt in body {
        scan.visit_stmt(stmt);
    }
    scan.out
}

#[derive(Default)]
struct Scan {
    /// Callsign of the definition currently being walked; empty at module
    /// level. Every reference is attributed to this.
    scope: String,
    /// Whether the innermost enclosing definition is a method — that is,
    /// whether `self` means anything here.
    in_class: Vec<bool>,
    out: FileRefs,
}

impl Scan {
    /// Walk `f` with `scope` extended by `name`, then restore. Every
    /// scope-entering override goes through this, so the scope stack cannot
    /// leak on an early return.
    fn nested(&mut self, name: &str, is_class: bool, f: impl FnOnce(&mut Self)) {
        let inner = if self.scope.is_empty() {
            name.to_owned()
        } else {
            format!("{}.{name}", self.scope)
        };
        let outer = std::mem::replace(&mut self.scope, inner);
        self.in_class.push(is_class);
        f(self);
        self.in_class.pop();
        self.scope = outer;
    }

    fn push(&mut self, kind: RefKind, path: Vec<String>) {
        self.out.refs.push(Reference {
            from: self.scope.clone(),
            kind,
            path,
        });
    }

    /// A decorator is attributed to the definition it decorates, which is not
    /// the scope the decorator expression is walked in — so it is recorded
    /// explicitly rather than falling out of the generic walk.
    fn decorators(&mut self, owner: &str, decorators: Vec<ast::Expr>) {
        let owner = if self.scope.is_empty() {
            owner.to_owned()
        } else {
            format!("{}.{owner}", self.scope)
        };
        for d in decorators {
            // `@d` and `@d(arg)` are the same reference to `d`; the arguments
            // are ordinary expressions and are walked as such.
            let target = match d {
                ast::Expr::Call(c) => {
                    for arg in c.args {
                        self.visit_expr(arg);
                    }
                    for kw in c.keywords {
                        self.visit_expr(kw.value);
                    }
                    *c.func
                }
                other => other,
            };
            if let Some(path) = dotted(&target) {
                self.out.refs.push(Reference {
                    from: owner.clone(),
                    kind: RefKind::Decorator,
                    path,
                });
            }
            self.visit_expr(target);
        }
    }

    /// `self.x = ...` in a method writes a field of the enclosing class —
    /// the one attribute write Python fixes lexically, no types needed.
    /// Anything else (a typed receiver, a module-level `self`) is declined.
    fn self_field_write(&mut self, target: &ast::Expr) {
        let ast::Expr::Attribute(a) = target else {
            return;
        };
        let ast::Expr::Name(n) = a.value.as_ref() else {
            return;
        };
        if n.id.as_str() != "self" || !self.in_class.iter().rev().nth(1).copied().unwrap_or(false) {
            return;
        }
        let segments: Vec<String> = self.scope.split('.').map(str::to_owned).collect();
        if let Some((_, class)) = segments.split_last() {
            if !class.is_empty() {
                self.push(RefKind::FieldWrite, class.to_vec());
            }
        }
    }
}

impl Visitor for Scan {
    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        self.decorators(node.name.as_str(), node.decorator_list);
        // Defaults and annotations evaluate in the *enclosing* scope, so they
        // are walked before the scope changes.
        self.visit_arguments(*node.args);
        self.nested(node.name.as_str(), false, |s| {
            for stmt in node.body {
                s.visit_stmt(stmt);
            }
        });
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        self.decorators(node.name.as_str(), node.decorator_list);
        self.visit_arguments(*node.args);
        self.nested(node.name.as_str(), false, |s| {
            for stmt in node.body {
                s.visit_stmt(stmt);
            }
        });
    }

    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        self.decorators(node.name.as_str(), node.decorator_list);
        self.nested(node.name.as_str(), true, |s| {
            for base in node.bases {
                if let Some(path) = dotted(&base) {
                    s.push(RefKind::Base, path);
                }
                s.visit_expr(base);
            }
            for kw in node.keywords {
                s.visit_expr(kw.value);
            }
            for stmt in node.body {
                s.visit_stmt(stmt);
            }
        });
    }

    fn visit_stmt_import(&mut self, node: ast::StmtImport) {
        for alias in node.names {
            let target = alias.name.to_string();
            let bound = alias.asname.as_ref().map_or_else(
                // `import a.b.c` binds `a`, not `a.b.c`.
                || target.split('.').next().unwrap_or(&target).to_owned(),
                ToString::to_string,
            );
            self.out.imports.push(Import {
                alias: bound,
                target,
                scope: self.scope.clone(),
            });
        }
    }

    fn visit_stmt_import_from(&mut self, node: ast::StmtImportFrom) {
        // Relative imports keep their dots; the resolver anchors them against
        // the importing module's own package.
        let dots = ".".repeat(node.level.map_or(0, |l| l.to_usize()));
        let module = node
            .module
            .as_ref()
            .map_or(String::new(), ToString::to_string);
        for alias in node.names {
            let name = alias.name.to_string();
            // `from x import *` binds no name we can follow.
            if name == "*" {
                continue;
            }
            let target = if module.is_empty() {
                format!("{dots}{name}")
            } else {
                format!("{dots}{module}.{name}")
            };
            self.out.imports.push(Import {
                alias: alias.asname.as_ref().map_or(name, ToString::to_string),
                target,
                scope: self.scope.clone(),
            });
        }
    }

    // --- Descent the generated visitor does not do for itself. ---
    // These `generic_visit_*` bodies are empty in rustpython-ast, so without
    // these overrides every call inside a comprehension, a default argument,
    // a keyword argument, a `with` header or a `match` arm would be invisible.

    fn visit_comprehension(&mut self, node: ast::Comprehension) {
        self.visit_expr(node.target);
        self.visit_expr(node.iter);
        for cond in node.ifs {
            self.visit_expr(cond);
        }
    }

    fn visit_arguments(&mut self, node: ast::Arguments) {
        for arg in node
            .posonlyargs
            .into_iter()
            .chain(node.args)
            .chain(node.kwonlyargs)
        {
            self.visit_arg(arg.def);
            if let Some(default) = arg.default {
                self.visit_expr(*default);
            }
        }
        for arg in node.vararg.into_iter().chain(node.kwarg) {
            self.visit_arg(*arg);
        }
    }

    fn visit_arg(&mut self, node: ast::Arg) {
        if let Some(annotation) = node.annotation {
            self.visit_expr(*annotation);
        }
    }

    fn visit_keyword(&mut self, node: ast::Keyword) {
        self.visit_expr(node.value);
    }

    fn visit_withitem(&mut self, node: ast::WithItem) {
        self.visit_expr(node.context_expr);
        if let Some(vars) = node.optional_vars {
            self.visit_expr(*vars);
        }
    }

    fn visit_match_case(&mut self, node: ast::MatchCase) {
        if let Some(guard) = node.guard {
            self.visit_expr(*guard);
        }
        for stmt in node.body {
            self.visit_stmt(stmt);
        }
    }

    fn visit_stmt_assign(&mut self, node: ast::StmtAssign) {
        for target in &node.targets {
            self.self_field_write(target);
        }
        self.generic_visit_stmt_assign(node);
    }

    fn visit_stmt_aug_assign(&mut self, node: ast::StmtAugAssign) {
        self.self_field_write(&node.target);
        self.generic_visit_stmt_aug_assign(node);
    }

    fn visit_stmt_ann_assign(&mut self, node: ast::StmtAnnAssign) {
        self.self_field_write(&node.target);
        self.generic_visit_stmt_ann_assign(node);
    }

    fn visit_expr_call(&mut self, node: ast::ExprCall) {
        if let Some(path) = dotted(&node.func) {
            // `self.m(...)` names a sibling of the enclosing definition; the
            // resolver needs to know that, and needs a `self` that is really
            // a method receiver rather than a module-level variable.
            match path.split_first() {
                Some((head, rest))
                    if head == "self"
                        && !rest.is_empty()
                        && self.in_class.iter().rev().nth(1).copied().unwrap_or(false) =>
                {
                    self.push(RefKind::SelfCall, rest.to_vec());
                }
                // `a.b(...)` is an attribute call: it may be a function in an
                // imported module, which the resolver binds, or a method on a
                // value, which needs a type it does not have. Extraction
                // cannot tell them apart, so it records the shape and lets
                // the resolver's own rules decide — a declined one is then
                // counted as a method call rather than as an unknown name.
                _ if path.len() > 1 => self.push(RefKind::MethodCall, path),
                _ => self.push(RefKind::Call, path),
            }
        }
        self.generic_visit_expr_call(node);
    }
}

/// Flatten a name or attribute chain into its dotted path. Anything else
/// (a subscript, a call result, a literal) has no name, so it has no path:
/// `None` is the honest answer and the caller records nothing.
fn dotted(expr: &ast::Expr) -> Option<Vec<String>> {
    match expr {
        ast::Expr::Name(n) => Some(vec![n.id.to_string()]),
        ast::Expr::Attribute(a) => {
            let mut path = dotted(&a.value)?;
            path.push(a.attr.to_string());
            Some(path)
        }
        _ => None,
    }
}
