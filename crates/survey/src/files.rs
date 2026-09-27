//! Walking a tree and reading the files a survey cares about.
//!
//! Discovery is deliberately dumb and deliberately total: it is `git
//! ls-files`, so the repo's own ignore rules decide what is source and nothing
//! in this crate has an opinion. A file grid cannot read is kept and marked,
//! not fatal — a survey that dies on one unreadable path is a survey that
//! never finishes on a real machine, and one that drops it charts a file
//! with nothing in it (verb sweep B6).
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
    /// The file's text; empty when [`SourceFile::unreadable`].
    pub source: String,
    /// The bytes could not be read, or are not UTF-8: no adapter could have
    /// parsed them, and the survey names the file rather than losing it.
    pub unreadable: bool,
}

/// One discovered file from its raw bytes: text when they are UTF-8, and a
/// named, empty, unreadable entry when they are not. The hash is of the
/// bytes either way, so the cache key does not depend on the outcome.
fn source_file(path: String, lang: Lang, bytes: Option<Vec<u8>>) -> SourceFile {
    let content_hash = bytes
        .as_deref()
        .map_or([0; 32], |b| *blake3::hash(b).as_bytes());
    let (source, unreadable) = match bytes.map(String::from_utf8) {
        Some(Ok(text)) => (text, false),
        _ => (String::new(), true),
    };
    SourceFile {
        path,
        lang,
        content_hash,
        source,
        unreadable,
    }
}

/// Directory -> crate-name bindings from every tracked `Cargo.toml`'s
/// `[package] name`, longest directory first. This is what lets a package
/// named `grid-chart` in a directory named `chart` file its modules under
/// `grid_chart`, which is the spelling a `use` statement reaches for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrateRoots {
    rs: Vec<(String, String)>,
    /// Directories holding a `pyproject.toml`: Python package roots. A file
    /// under one charts relative to it (an `src/` layout segment stripped),
    /// which is the spelling an import reaches for.
    py: Vec<String>,
}

impl CrateRoots {
    fn from_manifests(mut entries: Vec<(String, String)>, mut py: Vec<String>) -> Self {
        // Longest prefix first so a nested crate shadows its workspace; the
        // name tiebreak keeps the map a pure function of its inputs.
        entries.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        py.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        Self { rs: entries, py }
    }

