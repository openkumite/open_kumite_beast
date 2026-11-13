//! Stage geometry the fight uses: the ring, the outer area, the breakable walls.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Stage geometry the fight uses: the ring record (box or circle, ground drop
//! outside), the outer area and the breakable walls (the stage's wall table → the wall block); from
//! simulation v7 the extra stages ST9..STC take the engine's `EXTRA_BOUNDS` instead, and the engine's extra stage Q3DM17
//! (`extra_stage`, from v10) its own open box (engine additions); what stands inside an extra stage's ring (Q3DM17's
//! teleporter, v14) is `stage::extra::Extra::solids`, pushed against by `physics::solids`.
//! The ring floor itself is flat at y = 0.

use crate::fight::version::Gates;
use crate::formats::tables::{Table, Tables};
use serde::{Deserialize, Serialize};

pub mod anim;
pub mod br1;
pub mod extra;

/// 8-byte area record: kind 0 = box (half sizes), else circle (u32 at +4 = r²); `drop` = ground y outside.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct AreaRec {
    pub kind: u16,
    pub drop: i16,
    pub hx: i16,
    pub hz: i16,
    pub w4: i32,
}

impl AreaRec {
    /// Record `k` of the table `t`.
    pub fn read(t: &Table, k: usize) -> Self {
        let o = 8 * k;
        Self { kind: t.u16(o), drop: t.i16(o + 2), hx: t.i16(o + 4), hz: t.i16(o + 6), w4: t.u32(o + 4) as i32 }
    }

    /// 0 inside, `drop` outside.
    pub fn floor(&self, x: i32, z: i32) -> i32 {
        if self.kind == 0 {
            if z.abs() > self.hz as i32 || x.abs() > self.hx as i32 {
                return self.drop as i32;
            }
            return 0;
        }
        if (self.w4 as i64) - (x as i64 * x as i64) - (z as i64 * z as i64) < 0 {
            return self.drop as i32;
        }
        0
    }
}

/// The wall block. Sides: 0 = +X, 1 = −X, 2 = +Z, 3 = −Z.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Walls {
    pub model: [i16; 4],  // +0x34 wall model per side (−1 = none / broken)
    pub hp: [i16; 4],     // +0x3C 0 = no wall, −1 = indestructible
    pub pow: [i16; 4],    // +0x44 impact power
    pub st: [i16; 4],     // +0x4C 0 idle, −1 slam pending, 0x100+n shaking
    pub lock: u8,         // +0x21 no break allowed
    pub offset: [i32; 4], // shake offset of each wall this frame (along its normal)
    /// sides broken this frame (debris and sounds are the caller's)
    pub broke: Vec<usize>,
}

impl Walls {
    pub fn present(&self, side: usize) -> bool {
        self.hp[side] != 0
    }
}

/// One debris piece of a broken wall (0x34-byte records).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Piece {
    pub pos: [i32; 3],
    pub vel: [i32; 3],
    /// rotation x (spinning) and y (facing out of the wall), 4096 units
    pub rot: [i32; 2],
    pub spin: i32,
    pub alive: i32,
}

/// What the debris did this frame: sounds (stage bank ids 0x80+) and Tatami sinks (splash position).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DebrisEvents {
    pub sounds: Vec<i16>,
    pub sinks: Vec<[i32; 3]>,
}

/// What the imported game's debris did this frame (`Stage::update_debris_br1`): its stage sound calls (their argument), its
/// own sounds (the imported game's ids) and its splashes (a piece's point).
#[derive(Clone, Debug, Default)]
pub struct Br1DebrisEvents {
    pub stage: Vec<bool>,
    pub sounds: Vec<i16>,
    pub splash: Vec<[i32; 3]>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stage {
    pub id: usize,
    /// the hips' ring record (hips test, walls) — the same data as the edge parts' record
    pub ring: AreaRec,
    /// the outer area record (None: no outer bound)
    pub outer: Option<AreaRec>,
    pub walls: Walls,
    def_model: [i16; 4],
    def_hp: [i16; 4],
    /// debris pieces (W.14 = the stage's slot-3 models), W.248 sound countdown, the byte-table random counter
    pub debris: Vec<Piece>,
    pub debris_count: usize,
    debris_snd: i32,
    rand_i: usize,
    /// W+0x26 / +0x28 / +0x2A: the stage routine's counters (simulation version 6, `stage_anim`)
    pub anim: crate::fight::stage::anim::Counters,
    /// an imported stage's routine counters (simulation v17, `fight::stage::br1::Counters`)
    #[serde(default)]
    pub br1: crate::fight::stage::br1::Counters,
}

impl AreaRec {
    /// A box record (kind 0): half sizes, the word at +4 as the game holds it.
    const fn boxed(drop: i16, hx: i16, hz: i16) -> Self {
        Self { kind: 0, drop, hx, hz, w4: (hz as u16 as i32) << 16 | hx as u16 as i32 }
    }

