//! The in-fight pause menu.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The in-fight pause: controller with the normal page (RESUME / CONTROLLER
//! CONFIG. / QUIT and the YES/NO confirm) and the practice page (COM. ACTION, GUARD, GUARD AND ESCAPE,
//! DEFENSIVE FALL). Opening plays 0x0E and lowers the music; closing plays 0x11, restores it, and keeps the fight
//! frozen for 10 more frames. Drawn with font 0 in ot[0], 512×480 layout.

use crate::assets::text;
use crate::fight::input::pad::{DOWN, LEFT, R2, RIGHT, SELECT, START, TRIANGLE, UP};
use crate::output::font::Fonts;
use crate::output::prim::{self, Prim};
use serde::{Deserialize, Serialize};

const OK: u16 = 0xE0; // ○ × □

/// COM. ACTION's values (their texts' keys).
const ACTIONS: [&str; 5] = ["standing", "blocks_after_taking_damage", "crouching", "jumping", "dodging"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    None,
    /// QUIT → YES: the fight ends, back to the title
    Quit,
    /// practice settings changed (new M.D4): rebuild the round
    Reset(u8),
    /// CONTROLLER CONFIG. chosen
    ControllerConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pause {
    /// 0 closed, 1 open, −10..−1 closing
    pub state: i8,
    cursor: i8,
    quit: bool,
    quit_no: bool,
    /// practice values: COM. ACTION 0..4, GUARD, GUARD AND ESCAPE, DEFENSIVE FALL
    vals: [u8; 4],
    loaded: bool,
    /// the fight's music volume request of this frame: Some(true) = lowered (0x201), Some(false) = back (0x202)
    pub music_low: Option<bool>,
}

impl Pause {
    /// m.82 while the pause is open or closing.
    pub fn frozen(&self) -> bool {
        self.state != 0
    }

    fn items(practice: bool) -> i8 {
        if practice { 7 } else { 3 }
    }

    /// One frame. `pressed` = new presses of the human ports, `can_open` = round state 4. Sounds are COMMON_2 ids.
    pub fn tick(&mut self, pressed: u16, can_open: bool, practice: bool, watch: bool, d4: u8, sounds: &mut Vec<(i16, i16)>) -> Action {
        self.music_low = None;
        match self.state {
            0 => {
                if can_open && pressed & START != 0 {
                    sounds.push((0x0E, 0));
                    self.state = 1;
                    self.cursor = 0;
                    self.loaded = false;
                    self.music_low = Some(true);
                }
                Action::None
            }
            1 => {
                let n = Self::items(practice);
                if !self.quit {
                    if pressed & (DOWN | SELECT) != 0 {
                        sounds.push((0x0A, 0));
                        self.cursor = if self.cursor < n - 1 { self.cursor + 1 } else { 0 };
                    }
                    if pressed & UP != 0 {
                        sounds.push((0x0A, 0));
                        self.cursor = if self.cursor == 0 { n - 1 } else { self.cursor - 1 };
                    }
                }
                let (close, act) = if practice { self.practice_page(pressed, d4, sounds) } else { self.normal_page(pressed, watch, sounds) };
                if close {
                    sounds.push((0x11, 0));
                    self.state = -10;
                    self.cursor = 0;
                    self.music_low = Some(false);
                }
                act
            }
            _ => {
                self.state += 1;
                Action::None
            }
        }
    }

    /// QUIT and its YES / NO confirm (shared by both pages); returns (close, action).
    fn quit_item(&mut self, pressed: u16, sounds: &mut Vec<(i16, i16)>) -> (bool, Action) {
        if !self.quit {
            if pressed & OK != 0 {
                sounds.push((0x0C, 0));
                self.quit = true;
                self.quit_no = true; // the confirm opens on NO
            }
            return (false, Action::None);
        }
        if pressed & (LEFT | RIGHT) != 0 {
            sounds.push((0x0A, 0));
            self.quit_no = !self.quit_no;
        }
        if pressed & TRIANGLE != 0 {
            sounds.push((0x0D, 0));
            self.quit = false;
        }
        if pressed & OK != 0 {
            if !self.quit_no {
                sounds.push((0x0C, 0));
                return (false, Action::Quit);
            }
            sounds.push((0x0D, 0));
            self.quit = false;
        }
        (false, Action::None)
    }

    fn normal_page(&mut self, pressed: u16, watch: bool, sounds: &mut Vec<(i16, i16)>) -> (bool, Action) {
        if !self.quit && pressed & START != 0 {
            return (true, Action::None);
        }
        match self.cursor {
            0 => (pressed & OK != 0, Action::None),
            1 => {
                if pressed & OK != 0 && !watch {
                    sounds.push((0x0C, 0));
                    return (false, Action::ControllerConfig);
                }
                (false, Action::None)
            }
            _ => self.quit_item(pressed, sounds),
        }
    }

    fn practice_page(&mut self, pressed: u16, d4: u8, sounds: &mut Vec<(i16, i16)>) -> (bool, Action) {
        if !self.loaded {
            self.vals = [d4 & 7, d4 >> 3 & 1, d4 >> 4 & 1, d4 >> 5 & 1];
            self.loaded = true;
        }
        let apply = |s: &Self| {
            let v = s.vals[0] | s.vals[1] << 3 | s.vals[2] << 4 | s.vals[3] << 5;
            (true, if v != d4 { Action::Reset(v) } else { Action::None })
        };
        if !self.quit && pressed & START != 0 {
            return apply(self);
        }
        let spin = |v: &mut u8, max: u8, sounds: &mut Vec<(i16, i16)>| {
            if pressed == RIGHT {
                *v = if *v >= max { 0 } else { *v + 1 };
                sounds.push((0x0B, 0));
            } else if pressed == LEFT {
                *v = if *v == 0 { max } else { *v - 1 };
                sounds.push((0x0B, 0));
            }
        };
        match self.cursor {
            0 => {
                if pressed & OK != 0 {
                    return apply(self);
                }
            }
            1 => spin(&mut self.vals[0], 4, sounds),
            2..=4 => {
                let k = self.cursor as usize - 1;
                spin(&mut self.vals[k], 1, sounds);
            }
            5 => {
                if pressed & OK != 0 {
                    sounds.push((0x0C, 0));
                    return (false, Action::ControllerConfig);
                }
            }
            _ => return self.quit_item(pressed, sounds),
        }
        (false, Action::None)
    }

    /// The pause screen in 512×480 space (nothing while closing; R2 held hides the menu, not the confirm).
    pub fn draw(&self, f: &Fonts, practice: bool, watch: bool, held: u16) -> Vec<Prim> {
        let mut out = Vec::new();
        if self.state != 1 {
            return out;
        }
        let (w, h, l) = (512, 480, 16);
        let boxed = |out: &mut Vec<Prim>, x: i32, y: i32, bw: i32, bh: i32, subtract: bool| {
            let mut b = prim::f4(x, y, bw, bh, [16, 16, 32]);
            if subtract {
                b.semi = true;
                b.tpage = 0x40;
            }
            out.push(b);
        };
        if self.quit {
            let (x, y) = ((w - 64) / 2, h / 4);
            boxed(&mut out, x - 4, y - l / 2, 72, 3 * l, false);
            f.small_text(&mut out, text::get("quit"), x + 16, y, 4);
            f.small_text(&mut out, text::get("yes"), x, y + l, if self.quit_no { 0 } else { 1 });
            f.small_text(&mut out, text::get("no"), x + 48, y + l, if self.quit_no { 1 } else { 0 });
            return out;
        }
        if held & R2 != 0 {
            return out;
        }
        let sel = |k: i8| if self.cursor == k { 1 } else { 0 };
        if practice {
            let (x, y) = ((w - 320) / 2, h / 4);
            boxed(&mut out, x - 4, y - l / 2, 320, 9 * l, false);
            f.small_text(&mut out, text::get("practice_menu"), x + 20, y, 4);
            let onoff = |v: u8| text::get(if v != 0 { "on" } else { "off" });
            let rows: [(&str, Option<&str>); 7] = [
                (text::get("resume"), None),
                (text::get("com_action"), Some(text::get(ACTIONS[self.vals[0].min(4) as usize]))),
                (text::get("com_guard"), Some(onoff(self.vals[1]))),
                (text::get("com_guard_and_escape"), Some(onoff(self.vals[2]))),
                (text::get("com_defensive_fall"), Some(onoff(self.vals[3]))),
                (text::get("screen_title.5"), None),
                (text::get("quit"), None),
            ];
            for (k, (label, val)) in rows.iter().enumerate() {
                let yy = y + l * (k as i32 + 1);
                f.small_text(&mut out, label, x + 8, yy, sel(k as i8));
                if let Some(v) = val {
                    f.small_text(&mut out, v, x + 0x98, yy, sel(k as i8));
                }
            }
            f.small_text(&mut out, ">", x, y + l * (self.cursor as i32 + 1), 1);
            return out;
        }
        let (x, y) = ((w - 72) / 2, h / 4);
        boxed(&mut out, x - 4, y - l / 2, 88, 6 * l, true);
        f.small_text(&mut out, text::get("pause"), x + 20, y, 4);
        f.small_text(&mut out, text::get("resume"), x + 8, y + l, sel(0));
        let cfg = if watch { 2 } else { sel(1) };
        f.small_text(&mut out, text::get("controller"), x + 8, y + 2 * l, cfg);
        f.small_text(&mut out, text::get("config"), x + 8, y + 3 * l, cfg);
        f.small_text(&mut out, text::get("quit"), x + 8, y + 4 * l, sel(2));
        let cy = [y + l, y + 2 * l, y + 4 * l][self.cursor.clamp(0, 2) as usize];
        f.small_text(&mut out, ">", x, cy, 1);
        out
    }
}
