//! What one item promises about bytes an older version of itself wrote.
//!
//! Inside one repository both ends of a serde boundary share the struct and
//! the compiler forces them to move together. The case it cannot see is *old
//! bytes*: a config file, a cache row, a queued message written by the
//! previous snapshot. Reading that promise needs the field list with its
//! serde attributes, on both sides of the change — which is one `syn` parse
//! of one item, and lives here because this is the crate that owns what Rust
//! source means.
//!
//! Everything here is decidable or refuses. A derive the source does not
//! spell out — hidden behind `cfg_attr`, or produced by a macro whose
//! expansion the survey never sees — is [`SerdeShape::Hidden`], never
//! "compatible".
use quote::ToTokens as _;

/// What an item says about `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerdeShape {
    /// A `struct` or `enum` whose `Deserialize` derive is written out, with
    /// everything old bytes care about read off it.
    Visible(Shape),
    /// `Deserialize` is mentioned somewhere this reader cannot follow — a
    /// `cfg_attr`, a serde attribute that does not parse, or text that is
    /// not one item. Counted, never compared.
    Hidden,
    /// Nothing to do with `Deserialize`.
    Absent,
}

/// The deserializing shape of one type: the container's representation and
/// the members old bytes could name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    /// `#[serde(deny_unknown_fields)]`, which turns a removed field from a
    /// non-event into an error on old bytes.
    pub deny_unknown_fields: bool,
    /// The container attributes that change how bytes map onto the type at
    /// all — `rename_all`, `tag`, `content`, `untagged` — canonically joined
    /// so two spellings of the same representation compare equal.
    pub representation: String,
    /// Struct fields, in declaration order.
    pub fields: Vec<Field>,
    /// Enum variant names, after `rename`, in declaration order.
    pub variants: Vec<String>,
}

/// One field, as the bytes see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The name in the bytes: `#[serde(rename)]` when given, else the Rust
    /// name. Container-level `rename_all` is not applied — it is compared as
    /// part of `representation` instead, so a `rename_all` change is one
    /// finding rather than one per field.
    pub name: String,
    /// The declared type, canonically printed.
    pub ty: String,
    /// `default`, `default = "…"`, `skip_deserializing` or `skip`: bytes
    /// that lack this field still deserialize.
    pub defaulted: bool,
    /// The type is an `Option<…>`, which serde fills with `None` when the
    /// bytes are silent.
    pub optional: bool,
    /// `#[serde(alias)]` names old bytes may still use.
    pub aliases: Vec<String>,
}

/// Read one item's deserializing shape from its source text.
///
/// The text is the definition's own span, attributes included, which is what
/// the chart records a location for.
#[must_use]
pub fn deserialize_shape(item_source: &str) -> SerdeShape {
    // Every answer but `Absent` needs the word somewhere in the text: a
    // derive is spelled out, a `cfg_attr` is read as text, and an item that
    // does not parse is only interesting when it names the derive. One
    // substring scan therefore saves one `syn` parse for every definition
    // that is not a serde boundary at all, which is nearly all of them.
    if !item_source.contains("Deserialize") {
        return SerdeShape::Absent;
    }
    let Ok(item) = syn::parse_str::<syn::Item>(item_source) else {
        // Not one item: a macro body, or a span that starts mid-line. The
        // text mentions the derive (the guard above), so that is a boundary
        // this reader cannot check rather than one it can clear.
        return SerdeShape::Hidden;
    };
    match item {
        syn::Item::Struct(item) => of_struct(&item),
        syn::Item::Enum(item) => of_enum(&item),
        other => {
            if other.to_token_stream().to_string().contains("Deserialize") {
                SerdeShape::Hidden
            } else {
                SerdeShape::Absent
            }
        }
    }
}

fn of_struct(item: &syn::ItemStruct) -> SerdeShape {
    match visibility(&item.attrs) {
        Visibility::Absent => return SerdeShape::Absent,
        Visibility::Hidden => return SerdeShape::Hidden,
        Visibility::Derived => {}
    }
    let Some(container) = container(&item.attrs) else {
        return SerdeShape::Hidden;
    };
    let mut fields = Vec::new();
    for field in &item.fields {
        // A tuple struct's members are positional; old bytes name them by
        // index, and an index cannot be renamed or aliased, so there is
        // nothing here for these rules to decide.
        let Some(ident) = field.ident.as_ref() else {
            continue;
        };
        let Some(mut read) = field_of(ident, field) else {
            return SerdeShape::Hidden;
        };
        // A container-level `#[serde(default)]` defaults every field: an
        // added field on such a struct is not a required one.
        read.defaulted |= container.defaulted;
        fields.push(read);
    }
    SerdeShape::Visible(Shape {
        deny_unknown_fields: container.deny_unknown_fields,
        representation: container.representation,
        fields,
        variants: Vec::new(),
    })
}

