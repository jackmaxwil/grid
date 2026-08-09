//! Callsign patterns: the name-space half of unification.
//!
//! A pattern is a dot-separated glob over a callsign. `*` matches exactly one
//! segment, `**` matches any number of segments (including none). Every other
//! segment matches itself literally. There is no negation: invariants are
//! stated in positive scope terms (wire contract, `Invariant`).

/// Does `callsign` match `pattern`?
///
/// ```
/// # use grid_xte::pattern::matches;
/// assert!(matches("cortana.reaper.**", "cortana.reaper.Reaper.sweep"));
/// assert!(matches("cortana.*.sweep", "cortana.reaper.sweep"));
/// assert!(!matches("cortana.*.sweep", "cortana.reaper.Reaper.sweep"));
/// ```
#[must_use]
pub fn matches(pattern: &str, callsign: &str) -> bool {
    let pat: Vec<&str> = pattern.split('.').collect();
    let name: Vec<&str> = callsign.split('.').collect();
    seg_match(&pat, &name)
}

fn seg_match(pat: &[&str], name: &[&str]) -> bool {
    match pat.split_first() {
        // Both exhausted: a match. Pattern exhausted, name is not: no match.
        None => name.is_empty(),
        // `**` consumes zero or more segments; try every split point.
        Some((&"**", rest)) => (0..=name.len()).any(|skip| seg_match(rest, &name[skip..])),
        Some((head, rest)) => match name.split_first() {
            Some((first, tail)) if *head == "*" || head == first => seg_match(rest, tail),
            _ => false,
        },
    }
}

/// Does `outer` match every callsign `inner` matches?
///
/// Pattern containment, not sampling: `cortana.**` subsumes `cortana.boards.*`
/// because no callsign satisfies the second without satisfying the first.
/// Conservative — a `false` means "not provably contained", never "provably
/// disjoint", which is the safe direction for a lint that calls a plan
/// impossible.
///
/// ```
/// # use grid_xte::pattern::subsumes;
/// assert!(subsumes("cortana.**", "cortana.boards.*"));
/// assert!(!subsumes("cortana.boards.*", "cortana.**"));
/// ```
#[must_use]
pub fn subsumes(outer: &str, inner: &str) -> bool {
    let o: Vec<&str> = outer.split('.').collect();
    let i: Vec<&str> = inner.split('.').collect();
    seg_subsumes(&o, &i)
}

fn seg_subsumes(o: &[&str], i: &[&str]) -> bool {
    match (o.split_first(), i.split_first()) {
        (None, _) => i.is_empty(),
        // `**` on the left absorbs any prefix of the right, including a `**`.
        (Some((&"**", o_rest)), _) => (0..=i.len()).any(|skip| seg_subsumes(o_rest, &i[skip..])),
        // The right side outruns the left, or asks a one-segment matcher to
        // cover the unbounded language `**` denotes. Neither is containment.
        (Some(_), None | Some((&"**", _))) => false,
        // `*` covers any single segment, glob or literal.
        (Some((&"*", o_rest)), Some((_, i_rest))) => seg_subsumes(o_rest, i_rest),
        // A literal covers only itself.
        (Some((o_head, o_rest)), Some((i_head, i_rest))) => {
            o_head == i_head && seg_subsumes(o_rest, i_rest)
        }
    }
}