    /// The nearest ancestor Python package root of `path`, if any.
    fn nearest_py(&self, path: &str) -> Option<&str> {
        self.py.iter().map(String::as_str).find(|dir| {
            dir.is_empty()
                || path
                    .strip_prefix(*dir)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    /// Every `(directory, import name)` binding, longest directory first —
    /// the map `grid roots` prints.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.rs
            .iter()
            .map(|(dir, name)| (dir.as_str(), name.as_str()))
    }

    /// The nearest ancestor manifest of `path`: its directory and crate name.
    fn nearest(&self, path: &str) -> Option<(&str, &str)> {
        self.rs
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

/// The crate name a `use` reaches for (hyphens underscored): `[lib] name`
/// when the manifest renames the library, else `[package] name`.
///
/// ponytail: a line scan, not a TOML parser — a name split across lines or
/// inherited from the workspace falls back to the directory name, which
/// costs an unresolved edge and never a false one.
fn package_name(manifest: &str) -> Option<String> {
    let mut section = "";
    let mut package = None;
    let mut lib = None;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if section != "[package]" && section != "[lib]" {
            continue;
        }
        if let Some(rest) = line.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim().trim_matches('"');
                if !value.is_empty() {
                    let name = value.replace('-', "_");
                    if section == "[lib]" {
                        lib = Some(name);
                    } else {
                        package = Some(name);
                    }
                }
            }
        }
    }
    lib.or(package)
}

fn pyproject_dir(path: &str) -> Option<&str> {
    if path == "pyproject.toml" {
        Some("")
    } else {
        path.strip_suffix("/pyproject.toml")
    }
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
    let mut py = Vec::new();
    for rel in tracked(root)? {
        let path = rel.to_string_lossy().replace('\\', "/");
        if let Some(dir) = pyproject_dir(&path) {
            py.push(dir.to_owned());
        }
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
    Ok(CrateRoots::from_manifests(entries, py))
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

/// Tracked files under `root` minus what the ignore rules match, in git's
/// order. A tracked-but-ignored path (committed vendor code under a later
/// `.gitignore` line) is filtered out: the rules say not-source, and being
/// tracked must not override that.
///
/// Two `git ls-files` children, spawned together: the listing, and the
/// subset of it the standard exclude rules match (`-ci --exclude-standard`).
/// The second used to be `check-ignore --no-index --stdin` fed the first's
/// output, which put two spawns in series; on a machine whose `git` is the
/// Xcode shim each spawn is 9ms before git reads a byte, and the pair ran
/// 24ms against 12ms side by side.
///
/// # Errors
/// [`SurveyError::Git`] if `root` is not a git work tree or git fails.
pub fn tracked(root: &Path) -> Result<Vec<PathBuf>, SurveyError> {
    let (all, skip) = std::thread::scope(|scope| {
        let skip = scope.spawn(|| git(root, &["ls-files", "-z", "-ci", "--exclude-standard"]));
        let all = git(root, &["ls-files", "-z"]);
        let skip = skip
            .join()
            .unwrap_or_else(|_| git(root, &["ls-files", "-z", "-ci", "--exclude-standard"]));
        (all, skip)
    });
    let skip: std::collections::HashSet<String> = paths_z(&skip?).collect();
    Ok(paths_z(&all?)
        .filter(|p| !skip.contains(p))
        .map(PathBuf::from)
        .collect())
}

/// The paths in one NUL-separated git listing. `-z` because a path is bytes
/// and may contain a newline; git only quotes paths in the non-`-z` form,
/// so this is the parse that cannot be fooled.
fn paths_z(listing: &[u8]) -> impl Iterator<Item = String> + '_ {
    listing
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
}

/// The subset of `paths` the worktree's ignore rules match. `--no-index` so
/// tracking a path does not exempt it — that is the whole question here.
fn ignored(
    root: &Path,
    paths: &[String],
) -> Result<std::collections::HashSet<String>, SurveyError> {
    if paths.is_empty() {
        return Ok(std::collections::HashSet::new());
    }
    let as_git = |e: std::io::Error| SurveyError::Git(e.to_string());
    let mut child = Command::new("git")
        .args(["check-ignore", "--no-index", "--stdin", "-z"])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(as_git)?;
    let mut buf = Vec::new();
    for path in paths {
        buf.extend_from_slice(path.as_bytes());
        buf.push(0);
    }
    // Feed stdin on its own thread: with both pipes on one thread, a long
    // match list fills the stdout pipe while this side blocks on stdin.
    let stdin = child.stdin.take();
    let out = std::thread::scope(|scope| {
        scope.spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = std::io::Write::write_all(&mut stdin, &buf);
            }
        });
        child.wait_with_output()
    })
    .map_err(as_git)?;
    // check-ignore exits 1 for "nothing matched" — an answer, not a failure.
    match out.status.code() {
        Some(0 | 1) => Ok(out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect()),
        _ => Err(SurveyError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        )),
    }
}

/// Re-read one already-discovered file, in place.
///
/// The discovery half of [`sources`] — `git ls-files` and the crate-root scan
/// — does not change when a file's *contents* do, so a caller holding a file
/// list can refresh one entry and keep the rest. Returns `false` when the
/// path is not in the list, which is the caller's cue that discovery has to
/// run again: a new file is a new tracked path, not a changed one.
#[must_use]
pub fn reread(root: &Path, sources: &mut [SourceFile], path: &str) -> bool {
    let Some(slot) = sources.iter_mut().find(|s| s.path == path) else {
        return false;
    };
    let Ok(bytes) = std::fs::read(root.join(path)) else {
        // A file that vanished is a discovery change, not a content one.
        return false;
    };
    *slot = source_file(std::mem::take(&mut slot.path), slot.lang, Some(bytes));
    true
}

