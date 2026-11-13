//! The imported game's move decision: which state a fighter takes this frame.
//!
//! State: none of its own: updates the serialised `Br1Fighter`s and `Walls`
//!
//! Returns −1 (keep), a movement / reaction state (< 0x80), an attack `0x80 + index` (the main game: 0x90), a command
//! candidate `mv + 0x80`, or a chain follow-up `0x1080 + mv`. The main game's routine (`fight/move_decision/`) without
//! the tech rolls, the wall moves, the guard counters, the Beast Drive and the full-gauge circle rule.
//!
//! The code keeps the game's control flow: one function, its labels (`L::…`) in a loop, the
//! locals named after the registers that hold them (`s4` F.50, `s5` F.54, `t1` F.58, `s2` F.60 the hit result, `t4` F.5C, `a1`
//! F.64, all as at the entry; `a3` the held directions, `t2` the double-tapped ones, `t0` the recent buttons; `s1` the
//! state, `s3` the attack index). F.54 / F.64 are read back from the block where the game reloads them.

use super::input;
use super::object::Br1Fighter;
use super::physics::Walls;
use super::script::{Call, Ctx, Request, div, model_switch};

/// What the decision reads of the match: `G.98` (the dash / rave option byte), `*(G.BC)` + 4 (rounds to win), + 0x38
/// (the L1 / L2 moves 8 / 9 option word).
#[derive(Clone, Copy, Debug, Default)]
pub struct Opts {
    pub g98: u8,
    pub need: u8,
    pub opt38: u32,
    /// engine switch: the rave may start (BEAST RAVE; on in the imported game's VS default, which the reference runs set
    /// explicitly; off by default (`Default`): the rule in a main-game match, a user decision of the import)
    pub rave: bool,
}

/// The decision's labels: the sections its loop jumps between.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum L {
    /// the reaction choice
    Reaction,
    /// no new hit
    NoNewHit,
    /// after a hit: getting up, in the air, the walls
    AfterHit,
    /// the air recovery
    AirRecovery,
    /// knocked down
    KnockedDown,
    /// the movement states
    Movement,
    /// guard possible this frame (F.54 0x4000)
    MayGuard,
    /// a dash / run stopped by back or START (dash option), then the guard flag
    DashStop,
    /// the hit stun counted down
    StunTick,
    /// the shared tail
    Tail,
    /// the end: s3 ≥ 0 → s3 + 0x80, else s1
    End,
}

/// A section of the decision the frame runner's coverage counts (`at`): the labels of the loop (`L`) and the branches
/// inside them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Section {
    Reaction,
    NoNewHit,
    AfterHit,
    AirRecovery,
    KnockedDown,
    Movement,
    MayGuard,
    DashStop,
    StunTick,
    Tail,
    End,
    KnockDownRequest,
    ThrowMade,
    Thrown,
    NewHit,
    KoKnockBack,
    GettingUp,
    AirborneAfterHit,
    WallHit,
    AirRecoveryTaken,
    DownStay,
    GetUpDirection,
    InTheAir,
    DashRun,
    SideStepEnd,
    Neutral,
    Jump,
    DoubleTap,
    OptionMoves,
    InAttack,
    ChainFollowUp,
    AirAttacks,
    OnGround,
    GenericCommand,
    AttackTable,
}

impl Section {
    /// Every section, in tag order.
    pub const ALL: [Section; 35] = [
        Section::Reaction,
        Section::NoNewHit,
        Section::AfterHit,
        Section::AirRecovery,
        Section::KnockedDown,
        Section::Movement,
        Section::MayGuard,
        Section::DashStop,
        Section::StunTick,
        Section::Tail,
        Section::End,
        Section::KnockDownRequest,
        Section::ThrowMade,
        Section::Thrown,
        Section::NewHit,
        Section::KoKnockBack,
        Section::GettingUp,
        Section::AirborneAfterHit,
        Section::WallHit,
        Section::AirRecoveryTaken,
        Section::DownStay,
        Section::GetUpDirection,
        Section::InTheAir,
        Section::DashRun,
        Section::SideStepEnd,
        Section::Neutral,
        Section::Jump,
        Section::DoubleTap,
        Section::OptionMoves,
        Section::InAttack,
        Section::ChainFollowUp,
        Section::AirAttacks,
        Section::OnGround,
        Section::GenericCommand,
        Section::AttackTable,
    ];
}

/// The bit that marks a trace entry as a decision section (the script step's trace holds pcs, file offsets below it).
pub const SECTION: u32 = 1 << 31;

/// The section a trace entry names, if it is one.
pub fn section(tag: u32) -> Option<Section> {
    (tag & SECTION != 0).then(|| Section::ALL.get((tag & !SECTION) as usize).copied()).flatten()
}

impl L {
    fn section(self) -> Section {
        match self {
            L::Reaction => Section::Reaction,
            L::NoNewHit => Section::NoNewHit,
            L::AfterHit => Section::AfterHit,
            L::AirRecovery => Section::AirRecovery,
            L::KnockedDown => Section::KnockedDown,
            L::Movement => Section::Movement,
            L::MayGuard => Section::MayGuard,
            L::DashStop => Section::DashStop,
            L::StunTick => Section::StunTick,
            L::Tail => Section::Tail,
            L::End => Section::End,
        }
    }
}

