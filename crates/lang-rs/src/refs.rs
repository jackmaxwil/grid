//! Reference extraction: the raw material of edges, Rust spelling.
//!
//! Same conservatism contract as lang-py's extractor: this module records
//! what a definition reaches for *by name* and refuses to guess bindings.
//! Method calls on non-`self` receivers need types and are dropped; macro
//! bodies are opaque token streams syn's visitor never descends into, so
//! everything inside one is dropped — omissions, never invented edges.
use std::collections::HashMap;

use grid_datum::{
    FileRefs, Import, ParseError, Receiver, ReceiverForm, ReceiverOrigin, RefKind, Reference,
    TypeRef,
};
use syn::visit::Visit;
use syn::{Expr, ImplItem, TraitItem, UseTree};

use crate::extract::{path_dots, type_callsign};
use crate::types;

/// Attributes that are language furniture, not references: emitting them
/// would let a local definition named `test` or `Clone` capture a false
/// edge. Std derives are here for the same reason.
const BUILTIN_ATTRS: &[&str] = &[
    "test",
    "cfg",
    "cfg_attr",
    "allow",
    "warn",
    "deny",
    "forbid",
    "doc",
    "inline",
    "must_use",
    "non_exhaustive",
    "repr",
    "macro_use",
    "macro_export",
    "path",
    "no_std",
    "no_main",
    "derive",
    "ignore",
    "should_panic",
    "Clone",
    "Copy",
    "Debug",
    "Default",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
];

/// Extract the imports and references of a Rust source file.
///
/// # Errors
/// Returns `ParseError` if the source is not valid Rust.
pub fn references(source: &str) -> Result<FileRefs, ParseError> {
    let file = syn::parse_file(source).map_err(|e| ParseError::new(e.to_string()))?;
    Ok(references_of(&file))
}

pub(crate) fn references_of(file: &syn::File) -> FileRefs {
    let mut scan = Scan::default();
    // The same-file tables first: a chain may be typed by a return written
    // further down the file.
    types::collect(&file.items, "", &mut scan.types, &mut scan.out.field_types);
    for item in &file.items {
        scan.visit_item(item);
    }
    scan.out
}

/// One local's type, as the file states it or as the extractor read it off
/// the initializer, plus — for a field read the same file cannot type — the
/// field's name, for the resolver to look up against the base type.
#[derive(Debug, Clone)]
struct Binding {
    ty: TypeRef,
    origin: ReceiverOrigin,
    field: Option<String>,
}

#[derive(Default)]
struct Scan {
    /// Callsign of the definition currently being walked; empty at module
    /// level. Every reference is attributed to this.
    scope: String,
    /// Whether the innermost enclosing body is an impl or trait body — that
    /// is, whether `self` means anything here.
    in_receiver: Vec<bool>,
    /// The inline modules enclosing the current node, outermost first. What
    /// `self::` and `super::` anchor against: rustc resolves them from the
    /// *module* a path sits in, and an inline module is one the file's
    /// module path — the only anchor the resolver holds — does not spell.
    mods: Vec<String>,
    /// Same-file written returns and declared field types, for chains.
    types: types::FileTypes,
    /// The enclosing impl's own type — what `Self` and `self` mean.
    self_ty: Option<TypeRef>,
    /// Generic type parameter names in force: the impl's, then the fn's.
    generics: Vec<String>,
    /// Local bindings, innermost scope last: parameters and `let`s with the
    /// type the file states or the extractor inferred. `None` is a name
    /// bound by a pattern, a closure parameter or a `for` — no syntax here
    /// states its type — which shadows an outer binding and yields nothing.
    locals: Vec<HashMap<String, Option<Binding>>>,
    out: FileRefs,
}

