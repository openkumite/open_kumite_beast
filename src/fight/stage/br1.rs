//! The imported game's stages in a main-game match (engine addition, the import S8).
//!
//! State: `Counters` (serialised in `stage::Stage::br1`, simulation v17); the rest constants (a stage's ring and walls go
//! into `stage::Stage`'s serialised fields)
//!
//! The imported game's eleven stages (`G.94` 0..10: the nine characters' `XX/XX00.DAT` in the order EVE GRE ZIN MIT GAD VAK FOX RON URI,
//! then `CMN/SMA00.DAT` and `CMN/BIG00.DAT`) as stage numbers 15..25 after the main game's 13 and the extra stages, from
//! simulation v16 and only with the imported game's install (`SessionStart::br1`). Their fight runs in the main game's
//! rules as the main-game stage `STAND_IN` for everything the main game's static tables decide (M.98: the stage routine, the
//! stage effects, the camera's stage byte), with the imported game's own ring and walls read from the imported game's static tables
//! (`Bounds`, `fight_data::Loaded::br1_stage`): the hips' box as the main game's ring (the main game has one table
//! for the hips and the edge parts; the imported game's walls stand at its hips' box), its drop outside, the four walls'
//! models and hp (the same table format and values as the main game's: models 0 1 3 2, hp 180), no outer
//! area. An imported fighter's core reads both of the imported game's records and the stage byte `g94` as in the
//! imported game (`fight::sim::br1`).

use crate::fight::br1::physics::{RING_EDGE, RING_HIPS, Ring};
use crate::fight::version::Gates;
use crate::formats::tables::Tables;
use serde::{Deserialize, Serialize};

/// The stages whose props the engine draws (`present::br1_props`: ZIN's slider, VAK's wanderers; the others' are the imported game's intro
/// camera's).
pub const PROP_STAGES: [u8; 2] = [2, 5];

/// The main-game stage whose static tables the fight reads for an imported stage (STB, as the extra stages: its
/// stage routine and effects do nothing).
pub const STAND_IN: usize = 11;

/// One imported stage.
pub struct Br1Stage {
    /// the stage number (`MatchSpec::stage`), after the extra stages
    pub stage: u8,
    /// The imported game's stage byte G.94
    pub g94: u8,
    /// its code (`fight_data::stage_code`, a direct fight's `--stage`): the stage file's stem
    pub code: &'static str,
    /// the folder of its files in the imported game's install (`engine/<dir>/<code>.DAT`, `audio/sfx/<dir>/`)
    pub dir: &'static str,
    /// its stage bank (`audio/sfx/<dir>/<bank>.SND`, the imported game's sound slot 2)
    pub bank: &'static str,
    /// the stage's own fighter, its entry in the imported game's text list `names` (`name`); None for the two CMN stages
    pub fighter: Option<usize>,
}

impl Br1Stage {
    /// The stage's name (`Loaded::stage_name`): its own fighter's, else its code's stem (SMA, BIG).
    pub fn name(&self) -> &'static str {
        self.fighter.map_or(&self.code[..3], |k| crate::assets::text::br1_item("names", k))
    }
}

const fn st(g94: u8, code: &'static str, dir: &'static str, bank: &'static str, fighter: Option<usize>) -> Br1Stage {
    Br1Stage { stage: 15 + g94, g94, code, dir, bank, fighter }
}

/// The stages by G.94, identified in recorded arcade runs (G.94 against the
/// scene on screen; 9 and 10 by their ring sizes, the small and the big one).
pub const STAGES: [Br1Stage; 11] = [
    st(0, "EVE00", "EVE", "EVE10", Some(4)),
    st(1, "GRE00", "GRE", "GRE10", Some(2)),
    st(2, "ZIN00", "ZIN", "ZIN10", Some(0)),
    st(3, "MIT00", "MIT", "MIT10", Some(3)),
    st(4, "GAD00", "GAD", "GAD10", Some(1)),
    st(5, "VAK00", "VAK", "VAK10", Some(7)),
    st(6, "FOX00", "FOX", "FOX10", Some(5)),
    st(7, "RON00", "RON", "RON10", Some(6)),
    st(8, "URI00", "URI", "URI10", Some(8)),
    st(9, "SMA00", "CMN", "SMA10", None),
    st(10, "BIG00", "CMN", "BIG10", None),
];

/// The imported stage of stage number `n`.
pub fn by_stage(n: usize) -> Option<&'static Br1Stage> {
    STAGES.iter().find(|s| s.stage as usize == n)
}

/// The imported stage of code `code` that a simulation version of rules `g` offers (from `version::v::BR1_STAGES`).
pub fn by_code(code: &str, g: Gates) -> Option<&'static Br1Stage> {
    g.br1_stages.then(|| STAGES.iter().find(|s| s.code == code)).flatten()
}

