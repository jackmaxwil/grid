//! Walking a tree and reading the files a survey cares about.
//!
//! Discovery is deliberately dumb and deliberately total: it is `git
//! ls-files`, so the repo's own ignore rules decide what is source and nothing
//! in this crate has an opinion. A file grid cannot read is skipped, not
//! fatal — a survey that dies on one unreadable path is a survey that never
//! finishes on a real machine.
use std::path::{Path, PathBuf};
use std::process::Command;

use grid_wire::Lang;

use crate::SurveyError;

/// One source file, as the survey sees it: where it is, what language it is,
/// and the hash of what it contained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Path relative to the repo root, slash-separated. This is the stable
    /// name: an absolute path would put the machine into the snapshot id.
    pub path: String,
    /// The adapter that claims it.
    pub lang: Lang,
    /// blake3 of the file's bytes. The incremental key, and the only thing
    /// consulted to decide a file is unchanged.
    pub content_hash: [u8; 32],
    /// The file's text.
    pub source: String,
}

/// Directory -> crate-name bindings from every tracked `Cargo.toml`'s
/// `[package] name`, longest directory first. This is what lets a package
/// named `grid-chart` in a directory named `chart` file its modules under
/// `grid_chart`, which is the spelling a `use` statement reaches for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrateRoots(Vec<(String, String)>);

impl CrateRoots {
    fn from_manifests(mut entries: Vec<(String, String)>) -> Self {
        // Longest prefix first so a nested crate shadows its workspace; the
        // name tiebreak keeps the map a pure function of its inputs.
        entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        Self(entries)
    }

    /// The nearest ancestor manifest of `path`: its directory and crate name.
    fn nearest(&self, path: &str) -> Option<(&str, &str)> {
        self.0
            .iter()
            .find(|(dir, _)| {
                dir.is_empty()
                    || path
                        .strip_prefix(dir.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .map(|(dir, name)| (dir.as_str(), name.as_str()))
    }
}

/// The `[package] name` a manifest declares, spelled as the crate name a
/// `use` reaches for (hyphens underscored).
///
/// ponytail: a line scan, not a TOML parser — a name split across lines or
/// inherited from the workspace falls back to the directory name, which
/// costs an unresolved edge and never a false one.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = line.strip_prefix("name") {
                let rest = rest.trim_start();
                if let Some(value) = rest.strip_prefix('=') {
                    let value = value.trim().trim_matches('"');
                    if !value.is_empty() {
                        return Some(value.replace('-', "_"));
                    }
                }
            }
        }
    }
    None
}

fn manifest_dir(path: &str) -> Option<&str> {
    if path == "Cargo.toml" {
        Some("")
    } else {
        path.strip_suffix("/Cargo.toml")
    }
}

/// The crate-roots map alone, for callers that already have their sources
/// (the edit write path). Costs its own `git ls-files`.
///
/// # Errors
/// [`SurveyError::Git`] if the file list cannot be obtained.
pub fn crate_roots(root: &Path) -> Result<CrateRoots, SurveyError> {
    let mut entries = Vec::new();
    for rel in tracked(root)? {
        let path = rel.to_string_lossy().replace('\\', "/");
        let Some(dir) = manifest_dir(&path) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(root.join(&rel)) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if let Some(name) = package_name(&text) {
            entries.push((dir.to_owned(), name));
        }
    }
    Ok(CrateRoots::from_manifests(entries))
}

/// The language an extension names, or `None` for a file no frontend claims.
#[must_use]
pub fn lang_of(path: &str) -> Option<Lang> {
    match path.rsplit_once('.')?.1 {
        "py" | "pyi" => Some(Lang::Py),
        "rs" => Some(Lang::Rs),
        _ => None,
    }
}

/// Tracked files under `root`, in git's order.
///
/// # Errors
/// [`SurveyError::Git`] if `root` is not a git work tree or git fails.
pub fn tracked(root: &Path) -> Result<Vec<PathBuf>, SurveyError> {
    // `-z` because a path is bytes and may contain a newline; git only quotes
    // paths in the non-`-z` form, so this is the parse that cannot be fooled.
    Ok(git(root, &["ls-files", "-z"])?
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
        .collect())
}

