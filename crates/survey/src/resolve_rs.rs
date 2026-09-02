//! Name binding, Rust rules: a reference's dotted path to the datum it names.
//!
//! Same contract as the Python resolver: a binding is recorded only when the
//! name resolves to exactly one definition by a rule the language itself
//! would follow, and every other case resolves to nothing. Rust has no LEGB;
//! paths resolve through `use` bindings, the `crate`/`super`/`self` anchors,
//! and items in scope. A name a glob import would have supplied was never
//! bound by extraction, so it falls through and resolves to nothing — the
//! omission `docs/LANG_RS.md` promises, never a guess.
use grid_datum::{Import, RefKind, Reference};
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
        // A method call needs the receiver's type, which this frontend does
        // not have (`docs/LANG_RS.md`). Declined unconditionally — no
        // attempt, no guess — and now counted rather than invisible.
        if r.kind == RefKind::MethodCall {
            out.decline(Decline::Method);
            continue;
        }
        let Some(target) = target_of(index, &bindings, module, r) else {
            // A name a glob import would have supplied was never bound by
            // extraction, so it falls through to here. Counted, not guessed.
            out.decline(decline_of(index, &bindings, r));
            continue;
        };
        if src == target.datum {
            continue;
        }
        out.edges.push(ResolvedEdge {
            src,
            dst: target.datum,
            relation: relation_of(r.kind, target.is_class),
            grade: Grade::Proven,
        });
    }
    out
}

/// Which population a declined Rust reference belongs to.
///
/// A `use` moves the question to the binding's own head, which is where the
/// crate name actually is: `HashMap::new()` under `use std::collections::…`
/// is a path into `std`, not a bare name this tree lost. An anchor or a name
/// the tree roots is inside it; a bare unbound name is one a glob import
/// would have supplied, which is inside it too — and therefore a hole.
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
    if r.path.len() == 1 && bound.is_none() {
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
