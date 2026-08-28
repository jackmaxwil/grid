//! Wave 2 assurance evidence for the datum normalizer.
//!
//! Three properties, one per plan clause:
//!   * normalization is idempotent (renormalizing canonical output is a fixpoint);
//!   * `Datum` is invariant under formatting, comment and local-rename perturbation;
//!   * `span_hash` is *sensitive* to those same perturbations — the inverse property,
//!     which is what makes the pair usable as cache keys.
use std::fmt::Write as _;

use grid_datum::span_hash;
use grid_lang_py::{definitions, normalize};
use proptest::prelude::*;

/// A generated Python function: name, parameter names, and a body that uses
/// them, so renaming actually has something to move.
#[derive(Debug, Clone)]
struct Func {
    name: String,
    params: Vec<String>,
    locals: Vec<String>,
}

impl Func {
    fn render(&self, indent: &str, blank_lines: usize, comment: Option<&str>) -> String {
        let params = self.params.join(", ");
        let mut src = format!("def {}({}):\n", self.name, params);
        if let Some(c) = comment {
            let _ = writeln!(src, "{indent}# {c}");
        }
        for _ in 0..blank_lines {
            src.push('\n');
        }
        let mut last = self
            .params
            .first()
            .cloned()
            .unwrap_or_else(|| "0".to_owned());
        for local in &self.locals {
            let _ = writeln!(src, "{indent}{local} = {last} + 1");
            last.clone_from(local);
        }
        let _ = writeln!(src, "{indent}return {last}");
        src
    }

    /// The same function with every local and parameter renamed.
    fn alpha_renamed(&self) -> Self {
        let rename = |names: &[String], tag: &str| {
            names
                .iter()
                .enumerate()
                .map(|(i, _)| format!("{tag}zz{i}"))
                .collect::<Vec<_>>()
        };
        Self {
            name: self.name.clone(),
            params: rename(&self.params, "p"),
            locals: rename(&self.locals, "l"),
        }
    }
}

fn ident(prefix: &'static str) -> impl Strategy<Value = String> {
    (0usize..6).prop_map(move |i| format!("{prefix}{i}"))
}

fn func() -> impl Strategy<Value = Func> {
    (
        ident("f"),
        prop::collection::vec(ident("a"), 0..4),
        prop::collection::vec(ident("b"), 1..4),
    )
        .prop_map(|(name, params, locals)| {
            // Distinct names only: Python allows shadowing but duplicate
            // parameters are a syntax error.
            let dedup = |v: Vec<String>| {
                let mut seen = Vec::new();
                for x in v {
                    if !seen.contains(&x) {
                        seen.push(x);
                    }
                }
                seen
            };
            Func {
                name,
                params: dedup(params),
                locals: dedup(locals),
            }
        })
}

