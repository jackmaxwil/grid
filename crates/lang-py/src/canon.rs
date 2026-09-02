//! Canonicalize by parse -> alpha-rename locals -> erase ranges -> stable
//! serialization. Deterministic by construction: the folded AST carries `()`
//! ranges, so formatting never reaches the canonical bytes, and derived
//! `Debug` on a fixed AST type is a stable byte encoding within a crate
//! version. Locals canonicalize to `v0..` in binding order, so the canonical
//! form is invariant under local renaming; module- and class-scope names are
//! interface and are never renamed.
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;

use rustpython_parser::ast::fold::{self, Fold, Foldable};
use rustpython_parser::ast::{
    self, Alias, Arg, ArgWithDefault, Arguments, Comprehension, EmptyRange,
    ExceptHandlerExceptHandler, ExprDictComp, ExprGeneratorExp, ExprLambda, ExprListComp, ExprName,
    ExprSetComp, Identifier, Mod, PatternMatchAs, PatternMatchMapping, PatternMatchStar, Stmt,
    StmtAsyncFunctionDef, StmtClassDef, StmtFunctionDef, StmtGlobal, StmtNonlocal, StmtTypeAlias,
    TypeIgnore,
};
use rustpython_parser::text_size::TextRange;

/// One lexical scope during renaming. Only function-like scopes rename;
/// module and class scopes are transparent (their names are interface).
#[derive(Default)]
struct Scope {
    renaming: bool,
    map: HashMap<String, String>,
    globals: HashSet<String>,
    nonlocals: HashSet<String>,
    next: usize,
}

impl Scope {
    fn bind(&mut self, name: &str) -> String {
        if !self.renaming {
            return name.to_owned();
        }
        if let Some(canon) = self.map.get(name) {
            return canon.clone();
        }
        let canon = format!("v{}", self.next);
        self.next += 1;
        self.map.insert(name.to_owned(), canon.clone());
        canon
    }
}

/// Stateful folder: alpha-renames locals and erases ranges, yielding an AST
/// whose `Debug` print is the canonical form.
#[derive(Default)]
pub struct Renamer {
    scopes: Vec<Scope>,
}

