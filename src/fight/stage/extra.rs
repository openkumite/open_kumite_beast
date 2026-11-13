//! The engine's extra stages: Quake III Arena maps from the player's own local files (Q3DM17 "The Longest Yard",
//! Q3DM6 "The Camping Grounds").
//!
//! Engine addition.
//! State: none (constants; a ring goes into `stage::Stage`'s serialised fields)
//!
//! Stages the console game does not have (docs/extra_stage_q3dm17.md, docs/extra_stage_q3dm6.md): selectable after the main game's
//! stages on the stage select of the modes that have one (VS, PRACTICE, WATCH, netplay; not the ladders) when the
//! session says the files are there (`SessionStart::extra_stage`, `SessionStart::extra_q3dm6`), each from the
//! simulation version that brought it (`Extra::sim`). The fight runs one as the main-game stage `Extra::stand_in` for
//! everything the game's static tables decide (M.98, the light set, the sound bank, the stage routine), with a ring of its
//! own (`stage::Stage::new`): Q3DM17 an open box fitted to the map's central platform (a ring-out over every edge
//! into the void), Q3DM6 a box at its courtyard's walls (invisible indestructible walls on all four sides, as ST4's).
//! From simulation v14 Q3DM17's teleporter stands solid in its ring (`Extra::solids`, `physics::solids`). The map's
//! geometry is presentation only (`formats::q3_map`); the logic knows the ring and these constant footprints alone
//! (measured once in the map, never read from it: a fight runs the same without the files).
//!
//! Units: 1 Quake unit = `scale` game units (`version::Gates::q3_scale`): 7 from simulation v11 (a Quake III player model in its idle pose, 61
//! units from feet to head (the median), is then 428, the main-game fighters stand 424 on average;
//! docs/extra_stage_q3dm17.md), 8 at v10 (Q3DM17 only).
//! A map's point goes into the fight's space round its ring's centre (`Extra::centre`, `Extra::floor`): the game's
//! (x, z) = `Extra::turn` · (Quake x − centre x, Quake y − centre y) · scale, its y (down) = −(Quake z − floor) · scale.

use crate::fight::version::{Gates, v};

/// A solid part standing in an extra stage's ring (simulation v14, `physics::solids`): its footprint on the floor in
/// Quake units on the map (as `Extra::centre`), a box (half sizes along the map's x / y) or a disc (its radius),
/// solid from the floor up (as the ring's walls, it has no height).
pub enum Solid {
    Box { centre: [i32; 2], half: [i32; 2] },
    Disc { centre: [i32; 2], r: i32 },
}

/// A `Solid` in the fight's space (game units, x / z): a box's half sizes, or a disc's radius in `half[0]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameSolid {
    pub disc: bool,
    pub centre: [i32; 2],
    pub half: [i32; 2],
}

/// Q3DM17's teleporter on its platform's far edge (models/mapobjects/teleporter, measured in q3dm17.glb's world;
/// docs/extra_stage_q3dm17.md): its pad, a regular octagon round (287.6, 64.4) whose corners lie 41.3..42.4 from its
/// centre, as the disc through them; the pad's step towards the platform's middle (x 229.6..263.3, y 53.4..74.6);
/// the ring standing on the pad, its feet x 264.2..311.7 over y 2.6..126.0, its narrower sides and the knobs on them
/// x 277.6..299.0 over y −23.8..152.4 (all of it below a fighter's head, 61 units over the floor).
const Q3DM17_SOLIDS: [Solid; 4] = [
    Solid::Disc { centre: [288, 64], r: 42 },
    Solid::Box { centre: [246, 64], half: [17, 11] },
    Solid::Box { centre: [288, 64], half: [24, 62] },
    Solid::Box { centre: [288, 64], half: [11, 88] },
];

