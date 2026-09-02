//! The whole-tree symbol index: dotted name to datum.
//!
//! Built once per survey from every file's definitions, then read-only. It is
//! the only thing that knows about the tree as a whole, which is why edge
//! resolution lives above it rather than inside a per-file adapter: a call is
//! a fact about two files, so no per-file pass can prove one.
use std::collections::HashMap;

use grid_datum::{DefKind, Definition};
use grid_wire::Lang;

/// 1-based line of `at` in `source` — counted once per definition at survey
/// time, because the snapshot is of this exact text.
fn line_of(source: &str, at: usize) -> u32 {
    let head = source.get(..at).unwrap_or_default();
    u32::try_from(
        head.bytes()
            .filter(|b| *b == b'\n')
            .count()
            .saturating_add(1),
    )
    .unwrap_or(u32::MAX)
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
    ) {
        if let Some(root) = module.split('.').next() {
            self.roots.insert(root.to_owned());
        }
        for def in defs {
            let absolute = format!("{module}.{}", def.callsign);
            let symbol = Symbol {
                datum: datum_bytes(def),
                absolute: absolute.clone(),
                is_class: def.kind == DefKind::Class,
                lang,
                facts: grid_chart::DefFacts {
                    path: path.to_owned(),
                    line: line_of(source, def.span.start),
                    line_end: line_of(source, def.span.end),
                    span: (
                        u32::try_from(def.span.start).unwrap_or(u32::MAX),
                        u32::try_from(def.span.end).unwrap_or(u32::MAX),
                    ),
                    shape: shape_bytes(def),
                    complexity: def.complexity,
                    sig: sig_bytes(def),
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
