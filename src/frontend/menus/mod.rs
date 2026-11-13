//! The list menus of the title: OPTIONS, SOUND OPTIONS, CUSTOM, CONTROLLER CONFIG., GRAPHICS.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The list menus of the title: the menu screen frame, OPTIONS,
//! SOUND OPTIONS and CUSTOM, run by the generic menu engine.
//! Values are edited in a scratch copy and committed when the menu is left (△ or EXIT), as in the game.
//! GRAPHICS (OPTIONS item 10) is an engine addition: it edits the engine's `Prefs`, applied and saved at once. CHANGE GAME
//! DATA (OPTIONS item 11 from simulation v9, `Menu::data_row`) is one too: the app's setup screen over the menu.
//!
//! `actions.rs`: what each menu's rows do; `controller.rs`: CONTROLLER CONFIG.; `draw.rs`: the frame, the banner,
//! the rows; `flow.rs`: the front end running them (`impl Front`).

mod actions;
mod controller;
mod draw;
mod flow;

use crate::fight::input::pad::{DOWN, LEFT, RIGHT, UP};
use crate::frontend::options::{Prefs, Settings};
use serde::{Deserialize, Serialize};

pub use draw::{banner, frame, frame_with};

const CONFIRM: u16 = 0xE0; // □ × ○ (START does not confirm in the sub-menus)
const BACK: u16 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Options,
    Sound,
    Custom,
    /// CONTROLLER CONFIG.
    Controller,
    /// MOVIES AND PICTURES (items)
    Movies,
    /// PLAY DATA (items)
    PlayData,
    /// MEMORY CARD (items)
    MemCard,
    /// GRAPHICS (engine addition): the engine's display preferences
    Graphics,
}

impl Kind {
    /// A fixed number per menu (a replay's sync hash, `Front::sync_key`: a new menu takes a new number).
    pub fn sync_id(self) -> u32 {
        match self {
            Kind::Options => 1,
            Kind::Sound => 2,
            Kind::Custom => 3,
            Kind::Controller => 4,
            Kind::Movies => 5,
            Kind::PlayData => 6,
            Kind::MemCard => 7,
            Kind::Graphics => 8,
        }
    }
}

/// What the front end must do after a menu frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    None,
    /// leave with the scratch committed
    Leave,
    /// OPTIONS → SOUND OPTIONS
    OpenSound,
    /// OPTIONS → CONTROLLER CONFIG.
    OpenController,
    /// CUSTOM → GAME START by this pad
    GameStart(usize),
    /// SOUND OPTIONS changed output / volumes: apply now
    ApplySound,
    /// SOUND OPTIONS → DEFAULT: the music stopped (0x203), the defaults applied
    SoundDefault,
    /// BGM PLAYER: play this XA track (None = stop)
    PlayTrack(Option<u8>),
    /// SE PLAYER: play tone `n` of front-end bank `b`
    PlaySe(usize, usize),
    /// OPTIONS → MEMORY CARD
    OpenMemCard,
    /// MEMORY CARD: LOAD / SAVE confirmed or AUTO SAVE switched ON: the dialog (state 0x10) for this item
    Card(usize),
    /// OPTIONS → MOVIES AND PICTURES
    OpenMovies,
    /// OPTIONS → PLAY DATA
    OpenPlayData,
    /// MOVIES: play movie n (skippable)
    PlayMovie(usize),
    /// PICTURES: show picture n of /STY/PIC
    ShowPicture(usize),
    /// OPTIONS → GRAPHICS (engine addition)
    OpenGraphics,
    /// OPTIONS → CHANGE GAME DATA (engine addition, v9): the app's setup screen (the menu stays up under it)
    ChangeData,
    /// GRAPHICS changed a preference: apply and save it now (engine addition)
    ApplyGraphics,
}

/// The MOVIES entries' files (file table 8 + n; their names: the text list `movies`).
pub const MOVIE_FILES: [&str; 4] = ["USAOP", "BR", "ED1", "ED2"];
/// PICTURES: the percentage is of 95 pictures.
const PICTURE_TOTAL: usize = 95;

