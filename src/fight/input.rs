//! Input: pad mapping, per-frame processing, button timing and the command matcher.
//!
//! State: none of its own: updates the serialised `Fighter`
//!
//! Input: pad mapping, per-frame processing, button timing and the command matcher.
//! The pad mapping, the button timing, the command matcher, the chain programs and the reset helpers.

use crate::fight::fighter::*;
use crate::formats::act_ani::Ani;

/// PS1 pad bits (byte-swapped word as the game stores it; the menus read the same layout from the
/// pad words). After the facing swap a fighter's RIGHT / LEFT are FORWARD / BACK.
pub mod pad {
    pub const UP: u16 = 0x1000;
    pub const RIGHT: u16 = 0x2000;
    pub const DOWN: u16 = 0x4000;
    pub const LEFT: u16 = 0x8000;
    pub const FORWARD: u16 = RIGHT;
    pub const BACK: u16 = LEFT;
    pub const BACK_HELD: u16 = 0x0400; // set while BACK is held (guard)
    pub const START: u16 = 0x0800;
    pub const SELECT: u16 = 0x0100;
    pub const SQUARE: u16 = 0x0080; // punch
    pub const CROSS: u16 = 0x0040; // kick
    pub const CIRCLE: u16 = 0x0020; // beast / transform
    pub const TRIANGLE: u16 = 0x0010;
    pub const R1: u16 = 0x0008;
    pub const L1: u16 = 0x0004;
    pub const R2: u16 = 0x0002;
    pub const L2: u16 = 0x0001;
}

/// Directions pass through, each button ORs in its configured logical bits.
pub fn map_pad(raw_pad: u16, cfg: &[u16; 8]) -> u16 {
    let mut v = raw_pad & 0xF000;
    for (bit, logical) in [
        (pad::SQUARE, cfg[0]),
        (pad::CROSS, cfg[1]),
        (pad::CIRCLE, cfg[2]),
        (pad::TRIANGLE, cfg[3]),
        (pad::R1, cfg[4]),
        (pad::R2, cfg[5]),
        (pad::L1, cfg[6]),
        (pad::L2, cfg[7]),
    ] {
        if raw_pad & bit != 0 {
            v |= logical;
        }
    }
    v
}

// ---- reset helpers ----

/// Button counters.
pub fn reset_buttons(f: &mut Fighter) {
    f.btn_hold = [0; 16];
    f.btn_since = [100; 16];
}

/// Command matcher slots.
pub fn reset_cmd_slots(f: &mut Fighter) {
    f.cmd_slots = [[0; 4]; 32];
}

/// Chain state (+0x7C..+0x7E) and the chain slots.
pub fn reset_chain(f: &mut Fighter) {
    f.chain_pos = 0;
    f.chain_len = 0;
    f.chain_sel = -1;
    reset_chain_slots(f);
}

/// Pc / count / move of every chain slot.
pub fn reset_chain_slots(f: &mut Fighter) {
    for s in f.chain_slots.iter_mut() {
        s[0] = 0;
        s[1] = 0;
        s[2] = 0;
    }
}