impl Renamer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn bind_in_current(&mut self, name: &str) -> String {
        self.scopes
            .last_mut()
            .map_or_else(|| name.to_owned(), |s| s.bind(name))
    }

    fn resolve(&self, name: &str) -> Option<String> {
        for scope in self.scopes.iter().rev() {
            if scope.globals.contains(name) {
                return None;
            }
            if scope.nonlocals.contains(name) {
                continue;
            }
            if let Some(canon) = scope.map.get(name) {
                return Some(canon.clone());
            }
        }
        None
    }

    fn push(&mut self, renaming: bool) {
        self.scopes.push(Scope {
            renaming,
            ..Scope::default()
        });
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    /// Fold one parameter's annotation in the current (enclosing) scope
    /// without binding the name yet.
    fn fold_arg_outer(&mut self, arg: Arg<TextRange>) -> Result<Arg<()>, Infallible> {
        let Arg {
            range: _,
            arg,
            annotation,
            type_comment,
        } = arg;
        Ok(Arg {
            range: (),
            arg,
            annotation: annotation.fold(self)?,
            type_comment,
        })
    }

    fn bind_arg(&mut self, arg: &mut Arg<()>) {
        arg.arg = Identifier::new(self.bind_in_current(arg.arg.as_str()));
    }

    /// Fold an `Arguments` block. Defaults and annotations evaluate in the
    /// enclosing scope and must fold before this call's bindings; parameter
    /// names then bind in the (already pushed) function scope, canonicalized
    /// in declaration order.
    fn fold_arguments(&mut self, node: Arguments<TextRange>) -> Result<Arguments<()>, Infallible> {
        let Arguments {
            range: _,
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            kwarg,
        } = node;
        let fold_awd = |f: &mut Self,
                        awd: ArgWithDefault<TextRange>|
         -> Result<ArgWithDefault<()>, Infallible> {
            let ArgWithDefault {
                range: _,
                def,
                default,
            } = awd;
            Ok(ArgWithDefault {
                range: EmptyRange::default(),
                def: f.fold_arg_outer(def)?,
                default: default.fold(f)?,
            })
        };
        let mut posonlyargs = posonlyargs
            .into_iter()
            .map(|a| fold_awd(self, a))
            .collect::<Result<Vec<_>, _>>()?;
        let mut args = args
            .into_iter()
            .map(|a| fold_awd(self, a))
            .collect::<Result<Vec<_>, _>>()?;
        let mut vararg = vararg.map(|a| self.fold_arg_outer(*a)).transpose()?;
        let mut kwonlyargs = kwonlyargs
            .into_iter()
            .map(|a| fold_awd(self, a))
            .collect::<Result<Vec<_>, _>>()?;
        let mut kwarg = kwarg.map(|a| self.fold_arg_outer(*a)).transpose()?;
        for a in &mut posonlyargs {
            self.bind_arg(&mut a.def);
        }
        for a in &mut args {
            self.bind_arg(&mut a.def);
        }
        if let Some(a) = &mut vararg {
            self.bind_arg(a);
        }
        for a in &mut kwonlyargs {
            self.bind_arg(&mut a.def);
        }
        if let Some(a) = &mut kwarg {
            self.bind_arg(a);
        }
        Ok(Arguments {
            range: EmptyRange::default(),
            posonlyargs,
            args,
            vararg: vararg.map(Box::new),
            kwonlyargs,
            kwarg: kwarg.map(Box::new),
        })
    }

    /// Fold a function body in a fresh renaming scope (params included).
    fn fold_callable(
        &mut self,
        args: Arguments<TextRange>,
        body: Vec<Stmt<TextRange>>,
    ) -> Result<(Arguments<()>, Vec<Stmt<()>>), Infallible> {
        self.push(true);
        let args = self.fold_arguments(args)?;
        let body = body.fold(self)?;
        self.pop();
        Ok((args, body))
    }

    /// Fold a generator binding: target (binds), then conditions.
    fn fold_generator(
        &mut self,
        target: ast::Expr<TextRange>,
        ifs: Vec<ast::Expr<TextRange>>,
        is_async: bool,
        iter: ast::Expr<()>,
    ) -> Result<Comprehension<()>, Infallible> {
        Ok(Comprehension {
            range: EmptyRange::default(),
            target: target.fold(self)?,
            iter,
            ifs: ifs.fold(self)?,
            is_async,
        })
    }

    /// Open a comprehension scope and fold its generators, leaving the scope
    /// open so the element expression folds inside it. Caller must `pop`.
    /// The outermost iterable evaluates in the enclosing scope (Python
    /// semantics), so it folds before the scope opens.
    fn open_comprehension(
        &mut self,
        generators: Vec<Comprehension<TextRange>>,
    ) -> Result<Vec<Comprehension<()>>, Infallible> {
        let mut gens = generators.into_iter();
        let Some(first) = gens.next() else {
            return Ok(Vec::new());
        };
        let Comprehension {
            range: _,
            target,
            iter,
            ifs,
            is_async,
        } = first;
        let first_iter = iter.fold(self)?;
        self.push(true);
        let mut out = vec![self.fold_generator(target, ifs, is_async, first_iter)?];
        for gen in gens {
            let Comprehension {
                range: _,
                target,
                iter,
                ifs,
                is_async,
            } = gen;
            let iter = iter.fold(self)?;
            out.push(self.fold_generator(target, ifs, is_async, iter)?);
        }
        Ok(out)
    }
}

impl Fold<TextRange> for Renamer {
    type TargetU = ();
    type Error = Infallible;
    type UserContext = ();

    fn will_map_user(&mut self, _user: &TextRange) -> Self::UserContext {}

    fn map_user(
        &mut self,
        _user: TextRange,
        _context: Self::UserContext,
    ) -> Result<Self::TargetU, Self::Error> {
        Ok(())
    }

    fn fold_mod(&mut self, node: Mod<TextRange>) -> Result<Mod<()>, Infallible> {
        match node {
            Mod::Module(m) => {
                self.push(false);
                let folded = Mod::Module(ast::ModModule {
                    range: EmptyRange::default(),
                    body: m.body.fold(self)?,
                    type_ignores: m
                        .type_ignores
                        .into_iter()
                        .map(|ti| match ti {
                            TypeIgnore::TypeIgnore(t) => {
                                TypeIgnore::TypeIgnore(ast::TypeIgnoreTypeIgnore {
                                    range: EmptyRange::default(),
                                    lineno: t.lineno,
                                    tag: t.tag,
                                })
                            }
                        })
                        .collect(),
                });
                self.pop();
                Ok(folded)
            }
            other => fold::fold_mod(self, other),
        }
    }

    fn fold_stmt_function_def(
        &mut self,
        node: StmtFunctionDef<TextRange>,
    ) -> Result<StmtFunctionDef<()>, Infallible> {
        let StmtFunctionDef {
            range: _,
            name,
            args,
            body,
            decorator_list,
            returns,
            type_comment,
            type_params,
        } = node;
        // Enclosing scope: own-name binding, decorators, return annotation,
        // type params. Only then does the function scope open.
        let name = Identifier::new(self.bind_in_current(name.as_str()));
        let decorator_list = decorator_list.fold(self)?;
        let returns = returns.fold(self)?;
        let type_params = type_params.fold(self)?;
        let (args, body) = self.fold_callable(*args, body)?;
        Ok(StmtFunctionDef {
            range: (),
            name,
            args: Box::new(args),
            body,
            decorator_list,
            returns,
            type_comment,
            type_params,
        })
    }

