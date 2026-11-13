//! Native (C) effect tasks that drive 3-D effect models directly.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Native effect tasks that drive 3-D effect models directly: model_fx
//! (the most common move effect), wall marks, the recovery burst, and the fight-effect
//! state machine's one-shot spawns (Stun's aura, Beach) plus the per-frame triggers (Tatami ring-out splash,
//! beast-gauge-full pop-up). Also Busuzima's tongue and Shina's eye trails, drawn as screen prims by `prims`.

use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F60};
use crate::fight::fx::host::FxHost;
use crate::fight::fx::script_link::{BILL, bonepos, fighter_addr, fx_group, pad_addr};
use crate::fight::fx::script_vm::{M3, Vm};
use crate::fight::side::{PerSlot, bank_bit};
use crate::formats::tables::Tables;
use serde::{Deserialize, Serialize};

const ID: M3 = [[4096, 0, 0], [0, 4096, 0], [0, 0, 4096]];

fn diag(x: i32, y: i32, z: i32) -> M3 {
    [[x, 0, 0], [0, y, 0], [0, 0, z]]
}

fn read_bill(vm: &Vm) -> M3 {
    vm.read_m3(BILL)
}

fn vec3(vm: &Vm, a: u32) -> [i32; 3] {
    [0, 1, 2].map(|k| vm.mem.s32(a + 4 * k))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Task {
    /// model_fx: a common effect model at a position (the most common move effect)
    ModelFx { m: u32, pos_addr: u32, flags: u16, mode: u8, r: M3, s1: [i32; 3], v: [i32; 2], acc: i32, s2: i32 },
    /// a wall mark: a common model snapped onto the nearest ring wall
    WallMark { m: u32, time: i32, scale: i32, rot: M3 },
    /// the recovery burst between the pelvis and the hip bones
    Recovery { m: u32, side: usize, ang: i32, size: i32 },
    /// the op17 sub 4 timeline: a second burst 4 frames after the first
    RecoveryLater { side: usize, delay: i32 },
    /// Busuzima's tongue
    Tongue(Box<Tongue>),
    /// Shina's beast-form eye trails
    EyeTrail(Box<EyeTrail>),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub tasks: Vec<Task>,
    /// Wall marks alive (at most 3)
    pub wall_marks: i32,
    /// Stun's aura scripts
    pub aura: PerSlot<u32>,
    /// the gauge of the previous frame (pop-up edge)
    pub prev_gauge: PerSlot<i16>,
    /// the lens flare (City / Lab): 5 models and the sun
    pub flare: Option<Flare>,
    /// the flare's 2-D glow sprites of this frame (512×480 HUD space, drawn in ot[0])
    pub sprites: Vec<crate::output::prim::Prim>,
    /// the remaining native move effects (`move_effects`)
    pub mv: crate::fight::fx::move_effects::State,
    /// the fighters' F6C as last written to the VM's match block (script_link::write_back)
    pub f6c: PerSlot<u32>,
    rng: u32,
}

impl State {
    /// A decoded state's tasks fit a fight of `n` fighters (engine addition: `Match::check`, `TeamFight::check`): every
    /// fighter index below `n` (`m.fighters[side]`, `script_link::fx_group`), a tongue's tip on its curve (`q[ext]`,
    /// 0..=16) and its target bone as `tongue` clamps it; the move effects' (`move_effects::State::check`).
    pub fn check(&self, n: usize, tables: &crate::formats::tables::Tables) -> Result<(), String> {
        let ok = |t: &Task| match t {
            Task::Recovery { side, .. } | Task::RecoveryLater { side, .. } => *side < n,
            Task::Tongue(t) => t.side < n && t.opp < n && (0..=16).contains(&t.ext) && t.bone <= 15,
            Task::EyeTrail(t) => t.side < n,
            Task::ModelFx { .. } | Task::WallMark { .. } => true,
        };
        if !self.tasks.iter().all(ok) {
            return Err("native effect tasks".into());
        }
        self.mv.check(n, tables)
    }
}

/// The per-side parts of `State` for the team battle's fighters 2 and 3 (engine addition; `FxHost::ext`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SideExt {
    pub aura: PerSlot<u32>,
    pub prev_gauge: PerSlot<i16>,
    pub f6c: PerSlot<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Flare {
    m: [u32; 5],
    sun: [i32; 3],
}

/// atan_br2: octant-linear angle, 0 = −y, 0x400 = +x, 0x800 = +y, 0xC00 = −x.
fn atan_br2(x: i32, y: i32) -> i32 {
    let (x, y) = (if x == 0 { 1 } else { x }, if y == 0 { 1 } else { y });
    let (ax, ay) = (x.abs(), y.abs());
    if ay < ax {
        let n = if y > 0 { (y << 9) + (ax << 3) } else { (y << 9) - (ax << 3) };
        let v = n / ax;
        if x > 0 { v + 0x400 } else { 0xC00 - v }
    } else {
        let n = if x > 0 { (x << 9) + (ay << 3) } else { (x << 9) - (ay << 3) };
        let v = n / ay;
        if y > 0 {
            0x800 - v
        } else if x > 0 {
            v
        } else {
            v + 0xFFF
        }
    }
}

/// sqrt12: the GTE-assisted fixed-point square root with its hyperbolic CORDIC (sqrt(a)·64).
fn sqrt12(a: i32) -> i32 {
    if a == 0 {
        return 0;
    }
    let lz = if a > 0 { a.leading_zeros() as i32 } else { (!a).leading_zeros() as i32 };
    let v = 8 - lz;
    let (mut sh, a0) = if v >= 0 { (v >> 1, a >> (2 * (v >> 1))) } else { ((v >> 1) + 1, a.wrapping_shl((-2 * ((v >> 1) + 1)) as u32)) };
    sh -= 6;
    let (mut x, mut y) = (a0.wrapping_add(0x5D_50AD), a0.wrapping_sub(0x5D_50AD));
    for i in 1..=6 {
        for _ in 0..if i == 4 { 2 } else { 1 } {
            let (nx, ny) = if y >= 0 { (x.wrapping_sub(y >> i), y.wrapping_sub(x >> i)) } else { (x.wrapping_add(y >> i), y.wrapping_add(x >> i)) };
            x = nx;
            y = ny;
        }
    }
    if sh < 0 { x >> -sh } else { x.wrapping_shl(sh as u32) }
}

/// lens_flare at fight init for City (row 0) and Lab (row 1): its table `fx.lens_flare` holds the five
/// models (u8), their depths (i16 at +8) and the positions (8-byte vectors at +0x14, a row of five per stage).
fn lens_flare(m: &mut FxHost, row: usize, tables: &Tables) {
    let t = tables.table("fx.lens_flare");
    let pos_of = |i: usize| [0, 1, 2].map(|c| t.i16(0x14 + 8 * (5 * row + i) + 2 * c) as i32);
    let mut ms = [0u32; 5];
    let lower = crate::base::trace::flag("OKB_FLARETEST") || crate::base::trace::flag("BR2_FLARETEST"); // debug: sun and models lowered into view (none on the web)
    let pos = |i: usize| {
        if lower {
            let p = pos_of(i);
            [p[0] / 8, p[1] / 8, p[2]]
        } else {
            pos_of(i)
        }
    };
    for i in 0..5 {
        let h = m.vm.native_model(3, t.u8(i) as usize);
        if h != 0 {
            m.vm.with_model(h, |mi| {
                mi.set_depth(t.i16(8 + 2 * i) as i32);
                mi.show(false);
                mi.play_default_b();
            });
            m.vm.set_model_matrix(h, &ID, Some(pos(i)));
        }
        ms[i] = h;
    }
    m.fxn.flare = Some(Flare { m: ms, sun: pos(0) });
}

/// Project the sun, place the glows and the 5 models (runs also while paused).
fn flare_update(m: &mut FxHost, trig: &crate::fight::physics::Trig) {
    m.fxn.sprites.clear();
    let Some(fl) = m.fxn.flare.clone() else { return };
    // RTPS of the sun in the 512×480 screen (H = 360 over 320 px, interlaced: y doubled)
    let v = m.cam.view(trig) * glam::Vec4::new(fl.sun[0] as f32, fl.sun[1] as f32, fl.sun[2] as f32, 1.0);
    let z = v.z.abs().max(1.0);
    let (sx, sy) = ((256.0 + 1.6 * 360.0 * v.x / z) as i32, (240.0 + 720.0 * v.y / z) as i32);
    let tmp_r = Vm::rot_z((sx + sy) * 2);
    let d2 = (255 - sx) * (255 - sx) + (255 - sy) * (255 - sy);
    let dist = sqrt12(d2.wrapping_shl(12)) >> 12;
    if !(-50..563).contains(&sx) || !(-50..563).contains(&sy) {
        for h in fl.m {
            m.vm.with_model(h, |mi| mi.show(false));
        }
        return;
    }
    let mut rnd = || crate::base::libc_rand::rand(&mut m.fxn.rng);
    let d8 = dist / 8;
    let ang = atan_br2(255 - sx, 255 - sy) - 0x400;
    let dx = ((d8 * trig.cos(ang)) >> 12) as i16 as i32;
    let dy = ((d8 * trig.sin(ang)) >> 12) as i16 as i32;
    let px = (sx - 256) / 2 + 240;
    let py = (sy - 256) / 2 + 240;
    let mut sprites = Vec::new();
    let mut sprite = |x: i32, y: i32, w: i32, h: i32, u: i32, vv: i32| {
        // SPRT 0x67 (semi, raw) with tpage 0x39 (576,256) additive, CLUT (400,483)
        let mut p = crate::output::prim::ft4(x, y, w, h, (u, vv, w, h), 0x39, crate::output::prim::clut(400, 483));
        p.semi = true;
        sprites.push(p);
    };
    if rnd() & 3 != 0 {
        sprite(px, py, 31, 31, 0xC0, 0x20);
    }
    sprite(px - dx - 16, py - dy - 16, 63, 63, 0xC0, 0x40);
    if dist < 150 {
        if rnd() & 7 != 0 {
            sprite(224, 224, 64, 64, 0xC0, 0x40);
        }
        let (mut qx, mut qy) = (240, 240);
        for _ in 0..2 {
            qx += dx;
            qy += dy;
            if rnd() & 3 != 0 {
                sprite(qx, qy, 31, 31, 0xC0, 0x20);
            }
        }
    }
    let flick = [rnd() & 3 == 0, rnd() & 3 == 0];
    m.fxn.sprites = sprites;
    for (k, h) in fl.m.iter().enumerate() {
        let show = if k >= 3 { !flick[k - 3] } else { true };
        m.vm.with_model(*h, |mi| mi.show(show));
    }
    let k = dist * 16 - 0x200;
    let s = ((0x1000 - k) / 2 + 0x800).clamp(0, 0xFFF);
    let bill = read_bill(m.vm);
    let buf = m.vm.buf;
    for (i, h) in fl.m.iter().enumerate() {
        if i >= 1 {
            m.vm.with_model(*h, |mi| mi.set_time_b(k, buf));
        }
        let mat = match i {
            0 => Vm::mul(&bill, &tmp_r),
            1 => Vm::mul(&diag(s, s, s), &bill),
            _ => bill,
        };
        m.vm.set_model_matrix(*h, &mat, None);
    }
}

/// The fight-effect state reset (state 2): pools wiped without destructors, one-shot spawns.
pub fn reset(m: &mut FxHost, data: &Loaded) {
    m.vm.clear();
    *m.fxn = State { prev_gauge: PerSlot([m.fighters[0].gauge, m.fighters[1].gauge]), f6c: m.fxn.f6c, ..Default::default() };
    if let Some(e) = m.ext.as_deref_mut() {
        *e = SideExt { prev_gauge: PerSlot([m.fighters[2].gauge, m.fighters[3].gauge]), f6c: e.f6c, ..Default::default() };
    }
    m.fx.stage = data.stage_no;
    crate::fight::fx::script_link::refresh(m, data);
    if m.round_state >= 0xF || data.stage_no >= 9 {
        return;
    }
    if data.stage_no == 3 {
        m.vm.spawn(3, 1, None, &[]); // Beach: ST3_70
    }
    if data.stage_no == 0 || data.stage_no == 6 {
        lens_flare(m, if data.stage_no == 6 { 1 } else { 0 }, &data.tables);
    }
    for i in 0..m.fighters.len() {
        if m.fighters[i].chr == 8 {
            // STU_STUEFT_19, the permanent aura: F, BONEMAT(3), 0, 0
            let t = m.vm.spawn(fx_group(i), 8, None, &[fighter_addr(i), bonepos(i, 3) - 0x14, 0, 0]);
            *m.aura(i) = t;
        }
        // Shina's third costume: the permanent eye trail
        if m.fighters[i].chr == 4 && m.costume[i] == 2 {
            eye_trail(m, i, 0x0FFF_FFFF);
        }
    }
}

/// model_fx: common model `idx` at the VECTOR `pos_addr`; `flags` 0x1000 = shrinking ring, 0x2000 =
/// follow the bone; `mode` 0 along the attack, 1 flat on the floor, 2 camera facing, 3 flat; `clut` row.
#[allow(clippy::too_many_arguments)]
pub fn model_fx(
    m: &mut FxHost,
    tables: &crate::formats::tables::Tables,
    side: usize,
    idx: usize,
    pos_addr: u32,
    pos: [i32; 3],
    scale: i32,
    flags: u16,
    mode: u8,
    clut: u16,
) {
    if m.round_state >= 0xF {
        return;
    }
    let h = m.vm.native_model(0, idx);
    if h == 0 {
        return;
    }
    if m.vm.with_model(h, |mi| mi.play_b(0)) != Some(1) {
        m.vm.drop_model(h);
        return;
    }
    let f = &m.fighters[side];
    let mut t = pos;
    if clut != 0 {
        let c = ((((clut as u32) + 0x1E0) & 0x1FF) << 6 | 0x1A) as u16;
        m.vm.with_model(h, |mi| mi.set_clut(c));
    }
    let r = match mode {
        0 => Vm::rot2(0xC00 - f.o_db as i32 * 16, -(f.yaw as i32 + f.o_c6 as i32)),
        1 => {
            t[1] = if f.chest[1] >= 0 { f.o_94 as i32 } else { 0 };
            Vm::rot_x(-0x400)
        }
        2 => {
            m.vm.with_model(h, |mi| mi.set_depth(3));
            ID
        }
        3 => Vm::rot_x(-0x400),
        _ => ID,
    };
    let (s1, v, acc) = if flags & 0x1000 != 0 { ([196, 196, 2024], [500, 500], -30) } else { ([2500, 2500, 2024], [-230, -230], 5) };
    m.vm.set_model_matrix(h, &ID, Some(t));
    if idx == 0x17 {
        // model 0x17 gets the UV animator (with the static block's UV deltas and script); it dies with the model
        let block = tables.table(crate::fight::fx::STATIC_BLOCK).origin();
        m.vm.uv_anim(h, 0, block, block + 0x10);
    }
    m.fxn.tasks.push(Task::ModelFx { m: h, pos_addr, flags, mode, r, s1, v, acc, s2: scale });
}

/// wall_mark: common model `idx` snapped onto the nearest ring wall, plus dust chips thrown back
/// from the wall (`yaw` + 0x800) except in the Aquarium or with flags & 0xF0.
pub fn wall_mark(m: &mut FxHost, tables: &crate::formats::tables::Tables, pos: [i32; 3], yaw: i32, idx: usize, flags: u8) {
    if m.round_state >= 0xF || m.fxn.wall_marks >= 3 {
        return;
    }
    if m.fx.stage != 1 && flags & 0xF0 == 0 {
        let p = pos.map(|v| v as i16);
        let tl = tables.table(crate::fight::fx::TIMELINE_RECORDS).origin() + 0x3C;
        m.fx.emitter_spawn(p, None, 0xA, 2, tl, [(yaw + 0x800) as i16, 0, 0, 0]);
    }
    let h = m.vm.native_model(0, idx);
    if h == 0 {
        return;
    }
    let clut = flags & 0xF;
    if clut != 0 {
        m.vm.with_model(h, |mi| mi.set_clut(0x781A | (clut as u16) << 6));
    }
    if m.vm.with_model(h, |mi| mi.play_default_b()) != Some(1) {
        m.vm.drop_model(h);
        return;
    }
    let (hx, hz) = (m.stage.ring.hx as i32, m.stage.ring.hz as i32);
    // the platform's atan2 before simulation version 3, then libm's
    let a = (crate::base::portable::atan2_f64(pos[0] as f64, pos[2] as f64, m.vm.portable) * 4096.0 / std::f64::consts::TAU).round() as i32;
    let q = ((a + 0x200) & 0xFFF) >> 10;
    let mut t = pos;
    let ang = match q {
        0 => {
            t[2] = -hz;
            0x800
        }
        1 => {
            t[0] = hx;
            0x400
        }
        2 => {
            t[2] = hz;
            0
        }
        _ => {
            t[0] = -hx;
            0xC00
        }
    };
    m.vm.set_model_matrix(h, &ID, Some(t));
    m.fxn.wall_marks += 1;
    m.fxn.tasks.push(Task::WallMark { m: h, time: 0, scale: 0x138, rot: Vm::rot_y(ang) });
}

/// The recovery burst (common model 15) between the pelvis and the hip bones.
pub fn recovery_burst(m: &mut FxHost, side: usize) {
    if m.round_state >= 0xF {
        return;
    }
    let h = m.vm.native_model(0, 15);
    if h == 0 {
        return;
    }
    if m.vm.with_model(h, |mi| mi.play_b(0)) != Some(1) {
        m.vm.drop_model(h);
        return;
    }
    m.vm.with_model(h, |mi| {
        mi.show(false);
        mi.set_depth(3);
    });
    let ang = (m.rand & 0x7FFF) as i32;
    m.fxn.tasks.push(Task::Recovery { m: h, side, ang, size: 0x400 });
}

/// Per frame, before the effect scripts: Stun's aura follows the fighter's hidden flag, the Tatami ring-out
/// splash, the beast-gauge-full pop-up.
pub fn triggers(m: &mut FxHost, data: &Loaded) {
    if m.round_state >= 0xF {
        return;
    }
    for i in 0..m.fighters.len() {
        // the trail-cancel flags count down
        m.fighters[i].fx_cancel = m.fighters[i].fx_cancel.saturating_sub(1);
        let aura = *m.aura(i);
        if aura != 0 {
            let o = m.vm.obj_of(aura);
            if o != 0 {
                let hidden = m.fighters[i].flags[F60] & 2 != 0;
                m.vm.mem.w32(o + 0x14, hidden as u32);
            }
        }
        // Tatami: ring-out into the water
        if data.stage_no == 2 {
            let f = &m.fighters[i];
            if f.flags[F5C] & 0x8_0080 == 0x8_0000 && f.o_94 > 0 && f.flags[F60] & 2 == 0 {
                let hips = f.hips;
                let t = m.vm.spawn(3, 0, None, &[hips[0] as u32, hips[1] as u32, hips[2] as u32]);
                let _ = t;
                m.sound_queue.push((0x84, 0));
                m.fighters[i].flags[F60] |= 2;
            }
        }
        // beast gauge just became full (HUD): EFT_99 + EFT_92 with model 0x1B
        let (g, form) = (m.fighters[i].gauge, m.fighters[i].form);
        let prev = std::mem::replace(m.prev_gauge(i), g);
        if g >= 0x1E00 && prev < 0x1E00 && form == 0 {
            m.colour_flash(i, 0x30, 0);
            m.vm.spawn(0, 0x13, None, &[pad_addr(i), bonepos(i, 3), BILL]);
            crate::fight::fx::script_link::spawn_eft92(m, fighter_addr(i), 0x1B, bonepos(i, 3), 0xE00, 0x2000, 0, 5);
        }
    }
}

/// The stage pool (pool 2) before the effect lists: the lens flare.
pub fn stage_pool(m: &mut FxHost, data: &Loaded) {
    flare_update(m, &data.trig);
}

/// The C tasks of this frame (lists 5, 7, 8), after the scripts.
pub fn tick(m: &mut FxHost, data: &Loaded) {
    for side in 0..m.fighters.len() {
        let bones = std::array::from_fn(|b| vec3(m.vm, bonepos(side, b)));
        m.fx.set_bones(side, bones);
    }
    crate::fight::fx::move_effects::tick(m, &data.tables);
    let paused = m.vm.paused;
    let bill = read_bill(m.vm);
    let mut keep = Vec::new();
    for task in std::mem::take(&mut m.fxn.tasks) {
        match task {
            Task::ModelFx { m: h, pos_addr, flags, mode, r, mut s1, mut v, acc, s2 } => {
                if !m.vm.model_alive(h) {
                    continue;
                }
                let t = if flags & 0x2000 != 0 && pos_addr != 0 { Some(vec3(m.vm, pos_addr)) } else { None };
                if !paused {
                    v[0] += acc;
                    v[1] += acc;
                    s1[0] += v[0];
                    s1[1] += v[1];
                }
                let s1m = diag(s1[0], s1[1], s1[2]);
                let s2m = diag(s2, s2, s2);
                let mat = match mode {
                    0 | 1 | 3 => Vm::mul(&s2m, &Vm::mul(&r, &s1m)),
                    2 => Vm::mul(&Vm::mul(&bill, &s1m), &s2m),
                    _ => ID,
                };
                m.vm.set_model_matrix(h, &mat, t);
                keep.push(Task::ModelFx { m: h, pos_addr, flags, mode, r, s1, v, acc, s2 });
            }
            Task::WallMark { m: h, mut time, mut scale, rot } => {
                if !m.vm.model_alive(h) {
                    m.fxn.wall_marks = (m.fxn.wall_marks - 1).max(0);
                    continue;
                }
                if !paused {
                    scale += 0x80;
                    time += 0x200;
                    if time > 0x1000 {
                        m.vm.drop_model(h);
                        m.fxn.wall_marks = (m.fxn.wall_marks - 1).max(0);
                        continue;
                    }
                    let buf = m.vm.buf;
                    m.vm.with_model(h, |mi| mi.set_time_b(time, buf));
                }
                m.vm.set_model_matrix(h, &Vm::mul(&rot, &diag(scale, scale, scale)), None);
                keep.push(Task::WallMark { m: h, time, scale, rot });
            }
            Task::Recovery { m: h, side, mut ang, mut size } => {
                if !m.vm.model_alive(h) {
                    continue;
                }
                if !paused {
                    ang += 0x138;
                    size += 0x1C4;
                }
                m.vm.with_model(h, |mi| mi.show(true));
                let tmp = Vm::mul(&diag(size, size, 0x1000), &Vm::rot_z(ang));
                let (a, b) = (vec3(m.vm, bonepos(side, 1)), vec3(m.vm, bonepos(side, 5)));
                let t = [0, 1, 2].map(|k| (a[k] + b[k]) / 2);
                m.vm.set_model_matrix(h, &Vm::mul(&bill, &tmp), Some(t));
                keep.push(Task::Recovery { m: h, side, ang, size });
            }
            Task::Tongue(mut t) => {
                if tongue_tick(m, &mut t, paused) {
                    keep.push(Task::Tongue(t));
                }
            }
            Task::EyeTrail(mut t) => {
                if eye_tick(m, &mut t, paused) {
                    keep.push(Task::EyeTrail(t));
                }
            }
            Task::RecoveryLater { side, delay } => {
                if delay > 0 {
                    keep.push(Task::RecoveryLater { side, delay: delay - 1 });
                } else {
                    recovery_burst(m, side);
                }
            }
        }
    }
    // tasks created during this walk (recovery_burst) were pushed to m.fxn.tasks: keep both
    keep.append(&mut m.fxn.tasks);
    m.fxn.tasks = keep;
}

// ---------------------------------------------------------------------------------------------- Busuzima / Shina

use crate::fight::fx::effect_models::ModelPrim;
use crate::fight::fx::hit_effects::{Gte, rcos, rsin};

/// MIPS `div`: truncates toward 0; division by 0 gives −1 (a ≥ 0) or +1.
fn tdiv(a: i32, b: i32) -> i32 {
    if b == 0 {
        return if a >= 0 { -1 } else { 1 };
    }
    a.wrapping_div(b)
}

fn trunc12(x: i32) -> i32 {
    (if x < 0 { x + 0xFFF } else { x }) >> 12
}

fn s16(v: i32) -> i32 {
    v as i16 as i32
}

/// ApplyMatrixSV: `sat16((M·v) >> 12)`.
fn apply_sv(mt: &M3, v: [i32; 3]) -> [i32; 3] {
    [0, 1, 2].map(|i| ((mt[i][0] * v[0] + mt[i][1] * v[1] + mt[i][2] * v[2]) >> 12).clamp(-0x8000, 0x7FFF))
}

/// A bone's matrix and position (s32) of fighter `side` in the VM's model block.
fn bone(vm: &Vm, side: usize, b: usize) -> (M3, [i32; 3]) {
    let p = bonepos(side, b);
    (vm.read_m3(p - 0x14), vec3(vm, p))
}

/// `ApplyMatrixSV(M, v) + BONEPOS` with the low 16 bits of the position (s16 wrap).
fn bone_point(vm: &Vm, side: usize, b: usize, v: [i32; 3]) -> [i32; 3] {
    let (mt, p) = bone(vm, side, b);
    let r = apply_sv(&mt, v);
    [0, 1, 2].map(|k| s16(r[k] + p[k]))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tongue {
    side: usize,
    opp: usize,
    bone: usize,
    hold: i32,
    phase: u8,
    ext: i32,
    ctr: u32,
    rows: i32,
    /// the curve of this frame (17 world points) and the 4 history buffers of the blur sheet
    q: [[i32; 3]; 17],
    ring: [[[i32; 3]; 17]; 4],
    model: u32,
    v0: u8,
    clut: u16,
    blur_clut: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EyeTrail {
    side: usize,
    life: i32,
    cnt: i16,
    shina: bool,
    armed: bool,
    /// false after an unpaused human-form frame: nothing drawn
    shown: bool,
    hist: [[[i32; 3]; 6]; 2],
}

/// Busuzima's tongue toward bone `target` of the opponent, attached for `hold` frames.
pub fn tongue(m: &mut FxHost, side: usize, target: i32, hold: i32) {
    if m.round_state >= 0xF {
        return;
    }
    let f = &m.fighters[side];
    let (player, costume) = (f.player as usize, m.costume[side]);
    let h = m.vm.native_model(fx_group(side), 0);
    if h == 0 {
        return;
    }
    let clut = match (player, costume != 0) {
        (0, false) => 0x7806,
        (0, true) => 0x7846,
        (_, false) => 0x7807,
        (_, true) => 0x7847,
    };
    m.vm.with_model(h, |mi| {
        mi.play_default_b();
        if player == 1 {
            mi.uv_scroll(0, 0x40);
        }
        if player == 1 || costume != 0 {
            mi.set_clut(clut);
        }
    });
    let t = Tongue {
        side,
        opp: m.opp[side],
        bone: target.clamp(0, 15) as usize,
        hold,
        phase: 0,
        ext: 0,
        ctr: 0,
        rows: -1,
        q: [[0; 3]; 17],
        ring: [[[0; 3]; 17]; 4],
        model: h,
        v0: if player == 1 { 0x40 } else { 0 },
        clut,
        blur_clut: if costume != 0 { 0x791A } else { 0x7B1A },
    };
    m.fxn.tasks.push(Task::Tongue(Box::new(t)));
}

/// Busuzima's tongue, a frame; false = the task ends (nothing drawn this frame).
fn tongue_tick(m: &mut FxHost, t: &mut Tongue, paused: bool) -> bool {
    if !m.vm.model_alive(t.model) {
        return false;
    }
    let vm = &m.vm;
    let a = bone_point(vm, t.side, 2, [0, -10, -60]);
    let e = bone_point(vm, t.opp, t.bone, [0, 0, 0]);
    let (dx, dy, dz) = (e[0] - a[0], e[1] - a[1], e[2] - a[2]);
    let c = rcos(atan_br2(dx, dz) + 0x400);
    let n = dx.abs() << 12;
    let hz = if c != 0 { tdiv(n, c).wrapping_abs() } else { n };
    let c = rcos(atan_br2(hz, dy) + 0x400);
    let n = hz.wrapping_shl(12);
    let len = if c != 0 { tdiv(n, c).wrapping_abs() } else { n.wrapping_abs() };
    if !paused {
        match t.phase {
            0 => {
                t.ext += if len < 0x200 { 3 } else { 2 };
                if t.ext >= 16 {
                    t.ext = 16;
                    t.phase = 1;
                }
            }
            1 => {
                t.hold -= 1;
                if t.hold < 0 {
                    t.phase = 2;
                }
            }
            _ => {
                t.ext -= 3;
                if t.ext < 0 {
                    m.vm.drop_model(t.model);
                    return false;
                }
            }
        }
    }
    // the curve: a uniform quadratic B-spline (2 spans, 17 points) bent along the head's forward axis
    let (mf, _) = bone(m.vm, t.side, 2);
    let dv = apply_sv(&mf, [0, 0, s16(-tdiv(len, 3))]);
    let p0 = [0, 1, 2].map(|k| s16(a[k] - dv[k]));
    let p1 = [0, 1, 2].map(|k| s16(a[k] + dv[k]));
    let bs = |tt: i32, a: [i32; 3], b: [i32; 3], c: [i32; 3]| {
        let u = 0x1000 - tt;
        let (w0, w1, w2) = ((u * u) >> 13, (u * tt + 0x80_0000) >> 12, (tt * tt) >> 13);
        [0, 1, 2].map(|k| s16(trunc12(w0 * a[k] + w1 * b[k] + w2 * c[k])))
    };
    for k in 0..8 {
        t.q[k] = bs(0x200 * k as i32, p0, p1, e);
    }
    for k in 0..9 {
        t.q[8 + k] = bs(0x200 * k as i32, p1, e, e);
    }
    // the claw at the tip, aimed at the opponent's head (with the game's E/P0 + cos quirk, kept)
    let (tp, pos) = match t.phase {
        1 => (t.q[0], e),
        _ => (t.q[t.ext as usize], t.q[t.ext as usize]),
    };
    let ob2 = vec3(m.vm, bonepos(t.opp, 2));
    let yaw = atan_br2(ob2[0] - tp[0], ob2[2] - tp[2]);
    let hh = tdiv((e[0] - p0[0]).abs() << 12, rcos(yaw)).wrapping_abs();
    let pitch = atan_br2(hh, ob2[1] - tp[1]);
    m.vm.set_model_matrix(t.model, &Vm::rot2(0x800 - pitch, 0x800 - yaw), Some(pos));
    if !paused {
        t.ctr += 1;
        if t.ctr & 1 == 1 {
            t.rows += 1;
            t.ring.rotate_right(1);
        }
        t.ring[0] = t.q;
    }
    true
}

/// Shina's eye trails for `life` frames (0x0FFFFFFF = the permanent trail of costume 3).
pub fn eye_trail(m: &mut FxHost, side: usize, life: i32) {
    if m.round_state >= 0xF {
        return;
    }
    let f = &m.fighters[side];
    if f.chr == 4 && m.costume[side] == 2 && life != 0x0FFF_FFFF {
        return;
    }
    let shina = f.chr == 4;
    let t = EyeTrail { side, life, cnt: 0, shina, armed: shina && f.form == 0x11, shown: true, hist: [[[0; 3]; 6]; 2] };
    m.fxn.tasks.push(Task::EyeTrail(Box::new(t)));
}

/// Shina's eye trails, a frame; false = the task ends.
fn eye_tick(m: &mut FxHost, t: &mut EyeTrail, paused: bool) -> bool {
    if paused {
        return true;
    }
    t.shown = true;
    t.life -= 1;
    if t.life <= 0 {
        return false;
    }
    if t.shina {
        if m.fighters[t.side].form == 0 {
            t.armed = false;
            t.shown = false;
            return true;
        }
        if !t.armed {
            t.armed = true;
            t.cnt = 0;
        }
    }
    t.cnt = t.cnt.wrapping_add(1);
    for (eye, x) in [10, -10].into_iter().enumerate() {
        t.hist[eye][0] = bone_point(m.vm, t.side, 2, [x, -23, -40]);
        t.hist[eye].copy_within(0..5, 1);
    }
    true
}

/// The screen prims of the C tasks (tongue body, blur sheet, eye trails) for the model list.
pub fn prims(st: &State, gte: &Gte) -> Vec<ModelPrim> {
    let mut out = Vec::new();
    let h = 360;
    let proj = |p: [i32; 3]| gte.rtps(p).map(|q| (q.sx.trunc() as i32, q.sy.trunc() as i32, q.otz));
    for task in &st.tasks {
        match task {
            Task::Tongue(t) => {
                // fighters 2 / 3 of the team battle (engine addition): VRAM bank 1 (CLUT bit 15)
                let bank = bank_bit(t.side);
                let s: Vec<_> = t.q.iter().map(|&p| proj(p)).collect();
                let pt = |k: usize| s[k].map(|(x, y, _)| [x, y]).unwrap_or([0, 0]);
                let otz = |k: usize| s[k].map(|q| q.2).unwrap_or(0);
                let mut ang = [0i32; 17];
                let mut wid = [0i32; 17];
                for k in 0..16 {
                    let (p, q) = (pt(k), pt(k + 1));
                    ang[k] = atan_br2(q[0] - p[0], q[1] - p[1]);
                    wid[k] = tdiv(h << 5, otz(k) * 4);
                }
                ang[16] = ang[15];
                wid[16] = wid[15];
                for k in 1..17 {
                    let (p, c) = (ang[k - 1], ang[k]);
                    let md = (p + c) / 2;
                    ang[k] = if (p - c).abs() < 0x800 { md } else { (md + 0x800) & 0xFFF };
                }
                let edge = |i: usize, sg: i32| {
                    let p = pt(i);
                    [(p[0] + sg * ((rcos(ang[i]) * wid[i]) >> 12)) as f32, (p[1] + sg * ((rsin(ang[i]) * wid[i]) >> 12)) as f32]
                };
                let v0 = t.v0;
                for k in 0..t.ext.clamp(0, 16) as usize {
                    if s[k].is_none() || s[k + 1].is_none() || ((otz(k) - 4) as u32) >= 0x80B {
                        continue;
                    }
                    out.push(ModelPrim {
                        xy: [edge(k, 1), edge(k + 1, 1), edge(k, -1), edge(k + 1, -1)],
                        otz: otz(k),
                        rgb: [[128; 3]; 4],
                        uv: [[0x20, v0], [0x27, v0], [0x20, v0 + 15], [0x27, v0 + 15]],
                        tpage: 0x1F,
                        clut: t.clut | bank,
                        textured: true,
                        semi: false,
                    });
                }
                // the motion-blur sheet: the curve joined to its positions 1-2 / 3-4 / 5-6 frames ago (ot[16])
                let rows = t.rows.min(3);
                if rows > 0 {
                    let lines: Vec<Vec<Option<[f32; 2]>>> = (0..=rows as usize)
                        .map(|j| {
                            (0..17)
                                .map(|k| if j == 0 { s[k].map(|q| [q.0 as f32, q.1 as f32]) } else { proj(t.ring[j][k]).map(|q| [q.0 as f32, q.1 as f32]) })
                                .collect()
                        })
                        .collect();
                    for r in 0..rows as usize {
                        let g = |v: i32| [v.max(0) as u8; 3];
                        let (ga, gb) = (g(0xA0 - 0x3C * r as i32), g(0x78 - 0x3C * r as i32));
                        for k in 0..16 {
                            let (Some(a0), Some(a1), Some(a2), Some(a3)) = (lines[r][k], lines[r + 1][k], lines[r][k + 1], lines[r + 1][k + 1]) else {
                                continue;
                            };
                            out.push(ModelPrim {
                                xy: [a0, a1, a2, a3],
                                otz: 16,
                                rgb: [ga, gb, ga, gb],
                                uv: [[0x20, v0], [0x21, v0], [0x20, v0 + 1], [0x21, v0 + 1]],
                                tpage: 0x3F,
                                clut: t.blur_clut | bank,
                                textured: true,
                                semi: true,
                            });
                        }
                    }
                }
            }
            Task::EyeTrail(t) => {
                if !t.shown || t.cnt <= 0 {
                    continue;
                }
                let p: Vec<Vec<_>> = t.hist.iter().map(|e| e.iter().map(|&v| proj(v)).collect()).collect();
                let Some((_, _, oz)) = p[0][0] else { continue };
                let hw = tdiv(h << 4, oz * 16); // 4·H / SZ (SZ = otz·4)
                for eye in 0..2 {
                    for k in 0..5usize.min(t.cnt as usize) {
                        let (Some(a), Some(b)) = (p[eye][k], p[eye][k + 1]) else { continue };
                        let ang = atan_br2(b.0 - a.0, b.1 - a.1);
                        let (ox, oy) = ((hw * rcos(ang)) >> 12, (hw * rsin(ang)) >> 12);
                        let c = |v: i32| [v as u8, 0, 0];
                        let (ca, cb) = (c(255 - 50 * k as i32), c(255 - 50 * (k as i32 + 1)));
                        let xy = |q: (i32, i32, i32), sg: i32| [(q.0 + sg * ox) as f32, (q.1 + sg * oy) as f32];
                        out.push(ModelPrim {
                            xy: [xy(a, 1), xy(a, -1), xy(b, 1), xy(b, -1)],
                            otz: 16,
                            rgb: [ca, ca, cb, cb],
                            uv: [[0; 2]; 4],
                            tpage: 0x3F,
                            clut: 0,
                            textured: false,
                            semi: true,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Debug: (tongue phase/ext/hold/rows) and eye-trail (life/cnt) states.
pub fn debug_line(st: &State) -> String {
    let mut s = String::new();
    for t in &st.tasks {
        match t {
            Task::Tongue(t) => s += &format!(" tongue[p{} ext{} hold{} rows{} tip{:?}]", t.phase, t.ext, t.hold, t.rows, t.q[t.ext.clamp(0, 16) as usize]),
            Task::EyeTrail(t) => s += &format!(" eyes[life{} cnt{} shown{} {:?}]", t.life, t.cnt, t.shown, t.hist[0][0]),
            _ => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tongue(side: usize, opp: usize, ext: i32, bone: usize) -> Task {
        Task::Tongue(Box::new(Tongue {
            side,
            opp,
            bone,
            hold: 0,
            phase: 2,
            ext,
            ctr: 0,
            rows: -1,
            q: [[0; 3]; 17],
            ring: [[[0; 3]; 17]; 4],
            model: 0,
            v0: 0,
            clut: 0,
            blur_clut: 0,
        }))
    }

    fn eyes(side: usize) -> Task {
        Task::EyeTrail(Box::new(EyeTrail { side, life: 1, cnt: 0, shina: false, armed: false, shown: true, hist: [[[0; 3]; 6]; 2] }))
    }

    /// A decoded state's task naming a fighter the fight does not have, or a tongue's tip off its curve, is refused
    /// (Err, where `tick` / `prims` would index past the fighters or the curve).
    #[test]
    fn decoded_tasks_out_of_range_are_refused() {
        let tables = crate::formats::tables::Tables::default();
        let check = |t: Task, n: usize| State { tasks: vec![t], ..Default::default() }.check(n, &tables);
        assert!(check(Task::Recovery { m: 0, side: 1, ang: 0, size: 0 }, 2).is_ok());
        assert!(check(Task::Recovery { m: 0, side: 2, ang: 0, size: 0 }, 2).is_err());
        assert!(check(Task::Recovery { m: 0, side: 3, ang: 0, size: 0 }, 4).is_ok());
        assert!(check(Task::RecoveryLater { side: 4, delay: 1 }, 4).is_err());
        assert!(check(Task::RecoveryLater { side: usize::MAX, delay: 1 }, 2).is_err());
        assert!(check(tongue(0, 1, 16, 15), 2).is_ok());
        assert!(check(tongue(2, 3, 0, 0), 4).is_ok());
        assert!(check(tongue(2, 1, 0, 0), 2).is_err());
        assert!(check(tongue(0, 2, 0, 0), 2).is_err());
        assert!(check(tongue(0, 1, 17, 0), 2).is_err());
        assert!(check(tongue(0, 1, -1, 0), 2).is_err());
        assert!(check(tongue(0, 1, 0, 16), 2).is_err());
        assert!(check(eyes(1), 2).is_ok());
        assert!(check(eyes(2), 2).is_err());
        assert!(check(eyes(5), 4).is_err());
    }
}
