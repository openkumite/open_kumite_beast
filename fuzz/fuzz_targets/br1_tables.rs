#![no_main]
//! The imported game's static tables (`formats::tables::Tables`): any bytes parsed as a `tables.okt` (any table names,
//! origins, clip offsets, ranges and `image_len`), its imported check and the clip ranges; reads by address; the
//! AI's random table and its shuffle; the shared tables (`fight::br1::res::Br1Tables`: Err for clips outside the image,
//! no panic) and a fighter's static data built over them (the ANM and CMN.ACT from the same bytes).
use open_kumite_beast::fight::br1::ai::Table;
use open_kumite_beast::fight::br1::res::{Br1Res, Br1Tables};
use open_kumite_beast::formats::tables::Tables;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(okt) = Tables::parse(data) else { return };
    let seed = data.first().copied().unwrap_or(0) as u32;
    let _ = okt.require_br1();
    let _ = okt.clip_ranges();
    for a in [0, 0x8000_0000, seed.wrapping_mul(0x0101_0101), u32::MAX] {
        let _ = (okt.abs_u8(a), okt.abs_i16(a), okt.abs_u32(a));
    }
    let mut t = Table::boot(&okt);
    t.shuffle(&okt, seed);
    if let Ok(shared) = Br1Tables::new(okt) {
        let _ = Br1Res::new("MIT", data.to_vec(), data.to_vec(), &shared);
    }
});
