//! The link between the fight and the effect VM: the PS1 structures it reads, the effect opcodes.
//!
//! State: none of its own: steps the serialised `Match` (or the team battle's world) through `FxHost`
//!
//! The link between the fight and the effect VM: the PS1 structures the effect scripts read
//! through their parameters are kept up to date in the VM's memory every frame, at their PS1 addresses —
//! the match block and both fighters (`M_BASE` / `fighter_addr`), each fighter's model with the
//! bone matrices (`*(F+0x20)`, bone i at +0x20 + 0x50·i: 3×3 then the s32 position at +0x14), the move-script
//! state (`*(F+0x38)`, mirrored flag at +0x100), the billboard matrix `BILL` and `RING` —
//! and the move-script effect opcodes 0x16 / 0x17 / 0x18 are run.

use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F6C, F58, F60, F64};
use crate::fight::fx::host::FxHost;

pub(crate) const M_BASE: u32 = 0x1F80_0000;
const MODEL_BASE: u32 = 0x8090_0000;
const STATE_BASE: u32 = 0x8092_0000;
const GLOBALS: u32 = 0x801F_0400;
/// BILL: rot2(−pitch, yaw) of the camera, the inverse camera rotation used to billboard models
pub const BILL: u32 = 0x801F_0508;
const RING: u32 = 0x801F_06E0;
/// The callback words effect objects hold (object +0x18 of op 0x16's sparkle, +0x24 of Uriko's drive object): kept
/// as the game's values, since they are VM memory the state hashes and the scripts may read
const SPARKLE_CALLBACK: u32 = 0x8014_06D0;
const WALL_MARK_CALLBACK: u32 = 0x8013_F138;

/// Bone mirror table (left / right limbs swapped).
pub(crate) const REMAP: [u8; 17] = [0, 1, 2, 3, 4, 8, 9, 10, 5, 6, 7, 14, 15, 16, 11, 12, 13];

pub fn fighter_addr(side: usize) -> u32 {
    M_BASE + 0xE8 + 0x108 * side as u32
}

/// The script group (`ScriptGroup`) and emitter group of fighter `i`'s `<CHR>EFT` entries: P1 / P2's 1 / 2;
/// the team battle's fighters 2 / 3 (bank 1, engine addition) 5 / 6, over EffTab 100..139.
pub fn fx_group(i: usize) -> usize {
    let f = crate::fight::side::Fid::of(i);
    f.bank().vm_group(f.slot())
}

/// The address of fighter `i`'s held pad word: M.6C / M.6E (EFT_99 polls it); the team battle's fighters 2 / 3 after
/// the four fighter blocks (engine addition).
pub fn pad_addr(i: usize) -> u32 {
    if i < 2 { M_BASE + 0x6C + 2 * i as u32 } else { M_BASE + TEAM_PADS + 2 * (i as u32 - 2) }
}

/// The team battle's pad words of fighters 2 / 3 in its larger match block (engine addition).
const TEAM_PADS: u32 = 0x508;

fn model_addr(side: usize) -> u32 {
    MODEL_BASE + 0x2000 * side as u32
}

/// BONEPOS(i): the s32 world position of bone i (`MODEL + 0x34 + 0x50·i`), in 32-bit wrapping arithmetic as the
/// game's `addu`: a bone sign-extended from a halfword (op 0x17 subs 5 / 6, `arg as i16 as usize`, read back as a
/// signed word) addresses below the model as the game does.
pub fn bonepos(side: usize, bone: usize) -> u32 {
    model_addr(side).wrapping_add(0x34).wrapping_add((bone as u32).wrapping_mul(0x50))
}

/// World rotations of the posed skeleton nodes in game space (x, −y, −z of glTF), 4.12.
pub fn node_rotations(globals: &[glam::Mat4], model_xf: glam::Mat4) -> Vec<[[i32; 3]; 3]> {
    let d = glam::Mat3::from_diagonal(glam::Vec3::new(1.0, -1.0, -1.0));
    globals
        .iter()
        .map(|g| {
            let m = glam::Mat3::from_mat4(model_xf * *g);
            let n = glam::Mat3::from_cols(m.x_axis.normalize_or_zero(), m.y_axis.normalize_or_zero(), m.z_axis.normalize_or_zero());
            let r = d * n * d;
            [0, 1, 2].map(|row| [0, 1, 2].map(|col| (r.col(col)[row] * 4096.0).round() as i32))
        })
        .collect()
}

