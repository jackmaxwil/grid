//! The whole-tree symbol index: dotted name to datum.
//!
//! Built once per survey from every file's definitions, then read-only. It is
//! the only thing that knows about the tree as a whole, which is why edge
//! resolution lives above it rather than inside a per-file adapter: a call is
//! a fact about two files, so no per-file pass can prove one.
use std::collections::HashMap;

use grid_datum::{DefKind, Definition};
use grid_wire::Lang;

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
}

impl DefIndex {
    /// Add one file's definitions under its module path.
    pub fn insert_file(&mut self, module: &str, lang: Lang, defs: &[Definition]) {
        for def in defs {
            let absolute = format!("{module}.{}", def.callsign);
            let symbol = Symbol {
                datum: datum_bytes(def),
                absolute: absolute.clone(),
                is_class: def.kind == DefKind::Class,
                lang,
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

    /// Look up an absolute dotted name.
    #[must_use]
    pub fn lookup_absolute(&self, absolute: &str) -> Option<&Symbol> {
        if self.ambiguous.contains(absolute) {
            return None;
        }
        self.by_name.get(absolute)
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
    let mut out = [0u8; 32];
    let hex = def.datum.as_str().strip_prefix("b3:").unwrap_or_default();
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = hex
            .get(i * 2..i * 2 + 2)
            .and_then(|p| u8::from_str_radix(p, 16).ok())
            .unwrap_or(0);
    }
    out
}