/// Re-anchor a `self::`/`super::` path written inside an inline module so a
/// resolver anchoring against the file's module lands where rustc does:
/// `self::r` is `self::{mods}::r`; `super^k::r` with `k ≤ depth` is
/// `self::{mods[..depth−k]}::r`; a `super` chain deeper than the nesting
/// climbs the file's own module by the remainder, which the resolver already
/// handles. `crate::` and every bare path are untouched, so a file with no
/// inline module extracts exactly as it did before.
fn anchor_in_mod(mods: &[String], path: Vec<String>) -> Vec<String> {
    if mods.is_empty() {
        return path;
    }
    let supers = path.iter().take_while(|s| *s == "super").count();
    let rest = |from: usize| path.get(from..).unwrap_or_default().iter().cloned();
    match path.first().map(String::as_str) {
        Some("self") => std::iter::once("self".to_owned())
            .chain(mods.iter().cloned())
            .chain(rest(1))
            .collect(),
        Some("super") if supers <= mods.len() => std::iter::once("self".to_owned())
            .chain(
                mods.get(..mods.len().saturating_sub(supers))
                    .unwrap_or_default()
                    .iter()
                    .cloned(),
            )
            .chain(rest(supers))
            .collect(),
        Some("super") => std::iter::repeat_n("super".to_owned(), supers.saturating_sub(mods.len()))
            .chain(rest(supers))
            .collect(),
        _ => path,
    }
}

impl Scan {
    fn nested(&mut self, name: &str, is_receiver: bool, f: impl FnOnce(&mut Self)) {
        let inner = join(&self.scope, name);
        let outer = std::mem::replace(&mut self.scope, inner);
        self.in_receiver.push(is_receiver);
        f(self);
        self.in_receiver.pop();
        self.scope = outer;
    }

    fn push(&mut self, kind: RefKind, path: Vec<String>) {
        // A method path's head is a receiver, not an anchor; every other
        // path is one rustc reads from the enclosing module.
        let path = match kind {
            RefKind::MethodCall | RefKind::SelfCall => path,
            _ => anchor_in_mod(&self.mods, path),
        };
        self.out.refs.push(Reference {
            from: self.scope.clone(),
            kind,
            path,
            receiver: None,
        });
    }

    /// Walk `f` inside one more binding scope — a function body, a block, a
    /// closure, a match arm — so a name bound inside it shadows only inside
    /// it, which is the language's own rule.
    fn scoped(&mut self, f: impl FnOnce(&mut Self)) {
        self.locals.push(HashMap::new());
        f(self);
        self.locals.pop();
    }

