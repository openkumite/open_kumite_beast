//! What confirm and ←/→ do on each list menu's rows.
//!
//! State: none of its own: updates the serialised `Menu`
//!
//! What a confirm or ←/→ does on each list menu's rows: MEMORY CARD, MOVIES AND PICTURES,
//! OPTIONS, GRAPHICS (engine addition), SOUND OPTIONS and CUSTOM; values go into the menu's scratch copy.

use super::{Action, CHANGE_DATA, CONFIRM, Menu, SE_BANKS};
use crate::fight::input::pad::{LEFT, RIGHT};
use crate::frontend::options::{Aspect, Custom, HUD_SCALES, Prefs, RENDER_SCALES, WINDOW_SIZES};

impl Menu {
    /// AUTO SAVE ←/→ toggles opt.22 at once, ON starts the save flow; LOAD / SAVE start theirs.
    pub(super) fn memcard_action(&mut self, rep: u16, confirm: bool, sounds: &mut Vec<u16>) -> Action {
        match self.cursor {
            1 => {
                if Self::wrap(&mut self.s.auto_save, 1, rep, sounds) && self.s.auto_save != 0 {
                    return Action::Card(1);
                }
            }
            2 | 3 if confirm => {
                sounds.push(0x0C);
                return Action::Card(self.cursor);
            }
            4 if confirm => {
                sounds.push(0x0C);
                return Action::Leave;
            }
            _ => {}
        }
        Action::None
    }

    /// MOVIES cycles through the unlocked movies (opt.28), PICTURES through the unlocked pictures;
    /// confirm plays / shows (sfx 0x0C), EXIT leaves.
    pub(super) fn movies_action(&mut self, rep: u16, confirm: bool, sounds: &mut Vec<u16>) -> Action {
        match self.cursor {
            1 => {
                let unlocked = |m: usize| self.s.movies >> m & 1 != 0;
                let old = self.movie;
                if rep & RIGHT != 0 {
                    let mut m = self.movie;
                    while m < 3 {
                        m += 1;
                        if unlocked(m) {
                            self.movie = m;
                            break;
                        }
                    }
                } else if rep & LEFT != 0 {
                    let mut m = self.movie;
                    while m > 0 {
                        m -= 1;
                        if unlocked(m) {
                            self.movie = m;
                            break;
                        }
                    }
                }
                if self.movie != old {
                    sounds.push(0x0B);
                }
                if confirm {
                    sounds.push(0x0C);
                    return Action::PlayMovie(self.movie);
                }
            }
            2 => {
                let n = self.picture_count();
                if n > 0 {
                    if rep & RIGHT != 0 {
                        self.picture = (self.picture + 1) % n;
                        sounds.push(0x0B);
                    } else if rep & LEFT != 0 {
                        self.picture = (self.picture + n - 1) % n;
                        sounds.push(0x0B);
                    }
                    if confirm {
                        sounds.push(0x0C);
                        if let Some(p) = self.nth_picture(self.picture) {
                            return Action::ShowPicture(p);
                        }
                    }
                }
            }
            3 if confirm => {
                sounds.push(0x0C);
                return Action::Leave;
            }
            _ => {}
        }
        Action::None
    }

    pub(super) fn options_action(&mut self, rep: u16, confirm: bool, sounds: &mut Vec<u16>) -> Action {
        let item = self.options_item(self.cursor);
        let s = &mut self.s;
        match item {
            1 if confirm => {
                sounds.push(0x0C);
                return Action::OpenMemCard;
            }
            // engine addition (v9): CHANGE GAME DATA after GRAPHICS, the items 11..15 one down
            CHANGE_DATA if confirm => {
                sounds.push(0x0C);
                return Action::ChangeData;
            }
            2 if confirm => {
                sounds.push(0x0C);
                return Action::OpenController;
            }
            // engine addition: GRAPHICS after SOUND OPTIONS, the game's items 10..14 one down
            10 if confirm => {
                sounds.push(0x0C);
                return Action::OpenGraphics;
            }
            11 if confirm => {
                sounds.push(0x0C);
                return Action::OpenMovies;
            }
            12 if confirm => {
                sounds.push(0x0C);
                return Action::OpenPlayData;
            }
            3 => {
                Self::clamp(&mut s.difficulty, 7, rep, sounds);
            }
            4 => {
                Self::clamp(&mut s.attack_level, 7, rep, sounds);
            }
            5 => {
                Self::wrap(&mut s.time_limit, 4, rep, sounds);
            }
            6 => {
                Self::clamp(&mut s.match_point, 3, rep, sounds);
            }
            7 => {
                Self::wrap(&mut s.char_select, 1, rep, sounds);
            }
            8 => {
                Self::wrap(&mut s.display, 3, rep, sounds);
            }
            9 if confirm => {
                sounds.push(0x0C);
                return Action::OpenSound;
            }
            13 => {
                Self::wrap(&mut s.vibration, 1, rep, sounds);
            }
            14 if confirm => {
                sounds.push(0x0C);
                s.options_default();
            }
            15 if confirm => {
                sounds.push(0x0C);
                return Action::Leave;
            }
            _ => {}
        }
        Action::None
    }

