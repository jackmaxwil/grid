//! Definition extraction: functions, types, traits, and impl methods, each
//! with its own identity and raw-source span hash.
//!
//! Callsign spellings are the `docs/LANG_RS.md` table: dotted segments,
//! `Type@Trait` for trait impls, generics erased — the callsign is an
//! address, the datum is the identity.
use grid_datum::{span_hash, Datum, DefKind, Definition, ParseError};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::{ImplItem, Item, Stmt, TraitItem};

use crate::{canon, sig, types};

/// Extract every definition in a Rust source file, outermost first, each
/// nested definition following its parent.
///
/// # Errors
/// Returns `ParseError` if the source is not valid Rust.
pub fn definitions(source: &str) -> Result<Vec<Definition>, ParseError> {
    let file = syn::parse_file(source).map_err(|e| ParseError::new(e.to_string()))?;
    Ok(definitions_of(&file, source))
}

pub(crate) fn definitions_of(file: &syn::File, source: &str) -> Vec<Definition> {
    let mut out = Vec::new();
    walk(&file.items, source, "", &cfgs_of(&file.attrs), &mut out);
    out
}

/// The `#[cfg(...)]` predicates on one node, as written minus whitespace —
/// whitespace, because the predicate rides in a porcelain column that is
/// split on runs of spaces. A `cfg_attr(...)` conditions compilation only
/// when the attributes it may expand to include a `cfg(...)`; such a one is
/// recorded whole, as its own predicate text, and the evaluator reads it as
/// unknown — the honest answer for an attribute that may or may not have
/// conditioned anything. A `cfg_attr` that expands to a `derive`, an
/// `allow`, or a `doc` conditions nothing and is not a predicate at all;
/// recording it made every proven caller on a serde-using crate a `cfg
/// unknown` warning, which buried the records that gate.
pub(crate) fn cfgs_of(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter_map(|attr| {
            let text = if attr.path().is_ident("cfg") {
                attr.meta.require_list().ok()?.tokens.to_string()
            } else if attr.path().is_ident("cfg_attr") {
                let text = attr.meta.to_token_stream().to_string();
                if !text.contains("cfg (") && !text.contains("cfg(") {
                    return None;
                }
                text
            } else {
                return None;
            };
            Some(text.chars().filter(|c| !c.is_whitespace()).collect())
        })
        .collect()
}

/// The enclosing predicates plus this node's own, innermost last.
fn scoped(cfg: &[String], attrs: &[syn::Attribute]) -> Vec<String> {
    let mut out = cfg.to_vec();
    out.extend(cfgs_of(attrs));
    out
}

pub(crate) fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

/// The placeholder the definition's own name becomes in the *shape* hash.
/// The datum keeps the name (the chart interns datums uniquely, and two
/// same-body defs must not collapse); the shape is the rename-join key.
fn anon() -> proc_macro2::Ident {
    proc_macro2::Ident::new("__shape", proc_macro2::Span::call_site())
}

/// The three identities of one definition and what each is built from. They
/// always travel together: the node itself is the datum, the name-erased
/// clone is the shape, and the interface tokens are the sig.
struct Facts<'a> {
    kind: DefKind,
    callsign: String,
    shaped: &'a dyn ToTokens,
    interface: proc_macro2::TokenStream,
    complexity: u16,
    /// Every predicate conditioning this definition, outermost first.
    cfg: Vec<String>,
    /// The callee facts of a function, for the method-call rule.
    method: Option<grid_datum::MethodFacts>,
}

fn push<T: ToTokens + Spanned>(node: &T, facts: Facts, source: &str, out: &mut Vec<Definition>) {
    let span = node.span().byte_range();
    let text = source.get(span.clone()).unwrap_or_default();
    out.push(Definition {
        kind: facts.kind,
        callsign: facts.callsign,
        datum: Datum::of_canonical_bytes(&canon::canonical_bytes(node)),
        span_hash: span_hash(text),
        sig: Datum::of_canonical_bytes(&canon::sig_bytes(&facts.interface)),
        span,
        shape: Datum::of_canonical_bytes(&canon::canonical_bytes(&facts.shaped)),
        complexity: facts.complexity,
        cfg: grid_datum::cfg_join(&facts.cfg),
        method: facts.method,
    });
}

