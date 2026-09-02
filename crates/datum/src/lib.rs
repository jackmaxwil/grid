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
    /// `blake3` of the frontend-canonical *interface* bytes: everything a
    /// caller can observe without reading the body. A body-only edit moves
    /// the datum and holds the sig; a signature edit moves both. Same
    /// constructor discipline as the datum — only a frontend's normalizer
    /// builds one, and each frontend owns its canonical form under the one
    /// rule that a fact is in the sig iff a caller can observe it without
    /// the body.
    pub sig: Datum,
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
    /// The conjunction of every `#[cfg(...)]` predicate on the definition
    /// and on each enclosing module, as written (`all(...)`-joined when
    /// nested; a `cfg_attr(...)` is recorded as its own predicate text).
    /// Empty when nothing conditions it. A frontend with no such notion
    /// leaves it empty.
    pub cfg: String,
    /// What a method-call resolver needs to know about this definition as
    /// a callee: its receiver form, whether its impl is plain, its written
    /// return. `None` for a type, a trait method, or a frontend with no
    /// such notion. Rides beside the identities and enters none of them.
    pub method: Option<MethodFacts>,
}

/// How a receiver is passed: by value, by shared reference, by mutable
/// reference. The form of a method's own `self`, and the form of a receiver
/// expression at a call site — rustc's method lookup matches the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverForm {
    Value,
    Ref,
    RefMut,
}

/// Where a receiver's type came from. The written origins are the
/// language's own binding rule; the inferred ones are the frontend's reading
/// of syntax, which is why a resolver grades them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverOrigin {
    /// A parameter annotation, `x: &Foo`.
    Param,
    /// A `let` annotation, `let x: Foo = …`.
    LetAnnotated,
    /// `self.field` where the field's declared type is known.
    SelfField,
    /// `x.field` on a typed `x`; the field's type may be another file's,
    /// in which case [`Receiver::field`] names it for the resolver.
    TypedField,
    /// A local bound by `Type::f(..)` or `f(..)` with a written return.
    LocalCtor,
    /// A local bound by a struct literal `Foo { .. }`.
    LocalLiteral,
    /// A chain through a written return type, `x.g().m()`.
    ChainReturn,
    /// `?`, `.unwrap()` or `.expect(..)` on a written `Option<T>` /
    /// `Result<T, _>`.
    Unwrapped,
}

impl ReceiverOrigin {
    /// Whether the programmer wrote the type, as opposed to the frontend
    /// reading it off an initializer.
    #[must_use]
    pub const fn written(self) -> bool {
        matches!(
            self,
            Self::Param | Self::LetAnnotated | Self::SelfField | Self::TypedField
        )
    }
}

/// A type as a path, the way a resolver can look it up: the outermost
/// path, whether it carries generic arguments, the reference form around
/// it, and — for one level of wrapper — the first argument's own type, so
/// `Result<Foo, E>` unwraps to `Foo`. Wrappers are never looked through
/// here: `Option<Foo>` is `["Option"]` with `args`, and only an explicit
/// `?`/`unwrap` reaches `inner`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub path: Vec<String>,
    pub args: bool,
    pub form: ReceiverForm,
    pub inner: Option<Box<TypeRef>>,
}

/// What a file says about the receiver of one method call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receiver {
    /// The receiver's type, as written or as inferred, `Self` replaced by
    /// the impl's own type.
    pub ty: TypeRef,
    pub origin: ReceiverOrigin,
    /// For [`ReceiverOrigin::TypedField`]: the field read off `ty`, whose
    /// declared type the resolver finds in the index. `None` when `ty` is
    /// already the receiver's own type.
    pub field: Option<String>,
}

/// A callee's facts for the method-call rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodFacts {
    /// The method's own `self` form; `None` for an associated function or
    /// a free function.
    pub recv: Option<ReceiverForm>,
    /// True for `impl Foo` and `impl<T> Foo<T>` with no bounds and no
    /// where clause — the impls whose methods apply to every `Foo`. False
    /// for `impl Foo<u8>`, `impl<T: Bound> Foo<T>`, `impl<T> Foo<T> where …`.
    pub plain_impl: bool,
    /// The written return type, `Self` substituted, wrapper kept.
    pub ret: Option<TypeRef>,
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
    /// `x.m(...)` — a call whose callee is reached through a receiver the
    /// frontend has no type for. The path is the receiver's own dotted path
    /// (empty when the receiver is an expression, not a name) followed by the
    /// method name. Kept apart from [`RefKind::Call`] because resolving it
    /// would take types, and a survey emits none: it is counted, never
    /// guessed.
    MethodCall,
    /// `Foo { x: 1 }` — a struct literal writes the type's fields. The wire
    /// edge carries no field name, so the target is the type itself; field
    /// *reads* need receiver types and are never emitted.
    FieldWrite,
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
    /// For [`RefKind::MethodCall`]: what the file says about the
    /// receiver's type, when it says anything. `None` is the honest answer
    /// for a closure parameter, a pattern binding, a chain through an
    /// unwritten return — and the call then stays declined.
    pub receiver: Option<Receiver>,
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
    /// Out-of-line module declarations that relocate their file:
    /// `(mod name, declared file path)` from `#[path = "..."] mod name;`.
    /// The path is as written — relative to the declaring file's directory.
    pub path_mods: Vec<(String, String)>,
    /// Module declarations that carry a `#[cfg]`: `(module name relative
    /// to this file, predicate text)`. An out-of-line module's own file
    /// cannot see the attribute on its declaration, so the survey joins it
    /// back on by module path.
    pub mod_cfgs: Vec<(String, String)>,
    /// Declared field types: `(struct callsign, field, type)`, for every
    /// named field whose type is a path. What lets a resolver type
    /// `x.field` when `x` is typed and the struct lives in another file.
    pub field_types: Vec<(String, String, TypeRef)>,
    /// The name of every method every trait in this file declares, default
    /// or required. The tree-wide union is the resolver's over-approximation
    /// of "a trait in scope supplies `m`": a maybe is a refusal.
    pub trait_methods: Vec<String>,
}

/// The conjunction of cfg predicates as one predicate: nothing for none,
/// the predicate itself for one, `all(a,b,…)` for more. Empty parts are
/// skipped, so joining an unconditioned scope costs nothing.
#[must_use]
pub fn cfg_join<S: AsRef<str>>(parts: &[S]) -> String {
    let live: Vec<&str> = parts
        .iter()
        .map(AsRef::as_ref)
        .filter(|p| !p.is_empty())
        .collect();
    match live.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        many => format!("all({})", many.join(",")),
    }
}