    fn fold_stmt_async_function_def(
        &mut self,
        node: StmtAsyncFunctionDef<TextRange>,
    ) -> Result<StmtAsyncFunctionDef<()>, Infallible> {
        let StmtAsyncFunctionDef {
            range: _,
            name,
            args,
            body,
            decorator_list,
            returns,
            type_comment,
            type_params,
        } = node;
        let name = Identifier::new(self.bind_in_current(name.as_str()));
        let decorator_list = decorator_list.fold(self)?;
        let returns = returns.fold(self)?;
        let type_params = type_params.fold(self)?;
        let (args, body) = self.fold_callable(*args, body)?;
        Ok(StmtAsyncFunctionDef {
            range: (),
            name,
            args: Box::new(args),
            body,
            decorator_list,
            returns,
            type_comment,
            type_params,
        })
    }

    fn fold_stmt_class_def(
        &mut self,
        node: StmtClassDef<TextRange>,
    ) -> Result<StmtClassDef<()>, Infallible> {
        let StmtClassDef {
            range: _,
            name,
            bases,
            keywords,
            body,
            decorator_list,
            type_params,
        } = node;
        let name = Identifier::new(self.bind_in_current(name.as_str()));
        let bases = bases.fold(self)?;
        let keywords = keywords.fold(self)?;
        let decorator_list = decorator_list.fold(self)?;
        let type_params = type_params.fold(self)?;
        // Class scope is transparent: names bound in a class body are
        // interface (attributes), never locals, and methods do not close over
        // them. A non-renaming scope models exactly that.
        self.push(false);
        let body = body.fold(self)?;
        self.pop();
        Ok(StmtClassDef {
            range: (),
            name,
            bases,
            keywords,
            body,
            decorator_list,
            type_params,
        })
    }

    fn fold_stmt_type_alias(
        &mut self,
        node: StmtTypeAlias<TextRange>,
    ) -> Result<StmtTypeAlias<()>, Infallible> {
        let StmtTypeAlias {
            range: _,
            name,
            type_params,
            value,
        } = node;
        let type_params = type_params.fold(self)?;
        let value = value.fold(self)?;
        let name = Box::new(match *name {
            ast::Expr::Name(n) => ast::Expr::Name(ExprName {
                range: (),
                id: Identifier::new(self.bind_in_current(n.id.as_str())),
                ctx: n.ctx,
            }),
            other => other.fold(self)?,
        });
        Ok(StmtTypeAlias {
            range: (),
            name,
            type_params,
            value,
        })
    }

    fn fold_stmt_global(
        &mut self,
        node: StmtGlobal<TextRange>,
    ) -> Result<StmtGlobal<()>, Infallible> {
        if let Some(scope) = self.scopes.last_mut() {
            scope
                .globals
                .extend(node.names.iter().map(|n| n.as_str().to_owned()));
        }
        Ok(StmtGlobal {
            range: (),
            names: node.names,
        })
    }

    fn fold_stmt_nonlocal(
        &mut self,
        node: StmtNonlocal<TextRange>,
    ) -> Result<StmtNonlocal<()>, Infallible> {
        if let Some(scope) = self.scopes.last_mut() {
            scope
                .nonlocals
                .extend(node.names.iter().map(|n| n.as_str().to_owned()));
        }
        Ok(StmtNonlocal {
            range: (),
            names: node.names,
        })
    }

    fn fold_expr_name(&mut self, node: ExprName<TextRange>) -> Result<ExprName<()>, Infallible> {
        let ExprName { range: _, id, ctx } = node;
        let id = match ctx {
            ast::ExprContext::Load => self
                .resolve(id.as_str())
                .unwrap_or_else(|| id.as_str().to_owned()),
            ast::ExprContext::Store | ast::ExprContext::Del => self.bind_in_current(id.as_str()),
        };
        Ok(ExprName {
            range: (),
            id: Identifier::new(id),
            ctx,
        })
    }

    fn fold_expr_lambda(
        &mut self,
        node: ExprLambda<TextRange>,
    ) -> Result<ExprLambda<()>, Infallible> {
        let ExprLambda {
            range: _,
            args,
            body,
        } = node;
        self.push(true);
        let args = self.fold_arguments(*args)?;
        let body = body.fold(self)?;
        self.pop();
        Ok(ExprLambda {
            range: (),
            args: Box::new(args),
            body,
        })
    }