/// Cyclomatic complexity: 1 + branch points. Branches: `if`, `match` arms
/// beyond the first, loops, `&&`/`||`, `?`. A macro body is opaque tokens
/// and contributes nothing, the same omission the extractor makes.
fn complexity_of(block: &syn::Block) -> u16 {
    struct Cx(u16);
    impl Cx {
        fn bump(&mut self, by: u16) {
            self.0 = self.0.saturating_add(by);
        }
    }
    impl<'ast> syn::visit::Visit<'ast> for Cx {
        fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
            self.bump(1);
            syn::visit::visit_expr_if(self, node);
        }
        fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
            self.bump(u16::try_from(node.arms.len().saturating_sub(1)).unwrap_or(u16::MAX));
            syn::visit::visit_expr_match(self, node);
        }
        fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
            self.bump(1);
            syn::visit::visit_expr_for_loop(self, node);
        }
        fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
            self.bump(1);
            syn::visit::visit_expr_while(self, node);
        }
        fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
            self.bump(1);
            syn::visit::visit_expr_loop(self, node);
        }
        fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
            self.bump(1);
            syn::visit::visit_expr_try(self, node);
        }
        fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
            if matches!(node.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
                self.bump(1);
            }
            syn::visit::visit_expr_binary(self, node);
        }
    }
    let mut cx = Cx(1);
    syn::visit::visit_block(&mut cx, block);
    cx.0
}

/// A free function as a callee: no receiver, and its written return for a
/// local bound by `f(..)`.
fn free_fn_facts(sig: &syn::Signature) -> grid_datum::MethodFacts {
    grid_datum::MethodFacts {
        recv: None,
        plain_impl: true,
        ret: types::ret_of(sig, None, &types::generic_names(&sig.generics)),
    }
}

fn walk(items: &[Item], source: &str, prefix: &str, cfg: &[String], out: &mut Vec<Definition>) {
    for item in items {
        match item {
            Item::Fn(f) => {
                let callsign = join(prefix, &f.sig.ident.to_string());
                let mut shaped = f.clone();
                shaped.sig.ident = anon();
                let cfg = scoped(cfg, &f.attrs);
                push(
                    f,
                    Facts {
                        kind: DefKind::Function,
                        callsign: callsign.clone(),
                        shaped: &shaped,
                        interface: sig::of_fn(&f.vis, &f.sig),
                        complexity: complexity_of(&f.block),
                        cfg: cfg.clone(),
                        method: Some(free_fn_facts(&f.sig)),
                    },
                    source,
                    out,
                );
                walk_stmts(&f.block.stmts, source, &callsign, &cfg, out);
            }
            Item::Struct(s) => {
                let mut shaped = s.clone();
                shaped.ident = anon();
                push(
                    s,
                    Facts {
                        kind: DefKind::Class,
                        callsign: join(prefix, &s.ident.to_string()),
                        shaped: &shaped,
                        interface: sig::of_type(s, sig::keep_struct),
                        complexity: 1,
                        cfg: scoped(cfg, &s.attrs),
                        method: None,
                    },
                    source,
                    out,
                );
            }
            Item::Enum(e) => {
                let mut shaped = e.clone();
                shaped.ident = anon();
                push(
                    e,
                    Facts {
                        kind: DefKind::Class,
                        callsign: join(prefix, &e.ident.to_string()),
                        shaped: &shaped,
                        interface: sig::of_type(e, sig::keep_enum),
                        complexity: 1,
                        cfg: scoped(cfg, &e.attrs),
                        method: None,
                    },
                    source,
                    out,
                );
            }
            Item::Union(u) => {
                let mut shaped = u.clone();
                shaped.ident = anon();
                push(
                    u,
                    Facts {
                        kind: DefKind::Class,
                        callsign: join(prefix, &u.ident.to_string()),
                        shaped: &shaped,
                        interface: sig::of_type(u, sig::keep_union),
                        complexity: 1,
                        cfg: scoped(cfg, &u.attrs),
                        method: None,
                    },
                    source,
                    out,
                );
            }
            Item::Trait(t) => walk_trait(t, source, prefix, cfg, out),
            Item::Impl(i) => walk_impl(i, source, prefix, cfg, out),
            Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    walk(
                        items,
                        source,
                        &join(prefix, &m.ident.to_string()),
                        &scoped(cfg, &m.attrs),
                        out,
                    );
                }
            }
            _ => {}
        }
    }
}

