//! The usage surface: what the binary tells a caller it can do.
//!
//! A word here is a promise, and the sweep of merged `main` found several the
//! binary did not keep — a relation the usage line offered and the parse
//! rejected, verbs the listing omitted, `--help` parsed as a name. Keeping the
//! surface in one file is what makes the next such drift one diff to read.

use std::io::Write as _;

use crate::hint;

pub const PITCH: &str = "what your code promises vs what it does";

pub fn banner() {
    hint(&format!(
        "grid — {PITCH}\n  grid              what changed, and does it match the promise\n  \
         grid uses X       who reaches X, across languages and processes\n  \
         grid explain Y    what any word, symbol, or id in this output means"
    ));
}

/// One line per verb — the banner sells three, this lists them all.
///
/// Written past `hint`, not through it: `GRID_HINTS=0` means "do not teach me
/// unasked" and cannot mean "refuse to answer when I ask", which is what it
/// did — `GRID_HINTS=0 grid help` printed nothing at all, exit 0.
pub fn help() {
    let _ = write!(
        std::io::stderr(),
        "\nverbs:\n  \
         version                      engine, wire and porcelain versions\n  \
         survey [<rev>]               chart the worktree, or the tree at a rev; idempotent\n  \
         diff [base] [head]           what changed, in defs and edges\n  \
         drift [base] [head]          dependents of changed interfaces; exit 3 on suspects\n  \
         drift --tests                known tests whose recorded verdict no longer applies\n  \
         drift --compat               changed Deserialize types old bytes cannot read\n  \
         covenant <A> <B> --via <w>   declare a coupling no language establishes\n  \
         invariant persisted <p>      declare that old bytes of a type exist; --list shows the repo's rules\n  \
         watch [base]                 signed drift deltas, one file event per stdin line\n  \
         interfere <other-worktree>   what another worktree's edits and yours collide over\n  \
         resolve <name|file|f:line>   where it is; a bare file lists its defs\n  \
         uses <name> [--relation r]   proven inbound edges\n  \
         scope <pattern> [--depth N]  context pack around an edit\n  \
         scope <p> --bodies N         the same, with the text and its horizon\n  \
         scope <p> --orient|--edit    the two named depth presets (0/2 and 1/3)\n  \
         orphans [--under p] [--traced]  no proven caller (not the same as dead); --traced adds trace N\n  \
         hotspots [--under p]         complexity x churn, descending\n  \
         roots                        import name -> directory map\n  \
         log <name>                   a definition's history in identity space\n  \
         bisect <test>                the patch the retained chain first recorded it failing at\n  \
         suspects <test> [--closure [<since>]]  likely culprits, ranked; bare --closure uses the last snapshot it passed at\n  \
         test [base] [--trace|--all]  run what the change can reach; replay the rest; exit 3 on red\n  \
         gate <plan> [head]           promise vs delta; exit 3 on hold\n  \
         plan lint <file>             validate a promise file\n  \
         plan new [--from-diff <b>]   a starter promise file, or one drafted from the diff\n  \
         todo <plan>                  the promise as a worklist\n  \
         check [base] [head] [--quick]  everything the change did that nobody checked; exit 3\n  \
         check --plan <p> [--why]     the same, plus the gate verdict and why it says so\n  \
         check --format md            the same, as one markdown document for a PR comment\n  \
         check --compile [-- args]    wrap cargo check; the callers it did not compile; exit 3\n  \
         cache lookup|record ...      test verdicts keyed by identity closure\n  \
         edit apply <file>            compare-and-swap edits by datum\n  \
         trace ingest|diff ...        runtime traces -> observed edges\n  \
         trace export|import <dir>    retained traces as a directory that travels\n  \
         explain <anything>           define any word or id in grid output\n  \
         serve                        the same contract on a socket\n\n\
         every verb takes --json; hints live on stderr (GRID_HINTS=0)\n"
    );
}

/// `--json` is the wire result verbatim — the same bytes the daemon returns.
/// Refuse a flag where a revision is expected. These verbs take positionals
/// and know no flags of their own, so an unknown one reached git as a
/// revision: `grid survey --nonsense` reported `git: error: unknown option`,
/// blaming git for a grid usage mistake.
pub fn flag_where_a_rev_belongs(args: &[&str], usage: &str) -> bool {
    let Some(flag) = args.iter().find(|a| a.starts_with("--")) else {
        return false;
    };
    hint(&format!("grid: unknown flag `{flag}`\n{usage}"));
    true
}

/// The wire spelling of a relation, for `--relation` arguments. The serde
/// names are the vocabulary, so the flag cannot drift from the contract.
/// Every relation `--relation` accepts, in contract order. The usage line
/// advertised `imports`, which no chart has and the parse rejects with exit 2,
/// while omitting five that do exist; `tests/porcelain.rs` now feeds this
/// offer back through the parse and checks it against the contract's own enum.
pub const RELATIONS: &str = "calls|constructs|reads_field|writes_field|implements|dispatches_to|\
                         flows_into|serializes_to|deserializes_from|registers_under|sends_over|\
                         links_against";

pub fn relation_named(name: &str) -> Option<grid_wire::Relation> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

pub const ORPHANS_USAGE: &str =
    "usage: grid orphans [--lang <py|rs|ts|js>] [--under <path>] [--traced]";