    /// A circle record (kind 1): r² at +4, its halves read as hx / hz as the game's code does.
    const fn circle(drop: i16, r: i32) -> Self {
        let w4 = r * r;
        Self { kind: 1, drop, hx: w4 as i16, hz: (w4 >> 16) as i16, w4 }
    }
}

/// Engine addition (simulation v7): the extra stages' bounds fitted to their rooms, in place of the game's ±4096 box
/// without walls. Per stage 9..12: the ring box (the hips record's role), the
/// outer area (the outer record's) and the wall hp per side (+X, −X, +Z, −Z; −1 = invisible and indestructible as ST4's,
/// 0 = open: a ring-out); no wall models.
const EXTRA_BOUNDS: [(AreaRec, Option<AreaRec>, [i16; 4]); 4] = [
    // ST9: the hut, pillars' inner faces at x ±1344, z ±1728: a closed room as ST4
    (AreaRec::boxed(1, 1344, 1728), None, [-1; 4]),
    // STA: the same hut
    (AreaRec::boxed(1, 1344, 1728), None, [-1; 4]),
    // STB: the dome: the normal ring on the tiled platform, open (ring-out on the level floor, drop 1 as ST1),
    // the outer area inside the floor's edge (r 3584, the rim rising beyond it) as ST1 / ST7's circles
    (AreaRec::boxed(1, 1536, 1536), Some(AreaRec::circle(1, 3456)), [0; 4]),
    // STC: the corridor, floor z ±1024 between solid side walls (invisible walls there), open to the void at its
    // ends (x ±6912): a ring-out 768 before the floor ends, the outer area at the floor's end
    (AreaRec::boxed(1, 6144, 1024), Some(AreaRec::boxed(1, 6912, 1088)), [0, 0, -1, -1]),
];

/// Engine addition (the team battle): the stages offered with four fighters.
pub const TEAM_STAGES: [usize; 5] = [0, 2, 5, 7, 8];

/// Engine addition (the team battle T5, simulation v23): the big stages offered with four
/// fighters at their own size when their data is installed: Q3DM6 (`extra_stage`, the local Quake III files) and BIG
/// STAGE (`br1_stage`, the imported game's install).
pub const TEAM_BIG_STAGES: [usize; 2] = [14, 25];

/// Whether stage number `n` is a team battle's under the rules `g`.
pub fn team_stage(n: usize, g: Gates) -> bool {
    TEAM_STAGES.contains(&n) || (g.team_big_stages && TEAM_BIG_STAGES.contains(&n))
}

/// The team battle's ring and outer area scale (and the stage model's in x / z), in percent.
pub const TEAM_SCALE_PCT: u16 = 150;

impl AreaRec {
    /// The record scaled by `pct` percent (the team battle's rings, `Stage::team`): a box's half sizes, a circle's
    /// radius (from r², rounded down); the kind and the drop kept.
    fn scaled(&self, pct: i32) -> Self {
        if self.kind == 0 {
            Self::boxed(self.drop, (self.hx as i32 * pct / 100) as i16, (self.hz as i32 * pct / 100) as i16)
        } else {
            Self { kind: self.kind, ..Self::circle(self.drop, (self.w4 as i64).isqrt() as i32 * pct / 100) }
        }
    }
}

/// Engine addition: the ring of an extra stage (`extra_stage`): Q3DM17's an open box on the map's central platform
/// (`Extra::ring_half`), a ring-out over every side into the void (`Extra::ring_drop` below), no outer area (a
/// fighter flying out falls on), no walls; Q3DM6's a box at its courtyard's walls, invisible and indestructible on
/// all four sides (hp −1, drop 1, no outer area: a closed room as ST4 and ST9). Both follow the map's scale
/// (simulation v11: 7 game units per Quake unit, v10: 8).
const fn arena_bounds(e: &crate::fight::stage::extra::Extra, g: Gates) -> (AreaRec, Option<AreaRec>, [i16; 4]) {
    let [hx, hz] = e.ring_half(g);
    (AreaRec::boxed(e.ring_drop(g), hx, hz), None, if e.walled { [-1; 4] } else { [0; 4] })
}

impl Stage {
    /// The stage's records from the stage tables (`stage.ring`, `stage.ring_outer`, `stage.walls`); from `version::v::EXTRA_BOUNDS` the extra stages' `EXTRA_BOUNDS` (engine addition).
    /// `id` 13.. (`stage::extra::STAGES`): an extra stage, its `arena_bounds` over its stand-in's records (each first
    /// offered at its `Extra::sim`, so no older version runs it; Q3DM17's v10 ring is the larger one).
    pub fn new(tables: &Tables, id: usize, g: Gates) -> Self {
        let arena = crate::fight::stage::extra::by_stage(id);
        let chosen = id;
        let id = arena.map_or(id.min(12), |e| e.stand_in);
        let mut ring = AreaRec::read(tables.table("stage.ring"), id);
        let o = AreaRec::read(tables.table("stage.ring_outer"), id);
        let mut outer = (o.w4 != 0).then_some(o);
        let (walls, w) = (tables.table("stage.walls"), 16 * id);
        let mut def_model = [0, 1, 2, 3].map(|k| walls.i16(w + 2 * k));
        let mut def_hp = [0, 1, 2, 3].map(|k| walls.i16(w + 8 + 2 * k));
        if g.extra_bounds && id >= 9 {
            (ring, outer, def_hp) = EXTRA_BOUNDS[id - 9];
            def_model = [-1; 4];
        }
        if let Some(e) = arena {
            (ring, outer, def_hp) = arena_bounds(e, g);
            def_model = [-1; 4];
        }
        Self {
            id: if arena.is_some() { chosen } else { id },
            ring,
            outer,
            walls: Walls::default(),
            def_model,
            def_hp,
            debris: Vec::new(),
            debris_count: 0,
            debris_snd: 0,
            rand_i: 0,
            anim: Default::default(),
            br1: Default::default(),
        }
    }

