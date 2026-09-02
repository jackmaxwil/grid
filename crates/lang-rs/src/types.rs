//! Types as this frontend can read them off syntax: a written type as a
//! path, an impl's plainness, a method's receiver form, and the per-file
//! tables of written returns and declared field types the extractor types a
//! same-file chain with. Nothing here is a type checker — a type this module
//! cannot spell as a path is `None`, and the caller then records no hint.
use std::collections::HashMap;

use grid_datum::{ReceiverForm, TypeRef};
use syn::{GenericArgument, GenericParam, Item, PathArguments, ReturnType, Type};

/// The names of an item's generic type parameters: a path naming one is a
/// type parameter, not a type, and yields no hint.
pub(crate) fn generic_names(g: &syn::Generics) -> Vec<String> {
    g.type_params().map(|p| p.ident.to_string()).collect()
}

/// A written type as a [`TypeRef`], `Self` replaced by `self_ty`. `None`
/// for anything that is not a plain path — `impl Trait`, `dyn Trait`, a
/// tuple, an array, a pointer, a type parameter, an associated type, or a
/// `Self` outside any impl.
pub(crate) fn type_ref(
    ty: &Type,
    self_ty: Option<&TypeRef>,
    generics: &[String],
) -> Option<TypeRef> {
    match ty {
        Type::Reference(r) => {
            let mut inner = type_ref(&r.elem, self_ty, generics)?;
            // A reference to a reference: rustc's first lookup step is the
            // outer reference, which no single form spells — no hint.
            if inner.form != ReceiverForm::Value {
                return None;
            }
            inner.form = if r.mutability.is_some() {
                ReceiverForm::RefMut
            } else {
                ReceiverForm::Ref
            };
            Some(inner)
        }
        Type::Paren(p) => type_ref(&p.elem, self_ty, generics),
        Type::Group(g) => type_ref(&g.elem, self_ty, generics),
        Type::Path(p) if p.qself.is_none() => path_ref(&p.path, self_ty, generics),
        _ => None,
    }
}

fn path_ref(path: &syn::Path, self_ty: Option<&TypeRef>, generics: &[String]) -> Option<TypeRef> {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.first().is_some_and(|h| h == "Self") {
        // `Self` is the impl's type; `Self::Item` is an associated type,
        // which no syntax here resolves.
        return (segs.len() == 1).then(|| self_ty.cloned()).flatten();
    }
    if let [only] = segs.as_slice() {
        if generics.contains(only) {
            return None;
        }
    }
    let last = path.segments.last()?;
    let (args, inner) = match &last.arguments {
        PathArguments::None => (false, None),
        PathArguments::AngleBracketed(a) => {
            let mut types = a.args.iter().filter_map(|g| match g {
                GenericArgument::Type(t) => Some(t),
                _ => None,
            });
            match types.next() {
                // Lifetimes only: `Foo<'a>` is `Foo`, as the callsign says.
                None => (false, None),
                Some(first) => (true, type_ref(first, self_ty, generics).map(Box::new)),
            }
        }
        PathArguments::Parenthesized(_) => return None,
    };
    Some(TypeRef {
        path: segs,
        args,
        form: ReceiverForm::Value,
        inner,
    })
}

/// The written return of a signature, or `None` for an unwritten one.
pub(crate) fn ret_of(
    sig: &syn::Signature,
    self_ty: Option<&TypeRef>,
    generics: &[String],
) -> Option<TypeRef> {
    match &sig.output {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => type_ref(ty, self_ty, generics),
    }
}

/// A signature's own `self` form: `None` for no receiver — an associated
/// or free function — and for a receiver with an explicit type
/// (`self: Box<Self>`), which the first-step rule does not reach.
pub(crate) fn receiver_form(sig: &syn::Signature) -> Option<ReceiverForm> {
    let r = sig.receiver()?;
    if r.colon_token.is_some() {
        return None;
    }
    Some(match (&r.reference, &r.mutability) {
        (None, _) => ReceiverForm::Value,
        (Some(_), None) => ReceiverForm::Ref,
        (Some(_), Some(_)) => ReceiverForm::RefMut,
    })
}