    /// GRAPHICS (engine addition): ←/→ change a value (wrapping, SE 0x0B as the OPTIONS lists), applied at once;
    /// DEFAULT and EXIT confirm with SE 0x0C.
    pub(super) fn graphics_action(&mut self, rep: u16, confirm: bool, sounds: &mut Vec<u16>) -> Action {
        let old = self.g;
        let g = &mut self.g;
        let toggle = |v: &mut bool, sounds: &mut Vec<u16>| {
            let mut b = *v as u8;
            if Self::wrap(&mut b, 1, rep, sounds) {
                *v = b != 0;
            }
        };
        match self.cursor {
            1 => {
                let mut v = (g.aspect == Aspect::Four3) as u8;
                if Self::wrap(&mut v, 1, rep, sounds) {
                    g.aspect = if v != 0 { Aspect::Four3 } else { Aspect::Wide };
                }
            }
            2 => toggle(&mut g.fullscreen, sounds),
            3 => {
                Self::wrap(&mut g.window, WINDOW_SIZES.len() as u8 - 1, rep, sounds);
            }
            4 => toggle(&mut g.vsync, sounds),
            5 => toggle(&mut g.frame_cap, sounds),
            6 => {
                Self::wrap(&mut g.render_scale, RENDER_SCALES.len() as u8 - 1, rep, sounds);
            }
            7 => {
                Self::wrap(&mut g.hud_size, HUD_SCALES.len() as u8 - 1, rep, sounds);
            }
            8 => toggle(&mut g.perf, sounds),
            9 if confirm => {
                sounds.push(0x0C);
                // Android: the rows it hides keep their values (engine addition)
                let d = Prefs::default();
                *g = if self.android { Prefs { fullscreen: g.fullscreen, window: g.window, vsync: g.vsync, frame_cap: g.frame_cap, ..d } } else { d };
            }
            10 if confirm => {
                sounds.push(0x0C);
                return Action::Leave;
            }
            _ => {}
        }
        if self.g != old { Action::ApplyGraphics } else { Action::None }
    }

    pub(super) fn sound_action(&mut self, rep: u16, confirm: bool, sounds: &mut Vec<u16>) -> Action {
        let s = &mut self.s;
        let tracks = if s.bgm_select == 1 { 18 } else { 23 };
        match self.cursor {
            1 => {
                if Self::wrap(&mut s.output, 1, rep, sounds) {
                    return Action::ApplySound;
                }
            }
            2 => {
                if Self::wrap(&mut s.bgm_select, 1, rep, sounds) {
                    self.bgm_no = self.bgm_no.min(if s.bgm_select == 1 { 18 } else { 23 });
                }
            }
            3 => {
                if Self::clamp(&mut s.master, 100, rep, sounds) {
                    return Action::ApplySound;
                }
            }
            4 => {
                if Self::clamp(&mut s.bgm_volume, 100, rep, sounds) {
                    return Action::ApplySound;
                }
            }
            5 => {
                Self::wrap(&mut self.bgm_no, tracks, rep, sounds);
                if confirm {
                    self.bgm_playing = !self.bgm_playing;
                    // opt.D = 1: track n, opt.D = 0: track n + 0x13
                    let t = if s.bgm_select == 1 { self.bgm_no } else { self.bgm_no + 0x13 };
                    return Action::PlayTrack(self.bgm_playing.then_some(t));
                }
            }
            6 => {
                let mut b = self.se_bank as u8;
                if Self::wrap(&mut b, SE_BANKS as u8 - 1, rep, sounds) {
                    self.se_bank = b as usize;
                    self.se_no = 0;
                }
            }
            7 => {
                Self::wrap(&mut self.se_no, 63, rep, sounds);
                if confirm {
                    return Action::PlaySe(self.se_bank, self.se_no as usize);
                }
            }
            8 if confirm => {
                // SE 0x0C, 0x203 (the BGM PLAYER stops), the defaults, BGM / SE TYPE / SE NO. back to 0
                sounds.push(0x0C);
                s.sound_default();
                (self.bgm_no, self.se_bank, self.se_no, self.bgm_playing) = (0, 0, 0, false);
                return Action::SoundDefault;
            }
            9 if confirm => {
                sounds.push(0x0C);
                return Action::Leave;
            }
            _ => {}
        }
        Action::None
    }

    pub(super) fn custom_action(&mut self, rep: u16, port_press: [u16; 2], confirm: bool, sounds: &mut Vec<u16>) -> Action {
        let c = &mut self.s.custom;
        match self.cursor {
            1 => {
                Self::wrap(&mut c.game, 3, rep, sounds);
                for (p, pp) in port_press.iter().enumerate() {
                    if pp & CONFIRM != 0 {
                        sounds.push(0x0C);
                        return Action::GameStart(p);
                    }
                }
            }
            2 => {
                Self::wrap(&mut c.model, 3, rep, sounds);
            }
            3 => {
                Self::wrap(&mut c.blood, 1, rep, sounds);
            }
            4 => Self::flag(&mut c.flags, 13, false, rep, sounds),
            5 => {
                Self::clamp(&mut c.charge, 8, rep, sounds);
            }
            6 => {
                Self::clamp(&mut c.recover, 8, rep, sounds);
            }
            7 => Self::flag(&mut c.flags, 6, true, rep, sounds),
            8 => Self::flag(&mut c.flags, 11, false, rep, sounds),
            9 => Self::flag(&mut c.flags, 10, false, rep, sounds),
            10 if confirm => {
                sounds.push(0x0C);
                *c = Custom { game: 0, ..Custom::default() };
            }
            11 if confirm => {
                sounds.push(0x0C);
                return Action::Leave;
            }
            _ => {}
        }
        Action::None
    }
}
