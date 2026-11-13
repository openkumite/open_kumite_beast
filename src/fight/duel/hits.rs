//! Hit detection and application of both fighters, and the combo flags after them.
//!
//! State: none of its own: steps the serialised `Match`
//!
//! `Match::hits` runs the ordered pairs (0, 1) and (1, 0) through the pair functions `probe` and `land`, and
//! `combo_flags` through `combo_drop` / `combo_reset`; they take the attacker and the defender by index, so the team
//! world (engine addition, the team battle) runs its pairs through the same code.

use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F5C, F58, F64, F68, Fighter, pair_mut};
use crate::fight::hit;

/// What `probe` found for an attacker and a defender that `can_hit` lets through.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Probe {
    /// `hit::sphere_test`'s kind (0 = a miss)
    pub kind: i32,
    /// the attack value scaled by the multiplier (the trade rule compares it)
    pub dmg: i32,
    pub mult: i32,
    pub pos: [i32; 3],
}

/// Per ordered pair: attacker `a`'s hit test on defender `d` (the skeleton nodes `nodes` by fighter).
/// None when `can_hit` refuses; else `d`'s F68 takes the direction bits.
pub(crate) fn probe(fs: &mut [Fighter], a: usize, d: usize, data: &Loaded, nodes: &[Vec<[i32; 3]>]) -> Option<Probe> {
    if !hit::can_hit(&fs[a], &fs[d]) {
        return None;
    }
    let dir = hit::direction_bits(&fs[a], &fs[d]);
    fs[d].flags[F68] = dir;
    let (fa, fd) = pair_mut(fs, a, d);
    let mult = hit::multiplier(fa, fd);
    let dmg = (fa.o_da as i32 * mult) >> 7;
    let hurt = data.fighters[d].ani.spheres(2 + fd.beast() as usize);
    let (kind, pos) = hit::sphere_test(fa, fd, &nodes[a], &nodes[d], &hurt);
    Some(Probe { kind, dmg, mult, pos })
}

/// Attacker `a`'s hit of kind `kind` with multiplier `mult` lands on `d` (`hit::apply`, the launch).
pub(crate) fn land(fs: &mut [Fighter], a: usize, d: usize, mult: i32, kind: i32, data: &Loaded) -> hit::HitResult {
    let (fa, fd) = pair_mut(fs, a, d);
    if crate::base::trace::flag("HITDBG") && !crate::base::trace::is_muted() {
        eprintln!(
            "hit: att F64 {:08x} da {} dc {} | def F58 {:08x} F5C {:08x} F68 {:08x} kind {}",
            fa.flags[F64], fa.o_da, fa.o_dc, fd.flags[1], fd.flags[2], fd.flags[5], kind
        );
    }
    let r = hit::apply(fa, fd, mult, kind);
    if let Some(k) = r.launch {
        hit::launch(fa, fd, k, &data.trig);
    }
    if crate::base::trace::flag("HITDBG") && !crate::base::trace::is_muted() {
        eprintln!("   → def F68 {:08x} F5C {:08x}", fd.flags[5], fd.flags[2]);
    }
    r
}

/// The combo bookkeeping after the hits, for one fighter: out of hit-stun and down (F58 0x48000) it drops its "in a combo" flag
/// F5C bit 29 (unless F5C bit 26); bits 30 / 13 go when F58 bit 28 is clear (unless F5C bit 25).
pub(crate) fn combo_drop(f: &mut Fighter) {
    if f.flags[F58] & 0x4_8000 == 0 && f.flags[F5C] & 0x400_0000 == 0 {
        f.flags[F5C] &= !0x2000_0000;
    }
    if f.flags[F58] & 0x1000_0000 == 0 && f.flags[F5C] & 0x200_0000 == 0 {
        f.flags[F5C] &= 0xBFFF_DFFF;
    }
}

/// The rest of the combo bookkeeping: attacker `a` restarts its combo count (+0xDD) when its victim `d` is in no
/// combo (F5C 0x60000000).
pub(crate) fn combo_reset(fs: &mut [Fighter], a: usize, d: usize) {
    if fs[d].flags[F5C] & 0x6000_0000 == 0 {
        fs[a].o_dd = 0;
    }
}

/// `Match::hit_fx` on the fighters `fs` (by index), the effect tasks `fx` and the CUSTOM settings `custom` (the team
/// battle runs it too).
#[allow(clippy::too_many_arguments)]
pub(crate) fn hit_fx(
    fx: &mut crate::fight::fx::hit_effects::Fx,
    custom: &crate::fight::rules::Custom,
    data: &Loaded,
    fs: &[Fighter],
    a: usize,
    d: usize,
    r: &hit::HitResult,
    pos: [i32; 3],
    guard_fx: &mut Vec<(usize, [i32; 3])>,
) {
    let (fa, fd) = (&fs[a], &fs[d]);
    let sext12 = |x: i32| (x << 20) >> 20;
    let p16 = pos.map(|v| v as i16);
    let pitch = fa.o_db as i32 * 16;
    let beast = fa.flags[F64] & 0x2_0000 != 0;
    if r.guarded {
        if beast {
            guard_fx.push((a, pos));
        }
        let dir = sext12(fd.o_c4 as i32 + 0x400);
        fx.flash_spawn(2, r.level);
        fx.hit_spark_alt(&data.tables, p16, dir, pitch, beast);
    } else {
        let blood = matches!(fa.s_101[0], 2 | 12) && custom.blood != 0 && !beast;
        let dir = sext12(fd.o_c4 as i32 - 0x400);
        fx.flash_spawn(3, r.level);
        fx.hit_spark(&data.tables, p16, dir, pitch, r.level | 0x1000, beast, blood);
    }
}

/// The guard sparks of a beast attacker: model_fx(F, 0x13, pos, 0x320, 0x1000, 0, 1).
pub(crate) fn guard_fx(host: &mut crate::fight::fx::host::FxHost, tables: &crate::formats::tables::Tables, guard_fx: Vec<(usize, [i32; 3])>) {
    for (a, pos) in guard_fx {
        crate::fight::fx::native_tasks::model_fx(host, tables, a, 0x13, 0, pos, 0x320, 0x1000, 0, 1);
    }
}
