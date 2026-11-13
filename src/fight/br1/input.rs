//! The imported game's per-frame input: the pad word, the button timing, the chain programs, the command matcher and the
//! candidate lifetimes.
//!
//! State: none of its own: updates the serialised `Br1Fighter`s (F, S and `Input`)
//!
//! The main game's machine (`fight/input.rs`) with the imported game's differences: the raw pad is mapped inline with
//! the player's button config, 0x400 is the "may guard" bit (no back → 0x400 rule), no throw-escape latch, the chain
//! programs always run, the candidate lifetimes count down after the matching, the circle bit is dropped whenever the
//! fighter is human (F.5 == 0), a recent press lasts 9 frames, there is no source 7. The command and chain tables are
//! the fighter's ANM's (header words 3 / 4).

use super::object::Br1Fighter;
use crate::formats::br1_anm::Anm;

/// The raw pad bits, in the order of the button config's entries.
pub const RAW_BITS: [u16; 8] = [0x80, 0x40, 0x20, 0x10, 0x04, 0x01, 0x08, 0x02];

/// The VS default button config (the table `input.default_buttons` of the imported game's data): □ P, × K, ○ the beast
/// button, △ the rave, L1 0x08, L2 0x04, R1 0x800, R2 nothing.
pub fn default_buttons(t: &crate::formats::tables::Tables) -> [u16; 8] {
    std::array::from_fn(|k| t.table("input.default_buttons").u16(2 * k))
}

/// What the input reads besides the fighters: the raw pad (`G.68 + 2·player`), the camera's angle word
/// (`*(G.B8) + 2`) and, for a CPU fighter, the AI's pad (`*(F.34) + 6`, the AI being S10's).
#[derive(Clone, Copy, Debug, Default)]
pub struct Pad {
    pub raw: u16,
    pub cam: i16,
    pub ai: u16,
    /// engine addition (the import S7): the pad the main game's KO replay plays back instead (its recorder hook
    /// at the place of the imported game's)
    pub replay: Option<u16>,
}

/// Fighter `me`'s pad word and its processing. Returns the pad the KO replay's recorder sees.
pub fn process(fs: &mut [Br1Fighter; 2], me: usize, anm: &Anm, pad: Pad) -> u16 {
    let f = &mut fs[me];
    let ang = pad.cam as i32 + if f.f.f54 & 0x40 != 0 || f.f.f58 & 0x10 != 0 { f.anim.yaw_base } else { f.f.yaw } as i32;
    let swap = (ang << 20) >> 20 < 0;
    let mut a2: u16 = 0;
    if f.f.status & 0x80 == 0 {
        a2 = pad.raw & 0xF000;
        for (k, &bit) in RAW_BITS.iter().enumerate() {
            if pad.raw & bit != 0 {
                a2 |= f.input.buttons[k];
            }
        }
    }
    if f.f.cpu != 0 {
        // KO'd (F.0 & 0x8080, the low halfword's) 0, else the AI's pad (after the AI ran)
        a2 = if (f.f.status as u32 | (f.f.cpu as u32) << 8) & 0x8080 != 0 { 0 } else { pad.ai };
    } else {
        if swap {
            let mut v = a2 & 0x5FFF;
            if a2 & 0x8000 != 0 {
                v |= 0x2000;
            }
            if a2 & 0x2000 != 0 {
                v |= 0x8000;
            }
            a2 = v;
        }
        if f.f.f58 & 0x40_0000 == 0 && f.f.f4c & 1 != 0 && f.f.status & 0x80 == 0 {
            a2 |= 0x400;
        }
    }
    // the masks
    if (f.f.f58 as i32) < 0 {
        a2 &= 0x0FFF;
    }
    if f.f.f58 & 0x2_0000 != 0 {
        a2 &= 0xF400;
    }
    if swap {
        a2 |= 0x200;
    }
    // the KO replay's recorder passes the pad through while recording; its playback (the round flow's:
    // the main game's in a main-game match) gives the recorded pad
    let rec = a2;
    let a2 = pad.replay.unwrap_or(a2);
    let prev = f.f.held;
    f.f.held = a2;
    f.f.prev = prev;
    f.f.pressed = a2 & (a2 ^ prev);
    button_timing(f);
    chains(fs, me, anm);
    match_commands(fs, me, anm);
    // the candidates' lifetimes
    let s = &mut fs[me].s;
    if s.cand_life != 0 {
        s.cand_life = s.cand_life.wrapping_sub(1);
        if s.cand_life == 0 {
            s.cand_mv = -1;
        }
    }
    if s.cancel_life != 0 {
        s.cancel_life = s.cancel_life.wrapping_sub(1);
        if s.cancel_life == 0 {
            s.cancel_mv = -1;
        }
    }
    if s.att_life != 0 && s.att_flags & 0x8000 == 0 {
        s.att_life = s.att_life.wrapping_sub(1);
        if s.att_life == 0 {
            s.att_mv = -1;
            s.att_prio = -1;
        }
    }
    let f = &mut fs[me];
    if (f.f.f50 as i32) >= 0 && f.s.chain_pos != f.s.chain_len {
        f.s.chain_len = 0;
        f.s.chain_pos = 0;
        f.s.chain_sel = 0xFF;
    }
    rec
}

