//! Lists the programs of a VAB bank and plays one: `sndtest ../okb_export/engine/snd/COMMON_1.SND [program] [note]`.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).unwrap();
    let vab = open_kumite_beast::formats::vab::Vab::parse(&bytes).expect("not a VAB");
    for (i, p) in vab.programs.iter().enumerate().filter(|(_, p)| !p.tones.is_empty()) {
        let t: Vec<String> = p.tones.iter().map(|t| format!("vag{} c{} {}..{} v{}", t.vag, t.center, t.min, t.max, t.volume)).collect();
        println!("prog {i:3} vol {:3}: {}", p.volume, t.join(", "));
    }
    if let Some(t) = std::env::var("TONE").ok().and_then(|s| s.parse::<usize>().ok()) {
        // offline: linear tone with the SFX driver's envelope, written to /tmp-free path given by OUT
        let mut au = open_kumite_beast::audio::Audio::new();
        au.load_bank("b", &bytes);
        au.play_tone("b", t, 0x2000, 2);
        let pcm = au.render_offline(44100);
        let peak = pcm.iter().fold(0f32, |m, v| m.max(v.abs()));
        let last = pcm.iter().rposition(|v| v.abs() > 1e-4).unwrap_or(0) / 2;
        println!("tone {t}: peak {peak:.3}, audible for {:.3} s", last as f32 / 44100.0);
        if let Ok(out) = std::env::var("OUT") {
            let mut w = Vec::new();
            for v in &pcm {
                w.extend_from_slice(&((v * 32767.0) as i16).to_le_bytes());
            }
            let mut h = b"RIFF".to_vec();
            h.extend_from_slice(&(36 + w.len() as u32).to_le_bytes());
            h.extend_from_slice(b"WAVEfmt ");
            for v in [16u32, 0x0002_0001, 44100, 44100 * 4, 0x0010_0004] {
                h.extend_from_slice(&v.to_le_bytes());
            }
            h.extend_from_slice(b"data");
            h.extend_from_slice(&(w.len() as u32).to_le_bytes());
            h.extend_from_slice(&w);
            std::fs::write(out, h).unwrap();
        }
        return;
    }
    if let Some(p) = a.get(2).and_then(|s| s.parse::<usize>().ok()) {
        let note = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(vab.programs[p].tones.first().map(|t| t.center).unwrap_or(60));
        let mut au = open_kumite_beast::audio::Audio::new();
        au.load_bank("b", &bytes);
        au.start();
        println!("voice {:?}", au.play("b", p, note, 127, 64));
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
}
#[cfg(target_arch = "wasm32")]
fn main() {}