fn put16(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 2].copy_from_slice(&(v as i16).to_le_bytes());
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// The F6C bits the effect scripts changed since the last [`refresh`] go back to the fighters: the scripts set
/// their owner's requests directly (op02.1A on `*(obj+4) + 0x6C`, e.g. Uriko's drive object <CHR>EFT_11 sets
/// 0x40000000 / 0x20000000 per hit and 0x10000000 when done, which her move 0x16F and the victim's wait for).
pub fn write_back(m: &mut FxHost) {
    for i in 0..m.fighters.len() {
        let a = fighter_addr(i) + 0x6C;
        let changed = m.vm.mem.u32(a) ^ *m.f6c(i);
        if changed != 0 {
            let f = &mut m.fighters[i];
            f.flags[F6C] = (f.flags[F6C] & !changed) | (m.vm.mem.u32(a) & changed);
            *m.f6c(i) ^= changed;
        }
    }
}

/// Rebuild the engine-shared blocks in the VM memory from the match state.
pub fn refresh(m: &mut FxHost, data: &Loaded) {
    write_back(m);
    // match block with both fighters (the team battle's four, and its bank 1 pads after them: 0x510 bytes)
    let n = m.fighters.len();
    let mut mb = vec![0u8; if n > 2 { TEAM_PADS as usize + 4 } else { 0x400 }];
    put16(&mut mb, 0x82, m.vm.paused as i32);
    // held pads (M.6C / 6E): EFT_99 polls bit 0x20 of its side's word
    put16(&mut mb, 0x6C, m.pads[0] as i32);
    put16(&mut mb, 0x6E, m.pads[1] as i32);
    for i in 2..n {
        put16(&mut mb, (pad_addr(i) - M_BASE) as usize, m.pads[i] as i32);
    }
    mb[0x98] = data.stage_no as u8;
    mb[0xB1] = (m.round_state >= 0xF) as u8;
    for i in 0..n {
        let f = &m.fighters[i];
        let o = 0xE8 + 0x108 * i;
        mb[o] = f.o_00;
        mb[o + 1] = f.o_01;
        mb[o + 2] = f.player;
        mb[o + 3] = f.chr;
        mb[o + 4] = m.costume[i];
        mb[o + 5] = f.form;
        put32(&mut mb, o + 0x20, model_addr(i));
        put32(&mut mb, o + 0x2C, fighter_addr(m.opp[i]));
        put32(&mut mb, o + 0x38, STATE_BASE + 0x200 * i as u32);
        put16(&mut mb, o + 0x4E, f.yaw as i32);
        put32(&mut mb, o + 0x58, f.flags[F58]);
        put32(&mut mb, o + 0x5C, f.flags[F5C]);
        put32(&mut mb, o + 0x60, f.flags[F60]);
        put32(&mut mb, o + 0x64, f.flags[F64]);
        put32(&mut mb, o + 0x6C, f.flags[F6C]);
        put16(&mut mb, o + 0x74, f.o_74 as i32);
        put16(&mut mb, o + 0xAC, f.o_ac as i32);
        put16(&mut mb, o + 0xB0, f.o_b0 as i32);
        let f6c = f.flags[F6C];
        *m.f6c(i) = f6c;
    }
    m.vm.mem.map(M_BASE, mb);
    // models: bone matrices in game space
    for i in 0..n {
        let f = &m.fighters[i];
        let mdl = &data.fighters[i].models[f.beast() as usize];
        let mut b = vec![0u8; 0x2000];
        for (bone, &node) in mdl.joints.iter().enumerate().take(0x60) {
            let base = 0x20 + 0x50 * bone;
            if base + 0x20 > b.len() {
                break;
            }
            if let Some(r) = f.part_rots.get(node) {
                for row in 0..3 {
                    for col in 0..3 {
                        put16(&mut b, base + 2 * (3 * row + col), r[row][col]);
                    }
                }
            }
            let p = f.parts.get(node).copied().unwrap_or(f.m_t);
            for k in 0..3 {
                put32(&mut b, base + 0x14 + 4 * k, p[k] as u32);
            }
        }
        m.vm.mem.map(model_addr(i), b);
        let mut s = vec![0u8; 0x200];
        s[0x100] = f.anim.mirror as u8;
        m.vm.mem.map(STATE_BASE + 0x200 * i as u32, s);
    }
    // BILL and RING
    let mut g = vec![0u8; 0x500];
    let (sa, ca, sb, cb) = {
        let rs = crate::base::portable::rsin;
        let (a, b) = (-(m.cam.ang[0] as i32), m.cam.ang[1] as i32);
        (rs(a), rs(a + 0x400), rs(b), rs(b + 0x400))
    };
    let bill = [[cb, (sb * sa) >> 12, (sb * ca) >> 12], [0, ca, -sa], [-sb, (cb * sa) >> 12, (cb * ca) >> 12]];
    let ob = (BILL - GLOBALS) as usize;
    for row in 0..3 {
        for col in 0..3 {
            put16(&mut g, ob + 2 * (3 * row + col), bill[row][col]);
        }
    }
    let or = (RING - GLOBALS) as usize;
    let (hx, hz) = (m.stage.ring.hx as i32, m.stage.ring.hz as i32);
    for (k, v) in [hx - 0xC8, hx, hz - 0xC8, hz].iter().enumerate() {
        put16(&mut g, or + 2 * k, *v);
    }
    m.vm.mem.map(GLOBALS, g);
    // delayed EFT_89 tasks (list 8)
    let mut keep = Vec::new();
    for (side, bone, delay) in std::mem::take(m.fx_delayed) {
        if m.round_state >= 0xF || m.fighters[side].fx_cancel != 0 {
            continue; // the cancel flag (`fx_cancel`) also kills the delayed EFT_89
        }
        if delay > 0 {
            keep.push((side, bone, delay - 1));
            continue;
        }
        m.vm.spawn(0, 9, None, &[bonepos(side, bone as usize), BILL]);
    }
    *m.fx_delayed = keep;
}

