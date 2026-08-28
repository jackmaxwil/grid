//! Canonical bytes for one Rust node: erase doc attributes, print tokens.
//!
//! Token-stream printing is deterministic and whitespace-normalizing, which
//! buys formatting invariance; plain comments never reach the AST. Doc
//! comments do — syn keeps them as `#[doc = "..."]` attributes — so they are
//! filtered out of the stream here, or a doc edit would move the datum.
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;

/// The declared epoch: changing canonicalization changes every Rust datum,
/// so the prefix moves deliberately or not at all (same rule as lang-py).
const EPOCH: &[u8] = b"grid-lang-rs/v1\n";

pub(crate) fn canonical_bytes<T: ToTokens>(node: &T) -> Vec<u8> {
    let mut out = EPOCH.to_vec();
    out.extend_from_slice(normalize(node.to_token_stream()).to_string().as_bytes());
    out
}

/// Remove every `#[doc ...]` / `#![doc ...]` attribute and every trailing
/// comma rustfmt is free to add or drop, recursively.
fn normalize(stream: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while let Some(token) = tokens.get(i) {
        if let TokenTree::Punct(p) = token {
            if p.as_char() == '#' {
                let bang = matches!(tokens.get(i.saturating_add(1)),
                    Some(TokenTree::Punct(q)) if q.as_char() == '!');
                let at = i.saturating_add(1).saturating_add(usize::from(bang));
                if let Some(TokenTree::Group(g)) = tokens.get(at) {
                    if g.delimiter() == Delimiter::Bracket && is_doc(g) {
                        i = at.saturating_add(1);
                        continue;
                    }
                }
            }
        }
        match token {
            TokenTree::Group(g) => {
                let inner: Vec<TokenTree> = normalize(g.stream()).into_iter().collect();
                out.push(TokenTree::Group(Group::new(
                    g.delimiter(),
                    drop_trailing_comma(inner, g.delimiter(), call_shaped(out.last())),
                )));
            }
            other => out.push(other.clone()),
        }
        i = i.saturating_add(1);
    }
    out.into_iter().collect()
}

/// Keywords that put a following paren group in value or pattern position,
/// where `(x,)` really is a one-tuple. Any other ident before `(` is a call,
/// a signature, or a tuple-struct declaration — argument lists, all of them.
const VALUE_KEYWORDS: &[&str] = &[
    "let", "if", "else", "match", "return", "while", "loop", "for", "in", "move", "break",
    "continue", "yield", "async", "unsafe", "const", "static", "ref", "mut", "box", "do",
];

fn call_shaped(prev: Option<&TokenTree>) -> bool {
    match prev {
        Some(TokenTree::Ident(id)) => !VALUE_KEYWORDS.contains(&id.to_string().as_str()),
        // The close of a turbofish: `f::<T>(a,)`.
        Some(TokenTree::Punct(p)) => p.as_char() == '>',
        _ => false,
    }
}

/// A trailing comma is formatting everywhere except the one place Rust makes
/// it grammar: `(x,)` is a one-tuple and `(x)` is a parenthesized `x`. A
/// paren group in call shape strips it; one in value position keeps it
/// unless a second comma proves the tuple-ness either way.
fn drop_trailing_comma(
    mut tokens: Vec<TokenTree>,
    delimiter: Delimiter,
    call_shaped: bool,
) -> TokenStream {
    let trailing = matches!(tokens.last(), Some(TokenTree::Punct(p)) if p.as_char() == ',');
    if trailing {
        let commas = tokens
            .iter()
            .filter(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ','))
            .count();
        if delimiter != Delimiter::Parenthesis || call_shaped || commas >= 2 {
            tokens.pop();
        }
    }
    tokens.into_iter().collect()
}

fn is_doc(group: &Group) -> bool {
    matches!(group.stream().into_iter().next(),
        Some(TokenTree::Ident(id)) if id == "doc")
}
