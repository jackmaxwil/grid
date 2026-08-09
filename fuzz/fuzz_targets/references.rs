#![no_main]
//! Reference extraction must never panic and must agree with itself on every
//! input, valid Python or not. Wave 5 evidence for claim C14: the extractor
//! runs over whatever is in the repo, so "arbitrary bytes" is its real input
//! distribution, not a hypothetical.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(first) = grid_datum::references(source) {
        let second = grid_datum::references(source).expect("extraction is deterministic");
        assert_eq!(first, second, "same source, same references");
        // Every reference names something: an empty path would be a name that
        // resolution could match against anything.
        for r in &first.refs {
            assert!(!r.path.is_empty(), "a reference always names a path");
        }
        // Every import binds a name. A binding with no alias could shadow
        // whatever the resolver happened to look up next.
        for i in &first.imports {
            assert!(!i.alias.is_empty(), "an import always binds a name");
        }
    }
});