/// Sound effect banks of the SE TYPE item (the front end's banks; their names: the text list `se_banks`).
pub const SE_BANKS: usize = 4;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Menu {
    pub kind: Kind,
    /// 1-based, as the menu descriptor
    pub cursor: usize,
    /// the values being edited
    pub s: Settings,
    banner_x: i32,
    bgm_no: u8,
    bgm_playing: bool,
    pub se_bank: usize,
    se_no: u8,
    /// CONTROLLER CONFIG.: the button row of each player
    pub(crate) cc_row: [usize; 2],
    /// MOVIES AND PICTURES: the movie (+0x14) and the picture among the unlocked ones (+0x16)
    movie: usize,
    pub picture: usize,
    /// PLAY DATA: DISPLAY (+0x14) 0..4
    display: u8,
    /// GRAPHICS: the engine's preferences being edited (engine addition)
    pub g: Prefs,
    /// GRAPHICS: the browser's rows (`session::SessionStart::browser`, set by the front end that opens it; the same
    /// on every build otherwise, as it is state; a file without it takes the running build's, as it was recorded)
    #[serde(default = "crate::session::browser_build")]
    pub browser: bool,
    /// GRAPHICS: the Android build's rows (`session::SessionStart::android`, set by the front end that opens it;
    /// state, as `browser`)
    #[serde(default)]
    pub android: bool,
    /// OPTIONS: the CHANGE GAME DATA row after GRAPHICS (engine addition, simulation v9; set by the front end that opens
    /// it, `Front::options_menu`), the game's MOVIES AND PICTURES .. EXIT one down
    #[serde(default)]
    pub data_row: bool,
}

/// OPTIONS with `data_row`: the row of CHANGE GAME DATA, and its item number past the 15 of the menu without it.
const DATA_ROW: usize = 11;
const CHANGE_DATA: usize = 16;

/// CONTROLLER CONFIG.: the assignable actions (the table `menu.cc_values`; their names: the text list `cc_names`, the
/// button icons `cc_icons`).
pub(crate) fn cc_values() -> &'static [u16; 10] {
    &crate::assets::game_tables::get().cc_values
}

/// Palettes of the menu font (CLUT (352, 480 + pal)).
const HL: u16 = 0;
const NORMAL: u16 = 1;

impl Menu {
    pub fn new(kind: Kind, s: Settings) -> Self {
        // OPTIONS and SOUND OPTIONS open on EXIT, CUSTOM on GAME START
        let cursor = match kind {
            Kind::Options => 15,
            Kind::Sound => 9,
            Kind::Custom | Kind::Controller => 1,
            Kind::Movies => 3,
            Kind::PlayData => 2,
            Kind::MemCard => 4,
            Kind::Graphics => 10,
        };
        Self {
            kind,
            cursor,
            s,
            banner_x: 0,
            bgm_no: 0,
            bgm_playing: false,
            se_bank: 0,
            se_no: 0,
            cc_row: [0; 2],
            movie: 0,
            picture: 0,
            display: 0,
            g: Prefs::default(),
            browser: false,
            android: false,
            data_row: false,
        }
    }

    /// OPTIONS, with the CHANGE GAME DATA row when `data_row` (v9), the cursor on EXIT.
    pub fn options(s: Settings, data_row: bool) -> Self {
        let mut m = Self { data_row, ..Self::new(Kind::Options, s) };
        m.cursor = m.count();
        m
    }

    /// OPTIONS: row `i`'s item in the numbering without CHANGE GAME DATA (1..=15), or `CHANGE_DATA`.
    fn options_item(&self, i: usize) -> usize {
        match i {
            _ if !self.data_row || i < DATA_ROW => i,
            DATA_ROW => CHANGE_DATA,
            _ => i - 1,
        }
    }

    /// OPTIONS: the row of an item of the numbering without CHANGE GAME DATA (back from its sub-menus).
    pub(crate) fn options_row(&self, item: usize) -> usize {
        if self.data_row && item >= DATA_ROW { item + 1 } else { item }
    }

    /// A decoded state's menu: its cursor, sound bank, movie, controller rows and PLAY DATA page in range.
    pub fn check(&self) -> Result<(), String> {
        let ok = (1..=self.count()).contains(&self.cursor)
            && self.se_bank < SE_BANKS
            && self.movie < MOVIE_FILES.len()
            && self.cc_row.iter().all(|&r| r < 8)
            && self.display <= 4
            && self.picture < 96;
        if ok { Ok(()) } else { Err(format!("menu {:?} cursor {}", self.kind, self.cursor)) }
    }

    fn count(&self) -> usize {
        match self.kind {
            // the game's 14 and the engine's GRAPHICS (and CHANGE GAME DATA)
            Kind::Options => 15 + self.data_row as usize,
            Kind::Sound => 9,
            Kind::Custom => 11,
            Kind::Controller => 8,
            Kind::Movies => 3,
            Kind::PlayData => 2,
            Kind::MemCard => 4,
            Kind::Graphics => 10,
        }
    }

