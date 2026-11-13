//! The frame steps a fight of either kind runs the same way over its fighters: the sound queues' tick, the flash
//! tasks, the fades, the skeleton's parts and tail, the stage update.
//!
//! State: none of its own: steps the parts of the serialised `Match` / `TeamFight` it is lent
//!
//! Free functions shared by the match's frame (`fight::sim::frame`) and the team battle's (`fight::team::frame`,
//! engine addition), which keep their own step order and call these at their place in it.

use crate::fight::anim::spring::ExtraNode;
use crate::fight::fight_data::Loaded;
use crate::fight::fighter::{F6C, Fighter};
use crate::fight::fx::script_vm::Vm;
use crate::fight::stage::{Br1DebrisEvents, Stage};
use crate::formats::gltf_model::ModelData;

/// The main game's queue; the queue's SE part for the imported game's: count the delays down, start at
/// most one sound effect this frame. `started` is cleared first, then holds the id started.
pub(crate) fn sound_tick(queue: &mut Vec<(i16, i16)>, started: &mut Vec<i16>) {
    started.clear();
    let mut first = false;
    queue.retain_mut(|(id, d)| {
        if *d > 0 {
            *d -= 1;
            return true;
        }
        if first {
            return true;
        }
        first = true;
        started.push(*id);
        false
    });
}

/// The Beast Drive darkening and its step (the fade step's stage part) on the stage fade `g` (G.4A58):
/// while any of `fighters` is in a Beast Drive (F6C 0x8000) the stage fades to black (+16 a frame, at 255 it is no
/// longer drawn), otherwise it fades back (−8).
pub(crate) fn stage_fade_step(g: &mut [i16; 4], fighters: &[Fighter]) {
    let drive = fighters.iter().any(|f| f.flags[F6C] & 0x8000 != 0);
    g[1] = 0;
    g[3] = if drive { 0x10 } else { -8 };
    let v = g[2] as i32 + g[3] as i32;
    g[2] = v.clamp(0, 255) as i16;
    if !(0..=255).contains(&v) || v == 0 || v == 255 {
        g[3] = 0;
    }
}

/// The fade step on one palette fade `p` (P.4B18; op 0x2B sub 1/2, decide): its delay, else its step;
/// the fighter it fades (`f`, if any) blacked out (F6C 0x400, BKOUT) at 255.
pub(crate) fn fade_step(p: &mut [i16; 4], f: Option<&mut Fighter>) {
    if p[0] > 0 {
        p[0] -= 1;
    } else {
        let v = p[2] as i32 + p[3] as i32;
        p[2] = v.clamp(0, 255) as i16;
        if v <= 0 || v >= 255 {
            p[3] = 0;
        }
    }
    if let Some(f) = f {
        if p[2] == 255 {
            f.flags[F6C] |= 0x400; // BKOUT
        } else {
            f.flags[F6C] &= !0x400;
        }
    }
}

/// Main-game fighter `f`'s skeleton of this frame (`duel::skeleton`'s `nodes` / `rots`) on it: parts 2 / 3 / 4
/// (head, chest, pelvis: model +0x124 / +0x174, by `model`'s joints; the root where it has none) for the camera,
/// throws and the AI, and the nodes for the hits and the push-apart.
pub(crate) fn skeleton_parts(f: &mut Fighter, model: &ModelData, nodes: &[[i32; 3]], rots: &[[[i32; 3]; 3]]) {
    let root = f.m_t;
    let part = |k: usize| model.joints.get(k).and_then(|&j| nodes.get(j)).copied().unwrap_or(root);
    (f.head, f.chest, f.hips) = (part(2), part(3), part(4));
    f.parts = nodes.to_vec();
    f.part_rots = rots.to_vec();
}

/// The skeleton's tail of main-game fighter `f` once its parts are set (`skeleton_parts`): op 07.03's pelvis
/// lock (the offset keeping the pelvis in place on screen, then blending out with the animation), then the spring
/// chains (extra nodes of type 2: hair, tails) following the animated skeleton `nodes` / `rots`, `extras` its extra nodes
/// by form; their aims from world (game units, y down) into the glTF skeleton's model space, `portable` (simulation
/// version 3 on, `Loaded::portable`) by `portable::transform`.
pub(crate) fn skeleton_tail(f: &mut Fighter, extras: &[Vec<ExtraNode>; 2], nodes: &[[i32; 3]], rots: &[[[i32; 3]; 3]], portable: bool) {
    // op 07.03: keep the pelvis in place on screen, then let the offset blend out with the animation
    if let Some(h) = f.pelvis_lock.take() {
        f.draw_off = [0, 1, 2].map(|k| (h[k] - f.hips[k]) as f32);
    } else {
        let a = &f.anim;
        let frac = if a.left > a.speed && a.left > 0 { a.speed as f32 / a.left as f32 } else { 1.0 };
        f.draw_off = f.draw_off.map(|v| v * (1.0 - frac));
    }
    // extra nodes of type 2: the spring chains (hair, tails) follow the animated skeleton
    let form = f.beast() as u8;
    if f.spring_form != form {
        f.springs.clear();
        f.spring_form = form;
    }
    let aims = crate::fight::anim::spring::step(&mut f.springs, &extras[form as usize], nodes, rots, f.o_94 as i32, false, portable);
    // world (game units, y down) → model space of the glTF skeleton
    f.spring_aims = if portable {
        let inv = crate::base::portable::transform(f.m_t, f.yaw).inverse();
        aims.into_iter().map(|(n, d)| (n, inv.transform_vector3(glam::DVec3::new(d[0] as f64, -d[1] as f64, -d[2] as f64)).as_vec3())).collect()
    } else {
        let inv = super::transform(f).inverse();
        aims.into_iter().map(|(n, d)| (n, inv.transform_vector3(glam::Vec3::new(d[0], -d[1], -d[2])))).collect()
    };
}