    /// Engine addition (the team battle): stage `id` (one of `TEAM_STAGES`) with its ring
    /// and outer area at `TEAM_SCALE_PCT` (±1536 → ±2304); before simulation v25 its walls open (hp 0, no models: a
    /// ring-out at the edge), from v25 (T7, `version::v::TEAM_WALLS`) the game's walls at the scaled ring's edge with their models
    /// and hp, never broken (the team battle's stage step keeps them locked, `TeamFight::stage_step`). An extra stage
    /// (T5: Q3DM6) as a match has it, at its own size and with its walls.
    pub fn team(tables: &Tables, id: usize, g: Gates) -> Self {
        let mut s = Self::new(tables, id, g);
        if crate::fight::stage::extra::by_stage(id).is_some() {
            return s;
        }
        s.ring = s.ring.scaled(TEAM_SCALE_PCT as i32);
        s.outer = s.outer.map(|o| o.scaled(TEAM_SCALE_PCT as i32));
        if !g.team_walls {
            s.def_model = [-1; 4];
            s.def_hp = [0; 4];
        }
        s
    }

    /// Engine addition (the import S8, simulation v16): the imported game's stage `id` (`fight::stage::br1`) with its
    /// records `b` from the imported game's static tables: the hips' ring record as the ring (the imported game's walls stand at it),
    /// no outer area, the walls' models and hp of its walls' round start (the same table and round-start rule as the main game's
    /// wall reset).
    pub fn br1(id: usize, b: &crate::fight::stage::br1::Bounds) -> Self {
        let (w0, w1) = (b.hips.w0, b.hips.w1);
        let ring = AreaRec { kind: w0 as u16, drop: (w0 >> 16) as i16, hx: w1 as i16, hz: (w1 >> 16) as i16, w4: w1 as i32 };
        Self { id, ring, outer: None, walls: Walls::default(), def_model: b.model, def_hp: b.hp, ..Default::default() }
    }

