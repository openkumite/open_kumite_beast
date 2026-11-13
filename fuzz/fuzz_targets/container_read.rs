#![no_main]
//! The packed archive format of the game data (`formats/container.rs`).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = open_kumite_beast::formats::container::read(data);
});
