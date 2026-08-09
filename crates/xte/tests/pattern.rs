//! Callsign patterns: the properties a glob language has to have, and the
//! containment relation `track.lint` calls a plan impossible with.

use grid_xte::pattern::{matches, subsumes};
use proptest::prelude::*;

#[test]
fn glob_arity_is_the_whole_language() {
    assert!(matches(
        "cortana.reaper.Reaper.sweep",
        "cortana.reaper.Reaper.sweep"
    ));
    assert!(matches("cortana.reaper.**", "cortana.reaper.Reaper.sweep"));
    // `**` matches zero segments too, or it would not be "any number".
    assert!(matches("cortana.reaper.**", "cortana.reaper"));
    // `*` is exactly one segment, never two.
    assert!(matches("cortana.*.sweep", "cortana.reaper.sweep"));
    assert!(!matches("cortana.*.sweep", "cortana.reaper.Reaper.sweep"));
    assert!(!matches("cortana.reaper.**", "cortana.boards.render"));
    // A glob is a whole segment or it is a literal: no infix matching.
    assert!(!matches("cortana.re*", "cortana.reaper"));
}

#[test]
fn subsumption_is_containment_not_sampling() {
    assert!(subsumes("**", "cortana.boards.render"));
    assert!(subsumes("cortana.**", "cortana.boards.*"));
    assert!(subsumes("cortana.*", "cortana.boards"));
    // The narrow pattern does not contain the wide one.
    assert!(!subsumes("cortana.boards.*", "cortana.**"));
    assert!(!subsumes("cortana.*", "cortana.boards.**"));
    assert!(!subsumes("cortana.boards.**", "cortana.**"));
}

/// Segments of the shape real callsigns have, plus the two globs.
fn segment() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[a-z][a-zA-Z0-9_]{0,6}",
        1 => Just("*".to_string()),
        1 => Just("**".to_string()),
    ]
}

fn callsign() -> impl Strategy<Value = String> {
    prop::collection::vec("[a-z][a-zA-Z0-9_]{0,6}", 1..5).prop_map(|v| v.join("."))
}

fn pattern() -> impl Strategy<Value = String> {
    prop::collection::vec(segment(), 1..5).prop_map(|v| v.join("."))
}

proptest! {
    /// The reflexive case a glob language must not get wrong: a literal
    /// pattern matches its own callsign and nothing shorter or longer.
    #[test]
    fn literal_matches_itself(name in callsign()) {
        prop_assert!(matches(&name, &name));
    }

    /// `**` is the top of the lattice: it matches everything, and it contains
    /// every pattern.
    #[test]
    fn double_star_is_top(name in callsign(), pat in pattern()) {
        prop_assert!(matches("**", &name));
        prop_assert!(subsumes("**", &pat));
    }

    /// Subsumption is exactly what it claims: if `outer` contains `inner`,
    /// then every callsign `inner` matches, `outer` matches too. This is the
    /// property the lint's "impossible plan" verdict rests on — without it,
    /// lint would reject plans that could have been made good.
    #[test]
    fn subsumption_implies_matching(outer in pattern(), inner in pattern(), name in callsign()) {
        if subsumes(&outer, &inner) && matches(&inner, &name) {
            prop_assert!(
                matches(&outer, &name),
                "{outer} subsumes {inner}, which matches {name}, but {outer} does not"
            );
        }
    }

    /// Subsumption is reflexive and transitive — it is a preorder, so lint
    /// cannot depend on the order rules are written in.
    #[test]
    fn subsumption_is_a_preorder(a in pattern(), b in pattern(), cc in pattern()) {
        prop_assert!(subsumes(&a, &a));
        if subsumes(&a, &b) && subsumes(&b, &cc) {
            prop_assert!(subsumes(&a, &cc), "{a} >= {b} >= {cc}, but not {a} >= {cc}");
        }
    }

    /// Matching never panics and never recurses without bound, however many
    /// globs are stacked. `**` backtracking is the risk; this is the guard.
    #[test]
    fn matching_terminates_on_glob_soup(
        pat in prop::collection::vec(prop_oneof![Just("*"), Just("**"), Just("a")], 0..12),
        name in prop::collection::vec(Just("a"), 0..12),
    ) {
        let _ = matches(&pat.join("."), &name.join("."));
    }
}
