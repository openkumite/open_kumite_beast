//! An imported fighter as the game holds it: the fighter block F, the script state S and the RAM they point at.
//!
//! State: serialised (the whole `Br1Fighter`; static data stays in `Br1Res`)
//!
//! Each field's comment gives its block offset. `Block` and `State` hold every
//! scalar of the two blocks (pointers excluded: the static tables are `Br1Res`', the pointers into the ANM are
//! offsets: `pc`, `loop_mark`, `ret`, `Anim::clip` / `target`), fields of unknown meaning by their offset; `LAYOUT`
//! lists their byte layout (the reference runner compares the blocks with the recorded ones field by field).

use super::pose::{Anim, M, Parts, Rec};
use serde::{Deserialize, Serialize};

/// Little-endian access of one field of a block (`block!`).
pub trait Le: Sized {
    fn get(b: &[u8], o: usize) -> Self;
    fn put(&self, b: &mut [u8], o: usize);
}

macro_rules! le_int {
    ($($t:ty),*) => {$(
        impl Le for $t {
            fn get(b: &[u8], o: usize) -> Self {
                const N: usize = std::mem::size_of::<$t>();
                b.get(o..o + N).map_or(0, |s| <$t>::from_le_bytes(s.try_into().unwrap_or([0; N])))
            }
            fn put(&self, b: &mut [u8], o: usize) {
                const N: usize = std::mem::size_of::<$t>();
                if let Some(s) = b.get_mut(o..o + N) {
                    s.copy_from_slice(&self.to_le_bytes());
                }
            }
        }
    )*};
}
le_int!(u8, i8, u16, i16, u32, i32);

impl<T: Le + Copy + Default, const N: usize> Le for [T; N] {
    fn get(b: &[u8], o: usize) -> Self {
        let mut v = [T::default(); N];
        let k = std::mem::size_of::<T>();
        for (i, e) in v.iter_mut().enumerate() {
            *e = T::get(b, o + i * k);
        }
        v
    }
    fn put(&self, b: &mut [u8], o: usize) {
        let k = std::mem::size_of::<T>();
        for (i, e) in self.iter().enumerate() {
            e.put(b, o + i * k);
        }
    }
}

/// A block of fields at fixed offsets: the struct, its `LAYOUT` (name, offset, size) and `read` / `write` of the
/// game's little-endian bytes.
macro_rules! block {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* $off:literal $field:ident: $ty:ty,)* }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
        pub struct $name { $($(#[$fm])* pub $field: $ty,)* }
        impl $name {
            pub const LAYOUT: &'static [(&'static str, usize, usize)] =
                &[$((stringify!($field), $off, std::mem::size_of::<$ty>()),)*];
            pub fn read(b: &[u8]) -> Self {
                Self { $($field: <$ty as Le>::get(b, $off),)* }
            }
            pub fn write(&self, b: &mut [u8]) {
                $(Le::put(&self.$field, b, $off);)*
            }
        }
    };
}

