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

use crate::canon;

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
    walk(&file.items, source, "", &mut out);
    out
}

fn join(prefix: &str, name: &str) -> String {
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

fn push<T: ToTokens + Spanned>(
    kind: DefKind,
    callsign: String,
    node: &T,
    shaped: &dyn ToTokens,
    complexity: u16,
    source: &str,
    out: &mut Vec<Definition>,
) {
    let span = node.span().byte_range();
    let text = source.get(span.clone()).unwrap_or_default();
    out.push(Definition {
        kind,
        callsign,
        datum: Datum::of_canonical_bytes(&canon::canonical_bytes(node)),
        span_hash: span_hash(text),
        span,
        shape: Datum::of_canonical_bytes(&canon::canonical_bytes(&shaped)),
        complexity,
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

fn walk(items: &[Item], source: &str, prefix: &str, out: &mut Vec<Definition>) {
    for item in items {
        match item {
            Item::Fn(f) => {
                let callsign = join(prefix, &f.sig.ident.to_string());
                let mut shaped = f.clone();
                shaped.sig.ident = anon();
                let cx = complexity_of(&f.block);
                push(
                    DefKind::Function,
                    callsign.clone(),
                    f,
                    &shaped,
                    cx,
                    source,
                    out,
                );
                walk_stmts(&f.block.stmts, source, &callsign, out);
            }
            Item::Struct(s) => {
                let mut shaped = s.clone();
                shaped.ident = anon();
                let callsign = join(prefix, &s.ident.to_string());
                push(DefKind::Class, callsign, s, &shaped, 1, source, out);
            }
            Item::Enum(e) => {
                let mut shaped = e.clone();
                shaped.ident = anon();
                let callsign = join(prefix, &e.ident.to_string());
                push(DefKind::Class, callsign, e, &shaped, 1, source, out);
            }
            Item::Union(u) => {
                let mut shaped = u.clone();
                shaped.ident = anon();
                let callsign = join(prefix, &u.ident.to_string());
                push(DefKind::Class, callsign, u, &shaped, 1, source, out);
            }
            Item::Trait(t) => {
                let callsign = join(prefix, &t.ident.to_string());
                let mut shaped = t.clone();
                shaped.ident = anon();
                push(DefKind::Class, callsign.clone(), t, &shaped, 1, source, out);
                for ti in &t.items {
                    // Only default-bodied methods carry code; a required
                    // signature has no behavior to identify.
                    if let TraitItem::Fn(m) = ti {
                        if let Some(block) = &m.default {
                            let mc = format!("{callsign}.{}", m.sig.ident);
                            let mut shaped = m.clone();
                            shaped.sig.ident = anon();
                            let cx = complexity_of(block);
                            push(DefKind::Function, mc.clone(), m, &shaped, cx, source, out);
                            walk_stmts(&block.stmts, source, &mc, out);
                        }
                    }
                }
            }
            Item::Impl(i) => {
                let Some(ty) = type_callsign(&i.self_ty) else {
                    continue;
                };
                let receiver = match &i.trait_ {
                    Some((_, path, _)) => format!("{ty}@{}", path_dots(path)),
                    None => ty,
                };
                let receiver = join(prefix, &receiver);
                for ii in &i.items {
                    if let ImplItem::Fn(m) = ii {
                        let mc = format!("{receiver}.{}", m.sig.ident);
                        let mut shaped = m.clone();
                        shaped.sig.ident = anon();
                        let cx = complexity_of(&m.block);
                        push(DefKind::Function, mc.clone(), m, &shaped, cx, source, out);
                        walk_stmts(&m.block.stmts, source, &mc, out);
                    }
                }
            }
            Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    walk(items, source, &join(prefix, &m.ident.to_string()), out);
                }
            }
            _ => {}
        }
    }
}

/// Items nested directly in a body. Deeper nesting (an item inside an `if`
/// arm) is not descended into — the same depth rule lang-py holds.
fn walk_stmts(stmts: &[Stmt], source: &str, prefix: &str, out: &mut Vec<Definition>) {
    for stmt in stmts {
        if let Stmt::Item(item) = stmt {
            walk(std::slice::from_ref(item), source, prefix, out);
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
