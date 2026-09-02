//! The whole-tree symbol index: dotted name to datum.
//!
//! Built once per survey from every file's definitions, then read-only. It is
//! the only thing that knows about the tree as a whole, which is why edge
//! resolution lives above it rather than inside a per-file adapter: a call is
//! a fact about two files, so no per-file pass can prove one.
use std::collections::HashMap;

use grid_datum::{DefKind, Definition};
use grid_wire::Lang;

/// The byte offset of every newline in one file, indexed once so a
/// definition's line is a binary search. Counting newlines from the top of
/// the file per definition made a survey quadratic in definitions *per
/// file*: one 5 MB generated file took 94 s where the same 144,000
/// definitions across 144 files took 1.2 s (verb sweep, B7).
struct Lines(Vec<usize>);

impl Lines {
    fn of(source: &str) -> Self {
        Self(
            source
                .bytes()
                .enumerate()
                .filter(|(_, b)| *b == b'\n')
                .map(|(at, _)| at)
                .collect(),
        )
    }

    /// 1-based line of byte `at`: one more than the newlines before it.
    fn line(&self, at: usize) -> u32 {
        u32::try_from(self.0.partition_point(|&nl| nl < at).saturating_add(1)).unwrap_or(u32::MAX)
    }
}

/// One indexed definition, keyed by its absolute dotted name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Content identity.
    pub datum: [u8; 32],
    /// `module.Class.method` — the name this symbol is filed under.
    pub absolute: String,
    /// Whether this is a class, which decides `calls` versus `constructs`.
    pub is_class: bool,
    /// The adapter that found it.
    pub lang: Lang,
    /// The stored per-definition columns: location, shape, complexity, sig.
    pub facts: grid_chart::DefFacts,
    /// The callee facts of a function, for the method-call rule; `None`
    /// for a type, a trait method, or a Python definition.
    pub method: Option<grid_datum::MethodFacts>,
}

/// Every definition in a tree, addressable by absolute dotted name.
///
/// A name that two files both define is *ambiguous*, and an ambiguous name
/// resolves to nothing: the index would have to pick one, and picking one
/// would manufacture an edge that might be to the wrong definition. Refusing
/// is the only answer compatible with `proven`.
#[derive(Debug, Default)]
pub struct DefIndex {
    by_name: HashMap<String, Symbol>,
    /// Names seen more than once. Kept so a lookup can distinguish "no such
    /// name" from "too many", which is the difference between an omission and
    /// an error a human should hear about.
    ambiguous: std::collections::HashSet<String>,
    /// Re-export aliases: absolute alias name -> (declaring module, raw
    /// target as written). Raw, because the target anchors against the
    /// declaring module at lookup time — `pub use build::build` in
    /// `grid_chart` may mean `grid_chart.build.build` or an external path,
    /// and only the index as a whole can tell.
    aliases: HashMap<String, (String, String)>,
    /// First segment of every module a definition was filed under: the names
    /// this tree's own callsigns are rooted in. What makes "outside the tree"
    /// decidable without reading a lockfile or a `site-packages`.
    roots: std::collections::HashSet<String>,
    /// Declared field types: absolute struct name -> field -> type. What
    /// types `x.field` when `x` is typed and the struct is another file's.
    field_types: HashMap<String, HashMap<String, grid_datum::TypeRef>>,
    /// Every method name any trait in the tree declares — the over-
    /// approximation of "a trait in scope supplies `m`".
    trait_methods: std::collections::HashSet<String>,
    /// Method names supplied by trait impls, keyed by the implementing
    /// type's last segment: `impl T for Foo { fn m }` files `Foo -> m`.
    /// Answers "does any `Foo@*.m` exist", for the in-tree candidate check
    /// and for the guard on a `std` type the tree implements its own trait
    /// for.
    impl_methods: HashMap<String, std::collections::HashSet<String>>,
}

