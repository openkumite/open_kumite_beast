#![no_main]
//! ANI character description files (act_ani::Ani) and the tables read from them.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let ani = open_kumite_beast::formats::act_ani::Ani::parse(data.to_vec());
    for k in 0..4 {
        let _ = ani.spheres(k);
    }
    for form in 0..2 {
        let _ = ani.bone_map(form, false);
        let _ = ani.bone_map(form, true);
    }
    for c in 0..11 {
        let _ = ani.group(c);
    }
});
