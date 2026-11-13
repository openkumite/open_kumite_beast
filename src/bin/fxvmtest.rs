//! Smoke test of the effect VM on every real effect script.
//!
//! Smoke test of the effect VM on the real scripts: every script of EFT.DAT (entries 80..99) and of each
//! <CHR>EFT.DAT (entries 11..20) is spawned with dummy parameters and run for 300 frames.
//! `cargo run --release --bin fxvmtest -- ../okb_export`

use open_kumite_beast::fight::fx::script_vm::Vm;

fn main() {
    let root = std::env::args().nth(1).unwrap_or("../okb_export".into());
    let rd = |p: &str| std::fs::read(format!("{root}/engine/eft/{p}")).unwrap_or_default();
    let eft = open_kumite_beast::formats::container::read(&rd("EFT.DAT"));
    let mut groups: Vec<(String, usize, Vec<Option<Vec<u8>>>)> = vec![("EFT".into(), 0, (80..100).map(|k| eft.get(k).cloned().flatten()).collect())];
    for c in open_kumite_beast::assets::roster::CHARACTERS {
        let d = open_kumite_beast::formats::container::read(&rd(&format!("{c}EFT.DAT")));
        groups.push((c.to_string(), 1, (11..21).map(|k| d.get(k).cloned().flatten()).collect()));
    }
    // dummy engine data: a position vector, an owner block and an identity matrix
    const SHARED: u32 = 0x8300_0000;
    let mut total = (0, 0, 0);
    for (name, g, scripts) in &groups {
        for (n, s) in scripts.iter().enumerate() {
            let Some(s) = s else { continue };
            if s.len() < 0x1C || u32::from_le_bytes([s[0], s[1], s[2], s[3]]) != 0x1C {
                continue;
            }
            let mut vm = Vm::new();
            vm.set_group(*g, scripts);
            if *g == 1 {
                vm.set_group(2, scripts);
            }
            vm.set_group(0, &groups[0].2);
            let mut shared = vec![0u8; 0x100];
            for (k, v) in [4096i16, 0, 0, 0, 4096, 0, 0, 0, 4096].iter().enumerate() {
                shared[0x40 + 2 * k..0x42 + 2 * k].copy_from_slice(&v.to_le_bytes());
            }
            vm.mem.map(SHARED, shared);
            let h = vm.spawn(*g, n, None, &[SHARED, SHARED + 0x20, SHARED + 0x40, 0]);
            let (mut draws, mut peak) = (0, 0);
            let mut ended = None;
            for f in 0..300 {
                vm.tick();
                draws += vm.draws.len();
                peak = peak.max(vm.live());
                if vm.live() == 0 && ended.is_none() {
                    ended = Some(f);
                }
            }
            total.0 += 1;
            if !vm.unsupported.is_empty() {
                total.1 += 1;
            }
            total.2 += draws;
            println!(
                "{name:>4} script {n:2} (bank {g}): handle {} peak tasks {peak:3} draws {draws:5} {} {}",
                h != 0,
                ended.map(|f| format!("ended at {f}")).unwrap_or_else(|| format!("still {} live", vm.live())),
                if vm.unsupported.is_empty() { String::new() } else { format!("UNSUPPORTED {:02X?}", vm.unsupported) }
            );
        }
    }
    println!("{} scripts, {} with unsupported ops, {} billboard draws", total.0, total.1, total.2);
}
