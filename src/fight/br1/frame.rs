//! The imported game's fight frame: both inputs and the throws, each fighter's frame, the pair's movement.
//!
//! State: none of its own: updates the serialised `Br1Fighter`s and `Walls`
//!
//! The frame order is the game's: input P1, input P2, the throw resolution; P1's frame, P2's frame (health and
//! gauge, auto-turn, ring and ground, the per-frame flags, the decision, the script step, the velocity); then the
//! pair: integrate, landing, the hits, the push-apart, the walls, the angles. The KO "guts" save
//! of imported (a random survival of a small overflow) is dropped: an imported fighter is KO'd by the
//! main game's rule (a user decision of the import). The spring chains (per fighter, after the angles) are not
//! run here.

use super::decision::{self, Opts};
use super::hit;
use super::input::{self, Pad};
use super::object::Br1Fighter;
use super::physics::{self, RING_EDGE, RING_HIPS, Ring, Walls};
use super::res::Br1Res;
use super::script::{self, Call, Ctx, Request};
use super::throw;

/// What the frame reads of the match besides the fighters (the globals G and what they point at).
#[derive(Clone, Copy, Debug, Default)]
pub struct World {
    /// G.68: the raw pads of P1 / P2
    pub pads: [u16; 2],
    /// `*(G.B8) + 2`: the camera's angle word (the left / right swap)
    pub cam: i16,
    /// the AI's pads `*(F.34) + 6` (S10's AI; read for a CPU fighter)
    pub ai: [u16; 2],
    /// G.94: the stage (the ring records)
    pub stage: i8,
    /// G.98, `*(G.BC)` + 4 / + 0x38
    pub opts: Opts,
    /// `*(G.BC)` + 0x2C: bit 0x80 regenerates the red health in human form and keeps the permanent damage
    pub opt2c: u32,
    /// G.7E: the logic frozen (the round loop's pauses)
    pub frozen: bool,
    /// engine addition (the import S7): by side, the pad the main game's KO replay plays back (`input::Pad::replay`)
    pub replay: [Option<u16>; 2],
}

/// The parts of the frame owned by later stages, run where the game runs them.
pub trait Later {
    /// the spring chains of nodes 19.., per fighter after the angles
    fn chains(&mut self, fs: &mut [Br1Fighter; 2]);
}

/// Both inputs (P1 then P2), then the throw resolution. Returns the pads the KO replay's recorder saw.
pub fn inputs(fs: &mut [Br1Fighter; 2], ctx: &mut Ctx, w: &World) -> [u16; 2] {
    inputs_ai(fs, ctx, w, &mut |_, k| w.ai[k])
}

/// `inputs` with the AI run where the input runs it, for side `k` from both fighters as they are then
/// (P2's after P1's input): `think(fs, k)` gives its pad (`World::ai` unused).
pub fn inputs_ai(fs: &mut [Br1Fighter; 2], ctx: &mut Ctx, w: &World, think: &mut dyn FnMut(&[Br1Fighter; 2], usize) -> u16) -> [u16; 2] {
    let mut rec = [0; 2];
    for k in 0..2 {
        let ai = think(fs, k);
        let pad = Pad { raw: w.pads[fs[k].f.player as usize & 1], cam: w.cam, ai, replay: w.replay[k] };
        rec[k] = input::process(fs, k, &ctx.res[k].anm, pad);
    }
    throw::resolve(fs, ctx);
    rec
}