/// The shadows' floor records (8 bytes per stage, as the rings').
pub const SHADOW_FLOOR: &str = "stage.shadow_floor";

/// An imported stage's records from its static tables: the hips' and the edge parts' ring records and the walls' round
/// start (models and hp per side +X, −X, +Z, −Z; a wall of hp 0 has no model).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub g94: u8,
    pub hips: Ring,
    pub edge: Ring,
    pub model: [i16; 4],
    pub hp: [i16; 4],
    /// the table's first word (+0x58 of the walls' state): the pieces a broken wall falls into
    pub pieces: i16,
    /// the shadows' floor record (read by the ground test in the shadow draw; presentation)
    pub shadow: Ring,
    /// the byte table: the imported game's debris draws from it with the stage's own
    /// counter (`Counters::rand`, an engine choice: not the fighters' stream)
    pub rand: [u8; 256],
}

impl Bounds {
    /// Stage `s`'s records in `tables` (the walls' table `stage.walls`: 18 bytes per stage, +2 the models, +10 the hp).
    pub fn of(tables: &Tables, s: &Br1Stage) -> Self {
        let g = s.g94 as i8;
        let w = tables.table("stage.walls").origin() + 18 * s.g94 as u32;
        let random = tables.table("math.random_bytes");
        Self {
            g94: s.g94,
            hips: Ring::of(tables, RING_HIPS, g),
            edge: Ring::of(tables, RING_EDGE, g),
            model: [0, 1, 2, 3].map(|k| tables.abs_i16(w + 2 + 2 * k)),
            hp: [0, 1, 2, 3].map(|k| tables.abs_i16(w + 10 + 2 * k)),
            pieces: tables.abs_i16(w),
            shadow: Ring::of(tables, SHADOW_FLOOR, g),
            rand: std::array::from_fn(|k| random.u8(k)),
        }
    }
}

/// The counters of the imported game's stage routines that the fight reads (simulation v17), in the stage draw's block
/// G: G.4A, Uriko's stage's (G.94 8) frames since a fighter took the beast form (0 before, then 2..600).
/// The imported game's KO replay copies the block back with its snapshot, as the main game's does `Stage`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub g4a: i16,
    /// simulation v18: G.46 (the routine's first frame done), Uriko's stage's wall counters G.50 / 52 / 54 / 56 by
    /// wall (−1 standing, 0 shattered, 1..16 rising), the stage's counter into the byte table for the debris (`rand`,
    /// an engine choice), and the imported game's sound ids of this frame that no imported fighter's queue took (a match
    /// of two main-game fighters: the session plays them)
    #[serde(default)]
    pub init: bool,
    #[serde(default)]
    pub g50: [i16; 4],
    #[serde(default)]
    pub rand: u8,
    #[serde(default)]
    pub sounds: Vec<i16>,
}

/// What Uriko's stage routine did this frame (simulation v18): the walls it shattered (each into the imported game's debris) and its stage
/// sounds (the stage sound with argument 1).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UrikoEvents {
    pub shatter: Vec<usize>,
    pub thuds: usize,
}

/// Uriko's stage's walls: the G.4A each shatters at (by wall) and the counter that raises it
/// again (G.50 + 2 · this) from its G.4A.
const SHATTER: [i16; 4] = [0x0F, 0x2D, 0x1E, 0x3C];
const RISE: [(i16, usize); 4] = [(0x4B, 0), (0x55, 2), (0x5F, 1), (0x69, 3)];

impl Counters {
    /// The walls' round start: round 0 (G.95 0) clears G.46, so the stage routine's first frame clears its
    /// counters (G.48..G.4E 0).
    pub fn round_init(&mut self, round: i32) {
        if round == 0 {
            *self = Self::default();
        }
    }

    /// Stage `g94`'s routine as the stage draw calls it (once per drawn fight frame, after
    /// the fighters' scripts), its G.4A part: on Uriko's stage, the fighter it watches (`watched`: P2 if P2 is
    /// Uriko, else P1) in the beast form (F.05 bit 0, `form`) starts G.4A at 1, then the routine counts
    /// it on to 600. (G.0 8, the arcade's ending, clears it: no fight of the engine runs there.) Simulation v17
    /// only; from v18 `step_walls` runs the whole routine.
    pub fn step(&mut self, g94: u8, form: u8) {
        if g94 != 8 {
            return;
        }
        if form & 1 != 0 && self.g4a == 0 {
            self.g4a = 1;
        }
        if self.g4a != 0 && self.g4a < 600 {
            self.g4a += 1;
        }
    }

