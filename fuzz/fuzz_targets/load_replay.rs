#![no_main]
//! Replay files (session::replay::Replay::decode / decode_header): a `.okbr` from elsewhere must give an Err, not a
//! panic or a huge allocation; a file that decodes is played through its whole input stream.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = open_kumite_beast::session::replay::Replay::decode_header(data);
    if let Ok(r) = open_kumite_beast::session::replay::Replay::decode(data) {
        let _ = r.encode();
        let mut p = open_kumite_beast::session::replay::Player::new(r);
        while p.next_step().is_some() {}
    }
});
