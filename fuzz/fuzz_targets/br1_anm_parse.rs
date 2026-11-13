#![no_main]
//! The imported game's fighter file `XX.ANM` / `XX31.DAT` and its clip blocks (`formats::br1_anm`): the whole file, and the
//! clip block alone as CMN.ACT is read (at offset 0) and at the file's own clip offset.
use open_kumite_beast::formats::br1_anm::{Anm, Clips};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = Clips::parse(data, 0);
    if data.len() >= 4 {
        let base = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let _ = Clips::parse(data, base);
    }
    if let Ok(anm) = Anm::parse(data.to_vec()) {
        let _ = (anm.states.len(), anm.commands.len(), anm.chains.len(), anm.clips.clips.len());
    }
});
