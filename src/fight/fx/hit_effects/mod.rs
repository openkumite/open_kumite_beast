//! Hit effects: the effect tasks of the hit code (sparks, sprites, timelines, shards, rings, flash).
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Hit effects: the effect tasks of the hit code — billboard sprites,
//! UV animations, spawn timelines with the emitter handler, shards,
//! ring bursts and the light flash,
//! plus `hit_spark` and `hit_spark_alt`.
//!
//! Tasks live in the game's lists and run in its order each frame: 6 (animations, timelines), 8 (flash),
//! 10 (sprites), 11 (shards, ring parents), 14 (ring children). New tasks go to the head of their list and a
//! task created while its own list runs waits for the next frame; killing is two-phase (marked, freed with
//! the destructor on the next pass). Logic runs in `tick`; the quads a task draws are recorded with world
//! positions and turned into PS1 screen quads by `quads` with the current camera (the GTE projection).
//!
//! Here: the task types, the pool (`spawn`, `kill`), the shard / ring / flash spawners, the hit entry points and
//! `tick`. `sprites.rs` and `timeline.rs` hold the sprite and emitter spawners, `update.rs` a task's frame,
//! `quads.rs` the draw-list build, `trig.rs` the sine table and atan2.

use crate::base::libc_rand::rand;
use crate::formats::tables::Tables;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod quads;
mod sprites;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
mod timeline;
mod trig;
mod update;

pub use crate::output::gte::{Gte, Proj};
pub(crate) use trig::{atan_xz, rcos, rsin};

const LIST_ANIM: usize = 6;
const LIST_FLASH: usize = 8;
const LIST_SPRITE: usize = 10;
const LIST_PART: usize = 11;
const LIST_RING: usize = 14;

/// The parts of a POLY_FT4 the effects set (xy is computed when drawing).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Prim {
    pub uv: [[u8; 2]; 4],
    pub rgb: [u8; 3],
    pub clut: u16,
    pub tpage: u16,
    pub semi: bool,
}

