#![no_main]
//! The client decodes bytes it did not write. Every line the socket hands it
//! is either a typed response or a typed refusal — never a panic. Evidence
//! for claim C52 at har-verify rung 4.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = grid_client::decode_line(data);
});
