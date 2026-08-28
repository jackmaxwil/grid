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

fn push<T: ToTokens + Spanned>(
    kind: DefKind,
    callsign: String,
    node: &T,
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
    });
}

fn walk(items: &[Item], source: &str, prefix: &str, out: &mut Vec<Definition>) {
    for item in items {
        match item {
            Item::Fn(f) => {
                let callsign = join(prefix, &f.sig.ident.to_string());
                push(DefKind::Function, callsign.clone(), f, source, out);
                walk_stmts(&f.block.stmts, source, &callsign, out);
            }
            Item::Struct(s) => push(
                DefKind::Class,
                join(prefix, &s.ident.to_string()),
                s,
                source,
                out,
            ),
            Item::Enum(e) => push(
                DefKind::Class,
                join(prefix, &e.ident.to_string()),
                e,
                source,
                out,
            ),
            Item::Union(u) => push(
                DefKind::Class,
                join(prefix, &u.ident.to_string()),
                u,
                source,
                out,
            ),
            Item::Trait(t) => {
                let callsign = join(prefix, &t.ident.to_string());
                push(DefKind::Class, callsign.clone(), t, source, out);
                for ti in &t.items {
                    // Only default-bodied methods carry code; a required
                    // signature has no behavior to identify.
                    if let TraitItem::Fn(m) = ti {
                        if m.default.is_some() {
                            let mc = format!("{callsign}.{}", m.sig.ident);
                            push(DefKind::Function, mc.clone(), m, source, out);
                            if let Some(block) = &m.default {
                                walk_stmts(&block.stmts, source, &mc, out);
                            }
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
                        push(DefKind::Function, mc.clone(), m, source, out);
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