    /// The wall reset at every round start: walls restored, breaking locked unless this can be the last round.
    pub fn round_init(&mut self, round: i32, need: i32) {
        let w = &mut self.walls;
        w.lock = if round + 1 < need * 2 - 1 { 1 } else { 0 };
        for i in 0..4 {
            w.model[i] = self.def_model[i];
            w.hp[i] = self.def_hp[i];
            if w.hp[i] == 0 {
                w.model[i] = -1;
            }
            w.pow[i] = 0;
            w.st[i] = 0;
            w.offset[i] = 0;
        }
        w.broke.clear();
        self.debris.clear();
        self.debris_snd = 0;
        // an imported stage's
        self.br1.round_init(round);
    }

    /// The next byte of the game's random byte table.
    fn rnd(&mut self, table: &[u8]) -> i32 {
        let v = table.get(self.rand_i % table.len().max(1)).copied().unwrap_or(0) as i32;
        self.rand_i += 1;
        v
    }

    /// spawn_debris: the pieces of wall `side` spread along its plane, thrown outwards.
    pub fn spawn_debris(&mut self, side: usize, table: &[u8]) {
        let n = self.debris_count;
        if n == 0 {
            return;
        }
        self.debris_snd = 9;
        let (hx, hz) = (self.ring.hx as i32, self.ring.hz as i32);
        let half = if side < 2 { hz } else { hx };
        self.debris.clear();
        for k in 1..=n as i32 {
            let along = half - k * (2 * half / (n as i32 + 1));
            let out = ((self.rnd(table) + 0xC0) * 51) >> 8;
            let vy = (self.rnd(table) - 0xA0) >> 2;
            let spin = -(self.rnd(table) * 128) / 256;
            let (pos, vel, ry) = match side {
                0 => ([hx, -0x100, along], [out, vy, along / 64], 0x400),
                1 => ([-hx, -0x100, along], [-out, vy, along / 64], -0x400),
                2 => ([along, -0x100, hz], [along / 64, vy, out], 0x800),
                _ => ([along, -0x100, -hz], [along / 64, vy, -out], 0),
            };
            self.debris.push(Piece { pos, vel, rot: [0, ry], spin, alive: 16 });
        }
    }

    /// Simulation v18, an imported stage `b`: the imported game's own debris of a wall (its pieces
    /// G.0C, records of 0x34): the sound countdown at 9; per piece (along the wall from its half size
    /// down by 2 · half / (n + 1), in the edge record's half sizes) a spin −(r · 128) / 256, out of the wall
    /// (r + 0xC0) · 51 / 256 (the divisions toward 0), upwards r − 0x80, along it the place / 64, at height −0x100,
    /// alive 0x10; walls 0 / 1 (±x, turned ±0x400) draw spin, out, up; 2 / 3 (±z, turned 0 / 0x800) spin, up, out. Its
    /// random bytes come from the byte table on the stage's own counter (an engine choice; the game's is the
    /// fighters' stream).
    pub fn spawn_debris_br1(&mut self, side: usize, b: &crate::fight::stage::br1::Bounds) {
        self.debris_snd = 9;
        // (G.0C: the stage loader keeps at most 8)
        let n = self.debris_count.min(8) as i32;
        let (hx, hz) = (b.edge.w1 as i16 as i32, (b.edge.w1 >> 16) as i16 as i32);
        let half = if side < 2 { hz } else { hx };
        let step = 2 * half / (n + 1);
        self.debris.clear();
        let mut along = half;
        for _ in 0..n {
            let spin = -(self.br1.next_rand(&b.rand) << 7) / 256;
            along -= step;
            let (pos, vel, ry) = if side < 2 {
                let out = (self.br1.next_rand(&b.rand) + 0xC0) * 51 / 256;
                let up = self.br1.next_rand(&b.rand) - 0x80;
                let s = if side == 0 { 1 } else { -1 };
                ([s * hx, -0x100, along], [s * out, up, along / 64], s * 0x400)
            } else {
                let up = self.br1.next_rand(&b.rand) - 0x80;
                let out = (self.br1.next_rand(&b.rand) + 0xC0) * 51 / 256;
                let s = if side == 2 { 1 } else { -1 };
                ([along, -0x100, s * hz], [along / 64, up, s * out], if side == 2 { 0 } else { 0x800 })
            };
            self.debris.push(Piece { pos, vel, rot: [0, ry], spin, alive: 0x10 });
        }
    }