/// Run the chain programs (ANI header word 4: list of program offsets, one per slot).
/// Ops (u16 words): 0 end (reset all slots), 1 wait for the current attack (`n | 0x4000` = list of n),
/// 2 input during the attack (hold count, source, mask, value), 3 queue follow-up `a` (0x2000 = only once
/// per chain), 4 window (start, end in frames). A queued follow-up goes to +0x3C/+0x5C for the decision.
pub fn chains(f: &mut Fighter, ani: &Ani) {
    let b = &ani.bytes;
    let rd = |o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0);
    let list = u32::from_le_bytes(b[16..20].try_into().unwrap()) as usize;
    let mut found: i16 = -1;
    let mut found_wait: i16 = 0;
    let mut reset_all = false;
    if f.flags[F5C] & 0x40 == 0 && f.chain_sel < 0 {
        f.s_3a = 0;
        f.chain_pos = 0;
        f.chain_len = 0;
    }
    f.flags[F6C] &= 0xFCFF_FFFF;
    let mut cur = f.s_14;
    if f.chain_sel >= 0 {
        f.flags[F6C] |= 0x100_0000;
        cur = f.chain_moves[f.chain_sel as usize];
    }
    let chain_idle = |f: &Fighter| f.chain_pos == f.chain_len && f.flags[F5C] & 0x40 == 0;
    let mut slot = 0usize;
    let mut t = 0usize; // slot index (t6) – runs ahead of `slot` after a completed chain
    while t < 36 {
        let prog = u32::from_le_bytes(b.get(list + 4 * slot..list + 4 * slot + 4).map(|s| s.try_into().unwrap()).unwrap_or([0; 4])) as usize;
        if prog == 0 {
            break;
        }
        let mut advance = true;
        'op: loop {
            let pc = f.chain_slots[slot][0] as usize;
            let at = |k: usize| rd(prog + 2 * (pc + k));
            match at(0) {
                0 => {
                    f.chain_slots[slot][0] = 0;
                    reset_all = true;
                }
                1 => {
                    if (f.flags[F58] as i32) >= 0 {
                        f.chain_slots[slot][0] = 0;
                        break 'op;
                    }
                    let a3 = at(1);
                    let (first, count) = if a3 & 0x4000 != 0 { (1, (a3 & 0xFFF) as usize + 1) } else { (0, 1) };
                    let mut hit = None;
                    for k in first..count {
                        let e = at(k + 1);
                        if e & 0x1000 != 0 && !chain_idle(f) {
                            continue;
                        }
                        if (e & 0xFFF) as i16 == cur {
                            hit = Some(e);
                            break;
                        }
                    }
                    match hit {
                        Some(e) => {
                            let s = &mut f.chain_slots[slot];
                            s[2] = e as i16;
                            s[3] = -1;
                            s[4] = 0;
                            s[0] = (pc + 1 + count) as i16;
                            s[1] = 0;
                        }
                        None => f.chain_slots[slot][0] = 0,
                    }
                }
                2 => {
                    let m = f.chain_slots[slot][2] as u16;
                    if (f.flags[F58] as i32) >= 0 || (m & 0xFFF) as i16 != cur || (m & 0x1000 != 0 && !chain_idle(f)) {
                        f.chain_slots[slot][0] = 0;
                        break 'op;
                    }
                    let start = f.chain_slots[slot][3];
                    if start >= 0 && f.chain_sel < 0 && start < f.o_bc {
                        break 'op; // window not open yet
                    }
                    f.flags[F6C] |= 0x200_0000;
                    let v = match at(2) {
                        0 | 1 => f.held,
                        2 => f.pressed,
                        3 => f.recent,
                        4 => f.long,
                        5 => f.dbl,
                        6 => (f.held & 0xF000) | (f.pressed & 0xE8),
                        7 => {
                            let p = f.pressed & 0xE8;
                            if p != 0xC0 && p != 0x88 {
                                break 'op;
                            }
                            f.recent
                        }
                        _ => 0,
                    };
                    let v = if !f.beast() && f.gauge < 0x1E00 { v & 0xFFDF } else { v };
                    if v & at(3) != at(4) {
                        break 'op;
                    }
                    f.chain_slots[slot][1] += 1;
                    if (f.chain_slots[slot][1] as i32) < at(1) as i16 as i32 {
                        break 'op;
                    }
                    f.chain_slots[slot][0] = (pc + 5) as i16;
                    f.chain_slots[slot][1] = 0;
                    continue 'op;
                }
                3 => {
                    reset_all = false;
                    let a2 = at(1);
                    if a2 & 0x2000 != 0 && f.chain_moves[..f.chain_len as usize].contains(&((a2 & 0xFFF) as i16)) {
                        f.chain_slots[slot][0] = 0;
                        break 'op;
                    }
                    let mv = (a2 & 0xFFF) as i16;
                    found_wait = f.chain_slots[slot][4];
                    let s = &mut f.chain_slots[slot];
                    s[2] = mv;
                    s[3] = -1;
                    s[4] = 0;
                    s[0] = (pc + 2) as i16;
                    s[1] = 0;
                    found = mv;
                    if at(2) == 0 {
                        // the program ends here: the next pass meets its end op (all slots reset)
                        advance = false;
                    }
                }
                4 => {
                    let s = &mut f.chain_slots[slot];
                    s[3] = (at(1) as i16).wrapping_mul(16);
                    s[4] = (at(2) as i16).wrapping_mul(16);
                    s[0] = (pc + 3) as i16;
                    s[1] = 0;
                }
                _ => {}
            }
            break 'op;
        }
        if advance {
            slot += 1;
        }
        t += 1;
    }
    if reset_all {
        for s in f.chain_slots.iter_mut() {
            s[0] = 0;
        }
    }
    if found >= 0 {
        if f.flags[F5C] & 0x40 == 0 && f.chain_sel < 0 {
            f.s_3a = f.o_da as i16;
        }
        let n = f.chain_len as usize;
        f.chain_moves[n] = found;
        f.chain_wait[n] = found_wait;
        f.chain_sel = f.chain_len as i8;
        f.chain_len = ((f.chain_len as usize + 1) % 16) as u8;
    }
}

