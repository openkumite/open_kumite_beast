#![no_main]
//! The save file (settings.json): a hand-edited or corrupted save must not crash the game.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<open_kumite_beast::frontend::options::Settings>(s);
    }
});
