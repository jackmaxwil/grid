//! Determinism smoke over a real corpus: hashing a whole tree twice must
//! yield identical sets. Cortana is the plan's named corpus, but grid is its
//! own repo, so the tree is named by `GRID_CORPUS` (CI points it at a
//! checkout); without it the test is a no-op rather than a false green on an
//! empty tree.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use grid_datum::{definitions, normalize};

fn python_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !matches!(
                path.file_name().and_then(|n| n.to_str()),
                Some(".git" | ".venv" | "__pycache__" | "node_modules" | "target")
            ) {
                python_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "py") {
            out.push(path);
        }
    }
}

fn hash_tree(files: &[PathBuf]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for path in files {
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(module) = normalize(&source) else {
            continue; // syntax we cannot parse is not a determinism question
        };
        out.insert(path.display().to_string(), module.datum.to_string());
        for def in definitions(&source).unwrap_or_default() {
            out.insert(
                format!("{}::{}", path.display(), def.callsign),
                format!("{}/{}", def.datum, def.span_hash),
            );
        }
    }
    out
}

#[test]
fn hashing_a_real_tree_twice_is_identical() {
    let Ok(root) = std::env::var("GRID_CORPUS") else {
        eprintln!("GRID_CORPUS unset; determinism smoke skipped");
        return;
    };
    let mut files = Vec::new();
    python_files(Path::new(&root), &mut files);
    assert!(!files.is_empty(), "GRID_CORPUS={root} has no Python files");
    files.sort();
    assert_eq!(hash_tree(&files), hash_tree(&files));
}
