//! Fighter data, laid out after the game's fighter object and state block.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Fighter data, laid out after the game's two blocks per fighter:
//! the fighter object (fields `o_XX` = object +0xXX) and its script/state block
//! (fighter +0x38, fields `s_XX` = state +0xXX).
//! Fields whose meaning is known have names; the rest keep their offset as their serialised id (`o_XX`, `s_XX`),
//! so the saved layout stays the same.

use serde::{Deserialize, Serialize};

/// Flag words of the fighter object (+0x54 .. +0x6C), indices into `Fighter::flags`.
pub const F54: usize = 0;
pub const F58: usize = 1;
pub const F5C: usize = 2;
pub const F60: usize = 3;
pub const F64: usize = 4;
pub const F68: usize = 5; // hit events of this frame (height, strength, type)
pub const F6C: usize = 6; // requests (thrown, transform, revert …)

pub use crate::fight::anim::playback::Anim;

/// Command candidate block (state +0x18 / +0x20 / +0x30).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub flags: u32, // +0
    pub mv: i16,    // +4 (move index, -1 = none)
    pub life: i16,  // +6 frames left
    pub param: i16, // +8
    pub prio: i16,  // +0xA (record index; only in the +0x20 block)
}

impl Default for Candidate {
    fn default() -> Self {
        Self { flags: 0, mv: -1, life: 0, param: 0, prio: -1 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fighter {
    // ---- object ----
    pub o_00: u8,   // +0x00 status: 1 = fighting, 0x80/0x81 = KO
    pub o_01: u8,   // +0x01 CPU controlled (mode + 1; 0 = human)
    pub player: u8, // +0x02
    pub chr: u8,    // +0x03 character id (indexes per-opponent tables)
    pub form: u8,   // +0x05 bit 0 beast form, bit 4 set while transformed
    pub flags: [u32; 7],
    pub s_16: i16, // S.16 (op 05 sub 4; read by nothing here)
    pub o_24: i16, // weight / mass (ANI param 2 or 3)
    pub o_26: i8,  // attack state: -1 none, 0 active, >0 startup
    pub o_27: i8,  // -1 = cannot be hit
    /// per side: set to 2 when the move is cancelled or hit, counted down per frame; kills trails
    pub fx_cancel: u8,
    /// F.DF: RON's beast-string counter (op 0x2F)
    pub o_df: u8,
    /// the script-driven extra nodes (type 3, the beast's jaw): swing envelopes of nodes 19.. (op 0x25)
    pub swing: [NodeSwing; 8],
    /// spring chains (hair, tails): simulation state, the form it belongs to, and this frame's aims (model space)
    pub springs: Vec<crate::fight::anim::spring::Spring>,
    pub spring_form: u8,
    pub spring_aims: Vec<(usize, glam::Vec3)>,
    /// op 07.03: the pelvis before the snap, and the drawing offset that keeps it in place while the next
    /// animation blends in (the move start rewrites the pelvis pose translation; game units, y down)
    pub pelvis_lock: Option<[i32; 3]>,
    pub draw_off: [f32; 3],
    /// hide flags of the hand objects 11, 14 (fists) and 15, 16 (open hands) in the main object list F.18, per form
    pub hand_hide: [[u16; 4]; 2],
    /// S.A0: the animation viewer's counter (op 05 sub 0 with arg -1)
    pub anim_cycle: i16,
    pub o_aa: i16,
    /// +0xFA: 8.8 accumulator of the beast-form recovery
    pub o_fa: u16,
    pub o_c2: i16,                                      // hit stun
    pub o_c6: i16,                                      // knockback angle of the attack
    pub o_c8: i16,                                      // queued damage
    pub o_ca: i16,                                      // last damage (display)
    pub o_db: u8,                                       // knockback strength
    pub o_dc: u8,                                       // hit level (non-zero = attack can hit)
    pub o_dd: u8,                                       // combo counter
    pub o_fe: i16,                                      // attack factor human (ANI param 8)
    pub o_100: i16,                                     //   beast (param 9)
    pub o_102: i16,                                     // defence factor human (param 10)
    pub o_104: i16,                                     //   beast (param 11)
    pub hit_spheres: Vec<crate::fight::hit::HitSphere>, // +0x14 attack spheres
    pub s_101: [i8; 4],
    pub s_116: i16,
    pub s_3a: i16,
    pub ko: bool,
    pub o_4e: i16,
    pub o_4c: i16, // pitch
    pub yaw: i16,  // +0x4E absolute yaw = facing + animation yaw
    pub o_50: i16, // roll
    pub o_70: i16, // angle to the stage centre (|x| >= 0x400 = facing away)
    pub o_72: i16,
    pub o_74: i16, // angle to the opponent relative to the yaw
    pub o_76: i16, // distance to the opponent
    pub o_78: i16, // height difference to the opponent
    // input (0x8017F9xx)
    pub held: u16,    // +0x7C
    pub prev: u16,    // +0x7E
    pub pressed: u16, // +0x7A new this frame
    pub recent: u16,  // +0x80 pressed within the last 5 frames
    pub dbl: u16,     // +0x82 double tap
    pub long: u16,    // +0x84 held for 3+ frames
    pub o_86: i16,    // normal frame step
    pub step: i16,    // +0x88 frame step in 1/16 (16 = one frame)
    pub o_8a: i16,    // velocity source 1: angle (relative to the facing)
    pub o_8c: i16,    //   speed
    pub o_8e: i16,    //   decay per frame
    pub o_90: i16,
    pub o_92: i16,
    pub o_94: i16,
    pub o_96: i16,
    pub o_98: i16,
    pub vel: [i16; 3],  // +0x9A/+0x9C/+0x9E velocity of this frame
    pub frac: [i16; 2], // +0xA2/+0xA6 1/128 remainders of the polar steps
    pub o_a4: i16,
    pub o_ac: i16,
    pub o_ae: i16,
    pub o_b0: i16,
    pub o_b2: i16,
    pub o_b4: i16,
    pub o_b8: i16,
    pub o_ba: i16,
    pub o_bc: i16,
    pub o_be: i16,
    pub o_c0: i16,
    pub o_c4: i16,
    pub o_cc: i16,
    pub o_ce: i16, // recovery timer (no new action while > 0)
    pub o_d0: i16, // non-zero: cannot be thrown (writer not found yet)
    pub o_d2: i16,
    pub o_d4: i16,
    pub gauge: i16, // +0xD6 beast gauge (0x1E00 needed to transform)
    pub o_d8: i16,
    pub o_da: u8,
    pub o_e5: u8,
    pub o_106: u8, // fraction (1/128) carried between throw-damage ops
    pub o_fc: i16, // frame step in beast form
    // model (fighter +0x20): position +0x18 (s16) and coordinate translation +0x34 (s32)
    pub m_pos: [i16; 3],
    pub m_t: [i32; 3],
    // hips (model +0x174 / +0x178 / +0x17C)
    pub hips: [i32; 3],
    /// body object 0 = node 2 (head), world position
    pub head: [i32; 3],
    /// F->40[0].E..12: the root before this frame's animation step (push-apart direction)
    pub prev_root: [i16; 3],
    pub chest: [i32; 3], // part 3 (model +0x124)
    /// world positions of all skeleton nodes after the last physics pass (model +0x34 + n·0x50)
    pub parts: Vec<[i32; 3]>,
    /// world rotation of every skeleton node (game space, 4.12), for the effect VM's bone matrices
    pub part_rots: Vec<[[i32; 3]; 3]>,
    /// f.48[node]: per-node scale matrices from op 0x1D (MODEL TYPE), as (node, x/y/z scale)
    pub node_scale: Vec<(usize, glam::Vec3)>,
    pub model_268: i32,
    pub model_358: i32,

    // ---- script / state block ----
    pub s_04_group: u8,          // which per-opponent table the state's +4 pointer selects (= opponent chr)
    pub s_14: i16,               // attack index of the current attack move (move - 0x90)
    pub cand_generic: Candidate, // +0x18
    pub cand_attack: Candidate,  // +0x20
    pub s_2c: i16,               // throw: move the opponent is throwing us with
    pub cand_cancel: Candidate,  // +0x30
    pub chain_moves: [i16; 16],  // +0x3C queued chain follow-ups (attack index)
    pub chain_wait: [i16; 16],   // +0x5C recovery value (+0xBC) to wait for before each
    #[serde(with = "crate::base::serde_util::big_array")]
    pub chain_slots: [[i16; 5]; 36], // pc, count, move, window start, window end
    pub chain_pos: u8,           // +0x7C
    pub chain_len: u8,           // +0x7D
    pub chain_sel: i8,           // +0x7E
    pub move_req: i16,           // +0x80
    pub move_cur: i16,           // +0x82
    pub pc: usize,               // +0x84
    pub loop_mark: usize,        // +0x88
    pub ret: usize,              // +0x8C
    pub sleep: i32,              // +0x90 (1/16 frames)
    pub res: i32,                // +0x94
    pub s_98: i16,
    pub s_10a: i16, // auto-turn: frames (low 7 bits) | 0x80 always
    pub s_10c: i16, // auto-turn: angle range
    pub s_9a: i16,
    pub yaw_base: i16, // +0xD0 facing
    pub s_d2: i16,     // +0xD2 yaw change of the animation
    pub pos: [i16; 3], // +0xD4/+0xD6/+0xD8 base position
    pub s_dc: i16,     // +0xDC/+0xDE/+0xE0 animation root offset (1/16 units, facing-relative)
    pub s_de: i16,
    pub s_e0: i16,
    pub s_e4: i16, // +0xE4 turn request
    pub anim: Anim,
    pub halted: bool,
    // sound (state +0x10E .. +0x116)
    pub voice_id: i16,     // +0x10E voice to play when the timer runs out
    pub voice_timer: i16,  // +0x110 (1/16 frames)
    pub hit_sound: i16,    // +0x112 sound of the next hit taken (set by the attacker, op 26 sub 2)
    pub voice2_id: i16,    // +0x114 second delayed sound (only plays during an attack)
    pub voice2_timer: i16, // +0x116

    // per-player input history (button counters, command slots)
    pub btn_hold: [u32; 16],
    pub btn_since: [u32; 16],
    pub since_any_press: u32,
    pub cmd_slots: [[i16; 4]; 32], // step, hold, miss, timer

    /// debug: the unimplemented ops met (not state)
    #[serde(skip)]
    pub trace: Vec<String>,
    /// replay pad hook: the processed pad of this frame, and a recorded one to play instead
    pub pad_record: u16,
    pub pad_override: Option<u16>,
    /// the pose records (F->40, 0x1C bytes per node): what the pose sampler blends toward each key
    pub pose: [PoseRec; 19],
    /// `pose` holds a pose (the first sample snaps to its target instead of blending from the zeroed block)
    pub pose_set: bool,
    /// the head turn's accumulators (state +0x106 yaw, +0x108 pitch; simulation version 13): node 2's
    /// local matrix is turned by RotMatrix(pitch, −yaw, 0)
    #[serde(default)]
    pub head_yaw: i16,
    #[serde(default)]
    pub head_pitch: i16,
    /// node 2's world position of the integer world walk (model +0xD4 / +0xD8 / +0xDC, `pose::head_world`), read
    /// by the next frame's head turn (simulation version 13)
    #[serde(default)]
    pub head_w: [i32; 3],
}

/// One node's pose record (F->40 + 0x1C·n): +0 bone direction (10-bit ×32), +6 twist ×2, +8 stretch ×16
/// (node 1: the body translation, node 4: pose bytes 14..19, 1/16 units).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PoseRec {
    pub dir: [i16; 3],
    pub twist: i16,
    pub stretch: [i16; 3],
}

/// One extra node's swing envelope (node record +0x0A angle, +0x0E rise, +0x10 hold, +0x12 fall, +0x14 target).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct NodeSwing {
    pub ang: i16,
    pub rise: i16,
    pub hold: i16,
    pub fall: i16,
    pub target: i16,
}

impl NodeSwing {
    /// Ease to the target over `rise` calls, hold, ease back over `fall` (C division).
    pub fn step(&mut self) {
        if self.rise > 0 {
            self.ang = self.ang.wrapping_add((self.target as i32 - self.ang as i32).wrapping_div(self.rise as i32) as i16);
            self.rise -= 1;
        } else if self.hold > 0 {
            self.hold -= 1;
        } else if self.fall > 0 {
            self.ang = self.ang.wrapping_sub((self.ang as i32 / self.fall as i32) as i16);
            self.fall -= 1;
        }
    }
}

impl Fighter {
    /// Exchange the left / right hand hide flags (fists 11 <-> 14, open hands 15 <-> 16) of the main list,
    /// done wherever the mirror flag changes.
    pub fn swap_hands(&mut self) {
        let h = &mut self.hand_hide[(self.form & 1) as usize];
        h.swap(0, 1);
        h.swap(2, 3);
    }

