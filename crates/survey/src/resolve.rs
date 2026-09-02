//! Name binding: a reference's dotted path to the datum it names.
//!
//! This is where the survey earns the word *proven*. A reference is a name;
//! an edge needs a datum; the step between them is where a careless extractor
//! invents relationships that the code does not have. The rule here is that a
//! binding is recorded only when the name resolves to exactly one definition
//! by a rule Python itself would follow, and every other case resolves to
//! nothing at all.
//!
//! What is deliberately *not* attempted: values that only exist at runtime.
//! `obj.method()` where `obj` is a parameter needs types, and types are
//! `modeled`, not `proven` — so this wave drops it rather than guessing. That
//! is why `--tier` is not a flag: the tier is a property of the evidence, and
//! there is only one kind of evidence here.
use std::collections::HashMap;

use grid_datum::{Import, RefKind, Reference};
use grid_wire::{Grade, Relation};

use crate::index::{DefIndex, Symbol};

/// One resolved edge, in datum space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEdge {
    /// Datum of the definition the reference was found in.
    pub src: [u8; 32],
    /// Datum the name resolved to.
    pub dst: [u8; 32],
    /// What kind of relationship the reference position implies.
    pub relation: Relation,
    /// Always [`Grade::Proven`] at this wave: an unproven resolution is not
    /// recorded at all, so there is no weaker grade to report.
    pub grade: Grade,
}

/// Import bindings in force for one module, layered by scope.
///
/// A binding applies to a reference only when the binding's scope is a prefix
/// of the reference's callsign — Python's rule, and the reason a
/// function-local `import os` cannot rewrite `os` in a sibling function.
pub(crate) struct Bindings<'a> {
    pub(crate) imports: &'a [Import],
}

impl Bindings<'_> {
    /// The import target `name` is bound to, as visible from `scope`.
    ///
    /// The innermost applicable binding wins, and among equals the last one
    /// wins, because a later `import` rebinds the name.
    pub(crate) fn lookup(&self, scope: &str, name: &str) -> Option<&str> {
        self.imports
            .iter()
            .filter(|i| i.alias == name && scope_contains(&i.scope, scope))
            .max_by_key(|i| i.scope.len())
            .map(|i| i.target.as_str())
    }
}

/// Is `outer` the scope `inner` sits in, or an ancestor of it? The empty
/// scope is the module and contains everything.
pub(crate) fn scope_contains(outer: &str, inner: &str) -> bool {
    outer.is_empty()
        || inner == outer
        || inner
            .strip_prefix(outer)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// A file's resolved edges, plus what the resolver declined to resolve.
///
/// The count is the honest half. A reference whose name the language's own
/// rules cannot bind to exactly one definition — a name a glob import would
/// have supplied, a name from outside the tree, an ambiguous callsign —
/// yields no edge, and an unreported omission is indistinguishable from an
/// absence of coupling. Every consumer that answers "nothing reaches this"
/// has to be able to say how much it could not see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    /// Edges the language's binding rules establish.
    pub edges: Vec<ResolvedEdge>,
    /// References whose source definition resolved and whose *target* did
    /// not. Module-level references are not counted: they have no source
    /// definition, so they were never candidates for an edge. The sum of the
    /// three splits below.
    pub declined: u32,
    /// Of [`Resolved::declined`]: a name that was never this tree's to hold —
    /// `std`/`core`/`alloc` or another crate in Rust, a top-level module the
    /// tree does not root in Python. Costs intra-repo soundness nothing.
    pub declined_external: u32,
    /// Of [`Resolved::declined`]: a call through a receiver whose type would
    /// have to be known. Never resolved, always counted.
    pub declined_method: u32,
    /// Of [`Resolved::declined`]: a name this tree could have held and the
    /// resolver could not bind — glob-supplied, ambiguous, an unresolved
    /// re-export. The population that is an actual hole in what grid claims.
    pub declined_internal: u32,
}

/// Which population a declined reference belongs to.
///
/// One number for three meanings was the honesty gap: a path into `std` is
/// harmless to any claim about this repo, while a name a glob import would
/// have supplied is a hole in exactly the claim `grid drift` makes, and a
/// method call is a hole no static rule can close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decline {
    /// The name was never in this tree.
    External,
    /// The callee sits behind a receiver with no type.
    Method,
    /// A name this tree could have held.
    Internal,
}

impl Resolved {
    pub(crate) fn decline(&mut self, which: Decline) {
        self.declined = self.declined.saturating_add(1);
        let slot = match which {
            Decline::External => &mut self.declined_external,
            Decline::Method => &mut self.declined_method,
            Decline::Internal => &mut self.declined_internal,
        };
        *slot = slot.saturating_add(1);
    }
}