/// Generic candidate (+0x18) and command slots.
pub fn reset_generic(f: &mut Fighter) {
    f.cand_generic.flags = 0;
    f.cand_generic.mv = 0;
    f.cand_generic.life = 0;
    reset_cmd_slots(f);
}

/// Attack / throw candidate (+0x20).
pub fn reset_attack(f: &mut Fighter) {
    f.cand_attack.flags = 0;
    f.cand_attack.mv = -1;
    f.cand_attack.life = 0;
    f.cand_attack.prio = -1;
}

/// Per-frame input of one fighter (human-controlled path).
/// `logical` = mapped pad, `swap` = the fighter faces left on screen (Left/Right are exchanged).
/// `cpu`: the pad comes from the AI routine (already facing-relative: the mapping is skipped).
pub fn process(f: &mut Fighter, ani: &Ani, opp: &Fighter, logical: u16, swap: bool, cpu: bool) {
    let mut a2 = logical;
    if swap && !cpu {
        let mut v = a2 & 0x5FFF;
        if a2 & 0x8000 != 0 {
            v |= 0x2000;
        }
        if a2 & 0x2000 != 0 {
            v |= 0x8000;
        }
        a2 = v;
    }
    if !cpu && f.flags[F60] & 0x40_0000 == 0 && f.flags[F54] & 1 != 0 && f.o_00 & 0x80 == 0 {
        a2 |= 0x800;
    }
    if a2 & 0x8000 != 0 {
        a2 |= 0x400;
    }
    if (f.flags[F60] as i32) < 0 {
        a2 &= 0x0FFF;
    }
    if f.flags[F60] & 0x2_0000 != 0 {
        a2 &= 0xF000;
    }
    if swap {
        a2 |= 0x200;
    }
    // the replay recorder logs this pad; during a replay the logged pad replaces it (after the masks)
    f.pad_record = a2;
    if let Some(p) = f.pad_override {
        a2 = p;
    }
    f.prev = f.held;
    f.held = a2;
    f.pressed = a2 & (a2 ^ f.prev);

    // candidate lifetimes
    if f.cand_generic.life != 0 {
        f.cand_generic.life -= 1;
        if f.cand_generic.life == 0 {
            f.cand_generic.mv = -1;
        }
    }
    if f.cand_cancel.life != 0 {
        f.cand_cancel.life -= 1;
        if f.cand_cancel.life == 0 {
            f.cand_cancel.mv = -1;
        }
    }
    if f.cand_attack.life != 0 && f.cand_attack.flags & 0x8000 == 0 {
        f.cand_attack.life -= 1;
        if f.cand_attack.life == 0 {
            f.cand_attack.mv = -1;
            f.cand_attack.prio = -1;
        }
    }
    if (f.flags[F58] as i32) >= 0 && f.chain_pos != f.chain_len {
        f.chain_pos = 0;
        f.chain_len = 0;
        f.chain_sel = -1;
    }
    // word at +0x78 & 0x880000 == 0x80000  ⇔  (pressed & 0x88) == 0x08
    if f.pressed & 0x88 == 0x08 {
        f.flags[F60] |= 0x4_0000;
        reset_chain_slots(f);
        f.flags[F6C] &= !0x200_0000;
    } else if (f.flags[F58] as i32) >= 0 {
        f.flags[F60] &= !0x4_0000;
    }
    button_timing(f);
    if f.flags[F60] & 0x4_0000 == 0 {
        chains(f, ani);
    }
    match_commands(f, ani, opp);
}