    /// Simulation v18: the whole of the routine's fight part on Uriko's stage (`step`'s G.4A with its walls): its first
    /// frame (G.46) stands the walls (G.50.. −1); G.4A starts with the watched fighter's beast form; at G.4A 0x4B /
    /// 0x55 / 0x5F / 0x69 walls 0 / 2 / 1 / 3 start rising (their counter 1); a standing wall (counter non-zero, its
    /// model there: `standing`) on the far side of the camera (by `yaw`, its 12 bits) at its
    /// G.4A of `SHATTER` shatters (into the imported game's debris; counter 0); a rising one counts on to 16, where it calls
    /// the stage sound (1) when G.42 bit 2 is set (the engine's matches set it: the imported game's VS
    /// and arcade both have G.42 = 7); then G.4A on to 600. Its other parts (its VRAM moves from G.4A 0x4D,
    /// the walls' drawing) are the presentation's.
    pub fn step_walls(&mut self, g94: u8, form: u8, yaw: i16, standing: [bool; 4]) -> UrikoEvents {
        let mut ev = UrikoEvents::default();
        if g94 != 8 {
            return ev;
        }
        if !self.init {
            self.init = true;
            self.g4a = 0;
            self.g50 = [-1; 4];
        }
        if form & 1 != 0 && self.g4a == 0 {
            self.g4a = 1;
        }
        for (at, k) in RISE {
            if self.g4a == at {
                self.g50[k] = 1;
            }
        }
        let y = ((yaw as i32) << 20) >> 20;
        let far = |v: i32| (((v << 20) >> 20).abs()) >= 0x31C;
        let seen = [far(y + 0x400), far(y - 0x400), y.abs() < 0x4E4, y.abs() >= 0x31C];
        for k in 0..4 {
            if self.g50[k] != 0 && standing[k] && seen[k] && self.g4a == SHATTER[k] {
                ev.shatter.push(k);
                self.g50[k] = 0;
            }
            if (1..16).contains(&self.g50[k]) {
                self.g50[k] += 1;
                if self.g50[k] == 16 {
                    ev.thuds += 1;
                }
            }
        }
        if self.g4a != 0 && self.g4a < 600 {
            self.g4a += 1;
        }
        ev
    }

    /// The byte-table draw on the stage's own counter (`rand`; the game's is the fighters' stream).
    pub fn next_rand(&mut self, table: &[u8; 256]) -> i32 {
        let v = table[self.rand as usize] as i32;
        self.rand = self.rand.wrapping_add(1);
        v
    }

    /// The stage sound's choice (op 0x26 sub 7, `n` its argument 0 / 1): on Uriko's stage while G.4A < 0x4B, the
    /// imported Uriko's 0xC7 + n (0x107 + n for player 2: F.02), the first of the fighter blocks (P1, P2) whose
    /// F.03 is 8 (`uriko`: its F.02); else, or without one, the stage bank's 0x80 + n.
    pub fn stage_sound(&self, g94: u8, n: bool, uriko: Option<u8>) -> i16 {
        let n = n as i16;
        match uriko {
            Some(player) if g94 == 8 && self.g4a < 0x4B => 0xC7 + n + if player != 0 { 0x40 } else { 0 },
            _ => 0x80 + n,
        }
    }
}

/// The stage theme (with the stage setting): the imported game's XA track by G.94; the file
/// `audio/music/BGM0<t / 8>_ch<t % 8>.xa`.
pub fn music_track(g94: u8) -> u8 {
    const T: [u8; 11] = [22, 21, 20, 24, 25, 23, 26, 27, 28, 21, 20];
    T[g94.min(10) as usize]
}