/// Read every tracked file an adapter claims.
///
/// Unreadable and non-UTF-8 files are kept as [`SourceFile::unreadable`]:
/// no parser could have accepted them, a survey must survive a repo holding
/// a stray broken symlink, and the survey must still say the file is there.
///
/// # Errors
/// [`SurveyError::Git`] if the file list cannot be obtained.
pub fn sources(root: &Path) -> Result<(Vec<SourceFile>, CrateRoots), SurveyError> {
    let mut manifests = Vec::new();
    let mut py_roots = Vec::new();
    let claimed: Vec<(PathBuf, String, grid_wire::Lang)> = tracked(root)?
        .into_iter()
        .filter_map(|rel| {
            let path = rel.to_string_lossy().replace('\\', "/");
            if let Some(dir) = pyproject_dir(&path) {
                py_roots.push(dir.to_owned());
            }
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
                        .map(|(rel, path, lang)| {
                            source_file(path.clone(), *lang, std::fs::read(root.join(rel)).ok())
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
    Ok((out, CrateRoots::from_manifests(manifests, py_roots)))
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
        // Inside a Python package root the import spelling starts at the
        // package, so the root directory (and an `src/` layout segment)
        // are not module path.
        let path = roots.nearest_py(path).map_or(path, |dir| {
            let rest = path.get(dir.len()..).unwrap_or("").trim_start_matches('/');
            rest.strip_prefix("src/").unwrap_or(rest)
        });
        let stem = path
            .strip_suffix(".py")
            .or_else(|| path.strip_suffix(".pyi"))
            .unwrap_or(path);
        let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
        stem.replace('/', ".")
    }
}

/// The repo-relative path a `#[path = "..."]` declaration names: the
/// declared path resolved against the declaring file's directory.
pub(crate) fn relocated(declaring: &str, declared: &str) -> String {
    let mut segs: Vec<&str> = declaring.split('/').collect();
    segs.pop();
    for part in declared.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            p => segs.push(p),
        }
    }
    segs.join("/")
}

/// The cfg a cargo target directory implies: `tests/` and `benches/` are
/// compiled only under `--tests` / `--all-targets`, which is exactly
/// `cfg(test)` from a compile record's point of view.
///
/// ponytail: `examples/` (compiled under `--examples`, not `--tests`) and
/// custom `[[test]]` paths are not modeled; `cargo metadata`'s target list
/// is the upgrade when they matter.
#[must_use]
pub fn target_cfg(path: &str, roots: &CrateRoots) -> Option<&'static str> {
    let tail = match roots.nearest(path) {
        Some((dir, _)) => path.get(dir.len()..)?.trim_start_matches('/'),
        None => path,
    };
    (tail.starts_with("tests/") || tail.starts_with("benches/")).then_some("test")
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

/// Directory -> `[package] name`, as the manifests at a rev spell them.
type Manifests = Vec<(String, String)>;

/// What a rev survey wants a blob for. The two populations share one
/// `cat-file` child, so they have to share its request queue too.
enum Want {
    /// A `Cargo.toml` whose `[package] name` binds this directory.
    Manifest(String),
    /// A source file the adapter for `Lang` claims.
    Source(Lang),
}

/// Tracked files at a git revision, with their contents.
///
/// Surveying "at a rev" has to mean *the tree at that rev*, not the working
/// tree wearing a label. The gate compares two snapshots, and if both were
/// really the working tree the comparison would be empty no matter what
/// changed — so this reads blobs out of the object store rather than off disk.
///
/// Two git spawns, whatever the tree's size: `ls-tree` for the listing, then
/// one `cat-file --batch` fed every path it needs. A spawn per file cost yi
/// (18,398 tracked files) about ten seconds, and `drift`'s default base is a
/// rev, so the flagship verb inherited it.
///
/// # Errors
/// [`SurveyError::Git`] if the rev does not resolve or git fails.
pub fn sources_at(root: &Path, rev: &str) -> Result<(Vec<SourceFile>, CrateRoots), SurveyError> {
    let listing = git(root, &["ls-tree", "-r", "-z", "--name-only", rev])?;
    let all: Vec<String> = listing
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    // The worktree's ignore rules apply to both sides of a gate comparison:
    // a rev survey keeping vendor code the head survey drops would report
    // the difference as a mass deletion.
    let skip = ignored(root, &all)?;
    let mut py_roots = Vec::new();
    let mut wanted: Vec<(String, Want)> = Vec::new();
    for path in all.into_iter().filter(|p| !skip.contains(p)) {
        let path = path.replace('\\', "/");
        if let Some(dir) = pyproject_dir(&path) {
            py_roots.push(dir.to_owned());
        }
        // Manifests come from the same rev as the sources they name: roots
        // are part of the state being surveyed, not of the worktree.
        if let Some(dir) = manifest_dir(&path) {
            let dir = dir.to_owned();
            wanted.push((path, Want::Manifest(dir)));
        } else if let Some(lang) = lang_of(&path) {
            wanted.push((path, Want::Source(lang)));
        }
    }
    let (mut out, manifests) = cat_blobs(root, rev, wanted)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((out, CrateRoots::from_manifests(manifests, py_roots)))
}

/// Read every wanted blob at `rev` through one `git cat-file --batch` child.
///
/// The protocol is positional: one `rev:path` request per answer, in order,
/// so a path the rev cannot serve must consume its `<spec> missing` line and
/// nothing else. Getting that wrong would not lose one file, it would shift
/// every later file's bytes onto the wrong path.
fn cat_blobs(
    root: &Path,
    rev: &str,
    wanted: Vec<(String, Want)>,
) -> Result<(Vec<SourceFile>, Manifests), SurveyError> {
    let mut out = Vec::new();
    let mut manifests = Vec::new();
    // Requests end at a newline: `--batch -z` needs git 2.43, and git 2.34
    // exited on it, so every rev survey read "answers ran out". A path with a
    // newline in it cannot be asked for, so it is named unreadable, not read.
    let (wanted, unaskable): (Vec<_>, Vec<_>) = wanted
        .into_iter()
        .partition(|(path, _)| !path.contains('\n'));
    for (path, want) in unaskable {
        if let Want::Source(lang) = want {
            out.push(source_file(path, lang, None));
        }
    }
    if wanted.is_empty() {
        return Ok((out, manifests));
    }
    let as_git = |e: std::io::Error| SurveyError::Git(e.to_string());
    let mut child = Command::new("git")
        .args(["cat-file", "--batch"])
        .current_dir(root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(as_git)?;
    let mut buf = Vec::new();
    for (path, _) in &wanted {
        buf.extend_from_slice(rev.as_bytes());
        buf.push(b':');
        buf.extend_from_slice(path.as_bytes());
        buf.push(b'\n');
    }
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SurveyError::Git("git cat-file: no stdout".to_owned()))?;
    // Feed stdin on its own thread: with both pipes on one thread, the
    // answers fill the stdout pipe while this side is still blocked writing
    // requests.
    let result = std::thread::scope(|scope| {
        scope.spawn(move || {
            if let Some(mut stdin) = stdin {
                let _ = std::io::Write::write_all(&mut stdin, &buf);
            }
        });
        let mut reader = std::io::BufReader::new(stdout);
        let mut fail = None;
        for (path, want) in wanted {
            let mut header = Vec::new();
            match std::io::BufRead::read_until(&mut reader, b'\n', &mut header) {
                Ok(0) => {
                    fail = Some(SurveyError::Git("git cat-file: answers ran out".to_owned()));
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    fail = Some(as_git(e));
                    break;
                }
            }
            let (size, is_blob) = match batch_header(&header) {
                Ok(Some(answer)) => answer,
                // A path the listing named that the rev cannot serve: skip
                // it, never fabricate it as empty.
                Ok(None) => continue,
                Err(e) => {
                    fail = Some(e);
                    break;
                }
            };
            // git writes the payload plus one newline of its own.
            let mut bytes = vec![0u8; size.saturating_add(1)];
            if let Err(e) = std::io::Read::read_exact(&mut reader, &mut bytes) {
                fail = Some(as_git(e));
                break;
            }
            if !is_blob {
                continue;
            }
            bytes.truncate(size);
            match want {
                Want::Manifest(dir) => {
                    if let Some(name) = String::from_utf8(bytes)
                        .ok()
                        .as_deref()
                        .and_then(package_name)
                    {
                        manifests.push((dir, name));
                    }
                }
                // A blob that is not UTF-8 is not source an adapter could
                // parse; it is kept, marked, and named by the survey.
                Want::Source(lang) => out.push(source_file(path, lang, Some(bytes))),
            }
        }
        // Drain whatever is still queued so the writer thread is never left
        // blocked on a full pipe while this scope waits to join it.
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
        fail
    });
    let _ = child.wait();
    match result {
        Some(e) => Err(e),
        None => Ok((out, manifests)),
    }
}

/// One `git cat-file --batch` answer header: `<oid> <type> <size>`, or a
/// one-word refusal for a spec the rev cannot serve.
///
/// The refusals are `missing` (no such object), `ambiguous` (the spec names
/// more than one) and `submodule` (a gitlink, whose commit belongs to another
/// repository) — git 2.50 answers a gitlink `missing` and later versions
/// answer it `submodule`, so both are answers grid must expect. None of the
/// three carries a payload; an unknown word is refused rather than skipped,
/// because skipping an answer whose payload is real slides every later answer
/// onto the wrong path.
///
/// `Ok(None)` is a refusal. `Ok(Some((size, is_blob)))` has `size` payload
/// bytes to consume whatever the type — a gitlink whose commit this
/// repository does hold answers with that commit, and skipping its payload is
/// what keeps the stream aligned.
fn batch_header(header: &[u8]) -> Result<Option<(usize, bool)>, SurveyError> {
    let bad = || {
        SurveyError::Git(format!(
            "git cat-file: unreadable answer {:?}",
            String::from_utf8_lossy(header)
        ))
    };
    let line = std::str::from_utf8(header)
        .map_err(|_| bad())?
        .trim_end_matches('\n');
    let (rest, last) = line.rsplit_once(' ').ok_or_else(bad)?;
    if matches!(last, "missing" | "ambiguous" | "submodule") {
        return Ok(None);
    }
    let size: usize = last.parse().map_err(|_| bad())?;
    let (_, kind) = rest.rsplit_once(' ').ok_or_else(bad)?;
    Ok(Some((size, kind == "blob")))
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

#[cfg(test)]
mod batch_answers {
    use super::batch_header;

    /// The three words git uses for a spec it will not serve. A gitlink is
    /// `missing` on git 2.50 and `submodule` on later versions, and grid runs
    /// against whichever git the machine has.
    #[test]
    fn every_refusal_word_is_a_skip_and_never_an_error() {
        for word in ["missing", "ambiguous", "submodule"] {
            let line = format!("0000000000000000000000000000000000000001 {word}\n");
            assert_eq!(
                batch_header(line.as_bytes()).expect("a refusal is not an error"),
                None,
                "git answered {word} and the survey refused the whole rev"
            );
        }
    }

    /// Conservatism: an answer grid does not understand is refused, never
    /// treated as payload-free. Skipping a real payload would slide every
    /// later answer onto the wrong path — the failure this parser exists to
    /// prevent.
    #[test]
    fn an_answer_grid_does_not_know_is_refused() {
        let line = b"0000000000000000000000000000000000000001 sideways\n";
        assert!(
            batch_header(line).is_err(),
            "an unknown answer word was skipped as if it carried no payload"
        );
    }

    #[test]
    fn a_served_object_carries_its_size_and_says_whether_it_is_a_blob() {
        let blob = b"deadbeef blob 12\n";
        assert_eq!(batch_header(blob).expect("a blob header"), Some((12, true)));
        let commit = b"deadbeef commit 240\n";
        assert_eq!(
            batch_header(commit).expect("a commit header"),
            Some((240, false)),
            "a non-blob still reports its size, which is what keeps the stream aligned"
        );
    }
}