/// Read every tracked file an adapter claims.
///
/// Unreadable and non-UTF-8 files are skipped: they are not Python the parser
/// could have accepted anyway, and a survey must survive a repo containing a
/// stray broken symlink.
///
/// # Errors
/// [`SurveyError::Git`] if the file list cannot be obtained.
pub fn sources(root: &Path) -> Result<(Vec<SourceFile>, CrateRoots), SurveyError> {
    let mut manifests = Vec::new();
    let claimed: Vec<(PathBuf, String, grid_wire::Lang)> = tracked(root)?
        .into_iter()
        .filter_map(|rel| {
            let path = rel.to_string_lossy().replace('\\', "/");
            if let Some(dir) = manifest_dir(&path) {
                let name = std::fs::read(root.join(&rel))
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .as_deref()
                    .and_then(package_name);
                if let Some(name) = name {
                    manifests.push((dir.to_owned(), name));
                }
            }
            let lang = lang_of(&path)?;
            Some((rel, path, lang))
        })
        .collect();
    // Reads are pure per-path, so they fan out on scoped threads; the sort
    // below erases completion order the same way it erases git's order.
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let chunk = claimed.len().div_ceil(threads.max(1)).max(1);
    let mut out: Vec<SourceFile> = std::thread::scope(|scope| {
        let handles: Vec<_> = claimed
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|(rel, path, lang)| {
                            let bytes = std::fs::read(root.join(rel)).ok()?;
                            let source = String::from_utf8(bytes).ok()?;
                            Some(SourceFile {
                                content_hash: *blake3::hash(source.as_bytes()).as_bytes(),
                                path: path.clone(),
                                lang: *lang,
                                source,
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .flatten()
            .collect()
    });
    // git's order is already stable, but sorting here makes the survey
    // independent of the git version that produced the list.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((out, CrateRoots::from_manifests(manifests)))
}

/// The module path a file materializes. Python: `pkg/mod.py` -> `pkg.mod`,
/// `pkg/__init__.py` -> `pkg`. Rust: `crates/chart/src/lib.rs` -> `chart`,
/// `crates/survey/src/resolve.rs` -> `survey.resolve`.
///
/// This is the one place a path becomes a name, so it is the one place that
/// has to agree with what an import statement spells. The Rust root segment
/// is the nearest manifest's `[package] name`; a path no manifest claims
/// keeps the dir-derived approximation — ceilings in `docs/LANG_RS.md`.
#[must_use]
pub fn module_of(lang: Lang, path: &str, roots: &CrateRoots) -> String {
    if lang == Lang::Rs {
        module_of_rs(path, roots)
    } else {
        let stem = path
            .strip_suffix(".py")
            .or_else(|| path.strip_suffix(".pyi"))
            .unwrap_or(path);
        let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
        stem.replace('/', ".")
    }
}

fn module_of_rs(path: &str, roots: &CrateRoots) -> String {
    let (root, rest) = if let Some((dir, name)) = roots.nearest(path) {
        let tail = path.get(dir.len()..).unwrap_or("").trim_start_matches('/');
        (name.to_owned(), tail.strip_prefix("src/").unwrap_or(tail))
    } else if let Some(rest) = path.strip_prefix("src/") {
        ("crate".to_owned(), rest)
    } else if let Some((pre, rest)) = path.split_once("/src/") {
        // No manifest claims this path: the directory is the best guess,
        // with the hyphen swap the real crate name would have had.
        (
            pre.rsplit('/').next().unwrap_or("crate").replace('-', "_"),
            rest,
        )
    } else {
        // build.rs, tests/, examples/ — approximate names, never false edges.
        ("crate".to_owned(), path)
    };
    let stem = rest.strip_suffix(".rs").unwrap_or(rest);
    let stem = stem.strip_suffix("/mod").unwrap_or(stem);
    if stem == "lib" || stem == "main" {
        root
    } else {
        format!("{root}.{}", stem.replace('/', "."))
    }
}

/// Tracked files at a git revision, with their contents.
///
/// Surveying "at a rev" has to mean *the tree at that rev*, not the working
/// tree wearing a label. The gate compares two snapshots, and if both were
/// really the working tree the comparison would be empty no matter what
/// changed — so this reads blobs out of the object store rather than off disk.
///
/// # Errors
/// [`SurveyError::Git`] if the rev does not resolve or git fails.
pub fn sources_at(root: &Path, rev: &str) -> Result<(Vec<SourceFile>, CrateRoots), SurveyError> {
    let listing = git(root, &["ls-tree", "-r", "-z", "--name-only", rev])?;
    let mut out = Vec::new();
    let mut manifests = Vec::new();
    for rel in listing.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8_lossy(rel).replace('\\', "/");
        if let Some(dir) = manifest_dir(&path) {
            // Manifests come from the same rev as the sources they name:
            // roots are part of the state being surveyed, not of the worktree.
            let name = git(root, &["show", &format!("{rev}:{path}")])
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .as_deref()
                .and_then(package_name);
            if let Some(name) = name {
                manifests.push((dir.to_owned(), name));
            }
        }
        let Some(lang) = lang_of(&path) else { continue };
        // A blob that is not UTF-8 is not source an adapter could parse.
        let Ok(bytes) = git(root, &["show", &format!("{rev}:{path}")]) else {
            continue;
        };
        let Ok(source) = String::from_utf8(bytes) else {
            continue;
        };
        out.push(SourceFile {
            content_hash: *blake3::hash(source.as_bytes()).as_bytes(),
            path,
            lang,
            source,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((out, CrateRoots::from_manifests(manifests)))
}

/// Run git in `root` and return its stdout, or the failure it reported.
fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, SurveyError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| SurveyError::Git(e.to_string()))?;
    if !out.status.success() {
        return Err(SurveyError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(out.stdout)
}
