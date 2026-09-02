//! Name binding, Rust rules: a reference's dotted path to the datum it names.
//!
//! Same contract as the Python resolver: a binding is recorded only when the
//! name resolves to exactly one definition by a rule the language itself
//! would follow, and every other case resolves to nothing. Rust has no LEGB;
//! paths resolve through `use` bindings, the `crate`/`super`/`self` anchors,
//! and items in scope. A name a glob import would have supplied was never
//! bound by extraction, so it falls through and resolves to nothing — the
//! omission `docs/LANG_RS.md` promises, never a guess.
use grid_datum::{Import, ReceiverForm, ReceiverOrigin, RefKind, Reference, TypeRef};
use grid_wire::Grade;

use crate::index::{DefIndex, Symbol};
use crate::resolve::{relation_of, Bindings, Decline, Resolved, ResolvedEdge};

/// Resolve one Rust file's references against the whole-tree index.
#[must_use]
pub(crate) fn resolve(
    index: &DefIndex,
    module: &str,
    imports: &[Import],
    refs: &[Reference],
) -> Resolved {
    let bindings = Bindings { imports };
    let mut out = Resolved::default();
    for r in refs {
        let Some(src) = index.datum_of(module, &r.from) else {
            continue;
        };
        let (target, grade) = if r.kind == RefKind::MethodCall {
            match method_call(index, &bindings, module, r) {
                Ok(bound) => bound,
                Err(Refusal::Declined(which)) => {
                    out.decline(src, which);
                    continue;
                }
                Err(Refusal::Refused(_reason)) => {
                    out.refuse(src);
                    continue;
                }
            }
        } else {
            let Some(target) = target_of(index, &bindings, module, r) else {
                // A name a glob import would have supplied was never bound
                // by extraction, so it falls through to here. Counted, not
                // guessed.
                out.decline(src, decline_of(index, &bindings, r));
                continue;
            };
            (target, Grade::Proven)
        };
        if src == target.datum {
            continue;
        }
        out.edges.push(ResolvedEdge {
            src,
            dst: target.datum,
            relation: relation_of(r.kind, target.is_class),
            grade,
        });
    }
    out
}