    fn fold_expr_list_comp(
        &mut self,
        node: ExprListComp<TextRange>,
    ) -> Result<ExprListComp<()>, Infallible> {
        let generators = self.open_comprehension(node.generators)?;
        let elt = node.elt.fold(self)?;
        self.pop();
        Ok(ExprListComp {
            range: (),
            elt,
            generators,
        })
    }

    fn fold_expr_set_comp(
        &mut self,
        node: ExprSetComp<TextRange>,
    ) -> Result<ExprSetComp<()>, Infallible> {
        let generators = self.open_comprehension(node.generators)?;
        let elt = node.elt.fold(self)?;
        self.pop();
        Ok(ExprSetComp {
            range: (),
            elt,
            generators,
        })
    }

    fn fold_expr_dict_comp(
        &mut self,
        node: ExprDictComp<TextRange>,
    ) -> Result<ExprDictComp<()>, Infallible> {
        let generators = self.open_comprehension(node.generators)?;
        let key = node.key.fold(self)?;
        let value = node.value.fold(self)?;
        self.pop();
        Ok(ExprDictComp {
            range: (),
            key,
            value,
            generators,
        })
    }

    fn fold_expr_generator_exp(
        &mut self,
        node: ExprGeneratorExp<TextRange>,
    ) -> Result<ExprGeneratorExp<()>, Infallible> {
        let generators = self.open_comprehension(node.generators)?;
        let elt = node.elt.fold(self)?;
        self.pop();
        Ok(ExprGeneratorExp {
            range: (),
            elt,
            generators,
        })
    }

    fn fold_excepthandler_except_handler(
        &mut self,
        node: ExceptHandlerExceptHandler<TextRange>,
    ) -> Result<ExceptHandlerExceptHandler<()>, Infallible> {
        let ExceptHandlerExceptHandler {
            range: _,
            type_,
            name,
            body,
        } = node;
        let type_ = type_.fold(self)?;
        let name = name.map(|n| Identifier::new(self.bind_in_current(n.as_str())));
        let body = body.fold(self)?;
        Ok(ExceptHandlerExceptHandler {
            range: (),
            type_,
            name,
            body,
        })
    }

    fn fold_alias(&mut self, node: Alias<TextRange>) -> Result<Alias<()>, Infallible> {
        let Alias {
            range: _,
            name,
            asname,
        } = node;
        // An import binds a local: the asname if present, else the first
        // dotted segment of the module path. The rest of the path is the
        // module's interface and is left alone.
        let (name, asname) = if let Some(a) = asname {
            (
                name,
                Some(Identifier::new(self.bind_in_current(a.as_str()))),
            )
        } else {
            let first = name.as_str().split('.').next().unwrap_or_default();
            let canon = self.bind_in_current(first);
            let rest = &name.as_str()[first.len()..];
            (Identifier::new(canon + rest), None)
        };
        Ok(Alias {
            range: Default::default(),
            name,
            asname,
        })
    }

    fn fold_pattern_match_as(
        &mut self,
        node: PatternMatchAs<TextRange>,
    ) -> Result<PatternMatchAs<()>, Infallible> {
        let pattern = node.pattern.fold(self)?;
        let name = node
            .name
            .map(|n| Identifier::new(self.bind_in_current(n.as_str())));
        Ok(PatternMatchAs {
            range: (),
            pattern,
            name,
        })
    }

    fn fold_pattern_match_star(
        &mut self,
        node: PatternMatchStar<TextRange>,
    ) -> Result<PatternMatchStar<()>, Infallible> {
        Ok(PatternMatchStar {
            range: (),
            name: node
                .name
                .map(|n| Identifier::new(self.bind_in_current(n.as_str()))),
        })
    }

    fn fold_pattern_match_mapping(
        &mut self,
        node: PatternMatchMapping<TextRange>,
    ) -> Result<PatternMatchMapping<()>, Infallible> {
        let keys = node.keys.fold(self)?;
        let patterns = node.patterns.fold(self)?;
        let rest = node
            .rest
            .map(|n| Identifier::new(self.bind_in_current(n.as_str())));
        Ok(PatternMatchMapping {
            range: (),
            keys,
            patterns,
            rest,
        })
    }
}

/// Serialize a folded (range-free, alpha-renamed) AST to canonical bytes.
/// `Debug` on the fixed generic AST type is a deterministic encoding: field
/// order is declaration order, strings escape via Rust rules, and every
/// range is `()`.
pub fn canonical_bytes(mod_: &Mod<()>) -> Vec<u8> {
    let mut out = b"grid-datum/v1\n".to_vec();
    out.extend_from_slice(format!("{mod_:?}").as_bytes());
    out
}
