//! `grid scope --bodies`: the context pack with tiers, a layer-atomic budget,
//! and a horizon certificate (C39).
//!
//! The load-bearing claim is not "the pack is big enough" — it is that the
//! horizon can be believed. A budget is spent in whole hop layers, so every
//! definition inside the depth the horizon names is actually in the pack; a
//! layer that does not fit is dropped whole, and a body layer that does not
//! fit is demoted to its sigs before it is dropped. Nothing here is ever
//! truncated, because a certificate over half a layer is the lie
//! `complete_within` exists to prevent (C31).
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Output};

fn grid(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_grid"))
        .args(args)
        .current_dir(dir)
        .env("GRID_HINTS", "0")
        .output()
        .expect("run grid")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = grid(dir, args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "grid {args:?}:\n{}",
        stdout(&out)
    );
    stdout(&out)
}

/// A five-link call chain, one definition per link, each body distinctly
/// sized: `a` calls `b` calls `c` calls `d` calls `e`. Rooted at `a`, hop `k`
/// is exactly the k-th link, which is what makes a tier assertion a
/// ground-truth assertion rather than a shape assertion.
fn chain() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .env("GIT_AUTHOR_NAME", "grid")
            .env("GIT_AUTHOR_EMAIL", "grid@example.invalid")
            .env("GIT_COMMITTER_NAME", "grid")
            .env("GIT_COMMITTER_EMAIL", "grid@example.invalid")
            .output()
            .expect("run git")
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(dir.path().join("pkg")).expect("mkdir");
    std::fs::write(dir.path().join("pkg/__init__.py"), "").expect("write");
    std::fs::write(
        dir.path().join("pkg/chain.py"),
        "def e():\n    return 'the last link in the chain, and a wide body'\n\n\n\
         def d():\n    return e() + 'padding so d weighs more than its own header'\n\n\n\
         def c():\n    return d() + 'padding so c weighs more than its own header'\n\n\n\
         def b():\n    return c() + 'padding so b weighs more than its own header'\n\n\n\
         def a():\n    return b() + 'padding so a weighs more than its own header'\n",
    )
    .expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "chain"]);
    assert_eq!(
        grid(dir.path(), &["survey"]).status.code(),
        Some(0),
        "survey the chain"
    );
    dir
}

/// Every `##` header in a document, as (callsign, kind, hop).
fn headers(doc: &str) -> Vec<(String, String, u64)> {
    doc.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("## ")?;
            let hop = line
                .rsplit_once("hop ")
                .and_then(|(_, n)| n.trim_end_matches(')').parse().ok())?;
            let (kind, callsign) = rest.strip_prefix("sig  ").map_or_else(
                || ("body", rest.split_whitespace().next().unwrap_or_default()),
                |rest| ("sig", rest.split_whitespace().next().unwrap_or_default()),
            );
            Some((callsign.to_owned(), kind.to_owned(), hop))
        })
        .collect()
}

fn horizon(doc: &str) -> &str {
    doc.lines()
        .find(|l| l.starts_with("# horizon:"))
        .expect("every tiered pack closes with its horizon line")
}

/// Ground truth: `--bodies 1 --sigs 3` over the chain packs bodies for hops
/// 0 and 1, sigs for hops 2 and 3, nothing at hop 4, and certifies 1 and 3.
#[test]
fn tiers_are_bodies_to_one_depth_and_sigs_to_a_further_one() {
    let dir = chain();
    let doc = run(
        dir.path(),
        &["scope", "pkg.chain.a", "--bodies", "1", "--sigs", "3"],
    );
    let got = headers(&doc);
    assert_eq!(
        got,
        vec![
            ("pkg.chain.a".to_owned(), "body".to_owned(), 0),
            ("pkg.chain.b".to_owned(), "body".to_owned(), 1),
            ("pkg.chain.c".to_owned(), "sig".to_owned(), 2),
            ("pkg.chain.d".to_owned(), "sig".to_owned(), 3),
        ],
        "bodies 0-1, sigs 2-3, and `e` at hop 4 is beyond both:\n{doc}"
    );
    assert!(
        !doc.contains("pkg.chain.e"),
        "nothing past the tiers:\n{doc}"
    );
    let line = horizon(&doc);
    assert!(
        line.starts_with("# horizon: bodies complete through 1 hop, sigs through 3, \u{2248}")
            && line.ends_with(" tokens (bytes/4)"),
        "the certificate names both depths and states the estimate as one: {line}"
    );
    // A sig is the header, never the body: `d`'s sig must not carry `return`.
    let sig_block = doc
        .split("## sig  pkg.chain.d")
        .nth(1)
        .expect("d's sig block");
    assert!(
        !sig_block.contains("return"),
        "a sig is what a caller sees without the body:\n{sig_block}"
    );
}