/// Per pad bit the hold and since-press counters → F.7A double tap, F.78 recent (pressed within 9
/// frames), F.7C long (held 3 frames).
pub fn button_timing(f: &mut Br1Fighter) {
    let (held, pressed) = (f.f.held, f.f.pressed);
    let (mut dbl, mut recent, mut long) = (0u16, 0u16, 0u16);
    let c = &mut f.input;
    for b in 0..16 {
        let bit = 1u16 << b;
        if pressed & bit != 0 {
            if c.since[b] < 12 {
                dbl |= bit;
            }
            c.since[b] = 0;
            c.hold[b] = 0;
        } else {
            if held & bit != 0 {
                c.hold[b] = c.hold[b].wrapping_add(1);
                if c.hold[b] < 17 && f.f.dbl & bit != 0 {
                    dbl |= bit;
                }
            } else {
                c.hold[b] = 0;
            }
            c.since[b] = c.since[b].wrapping_add(1);
        }
        if c.since[b] < 9 {
            recent |= bit;
        }
        if c.hold[b] >= 3 {
            long |= bit;
        }
    }
    f.f.long = long;
    f.f.recent = recent;
    f.f.dbl = dbl;
}

/// A command / chain input source: 1 held, 2 pressed, 3 recent, 4 long, 5 double tap, 6 directions held with the
/// buttons pressed; None for the others (the code keeps its register's last value).
fn source(f: &Br1Fighter, src: i32) -> Option<u16> {
    Some(match src {
        1 => f.f.held,
        2 => f.f.pressed,
        3 => f.f.recent,
        4 => f.f.long,
        5 => f.f.dbl,
        6 => (f.f.held & 0xF000) | (f.f.pressed & 0xE0),
        _ => return None,
    })
}

/// The engine's bound on a command record's / chain slot's evaluations in one frame (above any file's 64 steps).
const REEVALS: usize = 256;