/// Q3DM6's courtyard corners (docs/extra_stage_q3dm6.md, Quake units): north-west (−896, −128)..(−768, 0), north-east
/// (−384, 0)..(−256, −128), south-west (−896, −415)..(−831, −480), south-east (−321, −480)..(−256, −415) (the south
/// ones where their diagonal meets the pillars' faces at y −480 and the side walls).
const Q3DM6_CHAMFERS: [[[i32; 2]; 2]; 4] = [[[-896, -128], [-768, 0]], [[-384, 0], [-256, -128]], [[-896, -415], [-831, -480]], [[-321, -480], [-256, -415]]];

/// A chamfer in the fight's space (team battle T7, `physics::chamfers`): the floor where `n · (x, z) > c` is stone,
/// `n` (±1, ±1) pointing out of the ring (|n| = √2), `c` > 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chamfer {
    pub n: [i32; 2],
    pub c: i32,
}

/// One extra stage.
pub struct Extra {
    /// the stage number (`MatchSpec::stage`), after the main game's 13
    pub stage: u8,
    /// its code (`fight_data::stage_code`, a direct fight's `--stage`) and the stage select's row label
    pub code: &'static str,
    /// the map's title (`fight_data::Loaded::stage_name`)
    pub name: &'static str,
    /// the first simulation version that offers it
    pub sim: u32,
    /// the main-game stage whose static tables the fight reads for it (STB: the extra stages skip the stage effects)
    pub stand_in: usize,
    /// the stage whose music BGM SELECT = ARCADE plays on it (`fight_data::music_tracks`; CONSOLE plays the
    /// opponent's track, as on every stage)
    pub music_stage: usize,
    /// the ring's centre on the map (Quake x, y) and its floor's height (Quake z)
    pub centre: [i32; 2],
    pub floor: i32,
    /// the game's (x, z) per Quake (x, y) round the centre: a quarter turn (rows: game x, game z)
    pub turn: [[i32; 2]; 2],
    /// the ring's half sizes (Quake units, along the game's x / z)
    pub half_q: [i32; 2],
    /// the ring's sides are walls (invisible, indestructible: hp −1 as ST4's), else open (a ring-out)
    pub walled: bool,
    /// an open ring's ground outside (Quake units below the floor: where a fighter rung out lands)
    pub drop_q: i32,
    /// what stands solid in the ring from simulation v14 (`v::EXTRA_SOLIDS`): footprints on the map
    pub solid_q: &'static [Solid],
    /// the ring box's corners the map cuts off at 45° (Quake x, y: the two ends of each chamfer's face on the floor),
    /// solid in the team battle from simulation v25 (`Extra::chamfers`, `physics::chamfers`)
    pub chamfer_q: &'static [[[i32; 2]; 2]],
    /// presentation only: the shader of the floor drawn on as far as the eye sees just under the ring's floor
    /// (`present::fight_scene::arena`), None for none. The main game's camera stands outside a courtyard's walls when the
    /// fighters are near them (it knows no stage geometry); looking through the walls' backs it would see the void
    /// under the floor's edge.
    pub ground: Option<&'static str>,
}

/// The extra stages in select order (`frontend`: their rows after EXTRA4, those the session has).
pub const STAGES: [Extra; 2] = [
    // the floating platform above the map's middle (the Quad Damage's, its floor x 32..352, y −96..224 at z 1280),
    // open to the void on all four sides; the fighters line up along the map's y axis, the camera starts on its −x
    // side, looking at the platform's teleporter behind them; a fighter rung out falls to the deck at z 0
    Extra {
        stage: 13,
        code: "Q3DM17",
        name: "The Longest Yard",
        sim: v::Q3DM17,
        stand_in: 11,
        // BUILDING2's: the night rooftop's track, an open platform high in the sky
        music_stage: 8,
        centre: [192, 64],
        floor: 1280,
        turn: [[0, -1], [1, 0]],
        half_q: [160, 160],
        walled: false,
        drop_q: 1280,
        // the teleporter
        solid_q: &Q3DM17_SOLIDS,
        chamfer_q: &[],
        ground: None,
    },
    // the centre courtyard (the Quad Damage's, floor z 0, open to the sky): walls x −896 / −256, y −480 (the south
    // arcade's pillars) / 0, its corners chamfered (docs/extra_stage_q3dm6.md); the fighters line up along the map's
    // x axis (the courtyard's long side), the camera starts on its north side, looking at the south arcade
    Extra {
        stage: 14,
        code: "Q3DM6",
        name: "The Camping Grounds",
        sim: v::Q3DM6,
        stand_in: 11,
        // ROOM's: the other stage closed in by walls on every side
        music_stage: 4,
        centre: [-576, -240],
        floor: 0,
        turn: [[1, 0], [0, 1]],
        half_q: [320, 240],
        walled: true,
        drop_q: 0,
        solid_q: &[],
        // the courtyard's corners (docs/extra_stage_q3dm6.md): the north ones 128 along each wall, the south ones 65
        chamfer_q: &Q3DM6_CHAMFERS,
        // the courtyard's flagstones
        ground: Some("textures/gothic_floor/largerblock3b3"),
    },
];

