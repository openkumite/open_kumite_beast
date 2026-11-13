#![no_main]
//! Save states (session::load_state, session::load_file): a snapshot, desync dump or save file from elsewhere
//! must give an Err, not a panic or a huge allocation (no static data here, so a state with a front end or a fight
//! stops at `rebind`; `decode_file` covers the whole decode of a file).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let files = Default::default();
    let a = open_kumite_beast::session::Assets { front: None, fight: None, backdrop: None, files: &files, prefs: Default::default() };
    let _ = open_kumite_beast::session::load_state(data, &a);
    let _ = open_kumite_beast::session::load_file(data, &a);
    let _ = open_kumite_beast::session::decode_file(data);
});
