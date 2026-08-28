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
use grid_wire::Zoc;

use crate::index::{DefIndex, Symbol};
use crate::resolve::{relation_of, Bindings, ResolvedEdge};

/// Resolve one Rust file's references against the whole-tree index.
#[must_use]
pub(crate) fn resolve(
    index: &DefIndex,
    module: &str,
    imports: &[Import],
    refs: &[Reference],
) -> Vec<ResolvedEdge> {
    let bindings = Bindings { imports };
    let mut out = Vec::new();
    for r in refs {
        let Some(src) = index.datum_of(module, &r.from) else {
            continue;
        };
        let Some(target) = target_of(index, &bindings, module, r) else {
            continue;
        };
        if src == target.datum {
            continue;
        }
        out.push(ResolvedEdge {
            src,
            dst: target.datum,
            relation: relation_of(r.kind, target.is_class),
            zoc: Zoc::Proven,
        });
    }
    out
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
    // target may itself be anchor-headed, spelled against this module.
    if let Some(target) = bindings.lookup(&r.from, head) {
        let segs: Vec<&str> = target.split('.').collect();
        let (thead, trest) = segs.split_first()?;
        let base = anchor_str(module, thead, trest).unwrap_or_else(|| segs.join("."));
        let full = if rest.is_empty() {
            base
        } else {
            format!("{base}.{}", rest.join("."))
        };
        return index.lookup_absolute(&full);
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
