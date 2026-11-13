//! The move scripts' sounds and voices: ops 26 / 27, the voice timers.
//!
//! State: none of its own: updates the serialised `Fighter`
//!
//! Sounds and voices of the scripts: ops 26 and 27, the voice timers after the script,
//! P2's bank remap and the beast form's end sound.

use crate::fight::fighter::{F58, F68, Fighter};

/// The beast form's end sound, the sound request (0x400, 7, 0): SE 7 (simulation version 4 on; before, queued as id 0x400
/// after 7 frames, which played nothing).
pub fn revert_sound(g: crate::fight::version::Gates) -> (i16, i16) {
    if !g.revert_sound { (0x400, 7) } else { (7, 0) }
}

/// P2's character bank sits in slot 4: ids of the character slot (0xC0..) move up by 0x40.
pub fn sound_remap(f: &Fighter, id: i16) -> i16 {
    if id >= 0xC0 && f.player != 0 { id + 0x40 } else { id }
}

/// Generic voices 0xFF / 0xFE / 0xFD per character.
const VOICE_GENERIC: [[i16; 3]; 11] = [
    [0xD3, 0xD4, 0xD5],
    [0xCF, 0xD0, 0xD1],
    [0xCD, 0xCE, 0xCF],
    [0xCD, 0xCE, 0xCF],
    [0xCE, 0xCF, 0xD0],
    [0xD0, 0xD1, 0xD2],
    [0xCD, 0xCE, 0xCF],
    [0xCB, 0xCC, 0xCD],
    [0xD2, 0xD3, 0xD0],
    [0xD2, 0xD3, 0xCC],
    [0xCC, 0xCD, 0xCE],
];
/// Shout bases in human form.
const SHOUT_BASE: [i16; 11] = [0xC8, 0xC8, 0xC7, 0xC9, 0xC7, 0xC8, 0xC7, 0xC7, 0xC8, 0xC7, 0xC7];

/// Voice timers after the script: play when the timer runs out.
pub fn voice_timers(f: &mut Fighter, sounds: &mut Vec<(i16, i16)>) {
    if f.voice_timer > 0 {
        f.voice_timer -= f.step;
        if f.voice_timer <= 0 {
            f.voice_timer = 0;
            if f.voice_id >= 0 {
                sounds.push((sound_remap(f, f.voice_id), 0));
            }
        }
    }
    if f.voice2_timer > 0 {
        f.voice2_timer -= f.step;
        if f.voice2_timer <= 0 {
            f.voice2_timer = 0;
            if f.voice2_id >= 0 && (f.flags[F58] as i32) < 0 {
                sounds.push((sound_remap(f, f.voice2_id), 0));
            }
        }
    }
}

/// Ops 26 and 27: sounds and voices of fighter `t`; `other` is the other fighter of the
/// pair (mode 2 sets its hit sound).
#[allow(clippy::too_many_arguments)]
pub(super) fn sound_op(fs: &mut [Fighter], t: usize, other: usize, op: u8, mode: u8, id: i16, time: i16, r: i32, sounds: &mut Vec<(i16, i16)>) {
    let f = &mut fs[t];
    let chr = (f.chr as usize).min(10);
    if op == 0x27 {
        match mode {
            0 => {
                let d = if f.step != 0 { ((time as i32) << 4) / f.step as i32 } else { 0 };
                sounds.push((sound_remap(f, id), d as i16));
            }
            1 => {
                let v = if f.beast() {
                    match id {
                        0 | 0xFD..=0xFF => 0xC4,
                        1..=10 => 0xC3,
                        _ => id,
                    }
                } else {
                    match id {
                        0xFF => VOICE_GENERIC[chr][0],
                        0xFE => VOICE_GENERIC[chr][1],
                        0xFD => VOICE_GENERIC[chr][2],
                        _ => id,
                    }
                };
                f.voice_id = v;
                f.voice_timer = time.wrapping_shl(4);
            }
            _ => {}
        }
        return;
    }
    match mode {
        0 => sounds.push((sound_remap(f, id), 0)),
        1 => {
            let h = f.hit_sound;
            let kind = match f.flags[F68] & 0x340 {
                0x100 => 1,
                0x200 => 2,
                _ => 0,
            };
            match h {
                -1 => {}
                0 | 0x40 => sounds.push((h + kind, 0)),
                _ => sounds.push((sound_remap(f, h), 0)),
            }
        }
        2 => fs[other].hit_sound = id,
        3 => f.voice2_id = id,
        4 => f.voice2_timer = id.wrapping_shl(4),
        5 | 8 => {
            let base = if f.beast() { 0x57 } else { SHOUT_BASE[chr] };
            let v = sound_remap(f, base + (r % 3) as i16);
            sounds.push((v, 0));
            if mode == 8 {
                sounds.push((v, 4));
            }
        }
        7 => sounds.push((if id == 0 { 0x80 } else { 0x81 }, 0)),
        9 => sounds.push((if id == 0 { 0x48 } else { 0x49 }, 0)),
        _ => {}
    }
}