/// `ponytail:` an enum is read as its variant list only — the rules that
/// fire on it are "variant removed" and "representation changed". Per-variant
/// fields would want the struct rules applied one level down; that lands when
/// a real enum boundary asks for it.
fn of_enum(item: &syn::ItemEnum) -> SerdeShape {
    match visibility(&item.attrs) {
        Visibility::Absent => return SerdeShape::Absent,
        Visibility::Hidden => return SerdeShape::Hidden,
        Visibility::Derived => {}
    }
    let Some(container) = container(&item.attrs) else {
        return SerdeShape::Hidden;
    };
    let mut variants = Vec::new();
    for variant in &item.variants {
        let Some(meta) = meta(&variant.attrs) else {
            return SerdeShape::Hidden;
        };
        variants.push(named(&meta, &variant.ident.to_string()));
    }
    SerdeShape::Visible(Shape {
        deny_unknown_fields: container.deny_unknown_fields,
        representation: container.representation,
        fields: Vec::new(),
        variants,
    })
}

/// Whether this item's `Deserialize` derive is one the source spells out.
enum Visibility {
    Derived,
    Hidden,
    Absent,
}

fn visibility(attrs: &[syn::Attribute]) -> Visibility {
    for attr in attrs {
        if attr.path().is_ident("derive")
            && attr
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .is_ok_and(|paths| {
                    paths
                        .iter()
                        .any(|p| p.segments.last().is_some_and(|s| s.ident == "Deserialize"))
                })
        {
            return Visibility::Derived;
        }
        // `#[cfg_attr(feature = "serde", derive(Deserialize))]`: whether the
        // derive is there at all depends on a feature the chart does not
        // record, so neither answer is available.
        if attr.path().is_ident("cfg_attr")
            && attr.to_token_stream().to_string().contains("Deserialize")
        {
            return Visibility::Hidden;
        }
    }
    Visibility::Absent
}

struct Container {
    deny_unknown_fields: bool,
    /// `#[serde(default)]` on the container: every field is defaulted.
    defaulted: bool,
    representation: String,
}

/// The container attributes, or `None` when a `#[serde(...)]` on the item did
/// not parse — an unparsed attribute is an unknown contract, not an empty one.
fn container(attrs: &[syn::Attribute]) -> Option<Container> {
    let meta = meta(attrs)?;
    let mut parts: Vec<String> = meta
        .iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "rename_all" | "rename_all_fields" | "tag" | "content" | "untagged"
            )
        })
        .map(|(key, value)| match value {
            Some(value) => format!("{key}={value}"),
            None => key.clone(),
        })
        .collect();
    // Sorted, because `#[serde(tag = "k", rename_all = "snake_case")]` and
    // the same two written the other way round are the same representation.
    parts.sort();
    Some(Container {
        deny_unknown_fields: meta.iter().any(|(key, _)| key == "deny_unknown_fields"),
        defaulted: meta.iter().any(|(key, _)| key == "default"),
        representation: parts.join(";"),
    })
}

/// One named field, or `None` for one this reader cannot describe: an
/// attribute that did not parse, or `#[serde(flatten)]`, whose name never
/// appears in the bytes and whose members live on another type — reading
/// it as a plain field named `inner` would report a rename that never
/// happened and hide every change inside the flattened type.
fn field_of(ident: &syn::Ident, field: &syn::Field) -> Option<Field> {
    let meta = meta(&field.attrs)?;
    if meta.iter().any(|(key, _)| key == "flatten") {
        return None;
    }
    let ty = field.ty.to_token_stream().to_string();
    Some(Field {
        name: named(&meta, &ident.to_string()),
        optional: matches!(&field.ty, syn::Type::Path(path)
            if path.path.segments.last().is_some_and(|s| s.ident == "Option")),
        ty,
        defaulted: meta
            .iter()
            .any(|(key, _)| matches!(key.as_str(), "default" | "skip_deserializing" | "skip")),
        aliases: meta
            .iter()
            .filter(|(key, _)| key == "alias")
            .filter_map(|(_, value)| value.clone())
            .collect(),
    })
}