/// Advance every command record's sequence (`S.8` = ANM header word 3), register the completed ones as
/// candidates.
pub fn match_commands(fs: &mut [Br1Fighter; 2], me: usize, anm: &Anm) {
    if fs[me].f.f50 & 0x8000 != 0 {
        return;
    }
    let b = &anm.bytes;
    let base = anm.header[3] as usize;
    let rd16 = |o: usize| o.checked_add(2).and_then(|e| b.get(o..e)).map_or(0, |s| i16::from_le_bytes([s[0], s[1]]));
    let rd32 = |o: usize| o.checked_add(4).and_then(|e| b.get(o..e)).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    // t3: the source word, kept from record to record (a source past 6 reuses it)
    let mut t3: u16 = 0;
    for t4 in 0..32usize {
        let r = base.wrapping_add(16 * t4);
        let flags = rd32(r);
        if flags == 0 {
            return;
        }
        let (mv, life, window, param, seq) =
            (rd16(r.wrapping_add(4)), rd16(r.wrapping_add(6)), rd16(r.wrapping_add(8)), rd16(r.wrapping_add(10)), rd32(r.wrapping_add(12)) as usize);
        // engine guard: at most REEVALS evaluations of a record per frame (the game loops while steps complete;
        // a file's commands have ≤ 64 steps, a corrupt one could cycle forever)
        for _ in 0..REEVALS {
            let f = &mut fs[me];
            let step = f.input.cmd[t4][0] as i32 - 1;
            let e = seq.wrapping_add(10 * step.max(0) as usize);
            let src = rd16(e.wrapping_add(4)) as i32;
            if src == 0 {
                f.input.cmd[t4][0] = 0;
                register(fs, me, t4, flags, mv, life, param);
                break;
            }
            if let Some(v) = source(f, src) {
                t3 = v;
            }
            if f.f.form == 0 {
                t3 &= 0xFFDF;
            }
            let slot = &mut f.input.cmd[t4];
            if t3 & rd16(e.wrapping_add(6)) as u16 == rd16(e.wrapping_add(8)) as u16 {
                if slot[0] == 0 {
                    slot[0] = 1;
                    slot[3] = 0;
                    slot[2] = 0;
                    slot[1] = 0;
                }
                slot[1] = slot[1].wrapping_add(1);
                if slot[1] >= rd16(e) {
                    slot[0] = slot[0].wrapping_add(1);
                    slot[1] = 0;
                    slot[2] = 0;
                    continue;
                }
            } else {
                if slot[0] == 0 {
                    break;
                }
                slot[2] = slot[2].wrapping_add(1);
                if slot[2] >= rd16(e.wrapping_add(2)) {
                    slot[0] = 0;
                }
            }
            // the whole command's window
            slot[3] = slot[3].wrapping_add(1);
            if slot[3] >= window {
                slot[0] = 0;
            }
            break;
        }
    }
}

/// A completed command `t4` into its candidate block, by the record's flags.
fn register(fs: &mut [Br1Fighter; 2], me: usize, t4: usize, fl: u32, mv: i16, life: i16, param: i16) {
    let opp_back = fs[1 - me].f.f54 & 0x40_0000 != 0;
    let f = &mut fs[me];
    if fl & 0x40 != 0 && f.f.form & 1 == 0 {
        return;
    }
    let attack = if fl & 0x80 != 0 {
        // throws: 0x1000 from the front, 0x2000 from behind (the opponent's F.54 0x400000: its opponent behind it),
        // never from the air
        if fl & 0x1000 != 0 && opp_back || fl & 0x2000 != 0 && !opp_back || f.f.f50 & 0x800 != 0 {
            return;
        }
        true
    } else if fl & 0x100 != 0 {
        if f.f.f50 & 0x800 == 0 {
            return;
        }
        true
    } else {
        false
    };
    let s = &mut f.s;
    if attack {
        if s.att_flags & 0x8000 != 0 || (s.att_prio as i32) >= t4 as i32 {
            return;
        }
        s.att_flags = fl;
        s.att_mv = mv;
        s.att_life = life;
        s.att_prio = t4 as i16;
        s.att_param = param;
    } else if fl & 0x1_0000 != 0 {
        let f50 = f.f.f50;
        if f50 & 0x1000 == 0 {
            return;
        }
        let ok = if fl & 0x2_0000 != 0 {
            f50 & 0x2800 == 0
        } else if fl & 0x4_0000 != 0 {
            f50 & 0x8000_2010 == 0x2000
        } else {
            f50 & 0x800 != 0
        };
        if !ok || s.cancel_mv >= mv {
            return;
        }
        s.cancel_flags = fl;
        s.cancel_mv = mv;
        s.cancel_life = life;
        s.cancel_param = param;
        if f.f.f4c & 8 != 0 {
            s.cancel_param = param / 2;
        }
    } else {
        if s.cand_mv >= mv {
            return;
        }
        s.cand_flags = fl;
        s.cand_mv = mv;
        s.cand_life = life;
    }
}