/// One recorded draw (world positions in game units; ot slot 3 = fixed front slot, else depth sorted).
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Draw {
    /// kinds 0 / 5: square of `size` around the point (cull: 4 <= otz < 0x80F, slot otz; else slot 3, no cull)
    Square { pos: [i32; 3], size: i32, prim: Prim, sorted: bool },
    /// kind 1: rotated square
    Rot { pos: [i32; 3], size: i32, angle: i32, prim: Prim },
    /// kind 6: floor puff (square squashed by the camera pitch)
    Floor { pos: [i32; 3], size: i32, prim: Prim },
    /// kind 7: wall puff between two points
    Wall { pos: [i32; 3], p2: [i32; 3], prim: Prim },
    /// shard: screen square `life/4` px
    Shard { pos: [i32; 3], life: i32, prim: Prim },
    /// ring arc: 6 quads around the projected point
    Ring { pos: [i32; 3], rad: i32, wid: i32, ang: i32, step: i32, prims: [Prim; 6] },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Sprite {
    kind: i16,
    prims: [Prim; 2],
    alive: bool,
    size: i32,
    pos: [i16; 3],
    ex: [i16; 8],
    p2: [i16; 3], // kind 7: second point
    /// kind 3: the (fighter, bone) whose BONEPOS the sprite follows
    follow: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UvAnim {
    sprite: usize,
    uvtab: u32,
    track: u32,
    cur: i32,
    timer: i16,
    loop_n: i16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Timeline {
    pos: [i16; 3],
    /// sprite kind (modes 0 / 1 / 31) or the velocity selector (mode 2)
    kind: i32,
    /// spawn mode (bits 0..6; bit 7 = follow the source)
    mode: i32,
    /// the source joint (fighter, bone) re-read before each spawn when following
    follow: Option<(usize, usize)>,
    /// caller parameters (mode 14: attacker, opponent, count, angle)
    e: [i16; 4],
    start: u32,
    cur: u32,
    frame: i16,
    loop_n: i16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Shards {
    prims: Vec<Prim>,
    pos: Vec<[i16; 3]>,
    vel: Vec<[i16; 3]>,
    life: Vec<i16>,
    origin: [i16; 3],
    frame: i16,
    n: i16,
    emitted: i16,
    yaw: i32,
    pitch: i32,
    power: i32,
    alt: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct RingRec {
    life: i16,
    rad: i16,
    d_rad: i16,
    wid: i16,
    d_wid: i16,
    ang: i16,
    d_ang: i16,
    step: i16,
    d_step: i16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RingParent {
    pos: [i16; 3],
    beast: bool,
    recs: usize,  // offset of the record table in EFT.DAT entry 88
    sched: usize, // offset of the current schedule entry
    frame: i16,
    life: i16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RingChild {
    pos: [i16; 3],
    r: RingRec,
    prims: [Prim; 6],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Flash {
    base: [i32; 3],
    delta: [i32; 3],
    t: i32,
    frame: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum Kind {
    Sprite(Sprite),
    UvAnim(UvAnim),
    Timeline(Timeline),
    Shards(Shards),
    RingParent(RingParent),
    RingChild(RingChild),
    Flash(Flash),
}

#[derive(Clone, Serialize, Deserialize)]
struct Task {
    dying: bool,
    kind: Kind,
}

/// Fight constants the effects read.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Arena {
    /// ring half size x / z; the inner zone is 200 less
    pub half: (i32, i32),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Fx {
    tasks: Vec<Option<Task>>,
    lists: [Vec<usize>; 16],
    seed: u32,
    buf: usize,
    draws: Vec<(usize, Draw)>,
    flash_task: Option<usize>,
    /// light #3 colour override of the flash (4.12 per channel), applied to the fighters' lighting
    pub flash: Option<[i32; 3]>,
    /// EFT.DAT entry 88 (hit-spark ring tables): static (`Fx::rebind`)
    #[serde(skip)]
    rings: Arc<[u8]>,
    pub arena: Arena,
    pub frozen: bool,
    /// BONEPOS of both fighters this frame (kind-3 sprites follow a joint)
    pub bones: [[[i32; 3]; 24]; 2],
    /// the team battle's fighters 2 and 3's (engine addition): written with `bones` every frame (`native_tasks::tick`) before
    /// anything reads them, so not state
    #[serde(skip)]
    bones_ext: [[[i32; 3]; 24]; 2],
    /// stage number (dust palettes)
    pub stage: usize,
    /// simulation version 6: a task that spawns while it runs keeps its slot, as a live pool entry does in the
    /// game. Up to version 5 the new task took the running task's (emptied) slot and the spawner was dropped:
    /// an emitter timeline ended after its first spawn, a kind-14 spark at its first trail, a ring burst after its
    /// first children.
    pub keep_slot: bool,
    /// the task `tick` is running
    #[serde(skip)]
    running: Option<usize>,
}

impl Fx {
    pub fn new(rings: Arc<[u8]>, arena: Arena) -> Self {
        Self {
            tasks: Vec::new(),
            lists: Default::default(),
            seed: 1,
            buf: 0,
            draws: Vec::new(),
            flash_task: None,
            flash: None,
            rings,
            arena,
            frozen: false,
            bones: [[[0; 3]; 24]; 2],
            bones_ext: [[[0; 3]; 24]; 2],
            stage: 0,
            keep_slot: false,
            running: None,
        }
    }

    /// BONEPOS of fighter `side` this frame (for the kind-3 sprites and the emitters that follow a joint).
    pub fn set_bones(&mut self, side: usize, b: [[i32; 3]; 24]) {
        match side {
            0 | 1 => self.bones[side] = b,
            _ => self.bones_ext[crate::fight::side::Fid::of(side).slot().index()] = b,
        }
    }

    /// Bone `bone` of fighter `side` (`set_bones`), the origin for a bone or fighter past the table.
    pub(crate) fn bone(&self, side: usize, bone: usize) -> [i32; 3] {
        let b = match side {
            0 | 1 => &self.bones[side],
            2 | 3 => &self.bones_ext[crate::fight::side::Fid::of(side).slot().index()],
            _ => return [0; 3],
        };
        b.get(bone).copied().unwrap_or([0; 3])
    }

    /// After deserialising: the static ring tables.
    pub fn rebind(&mut self, rings: &Arc<[u8]>) {
        self.rings = rings.clone();
    }

    /// A decoded state's task lists refer to its tasks.
    pub fn check(&self) -> Result<(), String> {
        let n = self.tasks.len();
        if self.lists.iter().flatten().all(|&i| i < n) && self.flash_task.is_none_or(|i| i < n) && self.buf <= 1 {
            Ok(())
        } else {
            Err("fx: task lists".into())
        }
    }

    fn spawn(&mut self, list: usize, kind: Kind) -> usize {
        let t = Task { dying: false, kind };
        let running = if self.keep_slot { self.running } else { None };
        let id = match self.tasks.iter().enumerate().position(|(i, t)| t.is_none() && Some(i) != running) {
            Some(i) => {
                self.tasks[i] = Some(t);
                i
            }
            None => {
                self.tasks.push(Some(t));
                self.tasks.len() - 1
            }
        };
        self.lists[list].insert(0, id);
        id
    }

    fn kill(&mut self, id: usize) {
        if let Some(Some(t)) = self.tasks.get_mut(id) {
            t.dying = true;
        }
    }

    fn sprite_mut(&mut self, id: usize) -> Option<&mut Sprite> {
        match self.tasks.get_mut(id) {
            Some(Some(Task { kind: Kind::Sprite(s), .. })) => Some(s),
            _ => None,
        }
    }

    /// Number of live tasks (debug).
    pub fn live(&self) -> usize {
        self.tasks.iter().filter(|t| t.is_some()).count()
    }

    // ------------------------------------------------------------------ spawners

    /// The libc rand of the effects.
    pub(crate) fn rand(&mut self) -> i32 {
        rand(&mut self.seed)
    }

    pub(crate) fn shards_spawn(&mut self, pos: [i16; 3], n: i16, yaw: i32, pitch: i32, power: i32, alt: bool) {
        let n_ = n as usize;
        self.spawn(
            LIST_PART,
            Kind::Shards(Shards {
                prims: vec![Prim::default(); n_],
                pos: vec![[0; 3]; n_],
                vel: vec![[0; 3]; n_],
                life: vec![0; n_],
                origin: pos,
                frame: 0,
                n,
                emitted: 0,
                yaw,
                pitch: 0x400 - pitch,
                power,
                alt,
            }),
        );
    }

    /// Table k of EFT.DAT entry 88.
    pub(crate) fn ring_spawn(&mut self, pos: [i16; 3], k: usize, beast: bool) {
        let u32at = |o: usize| self.rings.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0) as usize;
        if self.rings.len() < 0x20 {
            return;
        }
        let (recs, sched) = (u32at(4 * k), u32at(16 + 4 * k));
        self.spawn(LIST_PART, Kind::RingParent(RingParent { pos, beast, recs, sched, frame: 0, life: 0 }));
    }

    /// flash_spawn(channels, strength): 1 = R, 2 = B, 3 = R+G, else R+G+B.
    pub fn flash_spawn(&mut self, ch: i32, strength: i32) {
        if let Some(h) = self.flash_task.take() {
            self.kill(h);
        }
        // light #3 colour of the stage (the stage's light record, +0x24): zero on
        // every stage, so the flash colour is the delta alone
        let base = [0, 0, 0];
        let v = (strength << 8).min(0xFFF);
        let mut d = base;
        match ch {
            1 => d[0] = v,
            2 => d[2] = v,
            3 => {
                d[0] = v;
                d[1] = v;
            }
            _ => d = [v; 3],
        }
        let delta = [d[0] - base[0], d[1] - base[1], d[2] - base[2]];
        let id = self.spawn(LIST_FLASH, Kind::Flash(Flash { base, delta, t: 0xFFF, frame: 0 }));
        self.flash_task = Some(id);
    }

    /// hit_spark(pos, yaw, pitch, dmg, attacker): `beast` = attacker F64 & 0x20000; `blood` = the
    /// attacker's hit-effect type (S.101) is 2 or 12, BLOOD EFFECT is on and not `beast`: drops instead of the spark.
    #[allow(clippy::too_many_arguments)]
    pub fn hit_spark(&mut self, tables: &Tables, pos: [i16; 3], yaw: i32, pitch: i32, dmg: i32, beast: bool, blood: bool) {
        if blood {
            return self.hit_debris(tables, pos, yaw, pitch, dmg);
        }
        let dmg = dmg & 0xFFF;
        self.shards_spawn(pos, 6, yaw, pitch, dmg, false);
        self.sprite_spawn(tables, pos, 0, dmg, 0, 0);
        if dmg < 16 {
            self.ring_spawn(pos, 0, beast);
        } else {
            self.shards_spawn(pos, 8, yaw, pitch, dmg, false);
            self.ring_spawn(pos, if dmg < 23 { 1 } else { 2 }, beast);
        }
    }

    /// The blood branch of `hit_spark`: kind-0 flash + 4 / 8 blood drops.
    pub fn hit_debris(&mut self, tables: &Tables, pos: [i16; 3], yaw: i32, pitch: i32, dmg: i32) {
        let dmg = dmg & 0xFFF;
        self.sprite_spawn(tables, pos, 0, dmg, 0, 0);
        let tl = tables.table(sprites::HIT_SPRITES).origin() + if dmg < 15 { 0x32C } else { 0x340 };
        self.emitter_spawn(pos, None, 5, 0, tl, [yaw as i16, (0x400 - pitch) as i16, dmg.clamp(10, 23) as i16, 0]);
    }

    /// hit_spark_alt (guarded hit). The `beast` case's model_fx 0x13 is spawned by the caller (fight/duel/hits.rs).
    pub fn hit_spark_alt(&mut self, tables: &Tables, pos: [i16; 3], yaw: i32, pitch: i32, beast: bool) {
        self.sprite_spawn(tables, pos, 1, 0, 0, 0);
        if beast {
            self.shards_spawn(pos, 6, yaw, pitch, 0x18, true);
            self.shards_spawn(pos, 6, yaw + 0x800, 0x800 - pitch, 0x0C, true);
        }
    }

    // ------------------------------------------------------------------ frame

    /// One frame of all effect tasks (lists 6, 8, 10, 11, 14).
    pub fn tick(&mut self, tables: &Tables) {
        self.draws.clear();
        self.buf ^= 1;
        for list in [LIST_ANIM, LIST_FLASH, LIST_SPRITE, LIST_PART, LIST_RING] {
            let snapshot = self.lists[list].clone();
            for id in snapshot {
                let Some(task) = self.tasks.get_mut(id).and_then(|t| t.take()) else { continue };
                if task.dying {
                    self.lists[list].retain(|&i| i != id);
                    if let Kind::Sprite(s) = &task.kind {
                        self.sprite_dtor(tables, s);
                    }
                    continue;
                }
                let mut task = task;
                self.running = Some(id);
                let kill = self.update(tables, &mut task);
                self.running = None;
                task.dying |= kill;
                if self.tasks[id].is_none() {
                    self.tasks[id] = Some(task);
                }
            }
        }
    }

    fn draw(&mut self, d: Draw) {
        let n = self.draws.len();
        self.draws.push((n, d));
    }
}