/// The imported game's install's file of XA track `t` (`music_track`).
pub fn music_file(t: u8) -> String {
    format!("audio/music/BGM{:02}_ch{}.xa", t >> 3, t & 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        for (k, s) in STAGES.iter().enumerate() {
            assert_eq!((s.g94 as usize, s.stage as usize), (k, 15 + k));
            assert!(by_stage(s.stage as usize).is_some_and(|x| x.code == s.code));
            assert!(
                by_code(s.code, Gates::of(crate::fight::version::v::BR1_STAGES)).is_some()
                    && by_code(s.code, Gates::of(crate::fight::version::v::BR1_STAGES - 1)).is_none()
            );
        }
        // after the extra stages
        assert!(crate::fight::stage::extra::STAGES.iter().all(|e| (e.stage as usize) < 15));
        assert_eq!((music_file(music_track(0)), music_file(music_track(8))), ("audio/music/BGM02_ch6.xa".into(), "audio/music/BGM03_ch4.xa".into()));
    }

    /// G.4A and the stage sound as a recorded run shows them (run s8q:
    /// MITSUKO vs Uriko on stage 8, Uriko's gauge poked negative): G.4A 0 until Uriko's F.05 bit 0, then 2 on that
    /// frame, +1 per frame, held at 600 from 598 frames later; through later rounds too; the KO's sound 0x80.
    #[test]
    fn uriko_stage_counter() {
        let mut c = Counters::default();
        for _ in 0..100 {
            c.step(8, 0x10);
        }
        assert_eq!(c.g4a, 0);
        assert_eq!(c.stage_sound(8, false, Some(1)), 0x107);
        assert_eq!((c.stage_sound(8, true, Some(0)), c.stage_sound(8, true, None)), (0xC8, 0x81));
        c.step(8, 0x11);
        assert_eq!(c.g4a, 2);
        // the transformation's flicker (form 0x11 / 0x10 every 2 frames) does not matter
        for k in 0..72 {
            c.step(8, 0x10 | (k as u8 >> 1 & 1));
        }
        assert_eq!(c.g4a, 74);
        assert_eq!(c.stage_sound(8, false, Some(1)), 0x107);
        c.step(8, 0x10);
        assert_eq!(c.stage_sound(8, false, Some(1)), 0x80);
        for _ in 0..1000 {
            c.step(8, 0);
        }
        assert_eq!(c.g4a, 600);
        c.round_init(1);
        assert_eq!(c.g4a, 600);
        c.round_init(0);
        assert_eq!(c.g4a, 0);
        // other stages: never counted, the stage bank
        c.step(3, 1);
        assert_eq!((c.g4a, c.stage_sound(3, false, Some(0))), (0, 0x80));
    }

    /// The records of the imported game's static tables (data-gated): every stage a box with the four walls of the main game's
    /// stages, the hips' box 64 inside the edge parts' but for stage 7's.
    #[test]
    fn bounds() {
        let Some(tables) = crate::formats::tables::br1_export_tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        for s in &STAGES {
            let b = Bounds::of(&tables, s);
            assert_eq!((b.model, b.hp), ([0, 1, 3, 2], [180; 4]), "{}", s.code);
            assert_eq!(b.hips.w0 & 0xFFFF, 0, "{}", s.code);
        }
        let b = Bounds::of(&tables, &STAGES[0]);
        assert_eq!((b.hips.half(), b.edge.half(), b.hips.w0 >> 16), ((1472, 1472), (1536, 1536), 256));
    }

    /// Uriko's stage's walls (simulation v18): the first frame stands them; at yaw 0 walls 0, 2, 1 pass the far-side test
    /// and shatter at G.4A 15 / 30 / 45 (wall 3 neither passes nor has its model); every wall rises again and thuds
    /// at its counter 16, at G.4A 89 / 99 / 109 / 119; another stage runs none of it.
    #[test]
    fn uriko_walls() {
        let mut c = Counters::default();
        let mut shatter = Vec::new();
        let mut thuds = Vec::new();
        for f in 0..200 {
            let form = if f >= 10 { 0x11 } else { 0x10 };
            let ev = c.step_walls(8, form, 0, [true, true, true, false]);
            for k in ev.shatter {
                shatter.push((c.g4a - 1, k));
            }
            if ev.thuds > 0 {
                thuds.push((c.g4a - 1, ev.thuds));
            }
        }
        // yaw 0: walls 0 / 1 (|0x400| ≥ 0x31C) and 2 (|0| < 0x4E4) are seen, 3 (|0| ≥ 0x31C) is not; 3 has no model
        assert_eq!(shatter, [(0x0F, 0), (0x1E, 2), (0x2D, 1)]);
        // (as the recorded thuds at G.4A 89 / 99 / 109 / 119, run s8q)
        assert_eq!(thuds, [(89, 1), (99, 1), (109, 1), (119, 1)]);
        assert_eq!(c.g50, [16; 4]);
        // another stage: nothing
        let mut o = Counters::default();
        assert_eq!(o.step_walls(3, 0x11, 0, [true; 4]), UrikoEvents::default());
        assert!(!o.init);
    }

    /// The far-side test by yaw.
    #[test]
    fn uriko_walls_seen_by_yaw() {
        let shattered = |yaw: i16| {
            let mut c = Counters::default();
            let mut out = Vec::new();
            for _ in 0..70 {
                out.extend(c.step_walls(8, 0x11, yaw, [true; 4]).shatter);
            }
            out
        };
        assert_eq!(shattered(0), [0, 2, 1]);
        assert_eq!(shattered(0x800), [0, 1, 3]);
        assert_eq!(shattered(0x400), [0, 2, 3]);
        assert_eq!(shattered(-0x400), [2, 1, 3]);
    }
}
