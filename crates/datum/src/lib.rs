//! grid-datum — the language-agnostic identity vocabulary. Pure, no I/O,
//! no parser: every type here is what a frontend *produces*, never how.
//!
//! A [`Datum`] is what code *is*, independent of how it is written: reformat
//! it, rename its locals, rewrite its comments, and the datum is unchanged.
//! Change a name it calls, a literal, or its structure, and the datum changes.
//! It is the reference every other coordinate is measured from (wire schema
//! `Datum`, `^b3:[0-9a-f]{64}$`).
//!
//! [`span_hash`] is the opposite instrument: the raw source bytes, sensitive
//! to everything the datum ignores. The verdict cache needs both — lint keys
//! on the span, typecheck and test key on identity.
//!
//! A frontend (grid-lang-py today) owns parse and normalization for one
//! language and emits these shapes; a callsign is an opaque ordered string
//! whose canonical form the frontend defines. Dispatch by language lives in
//! grid-survey, the one layer that knows what a file is.

/// Content identity: `b3:` + 64 lowercase hex of the canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Datum(String);

impl Datum {
    /// Hash canonical bytes into a datum. The only constructor, and only a
    /// frontend's normalizer may call it: a datum that did not come from
    /// normalization is not a datum.
    #[must_use]
    pub fn of_canonical_bytes(bytes: &[u8]) -> Self {
        Self(format!("b3:{}", blake3::hash(bytes).to_hex()))
    }

    /// The wire form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Datum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `blake3` of raw source bytes: sensitive to formatting and comments by
/// design. The lint-kind cache key, where formatting *is* the subject.
#[must_use]
pub fn span_hash(source: &str) -> String {
    format!("b3:{}", blake3::hash(source.as_bytes()).to_hex())
}

/// Source a frontend's parser rejected. Grid declines to hash it; it never
/// guesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(String);

impl ParseError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// What kind of definition a `Definition` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefKind {
    Function,
    Class,
}

/// One extracted definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub kind: DefKind,
    /// Canonical path from module root in the frontend's own spelling,
    /// e.g. `TagContract.validate`. Opaque to everything above the frontend.
    pub callsign: String,
    /// `blake3(normalized AST)` — invariant under formatting, comments and
    /// local renaming.
    pub datum: Datum,
    /// `blake3(raw source bytes of the span)` — sensitive to exactly those.
    pub span_hash: String,
    /// Byte range of the definition in the source.
    pub span: std::ops::Range<usize>,
    /// Identity with the definition's own name erased — the rename-join
    /// key. Two same-shape defs are rename-equivalent; the datum itself
    /// keeps the name, because the chart interns datums uniquely and
    /// same-body defs must not collapse.
    pub shape: Datum,
    /// Cyclomatic complexity: 1 + branch points. A property of the
    /// normalized AST, like the datum.
    pub complexity: u16,
}

/// How a name was reached for. The kind decides the edge relation once the
/// name is bound: a base class is `implements`, a call of a class is
/// `constructs`, a call of a function is `calls`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    /// `f(...)` — the callee position of a call.
    Call,
    /// `class C(B)` — a base type in a type definition.
    Base,
    /// `@d` above a definition. Attributed to the decorated definition: the
    /// decorator runs because that definition exists.
    Decorator,
    /// `self.m(...)` inside a class body. Resolution needs the enclosing
    /// class, which the reference's `from` callsign carries, so it is kept
    /// apart from a plain [`RefKind::Call`].
    SelfCall,
}

/// One name reached for, with the definition that reached for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Callsign of the definition containing the reference, dotted from the
    /// module root. Empty means module level — code that runs on import and
    /// belongs to no definition.
    pub from: String,
    /// What kind of position the name appeared in.
    pub kind: RefKind,
    /// The dotted name as written: `["a", "b", "c"]` for `a.b.c`. Never
    /// empty. For [`RefKind::SelfCall`] the leading receiver is stripped.
    pub path: Vec<String>,
}

/// One `import` binding: the local name a module or symbol was bound to.
///
/// `scope` is what makes a function-local import local. A binding applies to a
/// reference only if the binding's scope is the reference's definition or an
/// ancestor of it, which is the language's own rule and stops a shadowing
/// import inside one function from rewriting names in another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The name bound in this scope (`np` for `import numpy as np`).
    pub alias: String,
    /// The dotted path it names (`numpy`, or `pkg.mod.symbol` for a
    /// from-import). Relative imports keep their leading dots, so the
    /// resolver can anchor them against the importing module.
    pub target: String,
    /// Callsign of the definition the `import` statement sits in; empty for
    /// module level.
    pub scope: String,
}

/// Everything about one file that edge resolution needs beyond its
/// definitions: what it imported, and what its definitions reached for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileRefs {
    /// Import bindings, in source order.
    pub imports: Vec<Import>,
    /// References, in source order.
    pub refs: Vec<Reference>,
    /// Bindings this file re-exports (`pub use` and its visible-scoped kin):
    /// names other files may reach through this module. A subset of
    /// `imports` by construction.
    pub reexports: Vec<Import>,
}
