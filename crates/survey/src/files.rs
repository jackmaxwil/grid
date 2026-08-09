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

/// The language an extension names, or `None` for a file no adapter claims.
///
/// Only Python is claimed at this wave. The others are named so the mapping
/// lives in one place when their adapters arrive.
#[must_use]
pub fn lang_of(path: &str) -> Option<Lang> {
    match path.rsplit_once('.')?.1 {
        "py" | "pyi" => Some(Lang::Py),
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
pub fn sources(root: &Path) -> Result<Vec<SourceFile>, SurveyError> {
    let mut out = Vec::new();
    for rel in tracked(root)? {
        let path = rel.to_string_lossy().replace('\\', "/");
        let Some(lang) = lang_of(&path) else { continue };
        let Ok(bytes) = std::fs::read(root.join(&rel)) else {
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
    // git's order is already stable, but sorting here makes the survey
    // independent of the git version that produced the list.
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// The module path a file materializes: `pkg/mod.py` -> `pkg.mod`, and
/// `pkg/__init__.py` -> `pkg`.
///
/// This is the one place a path becomes a name, so it is the one place that
/// has to agree with what an `import` statement spells.
#[must_use]
pub fn module_of(path: &str) -> String {
    let stem = path
        .strip_suffix(".py")
        .or_else(|| path.strip_suffix(".pyi"))
        .unwrap_or(path);
    let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
    stem.replace('/', ".")
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
pub fn sources_at(root: &Path, rev: &str) -> Result<Vec<SourceFile>, SurveyError> {
    let listing = git(root, &["ls-tree", "-r", "-z", "--name-only", rev])?;
    let mut out = Vec::new();
    for rel in listing.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8_lossy(rel).replace('\\', "/");
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
    Ok(out)
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
