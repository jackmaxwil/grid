//! Deserialize compatibility: what old bytes cannot read.
//!
//! Inside one repository both ends of a serde boundary share the struct, and
//! the compiler forces them to move together. The case it cannot see is
//! **old bytes**: a config file, a cache row, a queued message written by the
//! previous snapshot. `grid covenant` covers a declared *pair*; this is the
//! single-type case.
//!
//! For each definition in drift's changed set `C` that is a `struct` or
//! `enum` deriving `Deserialize` at either snapshot, the two shapes are
//! reconstructed from source — base out of the object store, head out of the
//! worktree — and compared under the six rules old bytes care about. Every
//! rule is a fact about the two field lists; nothing here guesses.
//!
//! Grade is **heuristic** by default: old bytes are not *known* to exist.
//! A standing `persisted` invariant is the human saying they do, and that
//! promotes a finding to **modeled**, which gates.
//!
//! Whatever cannot be read is counted, never cleared: a derive behind a
//! `cfg_attr`, one a macro produced, a serde attribute this reader cannot
//! parse, or a source file neither snapshot can serve.

use std::collections::HashMap;

use grid_chart::{DatumId, Ordinal, Segment};
use grid_lang_rs::{deserialize_shape, SerdeField, SerdeShape, SerdeTypeShape};
use grid_wire::{Callsign, CompatFinding, Datum, DefRef, Invariant, InvariantRule, Lang};

use crate::drift::Changed;

/// What the compatibility pass found.
#[derive(Debug, Default)]
pub struct Found {
    /// One finding per (type, rule), sorted by callsign then rule.
    pub findings: Vec<CompatFinding>,
    /// Changed types whose `Deserialize` boundary could not be read at all.
    pub unchecked: u64,
}

/// The six rules, in the order they are decided. The text is the `rule`
/// column, so it is contract: `grid check` will print these words too.
const REQUIRED_FIELD_ADDED: &str = "required field added";
const FIELD_RENAMED: &str = "field renamed without alias";
const TYPE_CHANGED: &str = "type changed";
const VARIANT_REMOVED: &str = "variant removed";
const FIELD_REMOVED_DENIED: &str = "field removed under deny_unknown_fields";
const REPRESENTATION_CHANGED: &str = "representation changed";

/// Where a definition's text is, and which frontend claims it.
struct Located {
    file: String,
    line: u64,
    line_end: u64,
    lang: Option<Lang>,
}

