#![no_main]
//! The imported game's select portraits (`assets::br1_art::from_files`): the two containers `CMN/SELE00.DAT` / `SELE01.DAT`
//! (the input split at its first byte's share) cut into the front end's pictures.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let at = data.first().map_or(0, |&b| b as usize * data.len() / 256);
    let (a, b) = data.split_at(at);
    let art = open_kumite_beast::assets::br1_art::from_files(a, b);
    let _ = (art.select.len(), art.faces.len(), art.vs.len());
});