/// The stage update: the walls' hp, shake and break (`run` false in AFTERIMAGE mode, M.9E != 0: it
/// returns at once, no wall damage, shake or break), the debris of a broken wall and its events (`events`, under
/// `frame`). On an imported stage from simulation v18 a broken wall falls into the imported game's own debris
/// (its sounds through its countdown, stepped with the stage's draw as the imported game's debris step steps it); else into
/// the main game's, its sounds queued, the Tatami debris' splash spawned in the effect VM `vm`, the break's sound 0x82.
pub(crate) fn stage_step(stage: &mut Stage, data: &Loaded, run: bool, vm: &mut Vm, sound_queue: &mut Vec<(i16, i16)>, events: &mut Vec<String>, frame: u64) {
    let br1 = data.br1_stage.as_ref().filter(|_| data.gates().br1_debris);
    if let Some(s) = br1 {
        if run {
            stage.update(false);
            for side in stage.walls.broke.clone() {
                stage.spawn_debris_br1(side, s);
            }
        }
        for &side in &stage.walls.broke {
            events.push(format!("frame {frame}: wall {side} broke"));
        }
        return;
    }
    if run {
        stage.update(false);
        for side in stage.walls.broke.clone() {
            stage.spawn_debris(side, &data.trig.random);
        }
        let ev = stage.update_debris(data.stage_no);
        for id in ev.sounds {
            sound_queue.push((id, 0));
        }
        for p in ev.sinks {
            // Tatami debris splash: ST2_72 at the piece
            vm.spawn(3, 3, None, &[p[0] as u32, p[1] as u32, p[2] as u32]);
        }
    }
    for &side in &stage.walls.broke {
        events.push(format!("frame {frame}: wall {side} broke"));
        sound_queue.push((0x82, 0));
    }
}

/// Simulation v18 on an imported stage `s` (its routine, in the imported game's draw): Uriko's stage's
/// walls on the form of the fighter it watches (`form`; `cam_yaw` the camera's), their shatters into the imported game's
/// debris, then the debris step (from the main game's wall breaks too: `stage_step`). Returns their
/// sounds as `duel::br1::stage_sounds` takes them (the walls' thuds first, the stage sounds' arguments; the raw ids,
/// the splashes).
pub(crate) fn br1_stage_debris(stage: &mut Stage, s: &crate::fight::stage::br1::Bounds, form: u8, cam_yaw: i16) -> Br1DebrisEvents {
    let standing = [0, 1, 2, 3].map(|w| stage.walls.model[w] >= 0);
    let ev = stage.br1.step_walls(s.g94, form, cam_yaw, standing);
    for w in ev.shatter {
        stage.spawn_debris_br1(w, s);
    }
    let d = stage.update_debris_br1(s);
    Br1DebrisEvents { stage: std::iter::repeat_n(true, ev.thuds).chain(d.stage).collect(), ..d }
}

/// One fighter's flash task (started by the colour flash): the back colour `back` of its pattern's current step
/// (`Trig::flash`), the pattern cycled; its duration only runs down unpaused (`frozen` false), the task ends at 0.
pub(crate) fn colour_flash(trig: &crate::fight::physics::Trig, task: &mut Option<(u8, u16, i16)>, back: &mut Option<[i32; 3]>, frozen: bool) {
    *back = None;
    let Some((pat, i, dur)) = *task else { return };
    let cols = trig.flash.get(pat.min(5) as usize).map_or(&[][..], Vec::as_slice);
    let n = cols.len().max(1);
    *back = Some(cols.get((i as usize).min(n - 1)).copied().unwrap_or([0; 3]));
    let ni = if (i as usize) + 1 < n { i + 1 } else { 0 };
    let nd = if frozen { dur } else { dur - 1 };
    *task = if nd == 0 { None } else { Some((pat, ni, nd)) };
}