/// Per-button hold / since-press counters → recent (+0x80), double tap (+0x82), long hold (+0x84).
pub fn button_timing(f: &mut Fighter) {
    let (held, pressed) = (f.held, f.pressed);
    let (mut dbl, mut recent, mut long) = (0u16, 0u16, 0u16);
    for b in 0..16 {
        let bit = 1u16 << b;
        if pressed & bit != 0 {
            if !(f.since_any_press < f.btn_since[b]) && f.btn_since[b] < 12 {
                dbl |= bit;
            }
            f.btn_since[b] = 0;
            f.btn_hold[b] = 0;
        } else {
            if held & bit != 0 {
                f.btn_hold[b] += 1;
                if f.btn_hold[b] < 17 && f.dbl & bit != 0 {
                    dbl |= bit;
                }
            } else {
                f.btn_hold[b] = 0;
            }
            f.btn_since[b] += 1;
        }
        if f.btn_since[b] < 5 {
            recent |= bit;
        }
        if f.btn_hold[b] >= 3 {
            long |= bit;
        }
    }
    if pressed != 0 {
        f.since_any_press = 0;
    } else {
        f.since_any_press += 1;
    }
    f.long = long;
    f.recent = recent;
    f.dbl = dbl;
}

/// One 16-byte command record (ANI header word 3).
struct Record {
    flags: u32,
    mv: i16,
    life: i16,
    window: i16,
    param: i16,
    seq: usize,
}

fn record(ani: &Ani, i: usize) -> Option<Record> {
    let base = u32::from_le_bytes(ani.bytes[12..16].try_into().unwrap()) as usize + 16 * i;
    if base + 16 > ani.bytes.len() {
        return None;
    }
    let flags = u32::from_le_bytes(ani.bytes[base..base + 4].try_into().unwrap());
    if flags == 0 {
        return None;
    }
    Some(Record {
        flags,
        mv: ani.i16(base + 4),
        life: ani.i16(base + 6),
        window: ani.i16(base + 8),
        param: ani.i16(base + 10),
        seq: u32::from_le_bytes(ani.bytes[base + 12..base + 16].try_into().unwrap()) as usize,
    })
}

/// Advance every command's input sequence and register completed commands as candidates.
pub fn match_commands(f: &mut Fighter, ani: &Ani, opp: &Fighter) {
    if f.flags[F58] & 0x8000 != 0 {
        return;
    }
    // (records flagged 0x100000 also need a global flag; treated as set)
    for t4 in 0..32usize {
        let Some(r) = record(ani, t4) else { return };
        'step: loop {
            let step = f.cmd_slots[t4][0];
            let idx = (step - 1).max(0) as usize;
            let e = r.seq + idx * 10;
            if e + 10 > ani.bytes.len() {
                break 'step;
            }
            let (hold_frames, miss_limit, source, mask, value) = (ani.i16(e), ani.i16(e + 2), ani.i16(e + 4), ani.u16(e + 6), ani.u16(e + 8));
            let t3: u16 = match source {
                0 => {
                    // sequence complete
                    f.cmd_slots[t4][0] = 0;
                    register(f, opp, &r, t4);
                    break 'step;
                }
                1 => f.held,
                2 => f.pressed,
                3 => f.recent,
                4 => f.long,
                5 => f.dbl,
                6 => (f.held & 0xF000) | (f.pressed & 0xE8),
                7 => {
                    let p = f.pressed & 0xE8;
                    if p != 0xC0 && p != 0x88 {
                        // no match
                        if miss(f, t4, miss_limit, r.window) {
                            continue;
                        }
                        break 'step;
                    }
                    f.recent
                }
                _ => 0, // unreachable in the data; the game reuses the previous value
            };
            let mut t3 = t3;
            if !f.beast() && f.gauge < 0x1E00 {
                t3 &= 0xFFDF; // Circle only counts with a full gauge in human form
            }
            if t3 & mask == value {
                let slot = &mut f.cmd_slots[t4];
                if slot[0] == 0 {
                    slot[0] = 1;
                    slot[3] = 0;
                    slot[2] = 0;
                    slot[1] = 0;
                }
                slot[1] += 1;
                if slot[1] >= hold_frames {
                    slot[0] += 1;
                    slot[1] = 0;
                    slot[2] = 0;
                    continue 'step; // evaluate the next step in the same frame
                }
                tick_window(f, t4, r.window);
            } else if miss(f, t4, miss_limit, r.window) {
                continue;
            }
            break 'step;
        }
    }
}

