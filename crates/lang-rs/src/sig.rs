//! Interface identity for one Rust node: the bytes a *caller* can observe
//! without reading the body.
//!
//! The inclusion rule is one sentence — a fact is in the sig iff a caller can
//! depend on it without the body — and the three item shapes fall out of it:
//!
//! - `fn`: visibility, `const`/`async`/`unsafe`/`extern`, name, generics with
//!   their bounds and where clause, parameter *types*, return type. Parameter
//!   patterns are erased to `_`: Rust callers pass positionally and cannot
//!   name a parameter, so a rename there is body-side.
//! - `struct`/`enum`/`union`: visibility, name, generics, fields and variants
//!   as declared, and the attributes that change the observable contract —
//!   `#[repr(..)]`, `#[serde(..)]`, `#[non_exhaustive]`. Every other
//!   attribute (`derive`, `allow`, docs) is body-side.
//! - `trait`: visibility, name, generics, supertraits, each method's
//!   signature (bodies dropped), associated types and consts.
//!
//! Printing goes through [`crate::canon`], so the sig inherits the datum's
//! formatting and doc-comment invariance for free; only the epoch prefix
//! differs, because sig bytes and datum bytes are different claims about the
//! same node and must not share a preimage.
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};

/// Attributes a caller can observe without the body. Everything else —
/// `derive`, `allow`, `inline`, doc comments — changes how the definition is
/// built or documented, not what its interface is.
const OBSERVABLE_ATTRS: &[&str] = &["repr", "serde", "non_exhaustive"];

fn observable(attr: &syn::Attribute) -> bool {
    attr.path()
        .get_ident()
        .is_some_and(|id| OBSERVABLE_ATTRS.contains(&id.to_string().as_str()))
}

fn keep_observable(attrs: &[syn::Attribute]) -> Vec<syn::Attribute> {
    attrs.iter().filter(|a| observable(a)).cloned().collect()
}

/// A signature with every parameter pattern erased to `_`. `self` receivers
/// stay: `&self` versus `&mut self` versus `self` is exactly what a caller
/// must satisfy.
fn erased(sig: &syn::Signature) -> syn::Signature {
    let mut sig = sig.clone();
    for input in &mut sig.inputs {
        if let syn::FnArg::Typed(arg) = input {
            *arg.pat = syn::Pat::Wild(syn::PatWild {
                attrs: Vec::new(),
                underscore_token: syn::token::Underscore::default(),
            });
        }
    }
    sig
}

/// The interface tokens of a free or associated function.
pub(crate) fn of_fn(vis: &syn::Visibility, sig: &syn::Signature) -> TokenStream {
    let sig = erased(sig);
    quote!(#vis #sig)
}

/// The interface tokens of a `struct`, `enum`, or `union`: the item itself
/// with its attributes filtered to the observable ones.
pub(crate) fn of_type<T: ToTokens + Clone>(item: &T, keep: impl FnOnce(&mut T)) -> TokenStream {
    let mut item = item.clone();
    keep(&mut item);
    item.to_token_stream()
}

/// The interface tokens of a trait: the declaration with every default body
/// dropped and every non-signature item removed.
pub(crate) fn of_trait(item: &syn::ItemTrait) -> TokenStream {
    let mut item = item.clone();
    item.attrs = keep_observable(&item.attrs);
    item.items.retain(|i| {
        matches!(
            i,
            syn::TraitItem::Fn(_) | syn::TraitItem::Type(_) | syn::TraitItem::Const(_)
        )
    });
    for i in &mut item.items {
        match i {
            syn::TraitItem::Fn(f) => {
                f.attrs = keep_observable(&f.attrs);
                f.sig = erased(&f.sig);
                // A default body is behavior, not interface: overriding it is
                // the implementor's business and calling it is unchanged.
                f.default = None;
                f.semi_token = Some(syn::token::Semi::default());
            }
            syn::TraitItem::Type(t) => t.attrs = keep_observable(&t.attrs),
            syn::TraitItem::Const(c) => c.attrs = keep_observable(&c.attrs),
            _ => {}
        }
    }
    item.to_token_stream()
}

/// Strip a struct's attributes, and its fields', down to the observable set.
pub(crate) fn keep_struct(item: &mut syn::ItemStruct) {
    item.attrs = keep_observable(&item.attrs);
    for field in &mut item.fields {
        field.attrs = keep_observable(&field.attrs);
    }
}

/// The same for an enum: item, variants, and each variant's fields.
pub(crate) fn keep_enum(item: &mut syn::ItemEnum) {
    item.attrs = keep_observable(&item.attrs);
    for variant in &mut item.variants {
        variant.attrs = keep_observable(&variant.attrs);
        for field in &mut variant.fields {
            field.attrs = keep_observable(&field.attrs);
        }
    }
}

/// And for a union, which has fields but no variants.
pub(crate) fn keep_union(item: &mut syn::ItemUnion) {
    item.attrs = keep_observable(&item.attrs);
    for field in &mut item.fields.named {
        field.attrs = keep_observable(&field.attrs);
    }
}