/// Health, gauge and the KO. Returns whether this frame KO'd the fighter.
fn health(f: &mut Br1Fighter, me: usize, ctx: &mut Ctx, w: &World) -> bool {
    if f.f.damage < 0 || f.f.f58 & 0x80_0000 != 0 || f.f.status != 1 {
        return false;
    }
    let keep = w.opt2c & 0x80 != 0;
    if f.f.f4c & 2 == 0 {
        // the queued gauge change
        let q = f.f.gauge_queued;
        if q < 0 {
            if f.f.gauge >= 0 {
                f.f.gauge = f.f.gauge.wrapping_add(q);
                f.f.gauge_queued = 0;
                if f.f.gauge < 0 {
                    f.f.gauge = -1;
                }
            }
        } else if q > 0 {
            if f.f.gauge >= 0 {
                f.f.gauge = f.f.gauge.wrapping_add(q);
                f.f.gauge_queued = 0;
                if f.f.gauge >= 0x3C00 {
                    f.f.gauge = 0x3BFF;
                }
            } else {
                f.f.gauge_queued = 0;
            }
        }
        if f.f.form & 0x10 == 0 {
            // human: the rave ends, the gauge fills by 1 a frame
            if f.f.rave != 0 {
                f.f.rave = -1;
            }
            if f.f.gauge >= 0 {
                f.f.gauge = f.f.gauge.wrapping_add(1);
                if f.f.gauge >= 0x3C00 {
                    f.f.gauge = 0x3BFF;
                }
            } else if f.f.f50 & 0x8000 == 0 {
                f.f.gauge = 0;
            }
        } else if f.f.rave != 0 {
            // the rave drains 8 a frame
            f.f.gauge = f.f.gauge.wrapping_sub(8);
            if f.f.gauge <= 0 {
                f.f.rave = -1;
                f.f.gauge = 0;
            }
        }
        if f.f.blue < f.f.damage_perm {
            f.f.blue = f.f.blue.wrapping_add(1);
        }
    }
    // the red health regenerates (beast form, or always with the option) every F.D1 frames
    let regen = if keep {
        (f.f.damage_perm as i32) + 1 < f.f.damage as i32
    } else {
        f.f.form & 0x10 != 0 && f.f.f50 & 0x1000_8000 == 0 && f.f.damage_perm < f.f.damage
    };
    if regen {
        if f.f.regen_count != 0 {
            f.f.regen_count = f.f.regen_count.wrapping_sub(1);
        } else {
            f.f.damage = f.f.damage.wrapping_sub(1);
            f.f.regen_count = f.f.regen_interval;
        }
    }
    // the queued damage (negative) or healing (positive), 70 % of it permanent
    let perm = |v: i16| -> i16 { (v as i32 * 70 / 100) as i16 };
    let q = f.f.queued_damage;
    if q < 0 {
        if f.f.f4c & 2 != 0 {
            f.f.blue = f.f.blue.wrapping_sub(q);
        } else {
            f.f.damage = f.f.damage.wrapping_sub(q);
            if !keep {
                f.f.damage_perm = f.f.damage_perm.wrapping_sub(perm(q));
            }
        }
        if f.f.damage_perm > f.f.max_hp {
            f.f.damage_perm = f.f.max_hp;
        }
        if f.f.damage > f.f.max_hp {
            f.f.damage = f.f.max_hp;
        }
        f.f.queued_damage = 0;
        f.f.last_damage = q.wrapping_neg();
    }
    let q = f.f.queued_damage;
    if q != 0 {
        if f.f.f4c & 2 != 0 {
            f.f.blue = f.f.blue.wrapping_add(q);
        } else {
            f.f.damage = f.f.damage.wrapping_add(q);
            if !keep {
                f.f.damage_perm = f.f.damage_perm.wrapping_add(perm(q));
            }
        }
        if f.f.damage_perm > f.f.max_hp {
            f.f.damage_perm = f.f.max_hp;
        }
        f.f.queued_damage = 0;
        f.f.last_damage = q;
    }
    if f.f.damage <= f.f.max_hp {
        return false;
    }
    // KO (the imported game's guts save before it is dropped)
    f.f.f54 |= 0x20_0000;
    ctx.out.push((me as u8, Request::Call { call: Call::BackgroundFlash, args: [4, 0xFF, 0xFF, 0xFF, 0, 0] }));
    f.f.status = 0x80;
    f.f.damage = -1;
    f.f.damage_perm = -1;
    f.f.down_timer = -1;
    f.f.rounds_lost = f.f.rounds_lost.wrapping_add(1);
    true
}

/// The ground under the hips (the hips' ring record) and the ring-out: the head, node 10 or
/// node 7 past the edge record.
fn ground(f: &mut Br1Fighter, rings: [Ring; 2]) {
    if f.f.f58 & 0x1000_0000 == 0 && f.f.f64 & 0x10_0000 == 0 {
        let hips = f.world(4);
        f.f.floor = physics::ground(hips[0], hips[2], rings[0], 0) as i16;
        if f.f.floor != 0 {
            let edge = rings[1];
            let g = |n: usize| {
                let p = f.world(n);
                physics::ground(p[0], p[2], edge, 0)
            };
            let (s0, s2, v1) = (g(2), g(10), g(7));
            if s0 | s2 | v1 != 0 {
                if f.world(0)[1] >= 0 && f.f.f64 & 0x10_0000 == 0 {
                    if s0 & s2 & v1 == 0 && f.f.f50 & 0x100_0000 == 0 {
                        if f.f.f54 & 0x80 != 0 {
                            f.f.f64 |= 0x8_0000;
                        }
                        f.f.f64 |= 0x10_0000;
                    }
                    if !(f.f.f54 & 0x80 != 0 && f.world(0)[1] < 0x200) {
                        // the ring-out KO
                        if f.f.status == 1 {
                            f.f.down_timer = -1;
                            f.f.status = 0x81;
                        }
                        f.f.damage = -1;
                        f.f.damage_perm = -1;
                        f.f.f54 |= 0x8_0000;
                    }
                }
                f.f.f58 |= 0x2000_0000;
            } else {
                f.f.floor = 0;
                f.f.f58 &= 0xDFFF_FFFF;
            }
        } else {
            f.f.floor = 0;
            f.f.f58 &= 0xDFFF_FFFF;
        }
    }
    if f.f.floor != 0 {
        f.f.f54 |= 0x8000_0000;
    } else {
        f.f.f54 &= 0x7FFF_FFFF;
    }
}

