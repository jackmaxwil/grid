//! `grid explain` — the help system. Answers what any porcelain token, wire
//! word, or `@handle` means, from a corpus generated at build time out of
//! the wire contract and `docs/ONTOLOGY.md` (see `build.rs`): there is no
//! hand-written help to drift out of date.
//!
//! Context answers come from the chart: `grid explain @9f2c` names the
//! definition the prefix pins. File-and-line materialization is not carried
//! by the segment format at this wave, so the answer stops at the callsign.

use crate::{exit, hint};

include!(concat!(env!("OUT_DIR"), "/explain.rs"));

/// `grid explain <token>`.
pub fn main(args: &[&str]) -> u8 {
    let Some(token) = args.first() else {
        hint("usage: grid explain <word|record|@datum-prefix>");
        return exit::BAD_INPUT;
    };
    if let Some(prefix) = token.strip_prefix('@') {
        return explain_handle(prefix);
    }
    if let Some(answer) = lookup(token) {
        println!("{token} — {answer}");
        return exit::OK;
    }
    hint(&format!(
        "grid: nothing known about `{token}`\nhint: try `grid explain datum`, \
         `grid explain hold`, or `grid explain @<datum-prefix>`"
    ));
    exit::BAD_INPUT
}

/// The corpus entry for a token, case-insensitive.
fn lookup(token: &str) -> Option<&'static str> {
    let token = token.to_lowercase();
    EXPLAIN
        .binary_search_by(|(key, _)| (*key).cmp(token.as_str()))
        .ok()
        .map(|i| EXPLAIN[i].1)
}

/// `grid explain @<prefix>` — what the chart calls the datum the prefix pins.
fn explain_handle(prefix: &str) -> u8 {
    let chart = match crate::chart_provider::ChartDelta::open(std::path::Path::new(".")) {
        Ok(chart) => chart,
        Err(e) => {
            hint(&format!("grid: {e}"));
            return exit::INTERNAL;
        }
    };
    let mut found = Vec::new();
    for segment in chart.head().segments() {
        for ord in 0..segment.def_count() {
            let ord = grid_chart::Ordinal(ord);
            if let (Some(datum), Some(callsign)) = (segment.datum(ord), segment.callsign(ord)) {
                let hex = datum.to_string();
                if hex
                    .strip_prefix("b3:")
                    .is_some_and(|h| h.starts_with(prefix))
                {
                    found.push((callsign.to_owned(), hex));
                }
            }
        }
    }
    match found.as_slice() {
        [] => {
            hint(&format!(
                "grid: nothing in the chart answers to @{prefix} — survey first?"
            ));
            exit::BAD_INPUT
        }
        matches => {
            for (callsign, datum) in matches {
                println!("@{prefix} = {callsign}  ({datum})");
            }
            if let Some(answer) = lookup("datum") {
                println!("the suffix is the definition's content hash — {answer}");
            }
            exit::OK
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corpus_covers_both_sources() {
        // ONTOLOGY.md, the curated answers.
        assert!(lookup("datum").is_some());
        assert!(lookup("hold").is_some());
        // The wire contract's schema descriptions, lowercased schema names.
        assert!(lookup("xteverdict").is_some());
        assert!(lookup("callsignpattern").is_some());
    }

    #[test]
    fn the_corpus_is_sorted_and_unique() {
        // The binary search depends on it, and the build script emits in
        // BTreeMap order — this test is the belt to that suspenders.
        let mut keys = EXPLAIN.iter().map(|(k, _)| *k);
        let mut prev = keys.next().expect("nonempty corpus");
        for key in keys {
            assert!(prev < key, "corpus keys out of order: {prev} !< {key}");
            prev = key;
        }
    }

    #[test]
    fn ontology_wins_a_collision() {
        // `Datum` is both a wire schema and an ontology noun; the curated
        // ontology answer is the one served.
        let answer = lookup("datum").expect("datum entry");
        assert!(answer.contains("identity"), "ontology answer: {answer}");
    }
}