/// One effect opcode of fighter `i` (bytes as in the script).
pub fn effect_op(m: &mut FxHost, data: &Loaded, i: usize, b: [u8; 6]) {
    if m.round_state >= 0xF {
        return; // M.B1: no effects on the continue / game-over screens
    }
    struct F {
        chr: u8,
        yaw: i16,
        o_74: i16,
    }
    let fr = &m.fighters[i];
    let f = F { chr: fr.chr, yaw: fr.yaw, o_74: fr.o_74 };
    let fa = fighter_addr(i);
    let side = fr.player as u32;
    let mirrored = fr.anim.mirror;
    let svec = |p: [i32; 3]| p.map(|v| v as i16);
    let bone_world = |m: &FxHost, bone: usize| {
        let mdl = &data.fighters[i].models[m.fighters[i].beast() as usize];
        mdl.joints.get(bone).and_then(|&n| m.fighters[i].parts.get(n)).copied().unwrap_or(m.fighters[i].m_t)
    };
    let obj_w32 = |m: &mut FxHost, t: u32, off: u32, v: u32| {
        let o = m.vm.obj_of(t);
        if o != 0 {
            m.vm.mem.w32(o + off, v);
        }
    };
    let skipped = |m: &mut FxHost, what: String| {
        let tr = &mut m.fighters[i].trace;
        if tr.len() < 64 {
            tr.push(format!("effect {what} not ported"));
        }
    };
    match b[0] {
        0xF0 => {
            // the automatic trail of op 0x0D (fight/move_script/step.rs): node, life, delay, fade
            let (life, delay, fade) = (b[2] as i8 as i32, b[3] as i8 as i32, b[4] as i8 as i32);
            crate::fight::fx::move_effects::trail(m, &data.tables, i, b[1] as usize, life, delay, fade << 16);
        }
        0x16 => {
            let k = b[1] as i8;
            if k < 0 {
                return;
            }
            if k <= 2 {
                let t = m.vm.spawn(0, 0, None, &[bonepos(i, 3), fa, BILL]);
                obj_w32(m, t, 0x18, SPARKLE_CALLBACK);
                obj_w32(m, t, 0x1C, 0);
                obj_w32(m, t, 0x14, k as u32);
            } else if k == 3 {
                let t = m.vm.spawn(fx_group(i), 0, None, &[fa, fighter_addr(m.opp[i]), side + 1, BILL]);
                obj_w32(m, t, 0x14, mirrored as u32);
                if f.chr == 3 {
                    // Uriko's drive object: M, RING half sizes, the wall_mark callback
                    obj_w32(m, t, 0x18, M_BASE);
                    obj_w32(m, t, 0x1C, m.stage.ring.hx as i32 as u32);
                    obj_w32(m, t, 0x20, m.stage.ring.hz as i32 as u32);
                    obj_w32(m, t, 0x24, WALL_MARK_CALLBACK);
                }
            }
        }
        0x17 => {
            let (sub, lo, hi) = (b[1], b[2], b[3]);
            let arg = u16::from_le_bytes([lo, hi]);
            match sub {
                1 => m.colour_flash(i, lo as i16, hi),
                2 | 3 => {
                    // wall_mark(BONEPOS(3), yaw ∓ 0x400, idx = shi, flags = lo)
                    let p = bone_world(m, 3);
                    let yaw = f.yaw as i32 + if sub == 2 { -0x400 } else { 0x400 };
                    crate::fight::fx::native_tasks::wall_mark(m, &data.tables, p, yaw, hi as i8 as usize, lo);
                }
                7 => {
                    // blood: only with BLOOD EFFECT on
                    if m.custom.blood != 0 {
                        crate::fight::fx::move_effects::blood(m, data, i, hi as i8 as i32, lo);
                    }
                }
                0xD => {
                    // 8 falling dust chips from the chest
                    let p = svec(bone_world(m, 3));
                    let e0 = m.fx.rand() as i16;
                    let tl = data.tables.table(crate::fight::fx::TIMELINE_RECORDS).origin() + 0x3C;
                    m.fx.emitter_spawn(p, None, 0x10, 2, tl, [e0, 0, 0, 0]);
                }
                0xE => {
                    // BAK: 101 flame puffs from the right foot aimed at the opponent
                    let p = svec(bone_world(m, 13));
                    let tl = data.tables.table(crate::fight::fx::EMITTER_TIMELINES).origin() + 0x14;
                    m.fx.emitter_spawn(p, Some((i, 13)), 0x12, 0x8E, tl, [i as i16, m.opp[i] as i16, 0, 0]);
                }
                4 => {
                    // recovery: two bursts, frames 0 and 4
                    crate::fight::fx::native_tasks::recovery_burst(m, i);
                    m.fxn.tasks.push(crate::fight::fx::native_tasks::Task::RecoveryLater { side: i, delay: 3 });
                }
                8 => crate::fight::fx::native_tasks::eye_trail(m, i, arg as i16 as i32),
                0 => {
                    let a1 = if mirrored && lo & 0x80 != 0 { lo } else { lo & 0x7F };
                    crate::fight::fx::move_effects::sparkle(m, &data.tables, i, a1, hi as i8 as i32);
                }
                5 => crate::fight::fx::move_effects::rays(m, &data.tables, i, arg as i16 as usize),
                6 => crate::fight::fx::move_effects::converge(m, i, arg as i16 as usize),
                0xA => crate::fight::fx::move_effects::streaks(m, &data.tables, arg as i16 as i32),
                0xB => {
                    let p = bonepos(i, arg as i16 as usize);
                    let pos = [0, 1, 2].map(|k| m.vm.mem.s32(p + 4 * k));
                    crate::fight::fx::move_effects::orbs(m, pos);
                }
                0xF => {
                    // 7 spiral sparks from the chest (emitter mode 0x83) + model_fx 0x12
                    let p = svec(bone_world(m, 3));
                    let tl = data.tables.table(crate::fight::fx::EMITTER_TIMELINES).origin();
                    m.fx.emitter_spawn(p, Some((i, 3)), 0xE, 0x83, tl, [0; 4]);
                    m.colour_flash(i, 0x30, 0);
                    let p = bone_world(m, 3);
                    crate::fight::fx::native_tasks::model_fx(m, &data.tables, i, 0x12, bonepos(i, 3), p, 0x420, 0x3000, 2, 2);
                }
                9 => {
                    let mut bone = arg as i16 as usize;
                    if mirrored {
                        bone = REMAP.get(bone).copied().unwrap_or(bone as u8) as usize;
                    }
                    m.vm.spawn(0, 0xF, None, &[bonepos(i, bone), BILL]);
                }
                0xC => {
                    let mut bone = (hi & 0x7F) as usize;
                    if mirrored && arg & 0x8000 != 0 {
                        bone = REMAP.get(bone).copied().unwrap_or(bone as u8) as usize;
                    }
                    let p = svec(bone_world(m, bone));
                    m.fx.ring_spawn(p, lo as usize, false);
                }
                _ => skipped(m, format!("op17.{sub:X}")),
            }
        }
        0x18 => {
            let bb = b[1];
            let mut bone = (bb & 0x7F) as usize;
            if mirrored && bb & 0x80 != 0 {
                bone = REMAP.get(bone).copied().unwrap_or(bone as u8) as usize & 0x7F;
            }
            let cmd = u16::from_le_bytes([b[2], b[3]]);
            let arg = u16::from_le_bytes([b[4], b[5]]);
            let (ty, n, a) = (cmd >> 12, ((cmd >> 8) & 0xF) as u32, (cmd & 0xFF) as u32);
            let sarg = arg as i16 as i32;
            let (alo, anib, ahi) = ((arg & 0xFF) as u32, ((arg >> 8) & 0xF) as u32, (arg >> 8) as u32);
            let p = bonepos(i, bone);
            match ty {
                0 | 7 | 0xF => {}
                1 => {
                    // model_fx(F, n | 0x10, BONEPOS(b), a << 4, sarg, alo, anib)
                    let pw = bone_world(m, bone);
                    crate::fight::fx::native_tasks::model_fx(
                        m,
                        &data.tables,
                        i,
                        (n | 0x10) as usize,
                        p,
                        pw,
                        (a << 4) as i32,
                        sarg as u16,
                        alo as u8,
                        anib as u16,
                    );
                }
                2 => {
                    let t = m.vm.spawn(0, 0xB, None, &[fa, BILL]);
                    for (off, v) in [(0xC, 0x19 + n), (0x10, p - 0x14), (0x14, a << 4), (0x18, sarg as u32), (0x1C, alo), (0x20, anib)] {
                        obj_w32(m, t, off, v);
                    }
                }
                3 => spawn_eft92(m, fa, 0x1A + n, p, a << 4, sarg as u32, alo, anib),
                6 => {
                    let t = m.vm.spawn(0, 0xA, None, &[]);
                    let mut yaw = (arg & 0xF00) as u32;
                    if mirrored && bb & 0x80 != 0 {
                        yaw = 0x1000 - yaw;
                    }
                    for (off, v) in [
                        (4, fa),
                        (8, p),
                        (0xC, (((cmd >> 4) & 0xF) as u32) << 8),
                        (0x10, ((cmd & 0xF) as u32) << 8),
                        (0x14, yaw),
                        (0x18, (((arg >> 4) & 0xF) as u32) << 7),
                        (0x1C, sarg as u32),
                        (0x20, (arg & 0xF) as u32),
                        (0x24, n),
                    ] {
                        obj_w32(m, t, off, v);
                    }
                }
                8 => m.fx_delayed.push((i, bone as u8, sarg.max(0) as i16)),
                9 => {
                    let pos = svec(bone_world(m, bone));
                    let yaw = ((((arg >> 4) & 0xF) as i32) << 8) + f.yaw as i32 + f.o_74 as i32 - 0x400;
                    m.fx.shards_spawn(pos, ahi as i16, yaw, ((arg & 0xF) as i32) << 8, a as i32, false);
                }
                0xA => {
                    let pos = svec(bone_world(m, bone));
                    m.fx.sprite_spawn(&data.tables, pos, arg as i16, 0x7F, 0x7F, 0);
                }
                0xC => match f.chr {
                    6 if a < 2 => {
                        let t = m.vm.spawn(fx_group(i), sarg as usize, None, &[fa, side + 1, p]);
                        let _ = t; // a == 0 moves it to list 2 (runs before the other scripts): order not modelled
                    }
                    9 if a == 0 => {
                        m.vm.spawn(fx_group(i), 8, None, &[side, p]);
                    }
                    9 if a == 1 => {
                        let pos = [0, 1, 2].map(|k| m.vm.mem.s32(p + 4 * k));
                        crate::fight::fx::move_effects::comet(m, i, pos, sarg);
                    }
                    5 if a == 0 => crate::fight::fx::native_tasks::tongue(m, i, sarg >> 11, (arg & 0x7FF) as i32),
                    8 => {
                        // STU: parameters of the persistent aura script
                        let t = *m.aura(i);
                        match a {
                            0 => {
                                obj_w32(m, t, 0x0C, ahi as i8 as i32 as u32);
                                obj_w32(m, t, 0x10, alo);
                            }
                            0xFF => obj_w32(m, t, 0x14, 1),
                            0xFE => obj_w32(m, t, 0x14, 0),
                            _ => {}
                        }
                    }
                    _ => skipped(m, format!("op18 type C chr {}", f.chr)),
                },
                0xE => {
                    let pos = svec(bone_world(m, bone));
                    let yaw = f.yaw as i32 + f.o_74 as i32 + ((((arg >> 4) & 0xF) as i32) << 8) - 0x400;
                    let fr = &m.fighters[i];
                    let beast = fr.flags[crate::fight::fighter::F64] & 0x2_0000 != 0;
                    let blood = matches!(fr.s_101[0], 2 | 12) && m.custom.blood != 0 && !beast;
                    m.fx.hit_spark(&data.tables, pos, yaw, ((arg & 0xF) as i32) << 8, ahi as i8 as i32, beast, blood);
                }
                0xB => {
                    // emitter of kind cmd & 0xFF, mode arg & 0xFF, timeline TIMELINES[arg >> 8]
                    let pos = svec(bone_world(m, bone));
                    let tl = data.tables.table(crate::fight::fx::TIMELINES).u32(4 * ((arg >> 8) as i8 as i32 as usize & 3));
                    m.fx.emitter_spawn(pos, Some((i, bone)), (cmd & 0xFF) as i32, (arg & 0xFF) as i32, tl, [0; 4]);
                }
                0xD => {
                    // claw / kick trail: life = arg & 0xFF, delay = arg >> 8, scale aa << 6, fade n·2
                    let packed = ((a << 6) + (n << 17)) as i32;
                    crate::fight::fx::move_effects::trail(m, &data.tables, i, bone, alo as i32, ahi as i32, packed);
                }
                _ => skipped(m, format!("op18 type {ty:X}")),
            }
        }
        _ => {}
    }
}

