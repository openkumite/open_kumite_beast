#![no_main]
//! ACT animation files (act_ani::Act).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let act = open_kumite_beast::formats::act_ani::Act::parse(data);
    for id in 0..8 {
        let _ = act.offset(id);
    }
});