/// The name in the bytes: `rename` when given, else the Rust name.
fn named(meta: &[(String, Option<String>)], rust: &str) -> String {
    meta.iter()
        .find(|(key, value)| key == "rename" && value.is_some())
        .and_then(|(_, value)| value.clone())
        .unwrap_or_else(|| rust.to_owned())
}

/// Every `#[serde(...)]` key on a node, with its string value when it has
/// one. `None` when an attribute did not parse: this reader refuses rather
/// than reading a partial contract.
fn meta(attrs: &[syn::Attribute]) -> Option<Vec<(String, Option<String>)>> {
    let mut out = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        attr.parse_nested_meta(|nested| {
            let key = nested
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            let value = nested
                .value()
                .ok()
                .and_then(|stream| stream.parse::<syn::LitStr>().ok())
                .map(|lit| lit.value());
            out.push((key, value));
            Ok(())
        })
        .ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{deserialize_shape, SerdeShape};

    fn shape(source: &str) -> super::Shape {
        match deserialize_shape(source) {
            SerdeShape::Visible(shape) => shape,
            other => panic!("expected a visible shape, got {other:?}"),
        }
    }

    #[test]
    fn a_written_out_derive_is_read_and_a_cfg_attr_one_is_not() {
        let visible = shape("#[derive(Deserialize)]\npub struct C { pub a: u32 }\n");
        assert_eq!(visible.fields.len(), 1);
        assert_eq!(visible.fields[0].name, "a");
        assert!(!visible.fields[0].defaulted && !visible.fields[0].optional);

        assert_eq!(
            deserialize_shape(
                "#[cfg_attr(feature = \"serde\", derive(Deserialize))]\npub struct C { pub a: u32 }\n"
            ),
            SerdeShape::Hidden,
            "a derive behind a cfg is a boundary that cannot be checked"
        );
        assert_eq!(
            deserialize_shape("pub struct C { pub a: u32 }\n"),
            SerdeShape::Absent,
            "and a type that deserializes nothing is not a boundary at all"
        );
        assert_eq!(
            deserialize_shape("declare! { struct C { a: u32 } derive Deserialize }"),
            SerdeShape::Hidden,
            "text that is not one item, but names the derive, is unchecked"
        );
    }

    #[test]
    fn serde_attributes_are_read_where_old_bytes_would_read_them() {
        let read = shape(
            "#[derive(Deserialize)]\n\
             #[serde(deny_unknown_fields, rename_all = \"camelCase\")]\n\
             pub struct C {\n\
             #[serde(rename = \"n\", alias = \"old_n\")] pub name: String,\n\
             #[serde(default)] pub retries: u32,\n\
             pub maybe: Option<u8>,\n\
             }\n",
        );
        assert!(read.deny_unknown_fields);
        assert_eq!(read.representation, "rename_all=camelCase");
        assert_eq!(read.fields[0].name, "n", "rename is the name in the bytes");
        assert_eq!(read.fields[0].aliases, ["old_n"]);
        assert!(read.fields[1].defaulted);
        assert!(read.fields[2].optional);
    }

    /// A container-level `default` defaults every field, and a flattened
    /// field is a boundary this reader cannot describe: its name is never in
    /// the bytes and its members are another type's.
    #[test]
    fn a_container_default_reaches_every_field_and_flatten_is_hidden() {
        let read = shape(
            "#[derive(Deserialize)]\n#[serde(default)]\n\
             pub struct C { pub a: u32, pub b: String }\n",
        );
        assert!(
            read.fields.iter().all(|f| f.defaulted),
            "every field is defaulted by the container: {:?}",
            read.fields.iter().map(|f| f.defaulted).collect::<Vec<_>>()
        );
        assert_eq!(
            deserialize_shape(
                "#[derive(Deserialize)]\npub struct C { pub a: u32, #[serde(flatten)] pub inner: Inner }\n"
            ),
            SerdeShape::Hidden,
            "a flattened field is not a field named `inner`"
        );
    }

    #[test]
    fn a_serde_attribute_that_does_not_parse_refuses() {
        assert_eq!(
            deserialize_shape(
                "#[derive(Deserialize)]\n\
                 pub struct C { #[serde(with(bad))] pub a: u32 }\n"
            ),
            SerdeShape::Hidden,
            "an attribute this reader cannot parse is an unknown contract"
        );
    }
}