/// True for `impl Foo` and `impl<T> Foo<T>` with no bounds and no where
/// clause — the impls whose methods apply to every `Foo`. False for
/// `impl Foo<u8>`, `impl<T: Bound> Foo<T>`, `impl<T> Foo<T> where …`, and
/// a const parameter: the callsign erased the argument, and the rule cannot
/// see whether the impl serves the receiver it is asked about.
pub(crate) fn plain_impl(i: &syn::ItemImpl) -> bool {
    if i.generics.where_clause.is_some() {
        return false;
    }
    let mut params = Vec::new();
    for p in &i.generics.params {
        match p {
            GenericParam::Lifetime(l) if l.bounds.is_empty() => {}
            GenericParam::Type(t) if t.bounds.is_empty() => params.push(t.ident.to_string()),
            _ => return false,
        }
    }
    let Type::Path(p) = &*i.self_ty else {
        return false;
    };
    p.path.segments.iter().all(|seg| match &seg.arguments {
        PathArguments::None => true,
        PathArguments::AngleBracketed(a) => a.args.iter().all(|g| match g {
            GenericArgument::Lifetime(_) => true,
            GenericArgument::Type(Type::Path(tp)) => {
                tp.qself.is_none()
                    && tp
                        .path
                        .get_ident()
                        .is_some_and(|id| params.contains(&id.to_string()))
            }
            _ => false,
        }),
        PathArguments::Parenthesized(_) => false,
    })
}

/// Same-file facts the extractor types a chain with: every function's
/// written return keyed by the type's written path and the method
/// (`Foo.new`, `other.Foo.new`; `free` for a free function), and every
/// struct's field types keyed by the struct's own name. A key two items
/// share is `None`: an ambiguous entry yields no hint rather than a wrong
/// one, and a base written as `other::Foo` never reads this file's `Foo`.
///
/// ponytail: keyed by the written path, not the resolved one — an inline
/// module's `Foo` and the file's `Foo` collide into a refusal, and the
/// same type written two ways (`Foo`, `self::Foo`) is two keys, the second
/// of which yields nothing. Cross-file returns are not followed at all;
/// phase 3 holds every method's return on `MethodFacts` and is the upgrade.
#[derive(Default)]
pub(crate) struct FileTypes {
    pub(crate) returns: HashMap<String, Option<TypeRef>>,
    pub(crate) fields: HashMap<String, Option<HashMap<String, TypeRef>>>,
}

fn put<V>(map: &mut HashMap<String, Option<V>>, key: String, value: Option<V>) {
    match map.entry(key) {
        std::collections::hash_map::Entry::Vacant(e) => {
            e.insert(value);
        }
        std::collections::hash_map::Entry::Occupied(mut e) => {
            e.insert(None);
        }
    }
}

/// Collect the same-file tables, the declared field types of every
/// named-field struct, `(struct callsign, field, type)`, and the name of
/// every method every trait declares.
pub(crate) fn collect(
    items: &[Item],
    prefix: &str,
    types: &mut FileTypes,
    field_types: &mut Vec<(String, String, TypeRef)>,
    trait_methods: &mut Vec<String>,
) {
    for item in items {
        match item {
            Item::Trait(t) => {
                for ti in &t.items {
                    if let syn::TraitItem::Fn(m) = ti {
                        trait_methods.push(m.sig.ident.to_string());
                    }
                }
            }
            Item::Fn(f) => {
                let generics = generic_names(&f.sig.generics);
                put(
                    &mut types.returns,
                    f.sig.ident.to_string(),
                    ret_of(&f.sig, None, &generics),
                );
            }
            Item::Struct(s) => {
                let generics = generic_names(&s.generics);
                let name = s.ident.to_string();
                let callsign = crate::extract::join(prefix, &name);
                let mut fields = HashMap::new();
                for field in &s.fields {
                    let (Some(id), Some(ty)) = (&field.ident, type_ref(&field.ty, None, &generics))
                    else {
                        continue;
                    };
                    field_types.push((callsign.clone(), id.to_string(), ty.clone()));
                    fields.insert(id.to_string(), ty);
                }
                put(&mut types.fields, name, Some(fields));
            }
            Item::Impl(i) if i.trait_.is_none() => {
                let generics = generic_names(&i.generics);
                let Some(self_ty) = type_ref(&i.self_ty, None, &generics) else {
                    continue;
                };
                let name = self_ty.path.join(".");
                for ii in &i.items {
                    let syn::ImplItem::Fn(m) = ii else { continue };
                    let mut generics = generics.clone();
                    generics.extend(generic_names(&m.sig.generics));
                    put(
                        &mut types.returns,
                        format!("{name}.{}", m.sig.ident),
                        ret_of(&m.sig, Some(&self_ty), &generics),
                    );
                }
            }
            Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    collect(
                        items,
                        &crate::extract::join(prefix, &m.ident.to_string()),
                        types,
                        field_types,
                        trait_methods,
                    );
                }
            }
            _ => {}
        }
    }
}
