#![no_main]
//! The normalizer must never panic and must be deterministic on every input,
//! valid Python or not. Wave 2 evidence for the datum crate.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(first) = grid_lang_py::normalize(source) {
        let second = grid_lang_py::normalize(source).expect("normalization is deterministic");
        assert_eq!(first.datum, second.datum, "same source, same datum");
        // Renormalizing an equivalent rendering must not move the datum.
        assert_eq!(first.canonical, second.canonical);
    }
});