proptest! {
    /// Normalization is deterministic and idempotent: the same source always
    /// yields the same datum, and normalizing twice changes nothing.
    #[test]
    fn normalization_is_idempotent(f in func()) {
        let src = f.render("    ", 0, None);
        let first = normalize(&src).unwrap();
        let second = normalize(&src).unwrap();
        prop_assert_eq!(&first.datum, &second.datum);
        prop_assert_eq!(&first.canonical, &second.canonical);
    }

    /// `Datum` is invariant under formatting, comments and local renaming.
    #[test]
    fn datum_invariant_under_perturbation(
        f in func(),
        blanks in 0usize..3,
        indent in prop::sample::select(vec!["    ", "\t", "        "]),
        comment in prop::option::of(prop::sample::select(vec!["note", "TODO: x"])),
    ) {
        let plain = normalize(&f.render("    ", 0, None)).unwrap();
        let perturbed = normalize(&f.alpha_renamed().render(indent, blanks, comment)).unwrap();
        prop_assert_eq!(plain.datum, perturbed.datum);
    }

    /// The inverse: `span_hash` sees every perturbation the datum ignores.
    /// Without this, the two hashes would be redundant and the cache's
    /// per-kind keys unsound.
    #[test]
    fn span_hash_sensitive_to_perturbation(f in func()) {
        let plain = f.render("    ", 0, None);
        let renamed = f.alpha_renamed().render("    ", 0, None);
        let commented = f.render("    ", 1, Some("note"));
        prop_assume!(plain != renamed);
        prop_assert_ne!(span_hash(&plain), span_hash(&renamed));
        prop_assert_ne!(span_hash(&plain), span_hash(&commented));
    }

    /// Renaming a definition's *own* name changes its identity: the callsign
    /// is interface, not a local.
    #[test]
    fn interface_names_are_identity(f in func()) {
        let plain = normalize(&f.render("    ", 0, None)).unwrap();
        let mut renamed_def = f.clone();
        renamed_def.name = format!("{}_other", f.name);
        let renamed = normalize(&renamed_def.render("    ", 0, None)).unwrap();
        prop_assert_ne!(plain.datum, renamed.datum);
    }

    /// Per-definition extraction agrees with whole-module normalization on
    /// the identity of a single-definition file.
    #[test]
    fn extraction_matches_module_identity(f in func()) {
        let src = f.render("    ", 0, None);
        let defs = definitions(&src).unwrap();
        prop_assert_eq!(defs.len(), 1);
        prop_assert_eq!(&defs[0].callsign, &f.name);
        prop_assert_eq!(&defs[0].datum, &normalize(&src).unwrap().datum);
    }
}

/// Comments are metadata, never identity: they survive normalization intact
/// and do not move the datum.
#[test]
fn comments_are_retained_as_metadata() {
    let src = "def f(x):\n    # first\n    y = x  # second\n    return y\n";
    let n = normalize(src).unwrap();
    let texts: Vec<_> = n.comments.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, vec!["# first", "# second"]);
    let stripped = "def f(x):\n    y = x\n    return y\n";
    assert_eq!(n.datum, normalize(stripped).unwrap().datum);
}

/// A `global` declaration makes the name interface, not a local: two bodies
/// differing only in the global they write are different definitions.
#[test]
fn globals_are_not_renamed() {
    let a = normalize("def f():\n    global count\n    count = 1\n").unwrap();
    let b = normalize("def f():\n    global other\n    other = 1\n").unwrap();
    assert_ne!(a.datum, b.datum);
}

/// Class bodies are interface: attribute names are part of identity.
#[test]
fn class_attributes_are_identity() {
    let a = normalize("class C:\n    attr = 1\n").unwrap();
    let b = normalize("class C:\n    other = 1\n").unwrap();
    assert_ne!(a.datum, b.datum);
}

/// Nested definitions are extracted with dotted callsigns.
#[test]
fn nested_definitions_get_dotted_callsigns() {
    let src = "class C:\n    def m(self):\n        return 1\n";
    let defs = definitions(src).unwrap();
    let names: Vec<_> = defs.iter().map(|d| d.callsign.as_str()).collect();
    assert_eq!(names, vec!["C", "C.m"]);
}

/// Invalid Python is an error, never a silent hash.
#[test]
fn invalid_source_is_an_error() {
    assert!(normalize("def f(:\n").is_err());
}

/// D7: the datum we emit must satisfy the wire contract's `Datum` pattern
/// (`^b3:[0-9a-f]{64}$`) — identities computed here cross the socket as-is.
#[test]
fn datum_matches_wire_pattern() {
    let datum = normalize("def f(x):\n    return x\n")
        .unwrap()
        .datum
        .to_string();
    assert!(datum.starts_with("b3:"));
    let hex = &datum[3..];
    assert_eq!(hex.len(), 64);
    assert!(hex
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    // Definitions carry the same guarantee on both of their hashes.
    let def = &definitions("def f(x):\n    return x\n").unwrap()[0];
    for id in [def.datum.as_str(), def.span_hash.as_str()] {
        assert!(id.starts_with("b3:") && id[3..].len() == 64);
    }
}