/// Check every changed type's deserializing shape.
///
/// `base_rev` is the rev the base snapshot was surveyed at, so base source
/// comes out of the object store; `head_rev` is `None` for the worktree,
/// which is what drift's everyday question compares against.
pub fn check<A: AsRef<[u8]>, B: AsRef<[u8]>>(
    root: &std::path::Path,
    changed: &[Changed],
    base: &[Segment<A>],
    head: &[Segment<B>],
    base_rev: &str,
    head_rev: Option<&str>,
    invariants: &[Invariant],
) -> Found {
    if changed.is_empty() {
        return Found::default();
    }
    let head_by_callsign = located_by_callsign(head);
    let mut sources = Sources::default();
    let mut found = Found::default();
    for change in changed {
        // Head first, and out of the worktree: it settles whether this is a
        // boundary at all, and it costs a file read rather than a child
        // process. A type that does not deserialize at head reads no bytes
        // there, so none of the six rules can apply to it and there is
        // nothing for a base side to be compared against.
        let after = head_by_callsign
            .get(&change.callsign)
            .and_then(|at| shape(root, &mut sources, head_rev, at));
        let after = match after {
            None | Some(SerdeShape::Absent) => continue,
            Some(SerdeShape::Hidden) => {
                found.unchecked = found.unchecked.saturating_add(1);
                continue;
            }
            Some(SerdeShape::Visible(after)) => after,
        };
        let before = located_by_datum(base, change.datum)
            .and_then(|at| shape(root, &mut sources, Some(base_rev), &at));
        let Some(SerdeShape::Visible(before)) = before else {
            // A boundary at head whose base side this reader cannot state:
            // the derive was hidden there, the source could not be served, or
            // the type did not deserialize at all. None of those is a clean
            // bill of health.
            found.unchecked = found.unchecked.saturating_add(1);
            continue;
        };
        let def = def_ref(change, head_by_callsign.get(&change.callsign));
        let declared = persisted(invariants, &change.callsign);
        found.findings.extend(
            rules(&before, &after)
                .into_iter()
                .map(|rule| CompatFinding {
                    def: def.clone(),
                    rule: rule.to_owned(),
                    declared,
                }),
        );
    }
    found.findings.sort_by(|a, b| {
        let key = |f: &CompatFinding| {
            (
                f.def
                    .callsign
                    .as_ref()
                    .map(|c| c.0.clone())
                    .unwrap_or_default(),
                f.rule.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    found
}

/// The rules old bytes trip on, in decision order.
///
/// Order matters once: a removed field paired with an added one of the same
/// type is a *rename*, and consuming the pair is what stops the same edit
/// from also reading as "required field added" and "field removed".
fn rules(before: &SerdeTypeShape, after: &SerdeTypeShape) -> Vec<&'static str> {
    let mut out = Vec::new();
    if before.representation != after.representation {
        out.push(REPRESENTATION_CHANGED);
    }
    for variant in &before.variants {
        if !after.variants.contains(variant) {
            out.push(VARIANT_REMOVED);
            break;
        }
    }

    let held = |fields: &[SerdeField], name: &str| fields.iter().any(|f| f.name == name);
    let mut removed: Vec<&SerdeField> = before
        .fields
        .iter()
        .filter(|f| !held(&after.fields, &f.name))
        .collect();
    let mut added: Vec<&SerdeField> = after
        .fields
        .iter()
        .filter(|f| !held(&before.fields, &f.name))
        .collect();

    // Renames first, pairing on the declared type: an alias naming the old
    // field is exactly the thing that keeps old bytes readable, so a pair
    // that carries one produces nothing at all.
    let mut renamed = false;
    removed.retain(|gone| {
        let Some(index) = added.iter().position(|new| new.ty == gone.ty) else {
            return true;
        };
        let new = added.remove(index);
        renamed |= !new.aliases.contains(&gone.name);
        false
    });
    if renamed {
        out.push(FIELD_RENAMED);
    }

    if added
        .iter()
        .any(|field| !field.defaulted && !field.optional)
    {
        out.push(REQUIRED_FIELD_ADDED);
    }
    if after.deny_unknown_fields && !removed.is_empty() {
        out.push(FIELD_REMOVED_DENIED);
    }
    if before.fields.iter().any(|old| {
        after
            .fields
            .iter()
            .any(|new| new.name == old.name && new.ty != old.ty)
    }) {
        out.push(TYPE_CHANGED);
    }
    out.sort_unstable();
    out
}

/// Does a standing `persisted` invariant name this type?
fn persisted(invariants: &[Invariant], callsign: &str) -> bool {
    invariants.iter().any(|inv| match &inv.rule {
        InvariantRule::Persisted { persisted } => {
            grid_gate::pattern::matches(&persisted.0, callsign)
        }
        InvariantRule::ForbidEdge { .. } | InvariantRule::RequireEdge { .. } => false,
    })
}

fn def_ref(change: &Changed, at: Option<&Located>) -> DefRef {
    DefRef {
        datum: Datum(change.datum.to_string()),
        callsign: Some(Callsign(change.callsign.clone())),
        lang: at.and_then(|at| at.lang),
        signature: None,
        cfg: None,
        materialization: at.map(|at| grid_wire::DefRefMaterialization {
            file: at.file.clone(),
            line: at.line,
        }),
    }
}

/// Source text, memoized per (rev, file): the changed set names a handful of
/// types, and two of them often share a file.
#[derive(Default)]
struct Sources(HashMap<(Option<String>, String), Option<String>>);

impl Sources {
    /// One file's text at a rev, or from the worktree when `rev` is `None`.
    ///
    /// `ponytail:` one `git show` child per file, not one batch — the compat
    /// pass reads the files of the changed `Deserialize` types, which is a
    /// handful. `grid_survey::files::sources_at` reads the whole tree through
    /// one `cat-file --batch`; that is the right shape for a survey and the
    /// wrong one for two files. Switch if a changed set ever names dozens.
    fn get(&mut self, root: &std::path::Path, rev: Option<&str>, file: &str) -> Option<&str> {
        self.0
            .entry((rev.map(ToOwned::to_owned), file.to_owned()))
            .or_insert_with(|| match rev {
                None => std::fs::read_to_string(root.join(file)).ok(),
                Some(rev) => std::process::Command::new("git")
                    .args(["show", &format!("{rev}:{file}")])
                    .current_dir(root)
                    .output()
                    .ok()
                    .filter(|out| out.status.success())
                    .and_then(|out| String::from_utf8(out.stdout).ok()),
            })
            .as_deref()
    }
}

/// One definition's deserializing shape, read from the source its location
/// names. `None` when the file could not be served at all.
fn shape(
    root: &std::path::Path,
    sources: &mut Sources,
    rev: Option<&str>,
    at: &Located,
) -> Option<SerdeShape> {
    // Rust only in this step. A Python `dataclass` or a pydantic model has no
    // attribute vocabulary this reader knows, so it is not a boundary it can
    // decide — recorded in the plan, not built.
    if at.lang != Some(Lang::Rs) {
        return Some(SerdeShape::Absent);
    }
    let source = sources.get(root, rev, &at.file)?;
    let skip = usize::try_from(at.line.saturating_sub(1)).unwrap_or(usize::MAX);
    let take = usize::try_from(at.line_end.saturating_sub(at.line).saturating_add(1))
        .unwrap_or(usize::MAX);
    let text: String = source
        .lines()
        .skip(skip)
        .take(take)
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return None;
    }
    Some(deserialize_shape(&text))
}

fn located_by_datum<B: AsRef<[u8]>>(segments: &[Segment<B>], datum: DatumId) -> Option<Located> {
    segments
        .iter()
        .find_map(|segment| segment.ordinal(&datum).map(|ord| (segment, ord)))
        .and_then(|(segment, ord)| located(segment, ord))
}

/// Head's definitions by callsign. Built once: a changed type's head-side
/// datum differs from its base-side one by construction, so the name is the
/// only key that crosses the change.
fn located_by_callsign<B: AsRef<[u8]>>(segments: &[Segment<B>]) -> HashMap<String, Located> {
    let mut out = HashMap::new();
    for segment in segments {
        for ord in 0..segment.def_count() {
            let ord = Ordinal(ord);
            if let (Some(callsign), Some(at)) = (segment.callsign(ord), located(segment, ord)) {
                out.insert(callsign.to_owned(), at);
            }
        }
    }
    out
}

fn located<B: AsRef<[u8]>>(segment: &Segment<B>, ord: Ordinal) -> Option<Located> {
    let loc = segment.location(ord)?;
    Some(Located {
        file: loc.path.to_owned(),
        line: u64::from(loc.line),
        line_end: u64::from(loc.line_end),
        lang: segment.lang(ord),
    })
}

#[cfg(test)]
mod tests {
    use super::{rules, SerdeField, SerdeTypeShape};

    fn field(name: &str, ty: &str) -> SerdeField {
        SerdeField {
            name: name.to_owned(),
            ty: ty.to_owned(),
            defaulted: false,
            optional: false,
            aliases: Vec::new(),
        }
    }

    fn shape(fields: Vec<SerdeField>) -> SerdeTypeShape {
        SerdeTypeShape {
            deny_unknown_fields: false,
            representation: String::new(),
            fields,
            variants: Vec::new(),
        }
    }

    /// A rename is one finding, not two: pairing the removed field with the
    /// added one is what stops the same edit reading as an addition *and* a
    /// removal.
    #[test]
    fn a_rename_is_one_finding_and_an_alias_is_none() {
        let before = shape(vec![field("old", "String")]);
        let after = shape(vec![field("new", "String")]);
        assert_eq!(rules(&before, &after), ["field renamed without alias"]);

        let mut aliased = field("new", "String");
        aliased.aliases.push("old".to_owned());
        assert!(
            rules(&before, &shape(vec![aliased])).is_empty(),
            "an alias naming the old field is exactly what keeps old bytes readable"
        );
    }

    /// A field a removal never paired with, under no `deny_unknown_fields`,
    /// is compatible — old bytes simply carry a key the type ignores.
    #[test]
    fn a_removed_field_is_silent_unless_unknown_keys_are_denied() {
        let before = shape(vec![field("a", "u32"), field("b", "String")]);
        let after = shape(vec![field("a", "u32")]);
        assert!(rules(&before, &after).is_empty());

        let mut denied = after;
        denied.deny_unknown_fields = true;
        assert_eq!(
            rules(&before, &denied),
            ["field removed under deny_unknown_fields"]
        );
    }
}
