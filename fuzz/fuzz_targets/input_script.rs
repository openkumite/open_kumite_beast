#![no_main]
//! The scripted-input format of the debug hooks (OKB_SCRIPT / OKB_FESCRIPT).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = open_kumite_beast::fight::input_script::parse_script(s);
    }
});
