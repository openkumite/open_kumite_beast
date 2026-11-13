#![no_main]
//! TIM images loaded into the 2D texture store (`formats/vram.rs`).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut v = open_kumite_beast::formats::vram::Vram::default();
    v.load_tim(data, None, None);
    v.load_tim(data, Some((0, 480)), Some((768, 256)));
});