block! {
    /// The fighter block F (0xEC bytes) without its pointers (+0x0C..+0x43).
    pub struct Block {
        /// F.00: 1 fighting, 0x80 / 0x81 KO
        0x00 status: u8,
        /// F.01: CPU controlled
        0x01 cpu: u8,
        /// F.02: side 0 / 1
        0x02 player: u8,
        /// F.03: character (ZIN 0, GRE 1, EVE 2, VAK 3, MIT 4, GAD 5, FOX 6, RON 7, URI 8)
        0x03 chr: u8,
        0x04 f04: u8,
        /// F.05: bit 0 beast, bit 4 transformed
        0x05 form: u8,
        /// F.06 / F.07: object / node count of the current form (form descriptor +0 / +1)
        0x06 objects: u8,
        0x07 nodes: u8,
        /// F.08 / F.09: body / hit sphere counts of the current form (descriptor +2 / +3)
        0x08 body_spheres: u8,
        0x09 hit_spheres: u8,
        /// F.0A: the ANM's clips are relocated (loader)
        0x0A f0a: u8,
        0x0B f0b: u8,
        /// F.20: weight (descriptor +0x18)
        0x20 weight: i16,
        /// F.22: attack state: −1 none, 0 active
        0x22 attack: i8,
        /// F.23: −1 = cannot be hit
        0x23 hittable: i8,
        /// F.44 / F.46 / F.48: node 0 pitch / yaw (facing + animation yaw) / roll
        0x44 pitch: i16,
        0x46 yaw: i16,
        0x48 roll: i16,
        0x4A f4a: u16,
        /// F.4C .. F.64: the flag words (the main game F54, F58, F5C, F60, F64, F68 hit result, F6C)
        0x4C f4c: u32,
        0x50 f50: u32,
        0x54 f54: u32,
        0x58 f58: u32,
        0x5C f5c: u32,
        0x60 f60: u32,
        0x64 f64: u32,
        /// F.68 / F.6A: angle / distance to the stage centre
        0x68 centre_angle: i16,
        0x6A centre_dist: i16,
        /// F.6C / F.6E: angle to the opponent relative to the yaw / distance to the opponent
        0x6C opp_angle: i16,
        0x6E opp_dist: i16,
        /// F.70: opponent hips y − own hips y
        0x70 opp_dy: i16,
        /// F.72 .. F.7C: pad words pressed, held, previous, recent, double tap, long
        0x72 pressed: u16,
        0x74 held: u16,
        0x76 prev: u16,
        0x78 recent: u16,
        0x7A dbl: u16,
        0x7C long: u16,
        /// F.7E / F.80: normal / current frame step (1/16 frames)
        0x7E step_normal: i16,
        0x80 step: i16,
        /// F.82 / F.84 / F.86: velocity source 1 angle / speed / decay
        0x82 v_angle: i16,
        0x84 v_speed: i16,
        0x86 v_decay: i16,
        /// F.88 / F.8A: vertical velocity (y down) / gravity
        0x88 vy: i16,
        0x8A gravity: i16,
        /// F.8C: floor height under the hips
        0x8C floor: i16,
        /// F.8E / F.90: launch angle / horizontal launch speed
        0x8E launch_angle: i16,
        0x90 launch_speed: i16,
        /// F.92 .. F.96: velocity of this frame
        0x92 vel: [i16; 3],
        0x98 f98: [i16; 5],
        /// F.A2: blue bar
        0xA2 blue: i16,
        /// F.A4 / F.A6 / F.A8: damage taken (−1 = KO) / its permanent part / max HP
        0xA4 damage: i16,
        0xA6 damage_perm: i16,
        0xA8 max_hp: i16,
        /// F.AA / F.AC: push speed / decay
        0xAA push_speed: i16,
        0xAC push_decay: i16,
        /// F.AE: move timer (+= step every frame)
        0xAE move_time: i16,
        /// F.B0 / F.B2 / F.B4: active time ×16 / startup time ×16 / recovery marker (op 0x10)
        0xB0 active: i16,
        0xB2 startup: i16,
        0xB4 recovery: i16,
        /// F.B6: guard stun (frames)
        0xB6 guard_stun: i16,
        /// F.B8: down / juggle timer
        0xB8 down_timer: i16,
        /// F.BA / F.BC: knockback direction / knockback angle of the attack (op 0x0F)
        0xBA knock_dir: i16,
        0xBC knock_angle: i16,
        /// F.BE / F.C0: queued damage / last damage
        0xBE queued_damage: i16,
        0xC0 last_damage: i16,
        /// F.C2: hit-stun timer ×16
        0xC2 hit_stun: i16,
        /// F.C4: recovery timer
        0xC4 recover_timer: i16,
        /// F.C6: cannot be thrown (×16)
        0xC6 no_throw: i16,
        /// F.C8 (op 0x14 sub 2, op 0x02 −6)
        0xC8 fc8: i16,
        /// F.CA / F.CC: beast gauge / queued gauge change
        0xCA gauge: i16,
        0xCC gauge_queued: i16,
        /// F.CE / F.CF: damage / knockback strength of the current attack (ops 0x0E / 0x0F)
        0xCE attack_damage: u8,
        0xCF knock_strength: u8,
        /// F.D0: combo counter
        0xD0 combo: u8,
        /// F.D1 / F.D2: red-health regeneration interval / countdown
        0xD1 regen_interval: u8,
        0xD2 regen_count: u8,
        /// F.D3: vitality factor
        0xD3 vitality: u8,
        /// F.D4: rave timer (≤ 0 with rave off)
        0xD4 rave: i8,
        0xD5 fd5: [u8; 5],
        0xDA fda: u8,
        /// F.DB: rounds lost
        0xDB rounds_lost: u8,
        0xDC fdc: [u32; 4],
    }
}