/// Q3DM17, the first extra stage (`STAGES[0]`).
pub const Q3DM17: &Extra = &STAGES[0];
/// Q3DM6 (`STAGES[1]`).
pub const Q3DM6: &Extra = &STAGES[1];

/// The extra stage of stage number `n`.
pub fn by_stage(n: usize) -> Option<&'static Extra> {
    STAGES.iter().find(|e| e.stage as usize == n)
}

/// The extra stage of code `code` that a simulation version of rules `g` offers.
pub fn by_code(code: &str, g: Gates) -> Option<&'static Extra> {
    STAGES.iter().find(|e| e.code == code && g.reaches(e.sim))
}

impl Extra {
    /// Its index in `STAGES` (the bit of `SessionStart::extras`).
    pub fn index(&self) -> usize {
        (self.stage - STAGES[0].stage) as usize
    }

    /// The ring's half sizes (game units, x / z): Q3DM17 the platform's edges (v11: 1120, v10: 1280), Q3DM6 the
    /// courtyard's walls (2240 / 1680).
    pub const fn ring_half(&self, g: Gates) -> [i16; 2] {
        let s = g.q3_scale();
        [(self.half_q[0] * s) as i16, (self.half_q[1] * s) as i16]
    }

    /// The ground outside the ring (game units below the floor): Q3DM17's deck at Quake z 0, `floor` below (v11:
    /// 8960, v10: 10240); a walled ring's 1 (ST4's: never reached).
    pub const fn ring_drop(&self, g: Gates) -> i16 {
        if self.walled { 1 } else { (self.drop_q * g.q3_scale()) as i16 }
    }

    /// A map point (Quake x, y) in the fight's space (game x, z): round `centre`, turned by `turn`, scaled as the ring.
    fn at(&self, p: [i32; 2], g: Gates) -> [i32; 2] {
        let s = g.q3_scale();
        let [[a, b], [c, d]] = self.turn;
        let (x, y) = (p[0] - self.centre[0], p[1] - self.centre[1]);
        [(a * x + b * y) * s, (c * x + d * y) * s]
    }

