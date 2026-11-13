#![no_main]
//! The extra stage's map (q3_map::parse): a player-provided glb and shader table must not crash the game. The input
//! is a little-endian u32 length, the glb, and the shader table's JSON after it.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(n) = data.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) else { return };
    let rest = &data[4..];
    let (glb, json) = rest.split_at(n.min(rest.len()));
    let shaders = serde_json::from_slice(json).unwrap_or(serde_json::Value::Null);
    if let Ok(map) = open_kumite_beast::formats::q3_map::parse(glb, "baseq3/maps/q3dm17.glb", &shaders) {
        let _ = map.textures();
    }
});