/// A trait and its default-bodied methods. A required signature has no
/// behavior to identify, so only defaults become definitions — but every
/// method's signature is part of the *trait's* sig, which `sig::of_trait`
/// owns.
fn walk_trait(
    t: &syn::ItemTrait,
    source: &str,
    prefix: &str,
    cfg: &[String],
    out: &mut Vec<Definition>,
) {
    let callsign = join(prefix, &t.ident.to_string());
    let mut shaped = t.clone();
    shaped.ident = anon();
    let cfg = scoped(cfg, &t.attrs);
    push(
        t,
        Facts {
            kind: DefKind::Class,
            callsign: callsign.clone(),
            shaped: &shaped,
            interface: sig::of_trait(t),
            complexity: 1,
            cfg: cfg.clone(),
            method: None,
        },
        source,
        out,
    );
    for ti in &t.items {
        let TraitItem::Fn(m) = ti else { continue };
        let Some(block) = &m.default else { continue };
        let mc = format!("{callsign}.{}", m.sig.ident);
        let mut shaped = m.clone();
        shaped.sig.ident = anon();
        let cfg = scoped(&cfg, &m.attrs);
        push(
            m,
            Facts {
                kind: DefKind::Function,
                callsign: mc.clone(),
                shaped: &shaped,
                // A trait method has no visibility of its own: it is as
                // public as the trait.
                interface: sig::of_fn(&syn::Visibility::Inherited, &m.sig),
                complexity: complexity_of(block),
                cfg: cfg.clone(),
                // A trait's default body is not an inherent method: no
                // receiver rule reaches it by the type's name.
                method: None,
            },
            source,
            out,
        );
        walk_stmts(&block.stmts, source, &mc, &cfg, out);
    }
}

/// An impl block's methods, under `Type` or `Type@Trait`.
fn walk_impl(
    i: &syn::ItemImpl,
    source: &str,
    prefix: &str,
    cfg: &[String],
    out: &mut Vec<Definition>,
) {
    let Some(ty) = type_callsign(&i.self_ty) else {
        return;
    };
    let receiver = match &i.trait_ {
        Some((_, path, _)) => format!("{ty}@{}", path_dots(path)),
        None => ty,
    };
    let receiver = join(prefix, &receiver);
    let cfg = scoped(cfg, &i.attrs);
    let impl_generics = types::generic_names(&i.generics);
    let self_ty = types::type_ref(&i.self_ty, None, &impl_generics);
    let plain_impl = types::plain_impl(i);
    for ii in &i.items {
        let ImplItem::Fn(m) = ii else { continue };
        let mc = format!("{receiver}.{}", m.sig.ident);
        let mut shaped = m.clone();
        shaped.sig.ident = anon();
        let cfg = scoped(&cfg, &m.attrs);
        // Only an inherent method is reachable by the type's name; a trait
        // impl's methods are candidates the rule refuses, never targets.
        let method = i.trait_.is_none().then(|| {
            let mut generics = impl_generics.clone();
            generics.extend(types::generic_names(&m.sig.generics));
            grid_datum::MethodFacts {
                recv: types::receiver_form(&m.sig),
                plain_impl,
                ret: types::ret_of(&m.sig, self_ty.as_ref(), &generics),
            }
        });
        push(
            m,
            Facts {
                kind: DefKind::Function,
                callsign: mc.clone(),
                shaped: &shaped,
                interface: sig::of_fn(&m.vis, &m.sig),
                complexity: complexity_of(&m.block),
                cfg: cfg.clone(),
                method,
            },
            source,
            out,
        );
        walk_stmts(&m.block.stmts, source, &mc, &cfg, out);
    }
}

/// Items nested directly in a body. Deeper nesting (an item inside an `if`
/// arm) is not descended into — the same depth rule lang-py holds.
fn walk_stmts(
    stmts: &[Stmt],
    source: &str,
    prefix: &str,
    cfg: &[String],
    out: &mut Vec<Definition>,
) {
    for stmt in stmts {
        if let Stmt::Item(item) = stmt {
            walk(std::slice::from_ref(item), source, prefix, cfg, out);
        }
    }
}

/// The dotted, generics-erased spelling of a self type, or `None` for a type
/// with no path to spell (a tuple, a pointer): its impl is then unaddressable
/// and skipped — an omission, never a wrong name.
pub(crate) fn type_callsign(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(p) => Some(path_dots(&p.path)),
        syn::Type::Reference(r) => type_callsign(&r.elem),
        syn::Type::Group(g) => type_callsign(&g.elem),
        syn::Type::Paren(p) => type_callsign(&p.elem),
        _ => None,
    }
}

pub(crate) fn path_dots(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join(".")
}
