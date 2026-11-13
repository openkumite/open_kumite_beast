//! Decodes one movie frame to a PPM.
//!
//! Decode one movie frame to a PPM: mdectest <bsf> <frame> <out.ppm>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let m = open_kumite_beast::formats::movie::Movie::parse(std::fs::read(&a[1]).unwrap()).unwrap();
    let k: usize = a[2].parse().unwrap();
    let t = std::time::Instant::now();
    let px = m.decode(k);
    eprintln!("decoded in {:?}, sector {}", t.elapsed(), m.frames[k].0);
    let mut out = format!("P6\n{} {}\n255\n", m.w, m.h).into_bytes();
    for p in px {
        out.extend([(p & 31) << 3, (p >> 5 & 31) << 3, (p >> 10 & 31) << 3].map(|v| v as u8));
    }
    std::fs::write(&a[3], out).unwrap();
}