/// Why a method call bound to nothing.
enum Refusal {
    /// The receiver could not be typed to a charted type: no hint, an
    /// unresolvable type path, or a type outside the tree. Counted in the
    /// population the variant names.
    Declined(Decline),
    /// The receiver *was* typed to a charted type and the rule refused, for
    /// the stated reason. Counted `method`, and counted refused.
    Refused(&'static str),
}

/// The method names the Rust 2021 prelude's traits provide. An inherent
/// method of one of these names matched only at a later lookup step is a
/// refusal, never a proven edge: the prelude trait is in scope in every
/// module, and its method may outrank the inherent one there.
///
/// ponytail: the prelude, not `std`. An inherent `fn map` beside a
/// `Foo: Iterator` at a later step is caught by the `Foo@Iterator.next`
/// guard when the impl is in the tree; beside a blanket external impl it
/// is the modeled tier's stated residual.
const PRELUDE_METHODS: &[&str] = &[
    "clone",
    "clone_from",
    "to_string",
    "to_owned",
    "into",
    "try_into",
    "from",
    "try_from",
    "eq",
    "ne",
    "cmp",
    "partial_cmp",
    "lt",
    "le",
    "gt",
    "ge",
    "max",
    "min",
    "clamp",
    "default",
    "as_ref",
    "as_mut",
    "drop",
    "next",
    "fmt",
    "hash",
    "borrow",
    "borrow_mut",
    "deref",
    "deref_mut",
    "into_iter",
    "extend",
    "from_iter",
    "size_hint",
];

/// Bind `x.m(..)` by rustc's own lookup order, or say why not.
///
/// The receiver type is resolved exactly as a `Call` path resolves; the
/// inherent method `{type}.{m}` is looked up under it; then the grade rule:
/// a written receiver type whose form matches the inherent method's at the
/// first lookup step is **proven** — at that step an inherent method
/// outranks every trait, in the tree or out of it; a written type matched at
/// a later step, or an inferred type, is **modeled** unless a trait grid can
/// see supplies `m`; everything else is refused with its reason.
fn method_call<'a>(
    index: &'a DefIndex,
    bindings: &Bindings,
    module: &str,
    r: &Reference,
) -> Result<(&'a Symbol, Grade), Refusal> {
    let Some(hint) = r.receiver.as_ref() else {
        return Err(Refusal::Declined(Decline::Method));
    };
    let method = r.path.last().ok_or(Refusal::Declined(Decline::Method))?;
    let mut ty = hint.ty.clone();
    let mut origin = hint.origin;
    // `x.field` with the struct in another file: the base resolves first,
    // then the field's declared type is read off the index.
    if let Some(field) = &hint.field {
        let base = type_symbol(index, bindings, module, r, &ty)
            .map_err(|_| Refusal::Declined(Decline::Method))?;
        let declared = index
            .field_type(&base.absolute, field)
            .ok_or(Refusal::Declined(Decline::Method))?;
        ty = declared.clone();
        origin = ReceiverOrigin::TypedField;
    }
    let receiver = match type_symbol(index, bindings, module, r, &ty) {
        Ok(sym) => sym,
        // A type outside the tree is never this tree's hole — unless the
        // tree implements one of its own traits for it, when `Vec@Tally.m`
        // is charted and a call of `m` stays what it is.
        Err(Decline::External) => {
            let last = ty.path.last().map_or("", String::as_str);
            return Err(Refusal::Declined(if index.impl_supplies(last, method) {
                Decline::Method
            } else {
                Decline::External
            }));
        }
        Err(_) => return Err(Refusal::Declined(Decline::Method)),
    };
    if ty.args {
        // The callsign erased the argument; whether `impl Foo<u8>` serves
        // this `Foo<u16>` is invisible to the rule.
        return Err(Refusal::Refused("wrapper or instantiated"));
    }
    let inherent = index
        .lookup_absolute(&format!("{}.{method}", receiver.absolute))
        .ok_or(Refusal::Refused("no inherent method"))?;
    let facts = inherent
        .method
        .as_ref()
        .ok_or(Refusal::Refused("no inherent method"))?;
    let recv = facts.recv.ok_or(Refusal::Refused("no inherent method"))?;
    if !facts.plain_impl {
        return Err(Refusal::Refused("generic impl"));
    }
    let type_name = receiver.absolute.rsplit('.').next().unwrap_or("");
    let candidate = index.trait_declares(method)
        || index.impl_supplies(type_name, method)
        || PRELUDE_METHODS.contains(&method.as_str());
    let first_step = match (ty.form, recv) {
        (ReceiverForm::Value, ReceiverForm::Value)
        | (ReceiverForm::Ref, ReceiverForm::Ref)
        | (ReceiverForm::RefMut, ReceiverForm::RefMut) => true,
        // Autoref, or a `&mut` reborrowed shared: a later step.
        (ReceiverForm::Value, ReceiverForm::Ref | ReceiverForm::RefMut)
        | (ReceiverForm::RefMut, ReceiverForm::Ref) => false,
        // `&Foo` cannot reach `&mut self`, and neither reference can reach
        // a by-value `self` without a move out of it: if this compiles, a
        // trait supplied `m`.
        (ReceiverForm::Ref, ReceiverForm::RefMut | ReceiverForm::Value)
        | (ReceiverForm::RefMut, ReceiverForm::Value) => {
            return Err(Refusal::Refused("trait candidate"));
        }
    };
    // At the first step the inherent method outranks every trait, so a
    // written type is proven there and an inferred one modeled; at a later
    // step a visible candidate refuses, and no candidate is modeled either
    // way — an invisible blanket impl is the stated residual.
    let grade = if first_step && origin.written() {
        Grade::Proven
    } else if first_step || !candidate {
        Grade::Modeled
    } else {
        return Err(Refusal::Refused("trait candidate"));
    };
    Ok((inherent, grade))
}

