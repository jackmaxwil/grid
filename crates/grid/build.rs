//! Build-time generation of the `grid explain` corpus.
//!
//! Two sources of truth, one generated table: the wire contract's schema
//! descriptions (`docs/wire-v1.openrpc.json`) and the normative vocabulary
//! (`docs/ONTOLOGY.md`). ONTOLOGY wins a collision — it is the curated
//! answer; the wire description is the fallback for everything else. Nothing
//! about help text is hand-maintained in the binary, so help cannot drift
//! from the contract (implementation plan, Wave 6).

// A build script has no runtime caller to hand a Result to: a missing input
// must stop the build loudly, which is what expect does here (HAR allows it
// in build scripts, tests and benches, and nowhere else).
#![allow(clippy::expect_used)]
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wire = manifest.join("../../docs/wire-v1.openrpc.json");
    let ontology = manifest.join("../../docs/ONTOLOGY.md");
    println!("cargo:rerun-if-changed={}", wire.display());
    println!("cargo:rerun-if-changed={}", ontology.display());

    let mut entries: BTreeMap<String, String> = BTreeMap::new();

    // Wire schema descriptions, keyed by the lowercased schema name.
    let spec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&wire).expect("wire contract is readable"))
            .expect("wire contract parses");
    if let Some(schemas) = spec["components"]["schemas"].as_object() {
        for (name, schema) in schemas {
            if let Some(description) = schema["description"].as_str() {
                entries.insert(name.to_lowercase(), one_line(description));
            }
        }
    }

    // Ontology table rows: `| `token` | the answer |`. Overrides win. The
    // `## Former names` table is `| old | new |` instead — an alias, not a
    // definition — so it is collected separately and never enters the corpus.
    let mut former: BTreeMap<String, String> = BTreeMap::new();
    let mut in_former = false;
    for line in std::fs::read_to_string(&ontology)
        .expect("ONTOLOGY.md is readable")
        .lines()
    {
        if let Some(heading) = line.strip_prefix("## ") {
            in_former = heading.trim() == "Former names";
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // split('|') on "| a | b |" yields ["", "a", "b", ""].
        let [empty, token, meaning, ..] = cells.as_slice() else {
            continue;
        };
        if !empty.is_empty() {
            continue;
        }
        let Some(token) = token.strip_prefix('`').and_then(|t| t.strip_suffix('`')) else {
            continue;
        };
        if token == "token" || token == "old" || token.starts_with('-') {
            continue;
        }
        if in_former {
            let now = meaning
                .strip_prefix('`')
                .and_then(|m| m.strip_suffix('`'))
                .expect("a former-names row names the new token in backticks");
            former.insert((*token).to_string(), now.to_string());
        } else {
            entries.insert((*token).to_string(), one_line(meaning));
        }
    }
    for (old, now) in &former {
        assert!(
            entries.contains_key(now),
            "former name `{old}` points at `{now}`, which ONTOLOGY.md does not define"
        );
    }

    let mut generated = String::from(
        "/// The explain corpus: token -> answer, sorted by token. GENERATED.\n\
         pub static EXPLAIN: &[(&str, &str)] = &[\n",
    );
    for (token, meaning) in &entries {
        writeln!(generated, "    ({token:?}, {meaning:?}),").expect("write to string");
    }
    generated.push_str("];\n");
    generated.push_str(
        "/// Retired words: old token -> the token that replaced it, sorted by\n\
         /// old token. GENERATED from ONTOLOGY.md's `## Former names` table.\n\
         pub static FORMER: &[(&str, &str)] = &[\n",
    );
    for (old, now) in &former {
        writeln!(generated, "    ({old:?}, {now:?}),").expect("write to string");
    }
    generated.push_str("];\n");
    std::fs::write(
        Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("explain.rs"),
        generated,
    )
    .expect("write generated corpus");
}

/// One line: whitespace runs collapse, so a description printed is at most a
/// screenful, never a page.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
