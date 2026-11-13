#![no_main]
//! A decoded AI block of the imported game (`fight::br1::ai::Br1Ai`, as a save state or a replay's snapshot carries it):
//! its fields from the input's first 0x60 bytes, the rest the fighter's ANM; a block `valid` accepts is run for
//! frames against a blank fighter, opponent and tables (every op outside the ANM must end in state 0, no panic).
use open_kumite_beast::fight::br1::ai::{Br1Ai, Ctx, Table};
use open_kumite_beast::fight::br1::object::{Block, Br1Fighter};
use open_kumite_beast::formats::tables::Tables;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 0x60 {
        return;
    }
    let h = |o: usize| i16::from_le_bytes([data[o], data[o + 1]]);
    let w = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let mut ai = Br1Ai {
        idx: data[0],
        pad: h(2) as u16,
        level: h(4) as u16,
        flags: h(6) as u16,
        pc: w(8),
        state: h(12),
        t2a: h(14),
        t2c: h(16),
        action: w(18) as i32,
        band: w(22) as i32,
        res: w(26) as i32,
        regs: std::array::from_fn(|k| h(30 + 2 * k)),
        stack: std::array::from_fn(|k| w(0x3E + 4 * k)),
        depth: h(0x5E) as u16,
    };
    if !ai.valid() {
        return;
    }
    let (me, opp, tables, rnd) = (Br1Fighter::default(), Block::default(), Tables::default(), Table::default());
    let c = Ctx { me: &me, opp: &opp, opp_hips: 0, anm: &data[0x60..], tables: &tables, rnd: &rnd, timer: 0 };
    for _ in 0..64 {
        ai.think(&c);
    }
});