/// The charted type a receiver hint names, resolved as a `Call` path from
/// the same definition would be; or which population its path falls in.
fn type_symbol<'a>(
    index: &'a DefIndex,
    bindings: &Bindings,
    module: &str,
    r: &Reference,
    ty: &TypeRef,
) -> Result<&'a Symbol, Decline> {
    let path = Reference {
        from: r.from.clone(),
        kind: RefKind::Call,
        path: ty.path.clone(),
        receiver: None,
    };
    match target_of(index, bindings, module, &path) {
        Some(sym) if sym.is_class => Ok(sym),
        Some(_) => Err(Decline::Internal),
        None => Err(decline_of(index, bindings, &path)),
    }
}

/// The names the Rust 2021 prelude puts in every module without a `use`:
/// the constructors, functions, types and traits — and the derive macros,
/// which the extractor records as decorator references — that a bare
/// unbound name can be without a glob import having supplied it. A name on
/// this list that the tree itself defines resolves before this table is
/// consulted, so the list only ever describes a name the tree did not hold.
const RUST_PRELUDE: &[&str] = &[
    "Some",
    "None",
    "Ok",
    "Err",
    "drop",
    "Box",
    "Vec",
    "String",
    "Option",
    "Result",
    "Copy",
    "Send",
    "Sized",
    "Sync",
    "Unpin",
    "Drop",
    "Fn",
    "FnMut",
    "FnOnce",
    "ToOwned",
    "Clone",
    "PartialEq",
    "PartialOrd",
    "Eq",
    "Ord",
    "AsRef",
    "AsMut",
    "Into",
    "From",
    "Default",
    "Iterator",
    "Extend",
    "IntoIterator",
    "DoubleEndedIterator",
    "ExactSizeIterator",
    "ToString",
    "TryFrom",
    "TryInto",
    "FromIterator",
    "Debug",
    "Hash",
];

/// The primitive types, which every module holds without any import and
/// which no tree defines: a receiver typed `&str` or `u32` is typed outside
/// the tree, and its method call is `external`, never a hole. A slice, an
/// array or a tuple spells no path and records no hint at all.
const PRIMITIVES: &[&str] = &[
    "bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64",
    "u128", "usize", "f32", "f64",
];

/// Which population a declined Rust reference belongs to.
///
/// A `use` moves the question to the binding's own head, which is where the
/// crate name actually is: `HashMap::new()` under `use std::collections::…`
/// is a path into `std`, not a bare name this tree lost. An anchor or a name
/// the tree roots is inside it; a bare unbound name is one a glob import
/// would have supplied, which is inside it too — and therefore a hole —
/// unless it is a prelude name, which every module holds without any import
/// and which was therefore never this tree's to lose. `Some(..)`, `Ok(..)`
/// and `drop(..)` alone were most of the `internal` count on this repo.
fn decline_of(index: &DefIndex, bindings: &Bindings, r: &Reference) -> Decline {
    let Some(head) = r.path.first() else {
        return Decline::Internal;
    };
    let bound = bindings.lookup(&r.from, head);
    let head = bound.map_or(head.as_str(), |t| t.split('.').next().unwrap_or(t));
    if matches!(head, "std" | "core" | "alloc") {
        return Decline::External;
    }
    if matches!(head, "crate" | "super" | "self" | "Self") || index.is_root(head) {
        return Decline::Internal;
    }
    if r.path.len() == 1
        && bound.is_none()
        && !RUST_PRELUDE.contains(&head)
        && !PRIMITIVES.contains(&head)
    {
        Decline::Internal
    } else {
        Decline::External
    }
}