block! {
    /// The script state S (0x140 bytes) without its pointers (+0x00..+0x13, +0x18, the pc registers, the clip
    /// tables and the bone-map pointers) and without the animation fields (`Anim`).
    pub struct State {
        0x14 s14: u32,
        /// S.1C: attack index (decision result − 0x80)
        0x1C attack_index: i16,
        /// S.1E (op 0x05 sub 4)
        0x1E s1e: i16,
        /// S.20 / S.24 / S.26: the generic command candidate {flags, state, life}
        0x20 cand_flags: u32,
        0x24 cand_mv: i16,
        0x26 cand_life: i16,
        /// S.28 .. S.32: the attack candidate {flags, mv, life, param, prio} (op 0x20 forces it)
        0x28 att_flags: u32,
        0x2C att_mv: i16,
        0x2E att_life: i16,
        0x30 att_param: i16,
        0x32 att_prio: i16,
        /// S.34: the thrower's move we are thrown with
        0x34 thrown_by: i16,
        0x36 s36: i16,
        /// S.38 .. S.40: the cancel candidate {flags, mv, life, param}
        0x38 cancel_flags: u32,
        0x3C cancel_mv: i16,
        0x3E cancel_life: i16,
        0x40 cancel_param: i16,
        /// S.42: damage sum of the move (op 0x0E)
        0x42 damage_sum: i16,
        /// S.44..S.83: the chain follow-up queue
        0x44 chain_queue: [i16; 32],
        /// S.84 / S.85 / S.86: chain pos / len / sel
        0x84 chain_pos: u8,
        0x85 chain_len: u8,
        0x86 chain_sel: u8,
        0x87 s87: u8,
        /// S.88 / S.8A: state requested / current (state = move + 0x80)
        0x88 state_req: i16,
        0x8A state_cur: i16,
        /// S.98: sleep (1/16 frames)
        0x98 sleep: i16,
        0x9A s9a: i16,
        /// S.9C: the result register
        0x9C res: i32,
        /// S.A0 / S.A2: the state to continue with / to take on landing (op 0x05 subs 2 / 3)
        0xA0 next_state: i16,
        0xA2 land_state: i16,
        0xA5 sa5: [u8; 3],
        /// S.A8: the clip viewer's counter (op 0x05 sub 0, clip −1)
        0xA8 clip_cycle: i16,
        0xAE sae: i16,
        0xE2 se2: i16,
        0xEA sea: i16,
        0xF2 sf2: i16,
        0xFA sfa: i16,
        0x102 s102: i16,
        0x10A s10a: i16,
        0x10E s10e: i16,
        0x129 s129: u8,
        /// S.12A / S.12C: the trail mode / time of op 0x0D (op 0x23)
        0x12A trail_mode: i16,
        0x12C trail_time: i16,
        /// S.132: the auto-turn request (ops 0x11 / 0x28)
        0x132 auto_turn: i16,
        /// S.134 / S.136: the delayed sound and its timer ×16 (op 0x27 sub 1)
        0x134 delayed_sound: i16,
        0x136 delayed_timer: i16,
        /// S.138 / S.13A: the impact-sound class (written by the opponent's op 0x10) / the own one
        0x138 impact_class: i16,
        0x13A impact_own: i16,
        /// S.13C: the startup copy (op 0x10), a sound timer
        0x13C sound_timer: i16,
        0x13E s13e: i16,
    }
}