    /// The chamfered corners in the fight's space (engine addition, the team battle T7): each face's half plane, its
    /// normal out of the ring (the ring's centre, the origin, inside).
    pub fn chamfers(&self, g: Gates) -> impl Iterator<Item = Chamfer> + '_ {
        self.chamfer_q.iter().map(move |q| {
            let (p, e) = (self.at(q[0], g), self.at(q[1], g));
            let mut n = [(e[1] - p[1]).signum(), -(e[0] - p[0]).signum()];
            let mut c = n[0] * p[0] + n[1] * p[1];
            if c < 0 {
                (n, c) = ([-n[0], -n[1]], -c);
            }
            Chamfer { n, c }
        })
    }

    /// The solid parts in the fight's space (none before simulation v14): the map's (x, y) round `centre` turned by
    /// `turn` and scaled as the ring; Q3DM17's teleporter at game z 378..966.
    pub fn solids(&self, g: Gates) -> impl Iterator<Item = GameSolid> + '_ {
        let s = g.q3_scale();
        let [[a, b], [c, d]] = self.turn;
        let at = move |p: [i32; 2]| self.at(p, g);
        let n = if g.extra_solids { self.solid_q.len() } else { 0 };
        self.solid_q[..n].iter().map(move |q| match *q {
            Solid::Box { centre, half } => {
                GameSolid { disc: false, centre: at(centre), half: [(a * half[0] + b * half[1]).abs() * s, (c * half[0] + d * half[1]).abs() * s] }
            }
            Solid::Disc { centre, r } => GameSolid { disc: true, centre: at(centre), half: [r * s, r * s] },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table's numbers follow the main game's 13 in order, its quarter turns are rotations, and the codes are found.
    #[test]
    fn table() {
        for (k, e) in STAGES.iter().enumerate() {
            assert_eq!((e.stage as usize, e.index()), (13 + k, k));
            let [[a, b], [c, d]] = e.turn;
            assert_eq!((a * d - b * c, a * a + b * b, c * c + d * d), (1, 1, 1), "{}", e.code);
            assert!(by_stage(e.stage as usize).is_some_and(|x| x.code == e.code));
            assert!(by_code(e.code, Gates::of(e.sim)).is_some() && by_code(e.code, Gates::of(e.sim - 1)).is_none());
        }
        assert!(by_stage(12).is_none() && by_code("ST0", Gates::of(99)).is_none());
        // the files' table (L1, `assets::extras::STAGES`) in the same order: each stage's map and older folder named by
        // its code, the paths relative to the install (ok_q3_converter's layout)
        let files = &crate::assets::extras::STAGES;
        assert_eq!(files.len(), STAGES.len());
        for (e, f) in STAGES.iter().zip(files) {
            let name = e.code.to_lowercase();
            assert_eq!((f.legacy, f.map, f.shaders), (format!("extras/{name}").as_str(), format!("baseq3/maps/{name}.glb").as_str(), "shaders.json"));
            assert!(f.sky.iter().all(|s| s.starts_with("baseq3/env/")), "{}", e.code);
        }
        assert_eq!((Q3DM6.ring_half(Gates::of(12)), Q3DM6.ring_drop(Gates::of(12))), ([2240, 1680], 1));
    }

    /// (T7) Q3DM6's chamfered corners in the fight's space: the north ones cut 896 off each side of the box's corner, the
    /// south ones 455 (128 / 65 Quake units at 7), their normals out of the ring; Q3DM17 has none.
    #[test]
    fn chamfers() {
        let c: Vec<_> = Q3DM6.chamfers(Gates::of(25)).map(|c| (c.n, c.c)).collect();
        assert_eq!(c, [([-1, 1], 3024), ([1, 1], 3024), ([-1, -1], 3465), ([1, -1], 3465)]);
        // each face meets the box's sides (2240, 1680) its cut from the corner
        for (n, c) in &c {
            let cut = (2240 + 1680) - c;
            assert_eq!(cut, if n[1] > 0 { 896 } else { 455 });
        }
        assert_eq!(Q3DM17.chamfers(Gates::of(25)).count(), 0);
    }

    /// Q3DM17's teleporter in the fight's space from v14 on (on the game's z axis, behind the fighters' line, inside
    /// the ring's far edge), nothing before; Q3DM6 has no solid parts.
    #[test]
    fn solids() {
        let g = |disc, centre, half| GameSolid { disc, centre, half };
        let v14: Vec<GameSolid> = Q3DM17.solids(Gates::of(14)).collect();
        assert_eq!(v14, [g(true, [0, 672], [294, 294]), g(false, [0, 378], [77, 119]), g(false, [0, 672], [434, 168]), g(false, [0, 672], [616, 77])]);
        let far = Q3DM17.ring_half(Gates::of(14))[1] as i32;
        assert!(v14.iter().all(|s| s.centre[1] + s.half[1] < far && s.centre[1] - s.half[1] > 0));
        assert_eq!((Q3DM17.solids(Gates::of(13)).count(), Q3DM17.solids(Gates::of(10)).count(), Q3DM6.solids(Gates::of(14)).count()), (0, 0, 0));
    }
}