    /// Simulation v18, an imported stage `b`: the debris part of its stage step. The countdown calls the stage sound (`stage`:
    /// its argument) at 8 and 5 (1) and 1 (0, then 1); per piece: gravity 2, the move; on MIT's and VAK's stages (3,
    /// 5) a piece reaching the edge record's floor splashes (`splash`: a still particle of type 0x10, CLUT 0xE, mode 4) with
    /// sound 0x82 and is gone; elsewhere one reaching it while not rising faster than 1 bounces (sound 0x82 from a
    /// fall of 13; its speeds and spin halved, the fall turned up) onto the floor, and settles by one (alive − 1) once
    /// slower than 2 sideways and not rising; in the air it spins.
    pub fn update_debris_br1(&mut self, b: &crate::fight::stage::br1::Bounds) -> Br1DebrisEvents {
        let mut ev = Br1DebrisEvents::default();
        match self.debris_snd {
            1 => ev.stage.extend([false, true]),
            5 | 8 => ev.stage.push(true),
            _ => {}
        }
        if self.debris_snd != 0 {
            self.debris_snd -= 1;
        }
        let floor = (b.edge.w0 >> 16) as i16 as i32;
        let water = matches!(b.g94, 3 | 5);
        for p in self.debris.iter_mut().filter(|p| p.alive != 0) {
            p.vel[1] = (p.vel[1] + 2) as i16 as i32;
            for k in 0..3 {
                p.pos[k] += p.vel[k];
            }
            let below = p.pos[1] - floor >= 0;
            if water && below {
                ev.splash.push(p.pos);
                ev.sounds.push(0x82);
                p.alive = 0;
            } else if !below || p.vel[1] < -1 {
                p.rot[0] = (p.rot[0] + p.spin) as i16 as i32;
            } else {
                if p.vel[1] >= 13 {
                    ev.sounds.push(0x82);
                }
                p.vel = [p.vel[0] / 2, -(p.vel[1] / 2), p.vel[2] / 2];
                p.spin /= 2;
                if p.vel[1] >= -1 && p.vel[0].abs() < 2 && p.vel[2].abs() < 2 {
                    p.alive -= 1;
                }
                p.pos[1] = floor;
            }
        }
        ev
    }

    /// Debris update: sounds, gravity, bounce on the outer floor (Tatami: sink), outer bound.
    pub fn update_debris(&mut self, stage_no: usize) -> DebrisEvents {
        let mut ev = DebrisEvents::default();
        if self.debris_snd > 0 {
            match self.debris_snd {
                8 | 5 => ev.sounds.push(0x82),
                1 => ev.sounds.extend([0x80, 0x82]),
                _ => {}
            }
            self.debris_snd -= 1;
        }
        let ground = self.outer.map(|o| o.drop as i32).unwrap_or(self.ring.drop as i32);
        let outer = self.outer;
        for p in self.debris.iter_mut() {
            if p.alive == 0 {
                continue;
            }
            p.vel[1] += 2;
            for k in 0..3 {
                p.pos[k] += p.vel[k];
            }
            p.rot[0] += p.spin;
            if p.pos[1] >= ground {
                if stage_no == 2 {
                    // Tatami: the piece sinks into the water (a splash + SFX 0x83)
                    ev.sinks.push(p.pos);
                    ev.sounds.push(0x83);
                    p.alive = 0;
                    continue;
                }
                p.pos[1] = ground;
                if p.vel[1] >= 13 {
                    ev.sounds.push(0x83);
                }
                p.vel[1] = -p.vel[1] / 2;
                p.vel[0] /= 2;
                p.vel[2] /= 2;
                p.spin /= 2;
                if p.vel[0].abs() < 2 && p.vel[2].abs() < 2 && p.alive > 1 {
                    p.alive -= 1;
                }
            }
            if let Some(o) = outer
                && o.kind == 0
            {
                for (k, h) in [(0usize, o.hx as i32), (2, o.hz as i32)] {
                    if p.pos[k].abs() > h && p.pos[k].signum() == p.vel[k].signum() {
                        p.vel[k] = -p.vel[k];
                        ev.sounds.push(0x83);
                    }
                }
            }
        }
        ev
    }