    /// `menu_run_mask`: CUSTOM items hidden until unlocked (opt.44); bit i = item i.
    fn mask(&self) -> u32 {
        match self.kind {
            Kind::Custom => {
                let u = self.s.custom_unlock;
                0xC03
                    | (u & 1) << 2
                    | (u >> 13 & 8)
                    | (u >> 9 & 0x10)
                    | (u >> 9 & 0x20)
                    | (u >> 9 & 0x40)
                    | (u << 1 & 0x80)
                    | (u >> 3 & 0x100)
                    | (u >> 1 & 0x200)
            }
            // GRAPHICS: WINDOW SIZE in a window only; WINDOW SIZE, VSYNC and FRAME CAP not in the browser (it sizes
            // and paces the canvas itself), nor WINDOW on Android (always full screen)
            Kind::Graphics if self.android => !0b11_1100,
            Kind::Graphics if self.browser => !0b11_1000,
            Kind::Graphics if self.g.fullscreen => !0b1000,
            _ => !0,
        }
    }

    fn enabled(&self, i: usize) -> bool {
        self.mask() >> i & 1 != 0
    }

    /// One frame: `press` = new presses of both ports, `rep` = auto-repeat of both, `port_press` per port.
    /// Returns the sounds to play (`play(0x400, id)`) and the action.
    pub fn tick(&mut self, press: u16, rep: u16, port_press: [u16; 2], port_rep: [u16; 2], held: [u16; 2], sounds: &mut Vec<u16>) -> Action {
        self.scroll(4);
        if self.kind == Kind::Controller {
            return self.controller(press, port_press, port_rep, held, sounds);
        }
        if press & BACK != 0 {
            sounds.push(0x0D);
            if self.bgm_playing {
                self.bgm_playing = false;
            }
            return Action::Leave;
        }
        // move_cursor: vertical, wraps, skips hidden items, sfx 0x0A
        let n = self.count();
        let step = if rep & DOWN != 0 {
            1
        } else if rep & UP != 0 {
            n - 1
        } else {
            0
        };
        if step != 0 {
            let mut c = self.cursor;
            for _ in 0..n {
                c = (c - 1 + step) % n + 1;
                if self.enabled(c) {
                    break;
                }
            }
            self.cursor = c;
            sounds.push(0x0A);
        }
        let confirm = press & CONFIRM != 0;
        match self.kind {
            Kind::Options => self.options_action(rep, confirm, sounds),
            Kind::Sound => self.sound_action(rep, confirm, sounds),
            Kind::Custom => self.custom_action(rep, port_press, confirm, sounds),
            Kind::Movies => self.movies_action(rep, confirm, sounds),
            Kind::PlayData => match self.cursor {
                1 => {
                    Self::wrap(&mut self.display, 4, rep, sounds);
                    Action::None
                }
                _ if confirm => {
                    sounds.push(0x0C);
                    Action::Leave
                }
                _ => Action::None,
            },
            Kind::MemCard => self.memcard_action(rep, confirm, sounds),
            Kind::Graphics => self.graphics_action(rep, confirm, sounds),
            Kind::Controller => Action::None,
        }
    }

    /// The banner scroll ((0, speed, speed, c)), wrapping at its repeat width.
    pub fn scroll(&mut self, speed: i32) {
        self.banner_x -= speed;
        let bw = self.banner().1;
        while self.banner_x < -bw {
            self.banner_x += bw;
        }
    }

    // value helpers (wrap on Right/Left only, clamp)
    fn wrap(v: &mut u8, max: u8, rep: u16, sounds: &mut Vec<u16>) -> bool {
        if rep == RIGHT {
            *v = if *v >= max { 0 } else { *v + 1 };
        } else if rep == LEFT {
            *v = if *v == 0 { max } else { *v - 1 };
        } else {
            return false;
        }
        sounds.push(0x0B);
        true
    }

    fn clamp(v: &mut u8, max: u8, rep: u16, sounds: &mut Vec<u16>) -> bool {
        let old = *v;
        if rep & RIGHT != 0 && *v < max {
            *v += 1;
        } else if rep & LEFT != 0 && *v > 0 {
            *v -= 1;
        }
        if *v != old {
            sounds.push(0x0B);
        }
        *v != old
    }

    fn flag(flags: &mut u32, bit: u32, inverted: bool, rep: u16, sounds: &mut Vec<u16>) {
        let mut v = ((*flags >> bit & 1 != 0) != inverted) as u8;
        if Self::wrap(&mut v, 1, rep, sounds) {
            *flags = (*flags & !(1 << bit)) | (((v != 0) != inverted) as u32) << bit;
        }
    }