impl DefIndex {
    /// Add one file's definitions under its module path.
    pub fn insert_file(
        &mut self,
        module: &str,
        lang: Lang,
        defs: &[Definition],
        path: &str,
        source: &str,
        inherited_cfg: &str,
    ) {
        if let Some(root) = module.split('.').next() {
            self.roots.insert(root.to_owned());
        }
        let lines = Lines::of(source);
        for def in defs {
            let absolute = format!("{module}.{}", def.callsign);
            // `prefix.Foo@Trait.m`: the type's last segment and the method.
            if let Some((owner, method)) = def.callsign.rsplit_once('.') {
                if let Some((ty, _trait)) = owner.rsplit_once('@') {
                    let ty = ty.rsplit('.').next().unwrap_or(ty);
                    self.impl_methods
                        .entry(ty.to_owned())
                        .or_default()
                        .insert(method.to_owned());
                }
            }
            let symbol = Symbol {
                datum: datum_bytes(def),
                absolute: absolute.clone(),
                is_class: def.kind == DefKind::Class,
                lang,
                method: def.method.clone(),
                facts: grid_chart::DefFacts {
                    path: path.to_owned(),
                    line: lines.line(def.span.start),
                    line_end: lines.line(def.span.end),
                    span: (
                        u32::try_from(def.span.start).unwrap_or(u32::MAX),
                        u32::try_from(def.span.end).unwrap_or(u32::MAX),
                    ),
                    shape: shape_bytes(def),
                    complexity: def.complexity,
                    sig: sig_bytes(def),
                    cfg: grid_datum::cfg_join(&[inherited_cfg, &def.cfg]),
                },
            };
            // The same datum under the same name twice is the same definition
            // seen twice (a file listed twice, a re-survey), not a collision.
            // Two *different* datums under one name is a name that cannot be
            // resolved, and both are then unreachable by name.
            let datum = symbol.datum;
            if let Some(prior) = self.by_name.insert(absolute.clone(), symbol) {
                if prior.datum != datum {
                    self.ambiguous.insert(absolute);
                }
            }
        }
    }

    /// Add one file's type facts: its structs' declared field types and the
    /// method names its traits declare.
    pub fn insert_types(
        &mut self,
        module: &str,
        field_types: &[(String, String, grid_datum::TypeRef)],
        trait_methods: &[String],
    ) {
        for (callsign, field, ty) in field_types {
            self.field_types
                .entry(format!("{module}.{callsign}"))
                .or_default()
                .insert(field.clone(), ty.clone());
        }
        self.trait_methods.extend(trait_methods.iter().cloned());
    }

    /// The declared type of `field` on the struct filed as `absolute`.
    #[must_use]
    pub fn field_type(&self, absolute: &str, field: &str) -> Option<&grid_datum::TypeRef> {
        self.field_types.get(absolute)?.get(field)
    }

    /// Does any trait in the tree declare a method named `method`?
    #[must_use]
    pub fn trait_declares(&self, method: &str) -> bool {
        self.trait_methods.contains(method)
    }

    /// Does any trait impl on a type whose last segment is `ty` supply
    /// `method` — is there a `{ty}@*.{method}`?
    #[must_use]
    pub fn impl_supplies(&self, ty: &str, method: &str) -> bool {
        self.impl_methods
            .get(ty)
            .is_some_and(|methods| methods.contains(method))
    }

    /// Record one re-export: `absolute` reaches whatever `target` names in
    /// `module`. Consulted only when no definition answers to the name, so
    /// a real definition always shadows an alias.
    pub fn insert_alias(&mut self, absolute: String, module: &str, target: &str) {
        self.aliases
            .insert(absolute, (module.to_owned(), target.to_owned()));
    }

    /// Look up an absolute dotted name, following re-export aliases.
    ///
    /// The depth cap bounds alias cycles (`pub use` loops do not compile,
    /// but the index must not trust that) — past it, refuse.
    /// Is `name` a root this tree's own callsigns hang under?
    ///
    /// The discriminator between a name grid could have charted and did not
    /// (a hole) and a name from outside the tree entirely (harmless to what
    /// grid claims about this repo).
    #[must_use]
    pub fn is_root(&self, name: &str) -> bool {
        self.roots.contains(name)
    }