/// The beast form's revert (its effect script and the sound 4), the gauge spent.
fn revert(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx) {
    let f = &mut fs[me];
    ctx.sound(f.f.player, me, 4, 0);
    ctx.out.push((me as u8, Request::Call { call: Call::EffectScript, args: [1, me as i32, 0, 0, 0, 0] }));
    f.f.gauge = 0;
    f.f.gauge_queued = 0;
    f.f.form = 0;
    f.f.damage_perm = f.f.damage;
    model_switch(f, 0);
}

/// The knock-down moves 0x6B..0x70 by the hit's side (F.50 0x20000 / 0x10000) and the stage-centre angle F.68.
fn knockdown(s4: u32, a68: i16) -> i32 {
    let far = (a68 as i32).abs() >= 0x400;
    if s4 & 0x2_0000 != 0 {
        if far { 0x6E } else { 0x6D }
    } else if s4 & 0x1_0000 != 0 {
        if far { 0x6F } else { 0x70 }
    } else if far {
        0x6C
    } else {
        0x6B
    }
}

/// The per-opponent move: the victim's row for the thrower's character (`S.4`), entry `S.34 − 0xB0`.
fn thrown_state(fs: &[Br1Fighter; 2], me: usize, ctx: &Ctx) -> i32 {
    let anm = &ctx.res[me].anm;
    let chr = fs[1 - me].f.chr as usize;
    let row = anm.u32((anm.header[2] as usize).wrapping_add(4 * chr)) as i64;
    let o = row + 2 * (fs[me].s.thrown_by as i64 - 0xB0);
    usize::try_from(o).map_or(0, |o| anm.i16(o)) as i32
}

/// Debug (the frame runner's coverage): the section `s` reached, into the context's trace.
fn at(ctx: &mut Ctx, s: Section) {
    if let Some(t) = ctx.trace.as_deref_mut() {
        t.push(SECTION | s as u32);
    }
}

fn press(t0: u16, p: i32, k: i32, b: i32, s3: &mut i32) -> bool {
    if t0 & 0x80 != 0 {
        *s3 = p;
    } else if t0 & 0x40 != 0 {
        *s3 = k;
    } else if t0 & 0x20 != 0 {
        *s3 = b;
    } else {
        return false;
    }
    true
}