/// Resolve one file's references against the whole-tree index.
///
/// `module` is the dotted module path of the file being resolved, used to
/// anchor relative imports and to look up names defined in the file itself.
#[must_use]
pub fn resolve(index: &DefIndex, module: &str, imports: &[Import], refs: &[Reference]) -> Resolved {
    let bindings = Bindings { imports };
    let mut out = Resolved::default();
    for r in refs {
        // A reference from module level has no source definition, so it has
        // no edge: an edge is a relationship between two definitions.
        let Some(src) = index.datum_of(module, &r.from) else {
            continue;
        };
        let Some(target) = target_of(index, &bindings, module, r) else {
            out.decline(decline_of(index, &bindings, module, r));
            continue;
        };
        // A definition referencing itself is a fact about recursion, not a
        // relationship between two definitions; the chart has no use for it.
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

/// The relation a reference position implies, once the target's kind is known.
///
/// Calling a class constructs an instance; calling a function calls it. This
/// is the only place the distinction is drawn, and it needs the *target*, so
/// it cannot be decided at extraction time.
pub(crate) fn relation_of(kind: RefKind, target_is_class: bool) -> Relation {
    match kind {
        RefKind::Base => Relation::Implements,
        RefKind::FieldWrite => Relation::WritesField,
        // A decorator is applied to the definition, which is a call of the
        // decorator with the definition as its argument.
        RefKind::Decorator | RefKind::Call | RefKind::SelfCall | RefKind::MethodCall
            if target_is_class =>
        {
            Relation::Constructs
        }
        _ => Relation::Calls,
    }
}

/// Which population a declined Python reference belongs to.
///
/// An import binding settles it: the module the head names is either one this
/// tree roots or one it does not. Without a binding, an attribute call is a
/// method call on a value, and a bare name is one this tree could have held —
/// a builtin, a glob's supply, or a definition the resolver could not find.
fn decline_of(index: &DefIndex, bindings: &Bindings, module: &str, r: &Reference) -> Decline {
    let Some(head) = r.path.first() else {
        return Decline::Internal;
    };
    if let Some(target) = bindings.lookup(&r.from, head) {
        let absolute = absolute(module, target);
        let root = absolute.split('.').next().unwrap_or(&absolute);
        return if index.is_root(root) {
            Decline::Internal
        } else {
            Decline::External
        };
    }
    if r.kind == RefKind::MethodCall {
        return Decline::Method;
    }
    Decline::Internal
}

/// Resolve a reference's dotted path to a definition, or to nothing.
fn target_of<'a>(
    index: &'a DefIndex,
    bindings: &Bindings,
    module: &str,
    r: &Reference,
) -> Option<&'a Symbol> {
    let (head, rest) = r.path.split_first()?;

    // `self.m()` — the receiver is the class the reference sits in, which is
    // the callsign minus its last segment. This is the one runtime value the
    // proven tier does resolve, because Python's own binding rules fix it.
    if r.kind == RefKind::SelfCall {
        let class = r.from.rsplit_once('.').map(|(c, _)| c)?;
        return index.lookup(module, &format!("{class}.{}", r.path.join(".")));
    }

    // A name bound by an import resolves through that import, and only then.
    if let Some(target) = bindings.lookup(&r.from, head) {
        let target = absolute(module, target);
        let full = if rest.is_empty() {
            target
        } else {
            format!("{target}.{}", rest.join("."))
        };
        return index.lookup_absolute(&full);
    }

    // Otherwise the name is defined in this module, or it is not resolvable.
    // Enclosing scopes are tried outermost-last, which is Python's LEGB order
    // restricted to the parts that are statically decidable.
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

/// Turn a possibly-relative import target into an absolute module path.
///
/// `.sib` in `pkg.mod` is `pkg.sib`; `..other` in `pkg.sub.mod` is
/// `pkg.other`. A relative import that climbs past the root does not resolve,
/// which is also what Python says.
fn absolute(module: &str, target: &str) -> String {
    let dots = target.len() - target.trim_start_matches('.').len();
    if dots == 0 {
        return target.to_owned();
    }
    // One dot means "this package": the module's own parent.
    let mut base: Vec<&str> = module.split('.').collect();
    base.pop();
    for _ in 1..dots {
        base.pop();
    }
    let rest = target.trim_start_matches('.');
    if base.is_empty() {
        return rest.to_owned();
    }
    format!("{}.{rest}", base.join("."))
}

/// A map from a module path to the file that defines it, used to turn an
/// import target into the module it names. Built once per survey.
pub type ModuleMap = HashMap<String, String>;
