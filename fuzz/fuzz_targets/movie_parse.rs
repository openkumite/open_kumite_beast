#![no_main]
//! STR movies (`formats/movie.rs`): the container and the MDEC frame decoder.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(m) = open_kumite_beast::formats::movie::Movie::parse(data.to_vec()) {
        let _ = m.decode(0);
    }
    let _ = open_kumite_beast::formats::movie::decode(data, 32, 32);
});