    /// Unlocked pictures (the bits of opt.54..5C).
    pub fn picture_count(&self) -> usize {
        self.s.pictures.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// The picture number of the `k`-th unlocked picture.
    pub fn nth_picture(&self, k: usize) -> Option<usize> {
        (0..96).filter(|&n| self.s.pictures[n / 32] >> (n % 32) & 1 != 0).nth(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::text::Label::{Game, Port};
    use crate::frontend::options::Aspect;

    #[test]
    fn options_gain_graphics_before_movies() {
        let mut m = Menu::new(Kind::Options, Settings::default());
        assert_eq!((m.cursor, m.label(10), m.label(15)), (15, Port("GRAPHICS"), Game("exit")));
        let mut sounds = Vec::new();
        m.cursor = 10;
        assert_eq!(m.tick(0x40, 0, [0x40, 0], [0; 2], [0; 2], &mut sounds), Action::OpenGraphics);
        assert_eq!(sounds, [0x0C]);
    }

    #[test]
    fn options_change_data_row() {
        let mut m = Menu::options(Settings::default(), true);
        assert_eq!(
            (m.cursor, m.label(10), m.label(11), m.label(12), m.label(16)),
            (16, Port("GRAPHICS"), Port("CHANGE GAME DATA"), Game("screen_title.7"), Game("exit"))
        );
        let mut sounds = Vec::new();
        for (row, act) in [(11, Action::ChangeData), (12, Action::OpenMovies), (13, Action::OpenPlayData), (16, Action::Leave)] {
            m.cursor = row;
            assert_eq!(m.tick(0x40, 0, [0x40, 0], [0; 2], [0; 2], &mut sounds), act, "row {row}");
        }
        // VIBRATION one down
        m.cursor = 14;
        m.tick(0, RIGHT, [0; 2], [RIGHT, 0], [0; 2], &mut sounds);
        assert_eq!(m.s.vibration, 1 - Settings::default().vibration);
        assert_eq!((m.options_row(11), m.options_row(10)), (12, 10));
        // without the row: the menu of simulation v8
        let m = Menu::options(Settings::default(), false);
        assert_eq!((m.cursor, m.label(11), m.options_row(11)), (15, Game("screen_title.7"), 11));
    }

    #[test]
    fn graphics_applies_each_change() {
        let mut m = Menu::new(Kind::Graphics, Settings::default());
        let mut sounds = Vec::new();
        assert_eq!(m.cursor, 10);
        m.cursor = 1;
        assert_eq!(m.tick(0, RIGHT, [0; 2], [RIGHT, 0], [0; 2], &mut sounds), Action::ApplyGraphics);
        assert_eq!(m.g.aspect, Aspect::Four3);
        m.cursor = 6;
        assert_eq!(m.tick(0, LEFT, [0; 2], [LEFT, 0], [0; 2], &mut sounds), Action::ApplyGraphics);
        assert_eq!(m.g.render_scale(), 0.5);
        // fullscreen hides WINDOW SIZE: DOWN from WINDOW skips it (desktop menus); the browser's skip VSYNC and FRAME
        // CAP too (the session's, whatever the build)
        m.g.fullscreen = true;
        for (browser, to) in [(false, 4), (true, 6)] {
            (m.browser, m.cursor) = (browser, 2);
            m.tick(0, DOWN, [0; 2], [DOWN, 0], [0; 2], &mut sounds);
            assert_eq!(m.cursor, to, "browser {browser}");
        }
        m.cursor = 9;
        assert_eq!(m.tick(0x40, 0, [0x40, 0], [0; 2], [0; 2], &mut sounds), Action::ApplyGraphics);
        assert_eq!(m.g, Prefs::default());
        assert_eq!(sounds, [0x0B, 0x0B, 0x0A, 0x0A, 0x0C]);
    }

    #[test]
    fn android_graphics_hides_the_window_rows() {
        let mut m = Menu::new(Kind::Graphics, Settings::default());
        m.android = true;
        let mut sounds = Vec::new();
        // SCREEN MODE ↔ RENDER QUALITY: WINDOW, WINDOW SIZE, VSYNC and FRAME CAP skipped both ways
        m.cursor = 1;
        m.tick(0, DOWN, [0; 2], [DOWN, 0], [0; 2], &mut sounds);
        assert_eq!(m.cursor, 6);
        m.tick(0, UP, [0; 2], [UP, 0], [0; 2], &mut sounds);
        assert_eq!(m.cursor, 1);
        // DEFAULT keeps the hidden rows' saved values (the file's), the others back to the defaults
        m.g = Prefs { fullscreen: true, window: 3, vsync: false, frame_cap: true, render_scale: 0, hud_size: 2, perf: true, ..Prefs::default() };
        m.cursor = 9;
        assert_eq!(m.tick(0x40, 0, [0x40, 0], [0; 2], [0; 2], &mut sounds), Action::ApplyGraphics);
        assert_eq!(m.g, Prefs { fullscreen: true, window: 3, vsync: false, frame_cap: true, ..Prefs::default() });
    }
}