/// An attack sphere of the list `*(F.14)`: {s8 flag (−1 = end), u8 node + 1, u16 radius}.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sphere {
    pub flag: i8,
    pub node: u8,
    pub radius: u16,
}

/// A model node (`*(F.1C)` + 0x50·n) as far as the fighter code reads or writes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// +0x04 / +0x18: local rotation (4.12, stored s16) and translation (s16)
    pub lm: M,
    pub lt: [i32; 3],
    /// +0x20 / +0x34: world rotation and translation (s32), from the last world walk
    pub wm: M,
    pub wt: [i32; 3],
}

/// One form's own lists: its pose records and model nodes (form descriptor +0x14 / +0x10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Form {
    pub rec: Vec<Rec>,
    pub md: Vec<Node>,
}

/// The per-form values of the form descriptor block `*(F.24)` + 0x1C·form that the code reads at run time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDesc {
    /// +0 / +1 / +2 / +3: object, node, body sphere, hit sphere counts
    pub objects: u8,
    pub nodes: u8,
    pub body_spheres: u8,
    pub hit_spheres: u8,
    /// +0x18: weight
    pub weight: i16,
    /// +0x1A: the factor K of op 0x14 sub 0 and op 0x19 (ANM +0x30 / +0x32)
    pub factor: i16,
}

/// The fighter's RAM of the input routines outside F and S, by player.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// per pad bit the frames held (+0) and the frames since its press (+0x40); 0x80 bytes a player
    pub hold: [u32; 16],
    pub since: [u32; 16],
    /// per command record {step, hold count, miss count, life}; 0x100 bytes a player
    pub cmd: [[i16; 4]; 32],
    /// per chain program {pc (words), count, move, window start, window end}; 0x140 bytes a player
    pub chain: [[i16; 5]; 32],
    /// the button config (0x10 bytes a player), the logical bits of raw □ × ○ △ L1 L2 R1 R2 (`input::RAW_BITS`:
    /// raw 0x80, 0x40, 0x20, 0x10, 0x04, 0x01, 0x08, 0x02)
    pub buttons: [u16; 8],
}

/// An imported fighter: F, S and its RAM.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Br1Fighter {
    pub f: Block,
    pub s: State,
    /// S.8C / S.90 / S.94: program counter, loop mark, return address (offsets into the own ANM)
    pub pc: u32,
    pub loop_mark: u32,
    pub ret: u32,
    pub anim: Anim,
    /// `*(F.14)`
    pub spheres: [Sphere; 8],
    /// which form's lists F.18 / F.1C / F.38 point at
    pub cur: u8,
    pub forms: [Form; 2],
    pub desc: [FormDesc; 2],
    /// both forms' object lists: hidden flags
    pub parts: Parts,
    /// the input routines' RAM (S5)
    #[serde(default)]
    pub input: Input,
    /// per form, objects 0 and 2's world translations (object + 0x70: the world walk sets them from their
    /// node, the model shift moves them; the push-apart reads them)
    #[serde(default)]
    pub objs: [[[i32; 3]; 2]; 2],
}

impl Br1Fighter {
    /// `F.5 & 1`.
    pub fn beast(&self) -> bool {
        self.f.form & 1 != 0
    }

    /// The current form's lists.
    pub fn form(&self) -> &Form {
        &self.forms[self.cur as usize & 1]
    }
    pub fn form_mut(&mut self) -> &mut Form {
        &mut self.forms[self.cur as usize & 1]
    }

    /// World translation of node `n` of the current form (MD + 0x34 + 0x50·n); 0 past the list.
    pub fn world(&self, n: usize) -> [i32; 3] {
        self.form().md.get(n).map_or([0; 3], |m| m.wt)
    }

    /// The hips' world y (MD+0x178).
    pub fn hips_y(&self) -> i32 {
        self.world(4)[1]
    }

    /// The op-14 factor K of the current form (`F.24 + 0x1C·(F.5 & 1) + 0x1A`).
    pub fn factor(&self) -> i32 {
        self.desc[self.f.form as usize & 1].factor as i32
    }
}