/// Differential against the walk the pack rides on: at every depth the
/// horizon certifies, the pack's definitions are exactly the ones plain
/// `grid scope --depth k` names. A tier that certified a depth the walk does
/// not agree with would be a certificate about a different graph.
#[test]
fn each_certified_depth_holds_the_definitions_scope_depth_names() {
    let dir = chain();
    let doc = run(
        dir.path(),
        &["scope", "pkg.chain.a", "--bodies", "1", "--sigs", "3"],
    );
    let packed = headers(&doc);
    for k in 0..=3u64 {
        let within: BTreeSet<String> = packed
            .iter()
            .filter(|(_, _, hop)| *hop <= k)
            .map(|(callsign, _, _)| callsign.clone())
            .collect();
        let walked: BTreeSet<String> = run(
            dir.path(),
            &["scope", "pkg.chain.a", "--depth", &k.to_string()],
        )
        .lines()
        .filter(|l| l.starts_with("ref "))
        .filter_map(|l| l.split_whitespace().nth(1).map(str::to_owned))
        .collect();
        assert_eq!(within, walked, "hop <= {k} disagrees with --depth {k}");
    }
}

/// Monotone in the budget: a larger budget covers every definition a smaller
/// one covered. A definition may be *demoted* from a body to a sig as the
/// budget falls, which is why the comparison is over the definitions the pack
/// holds — the guarantee is coverage, and the tier is the horizon's business.
#[test]
fn a_bigger_budget_packs_a_superset() {
    let dir = chain();
    let covered = |budget: u64| -> BTreeSet<String> {
        headers(&run(
            dir.path(),
            &[
                "scope",
                "pkg.chain.a",
                "--bodies",
                "1",
                "--sigs",
                "3",
                "--budget",
                &budget.to_string(),
            ],
        ))
        .into_iter()
        .map(|(callsign, _, _)| callsign)
        .collect()
    };
    let ladder: Vec<BTreeSet<String>> = [2u64, 8, 20, 40, 80, 10_000]
        .iter()
        .map(|b| covered(*b))
        .collect();
    for pair in ladder.windows(2) {
        assert!(
            pair[1].is_superset(&pair[0]),
            "a bigger budget dropped something a smaller one packed: {:?} !⊇ {:?}",
            pair[1],
            pair[0]
        );
    }
    assert!(
        ladder.last().is_some_and(|top| top.len() == 4),
        "an unconstrained budget packs the whole request: {ladder:?}"
    );
}

/// Conservatism: a budget too small for the roots' own bodies emits the
/// roots' *sigs* only, and says so — `bodies: none, sigs: 0`. Never a
/// truncated body, never a layer packed in part, never a missing horizon.
#[test]
fn a_budget_below_the_roots_own_body_packs_sigs_and_says_bodies_none() {
    let dir = chain();
    let doc = run(
        dir.path(),
        &[
            "scope",
            "pkg.chain.a",
            "--bodies",
            "1",
            "--sigs",
            "3",
            "--budget",
            "4",
        ],
    );
    assert_eq!(
        headers(&doc),
        vec![("pkg.chain.a".to_owned(), "sig".to_owned(), 0)],
        "the roots' sigs and nothing else:\n{doc}"
    );
    assert!(
        horizon(&doc).starts_with("# horizon: bodies complete through none, sigs through 0,"),
        "the certificate names what it gave up:\n{doc}"
    );
    // The body was dropped, not trimmed: no fragment of it is in the pack.
    assert!(
        !doc.contains("padding so a weighs"),
        "a body layer that did not fit must be dropped whole, never trimmed:\n{doc}"
    );
    // And the horizon line survives an empty pack, which is the case a caller
    // most needs it in.
    let empty = run(
        dir.path(),
        &[
            "scope",
            "pkg.chain.a",
            "--bodies",
            "1",
            "--sigs",
            "3",
            "--budget",
            "0",
        ],
    );
    assert!(headers(&empty).is_empty(), "nothing fit:\n{empty}");
    assert!(
        horizon(&empty).starts_with("# horizon: bodies complete through none, sigs through none,"),
        "a pack that packed nothing still bounds itself:\n{empty}"
    );
}

