#![no_main]
//! VAB sound banks and their ADPCM samples (`formats/vab.rs`).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = open_kumite_beast::formats::vab::Vab::parse(data);
    let _ = open_kumite_beast::formats::vab::decode_adpcm(data);
});
