//! Reference extraction: the raw material of edges, Rust spelling.
//!
//! Same conservatism contract as lang-py's extractor: this module records
//! what a definition reaches for *by name* and refuses to guess bindings.
//! Method calls on non-`self` receivers need types and are dropped; macro
//! bodies are opaque token streams syn's visitor never descends into, so
//! everything inside one is dropped — omissions, never invented edges.
use grid_datum::{FileRefs, Import, ParseError, RefKind, Reference};
use syn::visit::Visit;
use syn::{Expr, ImplItem, TraitItem, UseTree};

use crate::extract::{path_dots, type_callsign};

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
    for item in &file.items {
        scan.visit_item(item);
    }
    scan.out
}

#[derive(Default)]
struct Scan {
    /// Callsign of the definition currently being walked; empty at module
    /// level. Every reference is attributed to this.
    scope: String,
    /// Whether the innermost enclosing body is an impl or trait body — that
    /// is, whether `self` means anything here.
    in_receiver: Vec<bool>,
    out: FileRefs,
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
        self.out.refs.push(Reference {
            from: self.scope.clone(),
            kind,
            path,
        });
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
            path: path.segments.iter().map(|s| s.ident.to_string()).collect(),
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
        self.nested(&node.sig.ident.to_string(), false, |s| {
            s.visit_block(&node.block);
        });
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
                        s.nested(&m.sig.ident.to_string(), false, |s| s.visit_block(block));
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
                    path: path.segments.iter().map(|s| s.ident.to_string()).collect(),
                });
            }
        }
        let receiver = match &node.trait_ {
            Some((_, path, _)) => format!("{ty}@{}", path_dots(path)),
            None => ty,
        };
        for ii in &node.items {
            if let ImplItem::Fn(m) = ii {
                let owner = format!("{receiver}.{}", m.sig.ident);
                self.decorators(&owner, &m.attrs);
                self.nested(&receiver, true, |s| {
                    s.nested(&m.sig.ident.to_string(), false, |s| s.visit_block(&m.block));
                });
            }
        }
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if let Some((_, items)) = &node.content {
            self.nested(&node.ident.to_string(), false, |s| {
                for item in items {
                    s.visit_item(item);
                }
            });
        }
    }

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        flatten_use(
            &node.tree,
            &mut Vec::new(),
            &self.scope,
            &mut self.out.imports,
        );
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
        if let Some(head) = dotted(&node.receiver).as_deref() {
            if head == ["self"]
                && self
                    .in_receiver
                    .iter()
                    .rev()
                    .nth(1)
                    .copied()
                    .unwrap_or(false)
            {
                self.push(RefKind::SelfCall, vec![node.method.to_string()]);
            }
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
                self.push(RefKind::Call, path);
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