    /// Per frame: wall hp from pushes and slams, the break on the match-deciding KO, shake.
    pub fn update(&mut self, paused: bool) {
        let w = &mut self.walls;
        w.broke.clear();
        for i in 0..4 {
            w.offset[i] = 0;
            if w.hp[i] <= 0 {
                continue;
            }
            if w.st[i] == 0 {
                w.hp[i] -= w.pow[i] / 2;
                if w.hp[i] <= 0 {
                    w.hp[i] = 1; // a push can never break a wall
                }
                w.st[i] = 0x100;
            } else if w.st[i] < 0 {
                w.hp[i] -= w.pow[i];
                if w.hp[i] <= 0 {
                    if w.pow[i] == 10000 && w.lock == 0 {
                        // ===== wall break =====
                        w.model[i] = -1;
                        w.hp[i] = 0;
                        w.lock = 1;
                        w.broke.push(i);
                        continue;
                    }
                    w.hp[i] = 1;
                }
                w.st[i] = 0x100;
            }
            let off = if (w.st[i] & 3) < 2 { (w.pow[i] as i32).min(0x80) } else { 0 };
            if !paused {
                w.st[i] += 1;
                w.pow[i] = (w.pow[i] as i32 * 7 / 8) as i16;
                if w.pow[i] == 0 {
                    w.st[i] = 0;
                }
            }
            w.offset[i] = if i % 2 == 0 { off } else { -off };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring, outer area and walls per stage from the stage tables (skipped without the export): the extra stages
    /// ST9..STC have a ±4096 box with the ground 256 below outside it, no outer bound and no walls (the game's
    /// values, recorded with M.98 forced); ST4 has invisible indestructible
    /// walls round a ±1536 ring, ST0 breakable ones. From simulation v7 the extra stages take `EXTRA_BOUNDS`.
    #[test]
    fn extra_stage_bounds_from_the_tables() {
        let Some(tables) = crate::formats::tables::export_tables() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        for (id, half, drop, outer, hp) in [(0, 1536, 256, true, 180), (4, 1536, 1, false, -1), (9, 4096, 256, false, 0), (12, 4096, 256, false, 0)] {
            let mut s = Stage::new(&tables, id, Gates::of(6));
            s.round_init(0, 2);
            assert_eq!((s.ring.kind, s.ring.hx, s.ring.hz, s.ring.drop, s.outer.is_some()), (0, half, half, drop, outer), "stage {id}");
            assert_eq!(s.walls.hp, [hp; 4], "stage {id}");
            assert_eq!(s.walls.model, if hp > 0 { [0, 1, 3, 2] } else { [-1; 4] }, "stage {id}");
            assert_eq!((s.ring.floor(half as i32, -(half as i32)), s.ring.floor(half as i32 + 1, 0)), (0, drop as i32), "stage {id}");
        }
        for id in 10..=11 {
            let s = Stage::new(&tables, id, Gates::of(6));
            assert_eq!((s.ring.hx, s.ring.hz, s.ring.drop, s.outer.is_some(), s.def_hp), (4096, 4096, 256, false, [0; 4]), "stage {id}");
        }
        // simulation v7: the engine-made bounds (`EXTRA_BOUNDS`) on 9..12 only; records as the game would hold them
        for id in 0..=12 {
            let (v6, mut v7) = (Stage::new(&tables, id, Gates::of(6)), Stage::new(&tables, id, Gates::of(7)));
            v7.round_init(0, 2);
            if id < 9 {
                assert_eq!((v7.ring.w4, v7.outer.map(|o| o.w4), v7.def_hp), (v6.ring.w4, v6.outer.map(|o| o.w4), v6.def_hp), "stage {id}");
                continue;
            }
            assert_eq!(v7.walls.model, [-1; 4], "stage {id}");
            let (r, o) = (v7.ring, v7.outer);
            assert_eq!((r.kind, r.w4), (0, (r.hz as i32) << 16 | r.hx as i32), "stage {id}");
            assert_eq!(o.map(|o| (o.w4 as u32 & 0xFFFF, o.w4 as u32 >> 16)), o.map(|o| (o.hx as u16 as u32, o.hz as u16 as u32)), "stage {id}");
        }
        let b = |id| {
            let s = Stage::new(&tables, id, Gates::of(7));
            (s.ring.hx, s.ring.hz, s.ring.drop, s.outer.map(|o| (o.kind, o.w4)), s.def_hp)
        };
        assert_eq!(b(9), (1344, 1728, 1, None, [-1; 4]));
        assert_eq!(b(10), b(9));
        assert_eq!(b(11), (1536, 1536, 1, Some((1, 3456 * 3456)), [0; 4]));
        assert_eq!(b(12), (6144, 1024, 1, Some((0, 1088 << 16 | 6912)), [0, 0, -1, -1]));
        // the extra stage Q3DM17 (13): the open box on its platform (it is offered from v10 on; v11 scales the map
        // to 7 game units per Quake unit), a ring-out into the drop past each edge
        use crate::fight::stage::extra::{Q3DM6, Q3DM17};
        let (q17, q6) = (Q3DM17, Q3DM6);
        assert_eq!(
            (q17.ring_half(Gates::of(10)), q17.ring_drop(Gates::of(10)), q17.ring_half(Gates::of(11)), q17.ring_drop(Gates::of(11))),
            ([1280; 2], 10240, [1120; 2], 8960)
        );
        assert_eq!((q17.ring_half(Gates::of(12)), q17.ring_drop(Gates::of(12))), (q17.ring_half(Gates::of(11)), q17.ring_drop(Gates::of(11))));
        for v in [6, 7, 10, 11, 12] {
            let (h, d) = (q17.ring_half(Gates::of(v)), q17.ring_drop(Gates::of(v)));
            let mut s = Stage::new(&tables, q17.stage as usize, Gates::of(v));
            s.round_init(0, 2);
            assert_eq!((s.id, s.ring.kind, s.ring.hx, s.ring.hz, s.ring.drop, s.outer.is_none()), (13, 0, h[0], h[1], d, true), "v{v}");
            assert_eq!((s.ring.w4, s.walls.hp, s.walls.model), ((h[1] as i32) << 16 | h[0] as i32, [0; 4], [-1; 4]), "v{v}");
            let (hx, hz) = (h[0] as i32, h[1] as i32);
            assert_eq!([s.ring.floor(hx, hz), s.ring.floor(-hx, -hz), s.ring.floor(0, 0)], [0; 3]);
            assert_eq!([s.ring.floor(hx + 1, 0), s.ring.floor(-hx - 1, 0), s.ring.floor(0, hz + 1), s.ring.floor(0, -hz - 1)], [d as i32; 4]);
        }
        // Q3DM6 (14, from v12): the courtyard's walls, 320 × 240 Quake units from its centre, invisible and
        // indestructible on all four sides, no outer area (a closed room as ST4's)
        let mut s = Stage::new(&tables, q6.stage as usize, Gates::of(12));
        s.round_init(0, 2);
        assert_eq!((s.id, s.ring.kind, s.ring.hx, s.ring.hz, s.ring.drop, s.outer.is_none()), (14, 0, 2240, 1680, 1, true));
        assert_eq!((s.ring.w4, s.walls.hp, s.walls.model), (1680 << 16 | 2240, [-1; 4], [-1; 4]));
        assert!((0..4).all(|k| s.walls.present(k)));
    }
}