    pub fn new(player: u8, chr: u8) -> Self {
        Self {
            o_00: 1,
            o_01: 0,
            player,
            chr,
            form: 0,
            flags: [0; 7],
            s_16: 0,
            o_24: 0x80,
            o_26: -1,
            o_27: 0,
            fx_cancel: 0,
            o_df: 0,
            swing: Default::default(),
            springs: Vec::new(),
            spring_form: 0,
            spring_aims: Vec::new(),
            pelvis_lock: None,
            draw_off: [0.0; 3],
            hand_hide: [[0, 0, 1, 1]; 2],
            anim_cycle: 0,
            o_aa: 0,
            o_c2: 0,
            o_c6: 0,
            o_c8: 0,
            o_ca: 0,
            o_db: 0,
            o_dc: 0,
            o_dd: 0,
            o_fe: 0x80,
            o_100: 0x80,
            o_102: 0x80,
            o_104: 0x80,
            hit_spheres: Vec::new(),
            s_101: [-1; 4],
            s_116: 0,
            s_3a: 0,
            ko: false,
            o_4e: 0,
            o_4c: 0,
            yaw: 0,
            o_50: 0,
            o_70: 0,
            o_72: 0,
            o_74: 0,
            o_76: 0x200,
            o_78: 0,
            held: 0,
            prev: 0,
            pressed: 0,
            recent: 0,
            dbl: 0,
            long: 0,
            o_86: 16,
            step: 16,
            o_8a: 0,
            o_8c: 0,
            o_8e: 0,
            o_90: 0,
            o_92: 0,
            o_94: 0,
            o_96: 0,
            o_98: 0,
            vel: [0; 3],
            frac: [0; 2],
            o_a4: 0,
            o_ac: 0,
            o_ae: 0,
            o_b0: 0,
            o_b2: 0,
            o_b4: 0,
            o_b8: 0,
            o_ba: 0,
            o_bc: 0,
            o_be: 1,
            o_c0: 0,
            o_c4: 0,
            o_cc: 0,
            o_ce: 0,
            o_d0: 0,
            o_d2: 0,
            o_d4: 0,
            gauge: 0,
            o_d8: 0,
            o_fa: 0,
            o_da: 0,
            o_e5: 0,
            o_106: 0,
            o_fc: 16,
            m_pos: [0; 3],
            m_t: [0; 3],
            hips: [0; 3],
            head: [0; 3],
            prev_root: [0; 3],
            chest: [0; 3],
            parts: Vec::new(),
            part_rots: Vec::new(),
            node_scale: Vec::new(),
            model_268: 0,
            model_358: 0,
            s_04_group: 0,
            s_14: 0,
            cand_generic: Candidate::default(),
            cand_attack: Candidate::default(),
            s_2c: 0,
            cand_cancel: Candidate::default(),
            chain_moves: [0; 16],
            chain_wait: [0; 16],
            chain_slots: [[0; 5]; 36],
            chain_pos: 0,
            chain_len: 0,
            chain_sel: -1,
            move_req: 1,
            move_cur: -1,
            pc: 0,
            loop_mark: 0,
            ret: 0,
            sleep: 0,
            res: 0,
            s_98: 0,
            s_10a: 0,
            s_10c: 0,
            s_9a: 0,
            yaw_base: 0,
            s_d2: 0,
            pos: [0; 3],
            s_dc: 0,
            s_de: 0,
            s_e0: 0,
            s_e4: 0,
            anim: Anim { id: -1, ..Default::default() },
            halted: false,
            voice_id: -1,
            voice_timer: 0,
            hit_sound: -1,
            voice2_id: -1,
            voice2_timer: 0,
            btn_hold: [0; 16],
            btn_since: [0; 16],
            since_any_press: 0,
            cmd_slots: [[0; 4]; 32],
            trace: Vec::new(),
            pad_record: 0,
            pad_override: None,
            pose: [PoseRec::default(); 19],
            pose_set: false,
            head_yaw: 0,
            head_pitch: 0,
            head_w: [0; 3],
        }
    }