    fn bind(&mut self, name: String, binding: Option<Binding>) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name, binding);
        }
    }

    fn local(&self, name: &str) -> Option<Binding> {
        self.locals
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .and_then(Clone::clone)
    }

    /// A function body under its own parameters: `self` binds to the impl's
    /// type in the method's own receiver form, a simply named parameter to
    /// its annotation, and any other parameter pattern to nothing.
    fn body(&mut self, sig: &syn::Signature, block: &syn::Block) {
        let outer = self.generics.len();
        self.generics.extend(types::generic_names(&sig.generics));
        self.scoped(|s| {
            for input in &sig.inputs {
                match input {
                    syn::FnArg::Receiver(_) => {
                        let bound = s.self_ty.clone().zip(types::receiver_form(sig)).map(
                            |(mut ty, form)| {
                                ty.form = form;
                                Binding {
                                    ty,
                                    origin: ReceiverOrigin::Param,
                                    field: None,
                                }
                            },
                        );
                        s.bind("self".to_owned(), bound);
                    }
                    syn::FnArg::Typed(t) => {
                        if let syn::Pat::Ident(pi) = &*t.pat {
                            let bound = s.type_ref(&t.ty).map(|ty| Binding {
                                ty,
                                origin: ReceiverOrigin::Param,
                                field: None,
                            });
                            s.bind(pi.ident.to_string(), bound);
                        } else {
                            s.visit_pat(&t.pat);
                        }
                    }
                }
            }
            s.visit_block(block);
        });
        self.generics.truncate(outer);
    }

    fn type_ref(&self, ty: &syn::Type) -> Option<TypeRef> {
        types::type_ref(ty, self.self_ty.as_ref(), &self.generics)
    }

    /// `base.field` on a typed base: the field's declared type when the
    /// struct is in this file, else the base and the field's name for the
    /// resolver to look up in the index.
    fn field_of(&self, f: &syn::ExprField) -> Option<Binding> {
        let base = self.type_of(&f.base)?;
        let syn::Member::Named(id) = &f.member else {
            return None;
        };
        if base.field.is_some() || base.ty.args {
            return None;
        }
        let is_self = matches!(&*f.base, Expr::Path(p) if p.path.is_ident("self"));
        let field = id.to_string();
        match self.types.fields.get(base.ty.path.last()?) {
            // The struct is in this file: its declared type, or nothing for
            // a field no path spells.
            Some(Some(fields)) => fields.get(&field).map(|ty| Binding {
                ty: ty.clone(),
                origin: if is_self {
                    ReceiverOrigin::SelfField
                } else {
                    ReceiverOrigin::TypedField
                },
                field: None,
            }),
            Some(None) => None,
            // Elsewhere: the resolver reads the declared type off the
            // index, so the hint carries the base and the name.
            None => Some(Binding {
                ty: TypeRef {
                    form: ReceiverForm::Value,
                    ..base.ty
                },
                origin: ReceiverOrigin::TypedField,
                field: Some(field),
            }),
        }
    }

    /// What the file says an expression's type is, by the rules
    /// `docs/LANG_RS.md` names and no others: a local's binding, a field of
    /// a typed base, a method or associated-function call whose return is
    /// written in this file, a struct literal, `?`/`unwrap`/`expect` on a
    /// written `Option`/`Result`, and `&`/`&mut` around any of those.
    /// Everything else is `None` — a literal, an index, a macro, a tuple
    /// field, a chain through an unwritten return.
    fn type_of(&self, e: &Expr) -> Option<Binding> {
        match e {
            Expr::Paren(p) => self.type_of(&p.expr),
            Expr::Reference(r) => {
                let mut b = self.type_of(&r.expr)?;
                b.ty.form = if r.mutability.is_some() {
                    ReceiverForm::RefMut
                } else {
                    ReceiverForm::Ref
                };
                Some(b)
            }
            Expr::Path(p) if p.qself.is_none() => {
                let name = p.path.get_ident()?.to_string();
                self.local(&name)
            }
            Expr::Field(f) => self.field_of(f),
            Expr::MethodCall(mc) => {
                let base = self.type_of(&mc.receiver)?;
                if base.field.is_some() {
                    return None;
                }
                let method = mc.method.to_string();
                if method == "unwrap" || method == "expect" {
                    return unwrapped(base);
                }
                let key = format!("{}.{method}", base.ty.path.last()?);
                let ret = self.types.returns.get(&key)?.clone()?;
                Some(Binding {
                    ty: ret,
                    origin: ReceiverOrigin::ChainReturn,
                    field: None,
                })
            }
            Expr::Try(t) => unwrapped(self.type_of(&t.expr)?),
            Expr::Call(c) => {
                let path = dotted(&c.func)?;
                let key = match path.as_slice() {
                    [f] => f.clone(),
                    [.., ty, f] => {
                        let ty = if ty == "Self" {
                            self.self_ty.as_ref()?.path.last()?
                        } else {
                            ty
                        };
                        format!("{ty}.{f}")
                    }
                    [] => return None,
                };
                let ret = self.types.returns.get(&key)?.clone()?;
                Some(Binding {
                    ty: ret,
                    origin: ReceiverOrigin::LocalCtor,
                    field: None,
                })
            }
            Expr::Struct(s) if s.qself.is_none() => {
                let path: Vec<String> = s
                    .path
                    .segments
                    .iter()
                    .map(|p| p.ident.to_string())
                    .collect();
                let ty = if path.first().is_some_and(|h| h == "Self") {
                    (path.len() == 1).then(|| self.self_ty.clone()).flatten()?
                } else {
                    TypeRef {
                        path,
                        args: false,
                        form: ReceiverForm::Value,
                        inner: None,
                    }
                };
                Some(Binding {
                    ty,
                    origin: ReceiverOrigin::LocalLiteral,
                    field: None,
                })
            }
            _ => None,
        }
    }

    /// Attribute and derive references, attributed to the decorated
    /// definition (which is not the scope the attribute is walked in).
    fn decorators(&mut self, owner: &str, attrs: &[syn::Attribute]) {
        let owner = join(&self.scope, owner);
        for attr in attrs {
            if attr.path().is_ident("derive") {
                let parsed = attr.parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                );
                if let Ok(paths) = parsed {
                    for p in paths {
                        self.decorator(&owner, &p);
                    }
                }
                continue;
            }
            self.decorator(&owner, attr.path());
        }
    }

    fn decorator(&mut self, owner: &str, path: &syn::Path) {
        if let Some(ident) = path.get_ident() {
            if BUILTIN_ATTRS.contains(&ident.to_string().as_str()) {
                return;
            }
        }
        self.out.refs.push(Reference {
            from: owner.to_owned(),
            kind: RefKind::Decorator,
            path: anchor_in_mod(
                &self.mods,
                path.segments.iter().map(|s| s.ident.to_string()).collect(),
            ),
            receiver: None,
        });
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

/// `T` out of a written `Option<T>` / `Result<T, _>`, for `?`, `unwrap`
/// and `expect`; nothing for any other wrapper, which is not unwrapped.
fn unwrapped(base: Binding) -> Option<Binding> {
    if base.field.is_some() {
        return None;
    }
    let last = base.ty.path.last()?;
    if last != "Option" && last != "Result" {
        return None;
    }
    base.ty.inner.map(|inner| Binding {
        ty: *inner,
        origin: ReceiverOrigin::Unwrapped,
        field: None,
    })
}

/// A qself-free path as dotted segments, or `None` for anything without a
/// plain name — the honest answer, and the caller records nothing.
fn dotted(expr: &Expr) -> Option<Vec<String>> {
    if let Expr::Path(p) = expr {
        if p.qself.is_none() {
            return Some(
                p.path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect(),
            );
        }
    }
    None
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.decorators(&node.sig.ident.to_string(), &node.attrs);
        // A free function nested in a method sees no `Self`.
        let self_ty = self.self_ty.take();
        self.nested(&node.sig.ident.to_string(), false, |s| {
            s.body(&node.sig, &node.block);
        });
        self.self_ty = self_ty;
    }

    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.scoped(|s| syn::visit::visit_block(s, node));
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        // The initializer sees the *outer* binding of the name it rebinds.
        if let Some(init) = &node.init {
            self.visit_expr(&init.expr);
            if let Some((_, diverge)) = &init.diverge {
                self.visit_expr(diverge);
            }
        }
        match &node.pat {
            syn::Pat::Ident(pi) if pi.subpat.is_none() => {
                let bound = node.init.as_ref().and_then(|i| self.type_of(&i.expr));
                self.bind(pi.ident.to_string(), bound);
            }
            syn::Pat::Type(pt) => {
                if let syn::Pat::Ident(pi) = &*pt.pat {
                    let bound = self.type_ref(&pt.ty).map(|ty| Binding {
                        ty,
                        origin: ReceiverOrigin::LetAnnotated,
                        field: None,
                    });
                    self.bind(pi.ident.to_string(), bound);
                } else {
                    self.visit_pat(&pt.pat);
                }
            }
            other => self.visit_pat(other),
        }
    }

    fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
        // A pattern binding — a closure parameter, a `for`, a `match` arm,
        // a destructuring `let` — has a type no syntax here states. It
        // shadows, and what it shadows yields nothing.
        self.bind(node.ident.to_string(), None);
        syn::visit::visit_pat_ident(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.scoped(|s| syn::visit::visit_expr_closure(s, node));
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        self.scoped(|s| syn::visit::visit_arm(s, node));
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.scoped(|s| syn::visit::visit_expr_for_loop(s, node));
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        self.decorators(&node.ident.to_string(), &node.attrs);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        self.decorators(&node.ident.to_string(), &node.attrs);
    }

    fn visit_item_union(&mut self, node: &'ast syn::ItemUnion) {
        self.decorators(&node.ident.to_string(), &node.attrs);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        let name = node.ident.to_string();
        self.decorators(&name, &node.attrs);
        for ti in &node.items {
            if let TraitItem::Fn(m) = ti {
                if let Some(block) = &m.default {
                    self.nested(&name, true, |s| {
                        s.nested(&m.sig.ident.to_string(), false, |s| s.body(&m.sig, block));
                    });
                }
            }
        }
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let Some(ty) = type_callsign(&node.self_ty) else {
            return;
        };
        // `impl T for Foo` is the implements edge, from `Foo` — unless it is
        // a negative impl, which promises the opposite of an edge.
        if let Some((bang, path, _)) = &node.trait_ {
            if bang.is_none() {
                self.out.refs.push(Reference {
                    from: join(&self.scope, &ty),
                    kind: RefKind::Base,
                    path: anchor_in_mod(
                        &self.mods,
                        path.segments.iter().map(|s| s.ident.to_string()).collect(),
                    ),
                    receiver: None,
                });
            }
        }
        let receiver = match &node.trait_ {
            Some((_, path, _)) => format!("{ty}@{}", path_dots(path)),
            None => ty,
        };
        let outer_generics = self.generics.len();
        self.generics.extend(types::generic_names(&node.generics));
        let self_ty = std::mem::replace(
            &mut self.self_ty,
            types::type_ref(&node.self_ty, None, &self.generics),
        );
        for ii in &node.items {
            if let ImplItem::Fn(m) = ii {
                let owner = format!("{receiver}.{}", m.sig.ident);
                self.decorators(&owner, &m.attrs);
                self.nested(&receiver, true, |s| {
                    s.nested(&m.sig.ident.to_string(), false, |s| {
                        s.body(&m.sig, &m.block);
                    });
                });
            }
        }
        self.self_ty = self_ty;
        self.generics.truncate(outer_generics);
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        // An out-of-line module's file cannot see the `#[cfg]` on its
        // `mod x;`, so the declaration is reported for the survey to join
        // back on by module path. An inline module's items see it directly
        // (the extractor scopes it), and the survey keys the join by file,
        // so reporting one would be an entry nothing can read.
        if node.content.is_none() {
            let cfg = grid_datum::cfg_join(&crate::extract::cfgs_of(&node.attrs));
            if !cfg.is_empty() {
                self.out
                    .mod_cfgs
                    .push((join(&self.scope, &node.ident.to_string()), cfg));
            }
        }
        if let Some((_, items)) = &node.content {
            self.mods.push(node.ident.to_string());
            self.nested(&node.ident.to_string(), false, |s| {
                for item in items {
                    s.visit_item(item);
                }
            });
            self.mods.pop();
            return;
        }
        // `#[path = "..."] mod x;` relocates the module's file. Top-level
        // declarations only: inside an inline mod the path is relative to
        // the mod's own directory, a rule this frontend does not model.
        if !self.scope.is_empty() {
            return;
        }
        for attr in &node.attrs {
            if !attr.path().is_ident("path") {
                continue;
            }
            if let syn::Meta::NameValue(nv) = &attr.meta {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }) = &nv.value
                {
                    self.out
                        .path_mods
                        .push((node.ident.to_string(), lit.value()));
                }
            }
        }
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        let before = self.out.imports.len();
        flatten_use(
            &node.tree,
            &mut Vec::new(),
            &self.scope,
            &mut self.out.imports,
        );
        // A `use self::`/`use super::` inside an inline module anchors
        // against that module, the same as a path does.
        if !self.mods.is_empty() {
            for import in self.out.imports.iter_mut().skip(before) {
                let segs = import.target.split('.').map(str::to_owned).collect();
                import.target = anchor_in_mod(&self.mods, segs).join(".");
            }
        }
        // Any visibility beyond inherited makes the binding reachable from
        // other files — the raw material of alias resolution.
        if !matches!(node.vis, syn::Visibility::Inherited) {
            let reexported = self.out.imports.get(before..).unwrap_or_default().to_vec();
            self.out.reexports.extend(reexported);
        }
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let Some(path) = dotted(&node.func) {
            match path.split_first() {
                // `Self::m(...)` names a sibling under the enclosing impl,
                // which the resolver rebuilds from the `from` callsign.
                Some((head, rest))
                    if head == "Self"
                        && !rest.is_empty()
                        && self
                            .in_receiver
                            .iter()
                            .rev()
                            .nth(1)
                            .copied()
                            .unwrap_or(false) =>
                {
                    self.push(RefKind::SelfCall, rest.to_vec());
                }
                _ => self.push(RefKind::Call, path),
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let receiver = dotted(&node.receiver).unwrap_or_default();
        if receiver == ["self"]
            && self
                .in_receiver
                .iter()
                .rev()
                .nth(1)
                .copied()
                .unwrap_or(false)
        {
            self.push(RefKind::SelfCall, vec![node.method.to_string()]);
        } else {
            // Every other receiver needs a type to resolve. What the file
            // states about it rides beside the reference as a hint; a call
            // the file says nothing about is recorded all the same, so it is
            // a *declined* reference rather than an invisible one — the
            // difference between a chart that knows what it missed and a
            // chart that reports a hole as an absence.
            let hint = self.type_of(&node.receiver).map(|b| Receiver {
                ty: b.ty,
                origin: b.origin,
                field: b.field,
            });
            let mut path = receiver;
            path.push(node.method.to_string());
            self.out.refs.push(Reference {
                from: self.scope.clone(),
                kind: RefKind::MethodCall,
                path,
                receiver: hint,
            });
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_struct(&mut self, node: &'ast syn::ExprStruct) {
        if node.qself.is_none() {
            let path: Vec<String> = node
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if path.first().is_some_and(|h| h != "Self") {
                self.push(RefKind::Call, path.clone());
                // The literal also writes the type's fields — the provable
                // half of the field vocabulary; reads need types.
                self.push(RefKind::FieldWrite, path);
            }
        }
        syn::visit::visit_expr_struct(self, node);
    }
}

/// One `use` tree into flat bindings. A glob binds no name that can be
/// followed, so it is skipped — a name a glob would have supplied simply
/// never resolves, which is the omission the design promises.
fn flatten_use(tree: &UseTree, prefix: &mut Vec<String>, scope: &str, out: &mut Vec<Import>) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use(&p.tree, prefix, scope, out);
            prefix.pop();
        }
        // `use x::{self}` binds the module x itself, under its own name or
        // the rename — `self` is the one leaf that adds no segment.
        UseTree::Name(n) if n.ident == "self" => {
            if let Some(last) = prefix.last() {
                out.push(Import {
                    alias: last.clone(),
                    target: prefix.join("."),
                    scope: scope.to_owned(),
                });
            }
        }
        UseTree::Name(n) => out.push(Import {
            alias: n.ident.to_string(),
            target: target_of(prefix, &n.ident.to_string()),
            scope: scope.to_owned(),
        }),
        UseTree::Rename(r) if r.ident == "self" => out.push(Import {
            alias: r.rename.to_string(),
            target: prefix.join("."),
            scope: scope.to_owned(),
        }),
        UseTree::Rename(r) => out.push(Import {
            alias: r.rename.to_string(),
            target: target_of(prefix, &r.ident.to_string()),
            scope: scope.to_owned(),
        }),
        UseTree::Glob(_) => {}
        UseTree::Group(g) => {
            for item in &g.items {
                flatten_use(item, prefix, scope, out);
            }
        }
    }
}

fn target_of(prefix: &[String], last: &str) -> String {
    let mut segs: Vec<&str> = prefix.iter().map(String::as_str).collect();
    segs.push(last);
    segs.join(".")
}