/// EFT_92 with model `idx` at `pos`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_eft92(m: &mut FxHost, fa: u32, idx: u32, pos: u32, scale: u32, p18: u32, p1c: u32, p20: u32) {
    let t = m.vm.spawn(0, 0xC, None, &[fa, BILL]);
    let o = m.vm.obj_of(t);
    if o != 0 {
        for (off, v) in [(0xC, idx), (0x10, pos), (0x14, scale), (0x18, p18), (0x1C, p1c), (0x20, p20)] {
            m.vm.mem.w32(o + off, v);
        }
    }
}

#[cfg(test)]
mod tests {
    /// A bone sign-extended from op 0x17's halfword argument (`arg as i16 as usize`) addresses below the model as the
    /// game's signed `lw` / `addu` do, on every pointer width; a halfword's largest bone above it.
    #[test]
    fn bonepos_wraps_as_the_game() {
        let made = |arg: u32| arg as i16 as usize;
        assert_eq!(super::bonepos(0, made(0xFFFF)), super::MODEL_BASE + 0x34 - 0x50);
        assert_eq!(super::bonepos(1, made(0x8000)), (super::MODEL_BASE + 0x2000 + 0x34).wrapping_sub(0x50 * 0x8000));
        assert_eq!(super::bonepos(0, 0x7FFF), super::MODEL_BASE + 0x34 + 0x50 * 0x7FFF);
    }
}