    /// MODEL TYPE shaping for the pose: op 0x1D scales, root height /2 (KIDS) or ×7/8 (BIG ARMS).
    pub fn shape(&self) -> crate::formats::gltf_model::Shape {
        let f54 = self.flags[F54];
        let root_y = if f54 & 8 != 0 {
            0.5
        } else if f54 & 0x10 != 0 {
            0.875
        } else {
            1.0
        };
        // the beast's script-driven extra nodes (jaw): Rx(ang), 4096 = 360°
        let rot_x = if self.beast() {
            self.swing.iter().enumerate().filter(|(_, s)| s.ang != 0).map(|(k, s)| (19 + k, s.ang as f32 * std::f32::consts::TAU / 4096.0)).collect()
        } else {
            Vec::new()
        };
        crate::formats::gltf_model::Shape { scale: self.node_scale.clone(), root_y, rot_x, aims: Vec::new() }
    }

    /// The shape for drawing: `shape()` plus the spring chains' aims.
    pub fn shape_render(&self) -> crate::formats::gltf_model::Shape {
        crate::formats::gltf_model::Shape { aims: self.spring_aims.clone(), ..self.shape() }
    }

    pub fn beast(&self) -> bool {
        self.form & 1 != 0
    }
}

/// Fighters `a` and `b` (`a != b`) of `fs`, both mutable: a fighter and its opponent in the pair routines.
pub fn pair_mut(fs: &mut [Fighter], a: usize, b: usize) -> (&mut Fighter, &mut Fighter) {
    assert_ne!(a, b, "a pair of two fighters");
    if a < b {
        let (x, y) = fs.split_at_mut(b);
        (&mut x[a], &mut y[0])
    } else {
        let (x, y) = fs.split_at_mut(a);
        (&mut y[0], &mut x[b])
    }
}

#[cfg(test)]
mod tests {
    use super::{Fighter, pair_mut};

    #[test]
    fn pairs_of_any_two_fighters() {
        let mut fs: Vec<Fighter> = (0..4).map(|k| Fighter::new(k as u8, k as u8)).collect();
        for (a, b) in [(0, 1), (1, 0), (3, 1), (0, 2), (2, 3)] {
            let (fa, fb) = pair_mut(&mut fs, a, b);
            assert_eq!((fa.player as usize, fb.player as usize), (a, b));
            fa.o_dd = 7;
        }
        assert_eq!(fs.iter().map(|f| f.o_dd).collect::<Vec<_>>(), [7, 7, 7, 7]);
    }
}