/// Fighter `me`'s move decision.
pub fn decide(fs: &mut [Br1Fighter; 2], me: usize, ctx: &mut Ctx, o: Opts, walls: &mut Walls) -> i32 {
    let f = &mut fs[me];
    let a3 = f.f.held & 0xF000;
    let t2 = f.f.dbl & 0xF000;
    let t0 = if f.f.form & 1 != 0 { f.f.recent & 0xE0 } else { f.f.recent & 0xC0 };
    let (s4, s5, t1, s2, t4, a1) = (f.f.f50, f.f.f54, f.f.f58, f.f.f60, f.f.f5c, f.f.f64);
    let player = f.f.player;
    let mut s3: i32 = -1;
    let mut s1: i32 = -1;
    if f.f.recover_timer > 0 {
        f.f.recover_timer = f.f.recover_timer.wrapping_sub(f.f.step);
    }

    // ---- pending requests (F.64) ----
    if a1 & 0x10_0000 != 0 {
        // the knock-down request
        at(ctx, Section::KnockDownRequest);
        if f.anim.pos[1] < 0x100 {
            f.anim.pos[1] = 0;
        }
        f.f.vy = 0;
        if f.f.gravity == 0 {
            f.f.gravity = 0xC0;
        }
        f.f.push_speed = -0x1400;
        f.f.push_decay = 0x78;
        f.f.knock_dir = f.f.yaw.wrapping_add(f.f.centre_angle);
        f.f.f64 &= 0xFFF7_FFFF;
        if a1 & 0x8_0000 != 0 {
            return f.s.land_state as i32;
        }
        f.f.f64 &= 0xFFEF_FFFF;
        return knockdown(s4, f.f.centre_angle);
    }
    if a1 & 0x40_0000 != 0 {
        // a throw this fighter made (the resolution)
        at(ctx, Section::ThrowMade);
        f.f.f64 &= 0xFFBF_FFFF;
        s3 = f.s.att_mv as i32;
        f.s.att_mv = -1;
        f.s.att_prio = -1;
        return s3 + 0x80;
    }
    if a1 & 0x80_0000 != 0 {
        // thrown
        at(ctx, Section::Thrown);
        f.f.f64 &= 0xFF7F_FFFF;
        return thrown_state(fs, me, ctx) + 0x80;
    }
    let new_hit = if s4 & 0x4010_0000 != 0 {
        if a1 & 0x8_0000 != 0 {
            f.f.f64 &= 0xFFF7_FFFF;
            return f.s.land_state as i32;
        }
        if s4 & 0x4000_0000 == 0 || s5 & 0x400_0000 == 0 {
            f.f.f54 &= 0xFBFF_FFFF;
            return s1;
        }
        true
    } else {
        s5 & 0x400_0000 != 0
    };

    let mut lab = L::NoNewHit;
    if new_hit {
        // ---- a new hit ----
        at(ctx, Section::NewHit);
        f.f.f54 &= 0xFBFF_FFFF;
        f.f.f64 &= 0xFFF7_FFFF;
        f.s.state_cur = -1;
        ctx.out.push((me as u8, Request::Call { call: Call::MoveCancel, args: [player as i32, 0, 0, 0, 0, 0] }));
        input::reset_commands(f);
        if s2 & 0x200_0000 != 0 {
            if s4 & 0x2_0000 != 0 {
                return 0x69;
            }
            if s4 & 0x1_0000 != 0 {
                return 0x6A;
            }
        }
        if s2 & 0x0305_6800 != 0 && f.f.form & 1 != 0 && f.f.gauge < 0 && f.f.chr != 8 {
            revert(fs, me, ctx);
        }
        let f = &mut fs[me];
        if f.f.status & 0x80 != 0 {
            // KO'd: the knock-back doubled (to 0x6000)
            at(ctx, Section::KoKnockBack);
            f.f.f54 &= 0xFFDF_FFFF;
            let v = f.f.push_speed.wrapping_mul(2);
            f.f.push_speed = if v > 0x6000 { 0x6000 } else { v };
            f.f.push_decay = 0x78;
            if s2 & 0x0105_6800 == 0 {
                f.f.vy = -0x600;
                let v = div(f.f.vy as i32 * 1500, f.f.weight as i32);
                f.f.gravity = 0xC0;
                f.f.launch_speed = f.f.push_speed.wrapping_neg();
                f.f.f60 |= 0x4400;
                f.f.vy = v as i16;
                if s4 & 0x800 != 0 || s5 & 0x4_0000 != 0 {
                    f.f.vy /= 2;
                }
                if f.f.vy < -0x2000 {
                    f.f.vy = -0x2000;
                }
                if f.f.form & 1 != 0 {
                    if f.f.chr == 8 {
                        return 0x5B;
                    }
                    if !(f.f.rounds_lost < o.need && f.f.gauge >= 0) {
                        revert(fs, me, ctx);
                    }
                }
                return 0x5B;
            }
            let v = f.f.vy as i32 * 9;
            f.f.vy = ((if v < 0 { v + 7 } else { v }) >> 3) as i16;
            if f.f.vy < -0x2000 {
                f.f.vy = -0x2000;
            }
        }
        lab = L::Reaction;
    }

    loop {
        at(ctx, lab.section());
        let f = &mut fs[me];
        match lab {
            // ---- the hit reaction from the hit result s2 ----
            L::Reaction => {
                if s4 & 0x30 != 0 {
                    if s2 & 0x10_0000 != 0 {
                        s1 = 0x44;
                    } else if s2 & 0x60_0000 != 0 {
                        s1 = 0x43;
                    }
                } else if s2 & 0x10_0000 != 0 {
                    s1 = 0x42;
                } else if s2 & 0x20_0000 != 0 {
                    s1 = 0x41;
                } else if s2 & 0x40_0000 != 0 {
                    s1 = 0x40;
                }
                let side = s2 & 3;
                let corner = s2 & 0xC;
                if side == 2 {
                    s1 = 0x45;
                }
                if s2 & 0x800 != 0 {
                    if corner == 4 {
                        s1 = 0x59;
                    } else if corner == 8 {
                        s1 = 0x58;
                    }
                }
                if s2 & 0x800 != 0 && s5 & 0x10_0000 != 0 {
                    if corner == 4 {
                        s1 = 0x58;
                    } else if corner == 8 {
                        s1 = 0x59;
                    }
                }
                if s2 & 0x1000 != 0 {
                    s1 = if side == 2 { 0x53 } else { 0x4F };
                }
                if s2 & 0x2000 != 0 {
                    s1 = if side == 2 { 0x54 } else { 0x57 };
                }
                if s2 & 0x100_0000 != 0 {
                    s1 = if side == 2 { 0x51 } else { 0x50 };
                }
                if s2 & 0x4000 != 0 {
                    s1 = 0x5B;
                }
                if s2 & 0x8000 != 0 {
                    if corner == 4 {
                        s1 = 0x5F;
                    } else if corner == 8 {
                        s1 = 0x5E;
                    }
                    if s5 & 0x10_0000 != 0 {
                        s1 = 0x53;
                    }
                }
                if s2 & 0x1_0000 != 0 {
                    if corner == 4 {
                        s1 = 0x55;
                    } else if corner == 8 {
                        s1 = 0x56;
                    }
                    if side == 1 {
                        s1 = 0x60;
                    }
                }
                if s2 & 0x2_0000 != 0 {
                    s1 = 0x5C;
                }
                if s2 & 0x4_0000 != 0 {
                    s1 = if side == 2 { 0x57 } else { 0x54 };
                }
                if s2 & 0x80_0000 == 0 {
                    return s1;
                }
                if s4 & 0x30 != 0 {
                    return 0x65;
                }
                if s2 & 0x30_0000 != 0 {
                    return 0x64;
                }
                if s2 & 0x40_0000 != 0 {
                    return 0x63;
                }
                return s1;
            }
            // ---- no new hit ----
            L::NoNewHit => {
                if a1 & 0x8_0000 != 0 {
                    f.f.f64 &= 0xFFF7_FFFF;
                    return f.s.land_state as i32;
                }
                if s4 & 0x1000_0000 == 0 && s5 & 0x200_0000 == 0 {
                    lab = L::AfterHit;
                    continue;
                }
                // getting up
                at(ctx, Section::GettingUp);
                if (s4 as i32) < 0 {
                    if s5 & 0x200_0000 != 0 {
                        f.f.f54 &= 0xFDFF_FFFF;
                    }
                    lab = L::Tail;
                    continue;
                }
                if s5 & 0x200_0000 != 0 {
                    f.s.state_req = 0x30;
                    f.s.state_cur = -1;
                    f.f.hit_stun = 0;
                    f.f.f54 &= 0xFDFF_FFFF;
                }
                f.f.hit_stun = f.f.hit_stun.wrapping_add(f.f.step);
                if f.f.hit_stun < 0x61 && f.f.pressed & 0x2000 != 0 && s5 & 0x80 == 0 && !(f.f.chr == 8 && f.f.form & 1 != 0) {
                    f.f.f54 |= 0x2000;
                    return 0x35;
                }
                if f.s.state_req == 0x35 {
                    if f.f.hit_stun >= 0x81 {
                        lab = L::Tail;
                        continue;
                    }
                    f.f.f54 |= 0x2000;
                    return s1;
                }
                if s4 & 0x800 != 0 {
                    return 0x33;
                }
                s1 = if s4 & 0x10 != 0 {
                    0x32
                } else if s2 & 0x40_0000 != 0 {
                    0x30
                } else {
                    0x31
                };
                f.f.f54 |= 0x2000;
                return s1;
            }
            L::AfterHit => {
                if s4 & 0x4_0000 != 0 || s4 & 0x40 != 0 {
                    lab = L::Tail;
                    continue;
                }
                if s4 & 0x600_0000 == 0 {
                    lab = L::KnockedDown;
                    continue;
                }
                // airborne after a hit
                at(ctx, Section::AirborneAfterHit);
                if s4 & 0x800 == 0 {
                    return s1;
                }
                if f.f.vy > 0 {
                    let floor = f.f.floor as i32;
                    if floor < f.world(7)[1] || floor < f.world(10)[1] {
                        if s4 & 0x400_0000 == 0 {
                            return 0x5A;
                        }
                        return f.s.land_state as i32;
                    }
                }
                if f.f.hit_stun > 0 {
                    f.f.hit_stun = f.f.hit_stun.wrapping_sub(f.f.step);
                    if f.f.hit_stun < 0 {
                        f.f.hit_stun = 0;
                    }
                }
                if s4 & 0x400_0000 != 0 {
                    // the hit strength against a wall: the weight / 10 (F.60 0x1000000) or / 20
                    at(ctx, Section::WallHit);
                    let w = f.f.weight as i32;
                    let a0 = (if s2 & 0x100_0000 != 0 { w / 10 } else { w / 20 }) as i16;
                    if s5 & 0x100_0000 == 0 {
                        lab = L::AirRecovery;
                        continue;
                    }
                    if f.f.status & 0x80 != 0 && f.f.rounds_lost >= o.need {
                        // the last KO into a wall breaks it
                        walls.w44 = 0;
                        for k in 0..4 {
                            if a1 & (1 << k) != 0 {
                                walls.hit[k] = 10000;
                                walls.lock[k] = -1;
                            }
                        }
                    } else if f.hips_y() < -0x100 {
                        for k in 0..4 {
                            if a1 & (1 << k) != 0 {
                                walls.hit[k] = a0;
                                walls.lock[k] = -1;
                            }
                        }
                        s1 = if f.f.launch_angle == 0x800 { 0x61 } else { 0x62 };
                    }
                    lab = L::AirRecovery;
                    continue;
                }
                if f.f.push_speed == 0 && (f.f.launch_speed as i32).abs() < 0x400 {
                    lab = L::AirRecovery;
                    continue;
                }
                let y = f.hips_y();
                if y >= -0x100 {
                    lab = L::AirRecovery;
                    continue;
                }
                if y < -0x27F {
                    lab = L::AirRecovery;
                    continue;
                }
                if s5 & 0x100_0000 != 0 {
                    s1 = 0x62;
                }
                lab = L::AirRecovery;
            }
            L::AirRecovery => {
                // ---- the air recovery (P + K) ----
                if t0 & 0xC0 != 0xC0 {
                    lab = L::Tail;
                    continue;
                }
                if f.f.hit_stun != 0 || f.f.vy <= 0 {
                    lab = L::Tail;
                    continue;
                }
                at(ctx, Section::AirRecoveryTaken);
                let f = &mut fs[me];
                s1 = if f.hips_y() - (f.f.floor as i32) >= -0x98 {
                    0x3E
                } else if a3 & 0x1000 != 0 {
                    0x3D
                } else {
                    0x3C
                };
                f.f.f54 |= 0x800_0000;
                if f.f.gravity <= 0 {
                    f.f.gravity = 0xC0;
                }
                if s1 == 0x3E {
                    f.f.push_decay = 0x78;
                    let (v, l) = (f.f.push_speed, f.f.launch_speed);
                    f.f.push_speed = if l < 0 { v.wrapping_sub(l) } else { v.wrapping_add(l) };
                    f.f.launch_speed = 0;
                } else if a3 & 0x2000 != 0 {
                    f.f.push_decay = 0x78;
                    f.f.push_speed = 0xC00;
                    f.f.knock_dir = f.f.launch_angle.wrapping_add(f.f.yaw.wrapping_sub(f.anim.yaw));
                    if f.f.form & 1 != 0 {
                        f.f.push_speed = 0x1000;
                    }
                } else if a3 & 0x8000 != 0 {
                    f.f.push_decay = 0x78;
                    f.f.push_speed = 0xC00;
                    f.f.knock_dir = f.f.launch_angle.wrapping_add(f.f.yaw.wrapping_sub(f.anim.yaw)).wrapping_add(0x800);
                    if f.f.form & 1 != 0 {
                        f.f.push_speed = 0x1000;
                    }
                }
                lab = L::Tail;
            }
            // ---- knocked down ----
            L::KnockedDown => {
                if s4 & 0x3_0000 == 0 || t1 & 0x20 != 0 {
                    lab = L::Movement;
                    continue;
                }
                if s4 & 0x800_0000 != 0 {
                    if s5 & 0x8_0080 != 0x8_0080 || s4 & 0x20_0000 != 0 {
                        return s1;
                    }
                    return knockdown(s4, f.f.centre_angle);
                }
                at(ctx, Section::DownStay);
                let f = &mut fs[me];
                f.f.f54 |= 0x8000;
                if f.f.down_timer > 0 {
                    f.f.down_timer = f.f.down_timer.wrapping_sub(f.f.step);
                    if f.f.down_timer < 0 {
                        f.f.down_timer = 0;
                    }
                    if f.f.pressed & 0xF0E0 != 0 && f.f.down_timer != 0 {
                        f.f.down_timer = f.f.down_timer.wrapping_sub(f.f.step);
                        if f.f.down_timer < 0 {
                            f.f.down_timer = 0;
                        }
                    }
                    if f.f.down_timer < 0x80 && f.f.dbl & 0x40 != 0 {
                        f.f.f64 |= 0x1_0000;
                    }
                    lab = L::Tail;
                    continue;
                }
                if f.f.down_timer != 0 {
                    lab = L::Tail;
                    continue;
                }
                // the get-up direction, up / down mirrored when swapped
                at(ctx, Section::GetUpDirection);
                let f = &mut fs[me];
                let mut v1 = a3 as i32;
                if f.f.held & 0x200 != 0 {
                    v1 = (a3 & 0xA000) as i32;
                    if a3 & 0x1000 != 0 {
                        v1 |= 0x4000;
                    }
                    if a3 & 0x4000 != 0 {
                        v1 |= 0x1000;
                    }
                }
                if s4 & 0x2_0000 != 0 {
                    if a1 & 0x1_0000 != 0 {
                        s3 = 0x1E;
                    } else {
                        s1 = match v1 {
                            0x2000 => 0x73,
                            0x1000 => 0x76,
                            0x4000 => 0x75,
                            0x8000 => 0x74,
                            _ => 0x71,
                        };
                    }
                } else if a1 & 0x1_0000 != 0 {
                    s3 = 0x1F;
                } else {
                    s1 = match v1 {
                        0x2000 => 0x79,
                        0x1000 => 0x7C,
                        0x4000 => 0x7B,
                        0x8000 => 0x7A,
                        _ => 0x77,
                    };
                }
                lab = L::Tail;
            }
            // ---- the movement states ----
            L::Movement => {
                if s4 & 0x0108_8000 != 0 || (s4 as i32) < 0 {
                    lab = L::Tail;
                    continue;
                }
                if s4 & 0x1000 != 0 {
                    // in the air
                    at(ctx, Section::InTheAir);
                    let f = &mut fs[me];
                    if s4 & 0x40_0000 != 0 {
                        lab = L::Tail;
                        continue;
                    }
                    let v = s4 & 0x2800;
                    let ok = match v {
                        0x800 | 0x2800 => true,
                        0x2000 => o.g98 != 0,
                        _ => false,
                    };
                    if !ok || s4 & 0x4000 != 0 {
                        lab = L::Tail;
                        continue;
                    }
                    if s5 & 0x800_0000 == 0 && f.f.held & 0x400 != 0 && t1 & 0x100_0000 == 0 && s5 & 0x1400 == 0x1000 && a3 & 0x2000 == 0 {
                        f.f.f54 |= 0x2000;
                    }
                    lab = L::MayGuard;
                    continue;
                }
                if s4 & 0x200 != 0 {
                    if f.f.hit_stun > 0 {
                        lab = L::StunTick;
                        continue;
                    }
                    if f.f.hit_stun != 0 {
                        lab = L::Tail;
                        continue;
                    }
                    lab = L::MayGuard;
                    continue;
                }
                if s4 & 0x400 != 0 {
                    // a dash / run
                    at(ctx, Section::DashRun);
                    let f = &mut fs[me];
                    if s4 & 8 != 0 {
                        if s4 & 2 != 0 {
                            match a3 {
                                0x2000 => {}
                                0x1000 => s1 = 0x1A,
                                0x3000 => s1 = 0x1B,
                                0x9000 => s1 = 0x1C,
                                _ => s1 = 6,
                            }
                            if s5 & 0x180_0000 != 0 {
                                s1 = 6;
                            }
                        }
                        f.f.f54 |= 0x4000;
                    } else if a3 != 0x2000 {
                        f.s.next_state = 6;
                    }
                    lab = L::DashStop;
                    continue;
                }
                if s4 & 0x20 != 0 {
                    if f.f.hit_stun > 0 {
                        lab = L::StunTick;
                        continue;
                    }
                    if f.f.hit_stun == 0 && a3 & 0x4000 == 0 {
                        s1 = 0x12;
                    }
                    lab = L::Tail;
                    continue;
                }
                if t1 & 0x8000 != 0 {
                    // a side step's end: up / down step on by the mirror S.128, forward / back walk
                    at(ctx, Section::SideStepEnd);
                    let f = &mut fs[me];
                    let v1 = f.f.held;
                    s1 = 1;
                    let (swapped, mirror) = (v1 & 0x200 != 0, f.anim.mirror);
                    let (up, down) = (v1 & 0x1000 != 0, v1 & 0x4000 != 0);
                    if up || down {
                        // one branch for up (down when swapped), one for down (up when swapped)
                        let first = up != swapped;
                        s1 = if first == mirror { 0x1F } else { 0x1E };
                    }
                    if a3 & 0x2000 != 0 {
                        s1 = 2;
                    }
                    if a3 & 0x8000 != 0 {
                        s1 = 3;
                    }
                    lab = L::Tail;
                    continue;
                }
                // ---- neutral ----
                at(ctx, Section::Neutral);
                let f = &mut fs[me];
                f.f.f54 |= 0x1000_0000;
                if f.f.f58 & 1 != 0 {
                    lab = L::Tail;
                    continue;
                }
                if f.f.chr == 8 && f.f.form & 1 == 0 && f.f.gauge < 0 {
                    return 0xAE;
                }
                if f.f.held & 0x400 != 0 && t1 & 0x100_0000 == 0 && s5 & 0x1400 == 0x1000 && a3 & 0x2000 == 0 {
                    f.f.f54 |= 0x2000;
                }
                f.f.f54 |= 0x4000;
                s1 = 1;
                match a3 {
                    0x6000 => s1 = 0xD,
                    0x3000 | 0x2000 => s1 = 2,
                    0x4000 => s1 = 0xB,
                    0x9000 | 0x8000 => {
                        if s5 & 0x10_0000 != 0 {
                            f.s.state_cur = -1;
                        }
                        s1 = 3;
                    }
                    0xC000 => {
                        if s5 & 0x10_0000 != 0 {
                            f.s.state_cur = -1;
                        }
                        s1 = 0xE;
                    }
                    _ => {}
                }
                if f.f.held & 0x1000 != 0 {
                    // a jump: its direction for the state after the start
                    at(ctx, Section::Jump);
                    let f = &mut fs[me];
                    f.s.next_state = match a3 & 0xA000 {
                        0x2000 => 0x18,
                        0x8000 => 0x19,
                        _ => 0x17,
                    };
                    return 0x13;
                }
                if f.f.recover_timer > 0 {
                    return s1;
                }
                match t2 {
                    0x4000 => s1 = 0x11,
                    0x2000 => {
                        s1 = 4;
                        f.s.next_state = 5;
                    }
                    0x8000 => s1 = 7,
                    _ => {}
                }
                if t2 != 0 {
                    at(ctx, Section::DoubleTap);
                }
                if o.opt38 == 0 {
                    lab = L::Tail;
                    continue;
                }
                at(ctx, Section::OptionMoves);
                let f = &mut fs[me];
                let (pressed, swapped, mirror) = (f.f.pressed, f.f.held & 0x200 != 0, f.anim.mirror);
                if pressed & 8 != 0 {
                    return if swapped != mirror { 9 } else { 8 };
                }
                if pressed & 4 != 0 {
                    return if swapped == mirror { 9 } else { 8 };
                }
                lab = L::Tail;
            }
            L::MayGuard => {
                f.f.f54 |= 0x4000;
                lab = L::Tail;
            }
            L::DashStop => {
                if o.g98 != 0 {
                    let back = if s4 & 4 != 0 { a3 & 0x2000 } else { a3 & 0x8000 };
                    if back != 0 {
                        s1 = 1;
                        f.f.recover_timer = 0x100;
                    }
                    if f.f.pressed & 0x800 != 0 {
                        s1 = 1;
                        f.f.recover_timer = 0x100;
                    }
                }
                let v1 = f.s.state_req;
                if !(v1 == 6 || o.g98 != 0 && v1 == 7) {
                    lab = L::Tail;
                    continue;
                }
                if f.f.held & 0x400 != 0 && t1 & 0x100_0000 == 0 && s5 & 0x1400 == 0x1000 && a3 & 0x2000 == 0 {
                    f.f.f54 |= 0x2000;
                }
                lab = L::Tail;
            }
            L::StunTick => {
                f.f.hit_stun = f.f.hit_stun.wrapping_sub(f.f.step);
                if f.f.hit_stun < 0 {
                    f.f.hit_stun = 0;
                }
                lab = L::Tail;
            }
            // ---- the shared tail: candidates, chains, attacks ----
            L::Tail => {
                if f.f.f58 & 0x1_0000 != 0 {
                    lab = L::End;
                    continue;
                }
                if f.f.recover_timer > 0 {
                    return s1;
                }
                // whether a command candidate may cancel this attack (its last frame, or the rave)
                let mut a1c = false;
                if (s4 as i32) < 0 && f.f.recovery == f.f.step {
                    a1c = (t4 as i32) < 0;
                    if f.f.rave != 0 && f.f.f64 & 0x200_0000 == 0 {
                        a1c = true;
                    }
                }
                if s4 & 0x4_0000 != 0 || s4 & 0x40 != 0 || s4 & 0x8000 != 0 || s4 & 0x8008_0000 == 0x8_0000 || s4 & 0x100_0000 != 0 {
                    lab = L::End;
                    continue;
                }
                if (s4 as i32) < 0 {
                    // in an attack
                    at(ctx, Section::InAttack);
                    let f = &mut fs[me];
                    let a0 = f.s.cand_mv;
                    if a0 >= 0 && a1c && f.s.cand_flags & 0x20 != 0 {
                        let v1 = f.s.cand_flags;
                        let take = if s5 & 0x800 != 0 && v1 & 0x10 != 0 {
                            true
                        } else if s4 & 0x800 != 0 {
                            v1 & 0xC != 0
                        } else if s4 & 0x10 != 0 {
                            v1 & 2 != 0
                        } else {
                            v1 & 1 != 0
                        };
                        if take {
                            s3 = f.s.cand_mv as i32;
                            f.s.cand_mv = -1;
                            input::reset_chain(f);
                            return s3 + 0x80;
                        }
                    }
                    if f.s.chain_pos != f.s.chain_len && (t4 & 0x4_0000 != 0 || t4 & 0x2000 == 0 && s4 & 0x2000_0000 == 0) {
                        let a0 = f.f.recovery;
                        let k = f.s.chain_pos as usize;
                        if a0 > 0 && a0 >= f.s.chain_queue.get(k + 16).copied().unwrap_or(0) {
                            at(ctx, Section::ChainFollowUp);
                            let f = &mut fs[me];
                            s3 = f.s.chain_queue.get(k).copied().unwrap_or(0) as i32;
                            if k as i32 == f.s.chain_sel as i8 as i32 {
                                f.s.chain_sel = 0xFF;
                            }
                            f.s.chain_pos = ((f.s.chain_pos as i32 + 1) & 0xF) as u8;
                            f.s.state_cur = -1;
                            return s3 + 0x1080;
                        }
                    }
                    if o.g98 != 0 && f.f.startup > 0 && t4 & 0x4000_0000 != 0 && f.f.pressed & 0x800 != 0 && s4 & 0x1000 == 0 {
                        s1 = if s4 & 0x10 != 0 { 0xB } else { 1 };
                        ctx.out.push((me as u8, Request::Call { call: Call::MoveCancel, args: [player as i32, 0, 0, 0, 0, 0] }));
                        f.f.recover_timer = 0x80;
                    }
                    lab = L::End;
                    continue;
                }
                if s4 & 0x1000 != 0 {
                    // ---- in the air: the cancel candidate, the beast's air attacks ----
                    at(ctx, Section::AirAttacks);
                    let f = &mut fs[me];
                    if s4 & 0x40_0000 != 0 {
                        lab = L::End;
                        continue;
                    }
                    let v1 = f.s.cancel_flags;
                    let take = if v1 & 0x2_0000 != 0 {
                        s4 & 0x2800 == 0
                    } else if v1 & 0x4_0000 != 0 {
                        s4 & 0x2800 == 0x2000
                    } else {
                        f.hips_y() < f.s.cancel_param as i32 && s4 & 0x800 != 0
                    };
                    if take {
                        s3 = f.s.cancel_mv as i32;
                        f.s.cancel_mv = -1;
                    }
                    if f.f.form & 1 != 0 && s5 & 0xA00 != 0 {
                        let up = s4 & 0x2800 == 0 || s4 & 0x2808 == 0x808 && a3 & 0x1000 != 0;
                        if up && t0 != 0 {
                            s3 = 0x33;
                        }
                    }
                    let v1 = s4 & 0x2800;
                    if (v1 == 0x800 || v1 == 0x2800) && f.f.recent & 0x20 != 0 && f.f.gauge >= 0xF00 && f.f.form & 0x10 == 0 {
                        s3 = 0x2E;
                    }
                    lab = L::End;
                    continue;
                }
                // ---- on the ground ----
                at(ctx, Section::OnGround);
                let f = &mut fs[me];
                if s4 & 0x280 != 0 {
                    lab = L::End;
                    continue;
                }
                if s4 & 0x400 != 0 {
                    if s4 & 8 != 0 {
                        if s4 & 2 != 0 && a3 & 0x2000 != 0 {
                            if t0 & 0x80 != 0 {
                                s3 = 7;
                            } else if t0 & 0x40 != 0 {
                                s3 = 0x17;
                            } else if f.f.recent & 0x20 != 0 && (f.f.form & 0x10 != 0 || f.f.gauge >= 0xF00) {
                                s3 = 0x27;
                            }
                            lab = L::End;
                            continue;
                        }
                    } else {
                        if s4 & 2 != 0 {
                            press(t0, 6, 0x16, 0x26, &mut s3);
                        }
                        if s4 & 4 != 0 && o.g98 != 0 && t0 & 0x20 != 0 {
                            s3 = 0x29;
                        }
                        lab = L::End;
                        continue;
                    }
                } else if s4 & 0x20 != 0 {
                    press(t0, 0xA, 0x1A, 0x2A, &mut s3);
                    lab = L::End;
                    continue;
                }
                // the generic command candidate
                at(ctx, Section::GenericCommand);
                let f = &mut fs[me];
                let v1 = f.s.cand_mv;
                if v1 >= 0 {
                    if s5 & 0x800 != 0 && f.s.cand_flags & 0x10 != 0 {
                        f.s.cand_mv = -1;
                        return v1 as i32 + 0x80;
                    }
                    let fl = f.s.cand_flags;
                    let take = if s4 & 0x800 != 0 {
                        fl & 0xC != 0
                    } else if s4 & 0x10 != 0 {
                        fl & 2 != 0
                    } else {
                        fl & 1 != 0
                    };
                    if take {
                        s3 = f.s.cand_mv as i32;
                        f.s.cand_mv = -1;
                        return s3 + 0x80;
                    }
                }
                // the attack table
                at(ctx, Section::AttackTable);
                let f = &mut fs[me];
                if a3 & 0x4000 != 0 {
                    if s5 & 0x10_0000 != 0 {
                        press(t0, 0xC, 0x1C, 0x2C, &mut s3);
                    } else if a3 & 0x2000 != 0 {
                        press(t0, 4, 0x14, 0x24, &mut s3);
                    } else if a3 & 0x8000 != 0 {
                        press(t0, 5, 0x15, 0x25, &mut s3);
                    } else {
                        press(t0, 3, 0x13, 0x23, &mut s3);
                    }
                    if s5 & 0x800 != 0 && f.f.recent & 0x4000 != 0 {
                        press(t0, 0x30, 0x31, 0x32, &mut s3);
                    }
                } else if s5 & 0x10_0000 != 0 {
                    press(t0, 0xB, 0x1B, 0x2B, &mut s3);
                } else if a3 & 0x2000 != 0 {
                    press(t0, 1, 0x11, 0x21, &mut s3);
                } else if a3 & 0x8000 != 0 {
                    press(t0, 2, 0x12, 0x22, &mut s3);
                } else {
                    press(t0, 0, 0x10, 0x20, &mut s3);
                }
                // the beast button in human form: the transform (gauge ≥ 0xF00) or the gauge-less move
                if f.f.recent & 0x20 != 0 && f.f.form & 0x10 == 0 {
                    s3 = if f.f.gauge < 0xF00 { 0x2D } else { 0x2E };
                }
                // the rave (△, beast form, gauge left)
                if o.rave && o.g98 != 0 && f.f.form & 0x10 != 0 && f.f.rave == 0 && f.f.recent & 0x10 != 0 && f.f.gauge > 0 {
                    f.f.rave = 1;
                    s3 = 0x2D;
                }
                lab = L::End;
            }
            L::End => return if s3 >= 0 { s3 + 0x80 } else { s1 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SECTION, Section, section};

    /// `Section::ALL` lists the sections in declaration order, so a trace tag decodes to the section `at` pushed.
    #[test]
    fn section_tags_round_trip() {
        for (i, s) in Section::ALL.iter().enumerate() {
            assert_eq!(*s as usize, i);
            assert_eq!(section(SECTION | i as u32), Some(*s));
        }
        assert_eq!(section(SECTION | Section::ALL.len() as u32), None);
        assert_eq!(section(5), None);
    }
}