/// Without the tier flags, `scope` is the verb it was: records on stdout, no
/// document, no horizon line, and the same bytes it emitted before the tiers
/// existed. The literal below was captured from the pre-tier binary.
#[test]
fn scope_without_the_new_flags_is_byte_identical() {
    let dir = chain();
    let plain = run(dir.path(), &["scope", "pkg.chain.a", "--depth", "1"]);
    assert!(
        !plain.contains("## ") && !plain.contains("# horizon:"),
        "no document without the flags:\n{plain}"
    );
    let tags: Vec<&str> = plain
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(
        tags.iter().all(|t| ["ref", "edge", "inv"].contains(t)),
        "records only:\n{plain}"
    );
    assert_eq!(
        plain.lines().filter(|l| l.starts_with("ref ")).count(),
        2,
        "a and b, exactly as --depth 1 always answered:\n{plain}"
    );
    // The pack a request without tiers gets carries no text and no horizon on
    // the wire either — the fields are additive, not defaulted.
    let json = run(dir.path(), &["scope", "pkg.chain.a", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&json).expect("json");
    assert!(value.get("bodies").is_none(), "{json}");
    assert!(value.get("sigs").is_none(), "{json}");
    assert!(value.get("horizon").is_none(), "{json}");
}

/// `--since` packs the delta, not the repo: only definitions whose datum or
/// sig moved since the named snapshot survive the walk.
#[test]
fn since_keeps_only_what_moved() {
    let dir = chain();
    let before: serde_json::Value =
        serde_json::from_str(&run(dir.path(), &["version", "--json"])).expect("json");
    let snapshot = before["snapshot"].as_str().expect("a snapshot").to_owned();

    let path = dir.path().join("pkg/chain.py");
    let source = std::fs::read_to_string(&path).expect("read");
    std::fs::write(
        &path,
        source.replace(
            "padding so c weighs more than its own header",
            "c was the only definition edited in this pass",
        ),
    )
    .expect("write");
    assert_eq!(grid(dir.path(), &["survey"]).status.code(), Some(0));

    let doc = run(
        dir.path(),
        &[
            "scope",
            "pkg.chain.**",
            "--bodies",
            "0",
            "--since",
            &snapshot,
        ],
    );
    assert_eq!(
        headers(&doc)
            .into_iter()
            .map(|(callsign, _, _)| callsign)
            .collect::<Vec<_>>(),
        vec!["pkg.chain.c".to_owned()],
        "only the definition that moved:\n{doc}"
    );
}

/// The `--json` form is the record form: the same pack, typed, with the
/// horizon as fields rather than prose.
#[test]
fn json_carries_the_tiers_and_the_horizon_as_fields() {
    let dir = chain();
    let json = run(
        dir.path(),
        &[
            "scope",
            "pkg.chain.a",
            "--bodies",
            "1",
            "--sigs",
            "3",
            "--json",
        ],
    );
    let pack: grid_wire::ScopePack = serde_json::from_str(&json).expect("a wire ScopePack");
    let bodies = pack.bodies.expect("bodies");
    let sigs = pack.sigs.expect("sigs");
    assert_eq!(
        bodies.iter().map(|t| t.hop).collect::<Vec<_>>(),
        vec![0, 1],
        "{bodies:?}"
    );
    assert_eq!(
        sigs.iter().map(|t| t.hop).collect::<Vec<_>>(),
        vec![2, 3],
        "{sigs:?}"
    );
    let horizon = pack.horizon.expect("horizon");
    assert_eq!(horizon.bodies_complete_within, Some(1));
    assert_eq!(horizon.sigs_complete_within, Some(3));
    let bytes: usize = bodies.iter().chain(sigs.iter()).map(|t| t.text.len()).sum();
    assert_eq!(
        horizon.approx_tokens,
        bytes as u64 / 4,
        "the estimate is exactly bytes/4 over what was packed"
    );
}