/// The per-frame flags before the decision: airborne (F.54 0x40000), guard possible (0x1000),
/// the opponent lying (0xA00) or in a down attack (0x200).
fn flags(f: &mut Br1Fighter, opp50: u32) {
    f.f.f54 &= 0xEFFB_85FF;
    let air = f.f.floor as i32 - 0x66;
    if f.world(7)[1] < air && f.world(10)[1] < air && f.hips_y() < f.f.floor as i32 - 0x99 {
        f.f.f54 |= 0x4_0000;
    }
    if (opp50 as i32) < 0 {
        f.f.f54 |= 0x1000;
    }
    if opp50 & 0x3_0000 != 0 && opp50 & 0x4_0800 == 0 {
        f.f.f54 |= 0xA00;
    }
    if opp50 & 0x804_0000 == 0x800_0000 {
        f.f.f54 |= 0x200;
    }
}

/// A fighter's frame up to the script step: health and gauge, the auto-turn, the ground, the flags, the decision (its
/// result into S.88 / S.1C). Returns whether this frame KO'd the fighter.
pub fn before_script(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx, w: &World, walls: &mut Walls) -> bool {
    let ko = health(&mut fs[me], me, ctx, w);
    decide(fs, me, ctx, w, walls);
    ko
}

/// The part of a fighter's frame after the health (`before_script`): the auto-turn, the ground, the flags, the decision (in a
/// main-game match the health is the main game's, `bridge::health`).
pub fn decide(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx, w: &World, walls: &mut Walls) {
    let opp58 = fs[1 - me].f.f58;
    physics::auto_turn(&mut fs[me], opp58);
    let rings = [ctx.ring(RING_HIPS), ctx.ring(RING_EDGE)];
    ground(&mut fs[me], rings);
    let opp50 = fs[1 - me].f.f50;
    flags(&mut fs[me], opp50);
    let v = decision::decide(fs, me, ctx, w.opts, walls);
    let f = &mut fs[me];
    if v >= 0 {
        let st = (v & 0xFFF) as i16;
        f.s.state_req = st;
        if st >= 0x80 {
            f.f.f54 &= 0xEFFF_9FFF;
            f.s.attack_index = st - 0x80;
            if f.f.rave > 0 {
                let v = f.f.step_normal as i32 * 5;
                f.f.step = ((if v < 0 { v + 3 } else { v }) >> 2) as i16;
            }
        }
        if v & 0x1000 != 0 {
            f.f.f54 |= 0x40;
        } else {
            f.f.f54 &= !0x40;
        }
    }
}

/// The KO voices by character (human, beast; −1 = none).
const KO_VOICE: [[i32; 9]; 2] = [[0xC4, 0xC4, 0xC4, 0xC4, 0xC4, 0xC4, 0xC4, 0xC4, -1], [0xCD, 0xD0, -1, -1, 0xCE, 0xCD, 0xCD, 0xCD, 0xC6]];

/// A fighter's frame after the script step: the KO voice, the velocity.
pub fn after_script(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx, ko: bool) {
    let f = &mut fs[me];
    if ko {
        let id = KO_VOICE[(f.f.form & 1) as usize].get(f.f.chr as usize).copied().unwrap_or(-1);
        if id >= 0 {
            ctx.sound(f.f.player, me, id, 0);
        }
    }
    physics::velocity(f, &ctx.res[me].trig);
}

/// Fighter `me`'s frame.
pub fn fighter(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx, w: &World, walls: &mut Walls) {
    let ko = before_script(fs, me, ctx, w, walls);
    ctx.walls = walls.hp.map(|h| h >= 0);
    script::step(fs, me, ctx);
    after_script(fs, me, ctx, ko);
}

/// The pair's step up to the hits: both moved, both landed, F.54 0x40000000 while held (F.50 0x10000000).
pub fn pair_move(fs: &mut [Br1Fighter; 2], res: [&Br1Res; 2]) {
    physics::integrate(fs, res);
    physics::landing(&mut fs[0]);
    physics::landing(&mut fs[1]);
    for f in fs.iter_mut() {
        if f.f.f50 & 0x1000_0000 != 0 {
            f.f.f54 |= 0x4000_0000;
        }
    }
}

/// The pair's movement, the hits, the push-apart, the walls, the angles, the spring chains (`later`).
pub fn pair(fs: &mut [Br1Fighter; 2], ctx: &mut Ctx, w: &World, walls: &mut Walls, later: &mut dyn Later) {
    let res = ctx.res;
    pair_move(fs, res);
    hit::hits(fs, ctx, w.opts.g98);
    physics::after_hits(fs, res, ctx.ring(RING_HIPS), walls);
    later.chains(fs);
}

/// The frame's logic part: unless frozen, the inputs and throws, P1's and P2's frames and
/// the pair.
pub fn frame(fs: &mut [Br1Fighter; 2], ctx: &mut Ctx, w: &World, walls: &mut Walls, later: &mut dyn Later) {
    if w.frozen {
        return;
    }
    inputs(fs, ctx, w);
    fighter(fs, 0, ctx, w, walls);
    fighter(fs, 1, ctx, w, walls);
    pair(fs, ctx, w, walls, later);
}