fn target_of<'a>(
    index: &'a DefIndex,
    bindings: &Bindings,
    module: &str,
    r: &Reference,
) -> Option<&'a Symbol> {
    let (head, rest) = r.path.split_first()?;

    // `self.m()` / `Self::m()` — the receiver is the impl the reference sits
    // in, which is the callsign minus its method segment. Inherent methods
    // shadow trait methods, which is also Rust's own order.
    if r.kind == RefKind::SelfCall {
        let receiver = r.from.rsplit_once('.').map(|(recv, _)| recv)?;
        let method = r.path.join(".");
        let inherent = receiver.split('@').next().unwrap_or(receiver);
        return index
            .lookup(module, &format!("{inherent}.{method}"))
            .or_else(|| {
                (receiver != inherent)
                    .then(|| index.lookup(module, &format!("{receiver}.{method}")))
                    .flatten()
            });
    }

    // The three path anchors Rust's own grammar fixes.
    if let Some(anchored) = anchor(module, head, rest) {
        return index.lookup_absolute(&anchored);
    }

    // A name bound by a `use` resolves through it, and only then. The bound
    // target may itself be anchor-headed, spelled against this module — and
    // a bare target may be a local module's item (uniform paths), which is
    // why the final lookup tries module-relative before absolute.
    if let Some(target) = bindings.lookup(&r.from, head) {
        let segs: Vec<&str> = target.split('.').collect();
        let (thead, trest) = segs.split_first()?;
        let full = match anchor_str(module, thead, trest) {
            Some(base) if rest.is_empty() => base,
            Some(base) => format!("{base}.{}", rest.join(".")),
            None if rest.is_empty() => segs.join("."),
            None => format!("{}.{}", segs.join("."), rest.join(".")),
        };
        return match anchor_str(module, thead, trest) {
            Some(_) => index.lookup_absolute(&full),
            None => index.lookup(module, &full),
        };
    }

    // Otherwise the name is an item in this module or an enclosing
    // definition's body, tried innermost-first — or it is not resolvable.
    let dotted = r.path.join(".");
    let mut scope = r.from.as_str();
    loop {
        let candidate = if scope.is_empty() {
            dotted.clone()
        } else {
            format!("{scope}.{dotted}")
        };
        if let Some(sym) = index.lookup(module, &candidate) {
            return Some(sym);
        }
        match scope.rsplit_once('.') {
            Some((outer, _)) => scope = outer,
            None if scope.is_empty() => return None,
            None => scope = "",
        }
    }
}

fn anchor(module: &str, head: &str, rest: &[String]) -> Option<String> {
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    anchor_str(module, head, &rest)
}

/// Anchor a re-export target against its declaring module, or `None` for a
/// bare path (whose meaning the index decides — local module or absolute).
pub(crate) fn anchor_target(module: &str, target: &str) -> Option<String> {
    let segs: Vec<&str> = target.split('.').collect();
    let (head, rest) = segs.split_first()?;
    if matches!(*head, "crate" | "super" | "self") {
        anchor_str(module, head, rest).or_else(|| Some(target.to_owned()))
    } else {
        None
    }
}

/// `crate::` is the module root segment, `super::` climbs one module per
/// occurrence, `self::` is this module. Climbing past the root resolves to
/// nothing, which is also what rustc says.
fn anchor_str(module: &str, head: &str, rest: &[&str]) -> Option<String> {
    let joined = |base: &str, rest: &[&str]| {
        if rest.is_empty() {
            base.to_owned()
        } else if base.is_empty() {
            rest.join(".")
        } else {
            format!("{base}.{}", rest.join("."))
        }
    };
    match head {
        "crate" => Some(joined(module.split('.').next().unwrap_or(module), rest)),
        "self" => Some(joined(module, rest)),
        "super" => {
            let mut segs: Vec<&str> = module.split('.').collect();
            segs.pop();
            let mut rest = rest;
            while rest.first() == Some(&"super") {
                segs.pop();
                rest = rest.get(1..).unwrap_or_default();
            }
            if segs.is_empty() {
                return None;
            }
            Some(joined(&segs.join("."), rest))
        }
        _ => None,
    }
}