/// No-match path: count a miss, then the window timer. Returns false always (kept for symmetry).
fn miss(f: &mut Fighter, t4: usize, miss_limit: i16, window: i16) -> bool {
    let slot = &mut f.cmd_slots[t4];
    if slot[0] == 0 {
        return false;
    }
    slot[2] += 1;
    if slot[2] >= miss_limit {
        slot[0] = 0;
    }
    tick_window(f, t4, window);
    false
}

/// Total time allowed for the whole command.
fn tick_window(f: &mut Fighter, t4: usize, window: i16) {
    let slot = &mut f.cmd_slots[t4];
    slot[3] += 1;
    if slot[3] >= window {
        slot[0] = 0;
    }
}

/// Filter a completed command and store it in a candidate block.
fn register(f: &mut Fighter, opp: &Fighter, r: &Record, t4: usize) {
    let fl = r.flags;
    if fl & 0x8_0000 != 0 {
        if !f.beast() {
            return;
        }
    } else if fl & 0x40 != 0 && !f.beast() && (fl & 0x80 != 0 || f.gauge < 0x1E00) {
        return;
    }
    let f58 = f.flags[F58];
    if fl & 0x80 != 0 {
        // throws
        if fl & 0x3000 != 0x3000 {
            let opp_air = opp.flags[F5C] & 0x40_0000 != 0;
            if fl & 0x1000 != 0 && opp_air {
                return;
            }
            if fl & 0x2000 != 0 && !opp_air {
                return;
            }
        }
        if f58 & 0x800 != 0 || f.cand_attack.flags & 0x8000 != 0 || !(f.cand_attack.prio < t4 as i16) {
            return;
        }
        set_attack(f, r, t4);
    } else if fl & 0x100 != 0 {
        if f58 & 0x800 == 0 || f.cand_attack.flags & 0x8000 != 0 || !(f.cand_attack.prio < t4 as i16) {
            return;
        }
        set_attack(f, r, t4);
    } else if fl & 0x1_0000 != 0 {
        // cancels
        if f58 & 0x1000 == 0 {
            return;
        }
        let ok = if fl & 0x2_0000 != 0 {
            f58 & 0x2800 == 0
        } else if fl & 0x4_0000 != 0 {
            f58 & 0x8000_2010 == 0x2000
        } else {
            f58 & 0x800 != 0
        };
        if !ok || !(f.cand_cancel.mv < r.mv) {
            return;
        }
        f.cand_cancel.flags = fl;
        f.cand_cancel.mv = r.mv;
        f.cand_cancel.life = r.life;
        let mut p = r.param;
        let f54 = f.flags[F54];
        if f54 & 8 != 0 {
            p /= 2;
        } else if f54 & 0x10 != 0 {
            p = (p as i32 * 7 / 8) as i16;
        }
        f.cand_cancel.param = p;
    } else {
        if !(f.cand_generic.mv < r.mv) {
            return;
        }
        f.cand_generic.flags = fl;
        f.cand_generic.mv = r.mv;
        f.cand_generic.life = r.life;
    }
}

fn set_attack(f: &mut Fighter, r: &Record, t4: usize) {
    f.cand_attack.flags = r.flags;
    f.cand_attack.mv = r.mv;
    f.cand_attack.life = r.life;
    f.cand_attack.prio = t4 as i16;
    f.cand_attack.param = r.param;
}