/// Run the chain programs (`S.C` = ANM header word 4: their offsets until 0, one slot each). Ops (u16
/// words): 0 end (all slots reset), 1 wait for the current attack (`n | 0x4000` = a list of n), 2 input (hold count,
/// source, mask, value) inside the window, 3 queue follow-up (0x2000 = once per chain), 4 the window (start, end in
/// frames). A queued follow-up goes into S.44.. / S.64.. for the decision.
pub fn chains(fs: &mut [Br1Fighter; 2], me: usize, anm: &Anm) {
    let b = &anm.bytes;
    let rd = |o: usize| o.checked_add(2).and_then(|e| b.get(o..e)).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]));
    let rd32 = |o: usize| o.checked_add(4).and_then(|e| b.get(o..e)).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let list = anm.header[4] as usize;
    let f = &mut fs[me];
    let (mut found, mut found_end): (i16, i16) = (-1, 0);
    let mut reset_all = false;
    if f.f.f54 & 0x40 == 0 && (f.s.chain_sel as i8) < 0 {
        f.s.damage_sum = 0;
        f.s.chain_pos = 0;
        f.s.chain_len = 0;
    }
    f.f.f64 &= 0xFCFF_FFFF;
    let mut cur = f.s.attack_index;
    if (f.s.chain_sel as i8) >= 0 {
        f.f.f64 |= 0x100_0000;
        cur = f.s.chain_queue.get(f.s.chain_sel as usize).copied().unwrap_or(0);
    }
    let idle = |f: &Br1Fighter| f.s.chain_pos == f.s.chain_len && f.f.f54 & 0x40 == 0;
    // t5: the source word, kept from slot to slot (a source past 6 reuses it)
    let mut t5: u16 = 0;
    let (mut prog_k, mut slot) = (0usize, 0usize);
    for _ in 0..32 {
        let prog = rd32(list.wrapping_add(4 * prog_k)) as usize;
        if prog == 0 {
            break;
        }
        // whether t7 / the slot advance (op 3 at the program's end stays on the slot: the next pass meets its end op)
        let mut advance = true;
        // engine guard: op 2's re-evaluations of the slot bounded as the commands' (`REEVALS`)
        let mut evals = 0;
        'op: loop {
            let pc = f.input.chain[slot][0] as i32;
            let at = |k: i32| rd(prog.wrapping_add((2 * (pc + k)) as usize));
            match at(0) {
                0 => {
                    f.input.chain[slot][0] = 0;
                    reset_all = true;
                }
                1 => {
                    if (f.f.f50 as i32) >= 0 {
                        f.input.chain[slot][0] = 0;
                        break 'op;
                    }
                    let a3 = at(1);
                    let (first, count) = if a3 & 0x4000 != 0 { (1, (a3 & 0xFFF) as i32 + 1) } else { (0, 1) };
                    let mut hit = None;
                    for k in first..count {
                        let e = at(k + 1);
                        if e & 0x1000 != 0 && !idle(f) {
                            continue;
                        }
                        if cur == (e & 0xFFF) as i16 {
                            hit = Some(e);
                            break;
                        }
                    }
                    let s = &mut f.input.chain[slot];
                    match hit {
                        Some(e) => {
                            s[2] = e as i16;
                            s[3] = -1;
                            s[4] = 0;
                            s[0] = (pc + 1 + count) as i16;
                            s[1] = 0;
                        }
                        None => s[0] = 0,
                    }
                }
                2 => {
                    let m = f.input.chain[slot][2] as u16;
                    if (f.f.f50 as i32) >= 0 || (m & 0xFFF) as i16 != cur || m & 0x1000 != 0 && !idle(f) {
                        f.input.chain[slot][0] = 0;
                        break 'op;
                    }
                    let start = f.input.chain[slot][3];
                    if start >= 0 && (f.s.chain_sel as i8) < 0 && start < f.f.recovery {
                        break 'op;
                    }
                    f.f.f64 |= 0x200_0000;
                    if let Some(v) = source(f, at(2) as i32) {
                        t5 = v;
                    }
                    if f.f.form == 0 {
                        t5 &= 0xFFDF;
                    }
                    if t5 & at(3) != at(4) {
                        break 'op;
                    }
                    let s = &mut f.input.chain[slot];
                    s[1] = s[1].wrapping_add(1);
                    if (s[1] as i32) < at(1) as i32 {
                        break 'op;
                    }
                    s[0] = (pc + 5) as i16;
                    s[1] = 0;
                    evals += 1;
                    if evals < REEVALS {
                        continue 'op;
                    }
                }
                3 => {
                    reset_all = false;
                    let a2 = at(1);
                    if a2 & 0x2000 != 0 && f.s.chain_queue[..f.s.chain_len.min(16) as usize].contains(&((a2 & 0xFFF) as i16)) {
                        f.input.chain[slot][0] = 0;
                        break 'op;
                    }
                    let s = &mut f.input.chain[slot];
                    found_end = s[4];
                    s[2] = (a2 & 0xFFF) as i16;
                    s[3] = -1;
                    s[4] = 0;
                    s[0] = (pc + 2) as i16;
                    s[1] = 0;
                    found = (a2 & 0xFFF) as i16;
                    if at(2) == 0 {
                        advance = false;
                    }
                }
                4 => {
                    let s = &mut f.input.chain[slot];
                    s[3] = (at(1) as i16).wrapping_shl(4);
                    s[4] = (at(2) as i16).wrapping_shl(4);
                    s[0] = (pc + 3) as i16;
                    s[1] = 0;
                }
                _ => {}
            }
            break 'op;
        }
        if advance {
            prog_k += 1;
            slot += 1;
        }
    }
    if reset_all {
        for s in f.input.chain.iter_mut() {
            s[0] = 0;
        }
    }
    if found >= 0 {
        if f.f.f54 & 0x40 == 0 && (f.s.chain_sel as i8) < 0 {
            f.s.damage_sum = f.f.attack_damage as i16;
        }
        let n = f.s.chain_len as usize & 0xFF;
        if let Some(q) = f.s.chain_queue.get_mut(n) {
            *q = found;
        }
        if let Some(q) = f.s.chain_queue.get_mut(n + 16) {
            *q = found_end;
        }
        f.s.chain_sel = f.s.chain_len;
        f.s.chain_len = ((f.s.chain_len as i32 + 1) & 0xF) as u8;
    }
}

/// The command slots of the player cleared.
pub fn reset_commands(f: &mut Br1Fighter) {
    f.input.cmd = [[0; 4]; 32];
}

/// The chain slots' pc, count and move cleared.
pub fn reset_chain_slots(f: &mut Br1Fighter) {
    for s in f.input.chain.iter_mut() {
        s[0] = 0;
        s[1] = 0;
        s[2] = 0;
    }
}

/// The chain queue (S.84 / S.85 = 0, S.86 = −1) and the chain slots reset.
pub fn reset_chain(f: &mut Br1Fighter) {
    f.s.chain_len = 0;
    f.s.chain_pos = 0;
    f.s.chain_sel = 0xFF;
    reset_chain_slots(f);
}

/// The generic candidate cleared.
pub fn reset_generic(f: &mut Br1Fighter) {
    f.s.cand_flags = 0;
    f.s.cand_mv = 0;
    f.s.cand_life = 0;
}

/// The attack candidate cleared.
pub fn reset_attack(f: &mut Br1Fighter) {
    f.s.att_flags = 0;
    f.s.att_mv = -1;
    f.s.att_life = 0;
    f.s.att_prio = -1;
}