    #[must_use]
    pub fn lookup_absolute(&self, absolute: &str) -> Option<&Symbol> {
        let mut name = absolute.to_owned();
        for _ in 0..8 {
            if self.ambiguous.contains(&name) {
                return None;
            }
            if let Some(sym) = self.by_name.get(&name) {
                return Some(sym);
            }
            let (module, target) = self.aliases.get(&name)?;
            // A `crate::`/`super::`/`self::` head anchors to one spelling;
            // a bare target is uniform-path — a local module's item or an
            // absolute path, local preferred when the index answers to it.
            name = if let Some(anchored) = crate::resolve_rs::anchor_target(module, target) {
                anchored
            } else {
                let local = format!("{module}.{target}");
                if self.by_name.contains_key(&local) || self.aliases.contains_key(&local) {
                    local
                } else {
                    target.clone()
                }
            };
        }
        None
    }

    /// Look up a name relative to `module`, falling back to the name as an
    /// absolute path (which is how a fully-qualified reference resolves).
    #[must_use]
    pub fn lookup(&self, module: &str, name: &str) -> Option<&Symbol> {
        self.lookup_absolute(&format!("{module}.{name}"))
            .or_else(|| self.lookup_absolute(name))
    }

    /// The datum of a definition named relative to `module`. `callsign` may be
    /// empty, which means module level and has no datum.
    #[must_use]
    pub fn datum_of(&self, module: &str, callsign: &str) -> Option<[u8; 32]> {
        if callsign.is_empty() {
            return None;
        }
        self.lookup_absolute(&format!("{module}.{callsign}"))
            .map(|s| s.datum)
    }

    /// Every indexed symbol.
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.by_name.values()
    }

    /// How many names were defined in more than one place. Reported so a
    /// survey can say what it refused to resolve rather than silently
    /// dropping it.
    #[must_use]
    pub fn ambiguous_count(&self) -> usize {
        self.ambiguous.len()
    }
}

/// A `grid-datum` `Datum` as the 32 raw bytes the chart interns.
///
/// The wire form is `b3:<hex>` and the chart stores the digest, so this is a
/// parse, not a re-hash: the identity is `grid-datum`'s, always.
#[must_use]
pub fn datum_bytes(def: &Definition) -> [u8; 32] {
    hex_bytes(def.datum.as_str())
}

/// The shape digest, same wire-to-raw parse as the datum.
#[must_use]
pub fn shape_bytes(def: &Definition) -> [u8; 32] {
    hex_bytes(def.shape.as_str())
}

/// The interface digest, same wire-to-raw parse as the datum.
#[must_use]
pub fn sig_bytes(def: &Definition) -> [u8; 32] {
    hex_bytes(def.sig.as_str())
}

fn hex_bytes(wire: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    let hex = wire.strip_prefix("b3:").unwrap_or_default();
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = hex
            .get(i * 2..i * 2 + 2)
            .and_then(|p| u8::from_str_radix(p, 16).ok())
            .unwrap_or(0);
    }
    out
}

#[cfg(test)]
mod lines {
    use super::Lines;

    /// The same answer the from-the-top scan gave, at every offset that
    /// matters: the first byte, a byte on a newline, the byte after one, and
    /// one past the end of the file.
    #[test]
    fn a_line_is_one_more_than_the_newlines_before_it() {
        let source = "ab\ncd\n\nef";
        let scan = |at: usize| {
            u32::try_from(source.get(..at).unwrap_or_default().matches('\n').count() + 1)
                .expect("small")
        };
        let lines = Lines::of(source);
        for at in 0..=source.len() {
            assert_eq!(lines.line(at), scan(at), "offset {at}");
        }
        assert_eq!(lines.line(0), 1);
        assert_eq!(lines.line(2), 1, "the newline byte is still on its line");
        assert_eq!(lines.line(3), 2);
        assert_eq!(lines.line(7), 4);
    }
}
