//! STORY mode and the endings' still screens.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! - prologue: pictures 89..91 of /STY/PIC, fading in (120 / 60 frames) and out, advanced with □×○;
//! - the page player: the character's text pages (/STY/<CHR>/<CHR>nn.DAT, 8 TIMs of 256×256 4-bit, each
//!   8 strips of two 16-line rows) shown as subtitles over the story pictures. The rightmost pixel of the strip's
//!   rows carries the control codes: row 0 the speaker's face, rows 1/2 the picture (tens / units), row 3 its
//!   IN transition, row 4 the page's OUT transition (on □×○), row 31 the end code (0xE end of section, 0xF next
//!   TIM, else next strip);
//! - the per-character script (25 bytes): the intro page, then per fight the win / lose / next pages;
//!   the fights (the story tables: count, opponents, stages);
//! - CONTINUE? YES / NO after the lose pages;
//! - the arcade epilogue: <CHR>CON.DAT (four 256×240 quarters) for 360 frames with 60-frame fades.

use crate::assets::game_tables;
use crate::fight::input::pad::{LEFT, RIGHT, START};
use crate::frontend::{Resident, Tim};
use crate::output::font::Fonts;
use crate::output::layout::{Anchor, Clip};
use crate::output::prim::{self, Prim, clut, ft4};
use crate::session::Cached;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const DIRS: [&str; 11] = ["YUG", "ALI", "LON", "ULI", "MAR", "BUS", "BAK", "JEN", "STU", "GAD", "RON"];

const CONFIRM: u16 = 0xE0;

/// The story run: character, fight index (M.A4) and continues used (F.EC).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Run {
    pub chr: usize,
    pub fight: i16,
    pub continues: u16,
}

impl Run {
    pub fn new(chr: usize) -> Self {
        Self { chr: chr % 11, fight: 0, continues: 0 }
    }
    /// A decoded state's run: the character and the fight index in the tables' range.
    pub fn valid(&self) -> bool {
        self.chr < DIRS.len() && (-2..=game_tables::get().story_count[self.chr] as i16).contains(&self.fight)
    }

    pub fn fights(&self) -> i16 {
        game_tables::get().story_count[self.chr] as i16
    }
    /// Opponent and stage of the current fight.
    pub fn match_up(&self) -> (u8, u8) {
        let k = (self.fight.max(0) as usize).min(7);
        (game_tables::get().story_opponents[self.chr][k], game_tables::get().story_stages[self.chr][k])
    }
}

/// What the front end must do after a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    None,
    /// load this asset file and pass it to `fetched`
    Fetch(String),
    /// fight the current match-up
    Fight,
    /// all fights won: the ending (fight = -2 without continues, else -1)
    Ending,
    /// CONTINUE → NO
    GameOver,
    /// the prologue / epilogue finished
    Done,
}

/// Picture transition lengths by code (IN / OUT): 1 / 2 fade through black, 3 slides,
/// 4 / 5 fade through white; 2 and 5 take 60 frames, the others 16.
fn fx_len(code: u8) -> i32 {
    match code {
        2 | 5 => 60,
        1 | 3 | 4 => 16,
        _ => 0,
    }
}

/// A full-screen subtractive black (fade) of `v` (0..255).
fn fade(out: &mut Vec<Prim>, v: i32) {
    if v > 0 {
        let v = v.clamp(0, 255) as u8;
        let mut p = prim::f4(0, 0, 512, 480, [v; 3]);
        p.semi = true;
        p.tpage = 0x40;
        out.push(p.at(Anchor::AMBIENT)); // in the ambient fill's copy (engine addition)
    }
}

/// A full-screen additive white (flash) of `v` (0..255).
fn flash(out: &mut Vec<Prim>, v: i32) {
    if v > 0 {
        let mut p = prim::f4(0, 0, 512, 480, [v.clamp(0, 255) as u8; 3]);
        p.semi = true;
        p.tpage = 0x20;
        out.push(p.at(Anchor::AMBIENT)); // in the ambient fill's copy (engine addition)
    }
}

/// The full-screen story picture (loaded at (512, 0), CLUT (0, 511)) at `x`, clipped to the
/// 512-wide display as the GPU's drawing area does (the core scissor); the window around the display gets the
/// ambient fill (engine addition).
fn picture(out: &mut Vec<Prim>, x: i32) {
    if x > -512 && x < 512 {
        out.push(ft4(x, 0, 512, 480, (0, 0, 256, 240), 0x88, clut(0, 511)).at(Anchor { clip: Clip::Core, ..Anchor::AMBIENT }));
    }
}

/// Loads picture `n` (0-based, /STY/PIC/PICnn.DAT entry n % 4) into VRAM.
pub fn load_picture(vram: &mut Resident, dat: &[u8], n: usize) -> bool {
    let c = crate::formats::container::read(dat);
    match c.get(n % 4).cloned().flatten() {
        Some(t) => {
            vram.load(Tim::File(picture_file(n), (n % 4) as u8), &t, Some((0, 511)), Some((512, 0)));
            true
        }
        None => false,
    }
}

pub fn picture_file(n: usize) -> String {
    format!("engine/sty/PIC{:02}.DAT", n / 4)
}

fn unlock(pictures: &mut [u32; 3], n: usize) {
    if n < 96 {
        pictures[n / 32] |= 1 << (n % 32);
    }
}

// ---------------------------------------------------------------- prologue

/// The three prologue pictures.
#[derive(Clone, Serialize, Deserialize)]
pub struct Prologue {
    /// picture number (1-based 89..92)
    n: u16,
    loaded: Option<u16>,
    waiting: bool,
    fade_in: i32,
    fade_out: i32,
    frame: u32,
    done: bool,
}

impl Default for Prologue {
    fn default() -> Self {
        Self::new()
    }
}

impl Prologue {
    pub fn new() -> Self {
        Self { n: 89, loaded: None, waiting: false, fade_in: 0, fade_out: 0, frame: 0, done: false }
    }

    pub fn tick(&mut self, press: u16, pictures: &mut [u32; 3], sounds: &mut Vec<u16>) -> Out {
        self.frame += 1;
        if self.done {
            return Out::Done;
        }
        if press & START != 0 {
            self.done = true;
            return Out::Done;
        }
        if self.fade_out > 0 {
            self.fade_out -= 1;
            if self.fade_out == 0 && self.n >= 92 {
                self.done = true;
                return Out::Done;
            }
            return Out::None;
        }
        if self.loaded != Some(self.n) {
            if !self.waiting {
                self.waiting = true;
                return Out::Fetch(picture_file(self.n as usize - 1));
            }
            return Out::None;
        }
        if self.fade_in > 0 {
            self.fade_in -= 1;
            return Out::None;
        }
        let _ = pictures;
        if press & CONFIRM != 0 {
            sounds.push(0x0C);
            self.n += 1;
            self.fade_out = if self.n == 92 { 120 } else { 60 };
        }
        Out::None
    }

    pub fn fetched(&mut self, vram: &mut Resident, dat: &[u8], pictures: &mut [u32; 3]) {
        let n = self.n as usize - 1;
        if load_picture(vram, dat, n) {
            unlock(pictures, n);
        }
        self.loaded = Some(self.n);
        self.waiting = false;
        self.fade_in = if self.n == 89 { 120 } else { 60 };
    }

    pub fn draw(&self, f: &Fonts, out: &mut Vec<Prim>) {
        out.push(prim::f4(0, 0, 512, 480, [0, 0, 0]));
        if self.loaded.is_none() {
            return;
        }
        picture(out, 0);
        if self.fade_in > 0 {
            let len = if self.n == 89 { 120 } else { 60 };
            fade(out, self.fade_in * 255 / len);
        } else if self.fade_out > 0 {
            let len = if self.n == 92 { 120 } else { 60 };
            fade(out, 255 - self.fade_out * 255 / len);
        } else if self.frame & 0x30 != 0 {
            f.menu_text(out, "q", 488, 432, 0);
        }
    }
}

// ---------------------------------------------------------------- pages

/// One section of text pages; `mode` 0 = before a fight (or the ending), 1 = after a fight,
/// 2 = CONTINUE? YES / NO.
#[derive(Clone, Serialize, Deserialize)]
pub struct Pages {
    pub run: Run,
    mode: u8,
    /// strip number (TIM index × 8 + strip)
    page: u16,
    /// loaded STY file (index in the character's files)
    file: Option<usize>,
    /// its TIMs: static (`Pages::rebind` reads them again from the fetched file)
    #[serde(skip)]
    tims: Arc<Vec<Option<Vec<u8>>>>,
    /// the strip's TIM is in VRAM (768, 256) (M.8E < 0; cleared on every page advance)
    text_ready: bool,
    waiting: bool,
    /// story picture (sp+0x28, 1-based, 0 none) and the one being loaded (M.8C >= 0)
    pic: u16,
    pic_wait: Option<u16>,
    /// sp+0x40: > 0 the picture is visible, 0 black, -1 opaque white
    shown: i16,
    /// the running IN (sp+0x50) / OUT (sp+0x54) transition and its counter ($s7)
    fx_in: u8,
    fx_out: u8,
    s: i32,
    /// this frame's drawing: `shown` before the step, picture x, fill grey (additive?), subtitle layer
    vis: i16,
    pic_x: i32,
    fill: Option<(i32, bool)>,
    /// the subtitle layer is drawn, for strip `sub_page` (the page before this frame's advance)
    sub: bool,
    sub_page: u16,
    face: Option<u8>,
    /// won the last fight (0x48); CONTINUE choice NO (0x4C)
    won: bool,
    no: bool,
    frame: u32,
}

impl Pages {
    /// Section before the first fight.
    pub fn start(run: Run) -> Self {
        Self::with(run, 0, game_tables::get().story_script[run.chr][0] as u16 * 8)
    }

    /// Section after a fight (`won` by the player): the win pages (and the fight index advances) or the lose pages.
    pub fn after(mut run: Run, won: bool) -> Self {
        let k = run.fight.max(0) as usize;
        let page = if won {
            run.fight += 1;
            game_tables::get().story_script[run.chr][(1 + 3 * k).min(24)]
        } else {
            game_tables::get().story_script[run.chr][(2 + 3 * k).min(24)]
        };
        let mut p = Self::with(run, 1, page as u16 * 8);
        p.won = won;
        p
    }

    fn with(run: Run, mode: u8, page: u16) -> Self {
        Self {
            run,
            mode,
            page,
            file: None,
            tims: Default::default(),
            text_ready: false,
            waiting: false,
            pic: 0,
            pic_wait: None,
            shown: 0,
            fx_in: 0,
            fx_out: 0,
            s: 0,
            vis: 0,
            pic_x: 0,
            fill: None,
            sub: false,
            sub_page: 0,
            face: None,
            won: false,
            no: false,
            frame: 0,
        }
    }

    /// After deserialising: the loaded STY file's TIMs from the fetched files.
    pub fn rebind(&mut self, files: &crate::session::FileCache) -> Result<(), String> {
        if let Some(k) = self.file {
            let name = self.file_name(k);
            let b = files.get(&name).ok_or_else(|| format!("story file {name} not in the store"))?;
            self.tims = Arc::new(crate::formats::container::read(b));
        }
        Ok(())
    }

    /// The loaded STY file's name (kept in the app's store for `rebind`).
    pub fn file_used(&self) -> Option<String> {
        self.file.map(|k| self.file_name(k))
    }

    fn file_name(&self, k: usize) -> String {
        let d = DIRS[self.run.chr];
        format!("engine/sty/{d}/{d}{k:02}.DAT")
    }

    /// Control code `row` of the current strip (high nibble of the strip's rightmost pixel in that row).
    fn code(&self, row: usize) -> u8 {
        if self.file.is_none() {
            return 0;
        }
        let idx = (self.page >> 3) as usize;
        let Some(Some(t)) = self.tims.get(idx & 7) else { return 0 };
        let o = 20 + ((self.page & 7) as usize) * 4096 + row * 128 + 127;
        t.get(o).map(|b| b >> 4).unwrap_or(0)
    }

    /// Page, picture and transition state (debug description).
    pub fn state(&self) -> String {
        let fill = self.fill.map(|(v, add)| format!(" {}{v}", if add { '+' } else { '-' })).unwrap_or_default();
        format!(
            "mode {} page {} pic {} shown {} in {} out {} s {} x {}{fill} text {}",
            self.mode, self.page, self.pic, self.vis, self.fx_in, self.fx_out, self.s, self.pic_x, self.sub
        )
    }

    /// The picture shows, with the strip's IN transition (row 3).
    fn arm_in(&mut self) {
        self.shown = self.pic as i16;
        let c = self.code(3);
        self.fx_in = if (1..=5).contains(&c) { c } else { 0 };
        self.s = fx_len(self.fx_in);
    }

    /// The text / picture loader (only while no OUT runs). `cache` holds the files already fetched;
    /// any other is returned as `Out::Fetch`.
    fn load(&mut self, vram: &mut Resident, pictures: &mut [u32; 3], cache: &Cached) -> Out {
        if self.waiting {
            return Out::None;
        }
        if !self.text_ready {
            // the page's TIM (file = index / 8 of the character's files, TIM = index % 8)
            let idx = (self.page >> 3) as usize;
            let want = idx >> 3;
            if self.file != Some(want) {
                let name = self.file_name(want);
                match cache.get(&name) {
                    Some(b) => self.fetched(&name, b, vram),
                    None => {
                        self.waiting = true;
                        return Out::Fetch(name);
                    }
                }
            }
            if let (Some(f), Some(Some(t))) = (self.file, self.tims.get(idx & 7)) {
                vram.load(Tim::File(self.file_name(f), (idx & 7) as u8), t, None, Some((768, 256)));
            }
            self.text_ready = true;
            let face = self.code(0);
            self.face = (1..=11).contains(&face).then(|| face - 1);
            // the strip's picture: the same one re-arms its IN at once, a new one loads (and unlocks), else black
            let p = (self.code(2) + 10 * self.code(1)) as u16;
            if p == self.pic {
                self.arm_in();
            } else if (1..=95).contains(&p) {
                self.pic = p;
                self.pic_wait = Some(p);
                unlock(pictures, p as usize - 1);
            } else {
                self.pic = 0;
                self.shown = 0;
            }
        }
        if let Some(p) = self.pic_wait {
            let name = picture_file(p as usize - 1);
            match cache.get(&name) {
                Some(b) => self.fetched(&name, b, vram),
                None => {
                    self.waiting = true;
                    return Out::Fetch(name);
                }
            }
        }
        Out::None
    }

    /// One frame of the transition, OUT before IN (`len` + 1 frames, s = len … 0).
    fn step(&mut self) {
        self.vis = self.shown;
        self.pic_x = 0;
        self.fill = None;
        if self.shown <= 0 {
            // the switch does not run: clear a code that would never end (JEN p271, RON p290: no picture, OUT 2)
            self.fx_in = 0;
            self.fx_out = 0;
            return;
        }
        if self.fx_out != 0 {
            let c = self.fx_out;
            if c == 3 {
                // out to the left: x0 = s·W/16 − W after the decrement (−32 … −544)
                self.s -= 1;
                self.pic_x = self.s * 512 / 16 - 512;
            } else {
                self.fill = Some((255 - self.s * 255 / fx_len(c), c >= 4));
                self.s -= 1;
            }
            if self.s == -1 {
                self.fx_out = 0;
                self.shown = if c >= 4 { -1 } else { 0 };
            }
        } else if self.fx_in != 0 {
            let c = self.fx_in;
            if c == 3 {
                // in from the right: x0 = s·W/16 (512 … 0)
                self.pic_x = self.s * 512 / 16;
            } else {
                self.fill = Some((self.s * 255 / fx_len(c), c >= 4));
            }
            self.s -= 1;
            if self.s == -1 {
                self.fx_in = 0;
            }
        }
    }

    /// One frame; `held` is the human pad's held word (a shoulder button hides the text and blocks the input).
    pub fn tick(&mut self, press: u16, held: u16, vram: &mut Resident, pictures: &mut [u32; 3], sounds: &mut Vec<u16>, cache: &Cached) -> Out {
        self.frame += 1;
        let mut out = Out::None;
        if self.mode != 2 && self.fx_out == 0 {
            out = self.load(vram, pictures, cache);
        }
        self.step();
        // the subtitle layer and the input wait for the loads and the transitions
        self.sub = self.text_ready && !self.waiting && self.pic_wait.is_none() && self.fx_in == 0 && self.fx_out == 0 && held & 0xF == 0;
        self.sub_page = self.page;
        if !self.sub {
            return out;
        }
        if self.mode == 2 {
            // CONTINUE? LEFT = YES, RIGHT = NO (held; sfx 0x0B); confirm decides
            let old = self.no;
            if held & RIGHT != 0 {
                self.no = true;
            }
            if held & LEFT != 0 {
                self.no = false;
            }
            if self.no != old {
                sounds.push(0x0B);
            }
            if press & CONFIRM != 0 {
                sounds.push(0x0C);
                if self.no {
                    return Out::GameOver;
                }
                self.run.continues += 1;
                return Out::Fight;
            }
            return Out::None;
        }
        if press & START != 0 {
            return self.section_end();
        }
        if press & CONFIRM != 0 {
            sounds.push(0x0C);
            // the page's OUT transition (row 4), then the end code
            let c = self.code(4);
            self.fx_out = if (1..=5).contains(&c) { c } else { 0 };
            self.s = fx_len(self.fx_out);
            match self.code(31) {
                0xE => return self.section_end(),
                0xF => self.page = (self.page + 8) & !7,
                _ => self.page += 1,
            }
            self.text_ready = false;
        }
        Out::None
    }

    /// A file asked for with `Out::Fetch`: the pending picture (its IN transition starts) or the page's STY file.
    pub fn fetched(&mut self, name: &str, bytes: &[u8], vram: &mut Resident) {
        self.waiting = false;
        if let Some(p) = self.pic_wait
            && name.contains("PIC")
        {
            load_picture(vram, bytes, p as usize - 1);
            self.pic_wait = None;
            self.arm_in();
            return;
        }
        let k = (self.page >> 6) as usize;
        self.file = Some(k);
        self.tims = Arc::new(crate::formats::container::read(bytes));
    }

    /// The end of a section.
    fn section_end(&mut self) -> Out {
        match self.mode {
            0 => {
                if self.run.fight < self.run.fights() {
                    Out::Fight
                } else {
                    self.run.fight = if self.run.continues == 0 { -2 } else { -1 };
                    Out::Ending
                }
            }
            1 => {
                if self.won {
                    // the next fight's pages
                    let k = (3 * self.run.fight.max(0) as usize).min(24);
                    self.mode = 0;
                    self.page = game_tables::get().story_script[self.run.chr][k] as u16 * 8;
                    self.text_ready = false;
                    Out::None
                } else {
                    self.mode = 2;
                    self.no = false;
                    Out::None
                }
            }
            _ => Out::None,
        }
    }

    pub fn draw(&self, f: &Fonts, out: &mut Vec<Prim>) {
        // a sliding picture: the ambient fill around the whole display, the black part included
        out.push(prim::f4(0, 0, 512, 480, [0, 0, 0]).at(if self.vis > 0 { Anchor::AMBIENT } else { Anchor::CORE }));
        // the fill (OT slot 0) over the picture (slot 1); white between an OUT 4 / 5 and the next picture
        if self.vis > 0 {
            picture(out, self.pic_x);
            match self.fill {
                Some((v, true)) => flash(out, v),
                Some((v, false)) => fade(out, v),
                None => {}
            }
        } else if self.vis < 0 {
            out.push(prim::f4(0, 0, 512, 480, [255; 3]));
        }
        if !self.sub {
            return;
        }
        // the subtitle box: 50 % subtractive grey
        let mut bx = prim::f4(0, 360, 512, 80, [0x80; 3]);
        bx.semi = true;
        bx.tpage = 0x40;
        out.push(bx);
        if self.mode == 2 {
            f.menu_text(out, crate::assets::text::get("continue"), 160, 392, 0);
            f.menu_text(out, crate::assets::text::get("yes"), 304, 392, self.no as u16);
            f.menu_text(out, crate::assets::text::get("no"), 368, 392, (!self.no) as u16);
            return;
        }
        if let Some(c) = self.face {
            let c = c as i32;
            out.push(ft4(4, 368, 56, 63, ((c & 3) * 64, (c >> 2) * 64 + 1, 56, 63), 0x9D, clut(0, 480 + c as u16)));
        }
        let row = (self.sub_page & 7) as i32 * 32;
        // uv (0..255, 0..15) as the game: the control-code column 255 is not drawn
        out.push(ft4(62, 364, 446, 31, (0, row, 255, 15), 0x1C, clut(352, 495)));
        out.push(ft4(62, 404, 446, 30, (0, row + 16, 255, 15), 0x1C, clut(352, 495)));
        if self.frame & 0x30 != 0 {
            f.menu_text(out, "q", 488, 432, 0);
        }
    }
}

// ---------------------------------------------------------------- arcade epilogue

/// The imported game Uriko's arcade ending picture (`Epilogue::br1`; URIKO 1 of that game's gallery).
pub const BR1_ART: &str = "ART1/ART124.TIM";

/// `BR1_ART`'s fetched name (`Epilogue::br1`; a decoded state's `Tim::Raw` can only be it).
pub fn br1_art_file() -> String {
    format!("{}pictures/{BR1_ART}", crate::assets::BR1_PREFIX)
}

/// The character's ending illustration for 360 frames (60-frame fade in and out). Its own loop, not
/// the page player: no text, no transitions, no input. State 0 loads and draws a black tile; state 1
/// counts s1 = 359..0 with the four quarters (ot 1) under a subtractive tile (ot 0, mode 3) of
/// 255·(s1 − 300)/60 above 300 and 255 − 255·s1/60 below 60; s1 = 0 is an empty frame, then 0xF0 stops the music.
///
/// Engine addition (the import S10): with `art` the imported game's install's picture of that name instead (a 15-bit
/// 640×480 TIM of its galleries, `ART0/ART000.TIM` .. `ART1/ART125.TIM`), at (0, 0) and drawn over the screen, with
/// the same timing: the imported game Uriko's arcade ending.
#[derive(Clone, Serialize, Deserialize)]
pub struct Epilogue {
    pub chr: usize,
    loaded: bool,
    waiting: bool,
    t: i32,
    #[serde(default)]
    art: Option<String>,
    /// engine addition (simulation v21): the imported picture did not arrive, its 360 frames run black
    #[serde(default)]
    blank: bool,
}

impl Epilogue {
    pub fn new(chr: usize) -> Self {
        Self { chr: chr % 11, loaded: false, waiting: false, t: 360, art: None, blank: false }
    }

    /// The imported picture `art` (its path in that install, e.g. `ART1/ART124.TIM`).
    pub fn br1(art: &str) -> Self {
        Self { art: Some(format!("{}pictures/{art}", crate::assets::BR1_PREFIX)), ..Self::new(0) }
    }

    pub fn tick(&mut self) -> Out {
        if !self.loaded {
            if !self.waiting {
                self.waiting = true;
                return Out::Fetch(self.file());
            }
            return Out::None;
        }
        self.t -= 1;
        if self.t < 0 { Out::Done } else { Out::None }
    }

    /// Quarters at (512,0) (640,0) (576,256) (704,256), CLUTs (0, 496 + k).
    /// The character's /CON file.
    fn file(&self) -> String {
        match &self.art {
            Some(a) => a.clone(),
            None => format!("engine/con/{}CON.DAT", DIRS[self.chr % 11]),
        }
    }

    pub fn fetched(&mut self, vram: &mut Resident, bytes: &[u8]) {
        if self.art.is_some() {
            vram.load(Tim::Raw(self.file()), bytes, None, Some((0, 0)));
            self.loaded = true;
            self.waiting = false;
            return;
        }
        let c = crate::formats::container::read(bytes);
        for (k, at) in [(512, 0), (640, 0), (576, 256), (704, 256)].into_iter().enumerate() {
            if let Some(Some(t)) = c.get(k) {
                vram.load(Tim::File(self.file(), k as u8), t, Some((0, 496 + k)), Some(at));
            }
        }
        self.loaded = true;
        self.waiting = false;
    }

    /// The fetch completed without the imported picture (an install without its `pictures/`; engine addition, the
    /// the import S10): no epilogue, the ending movie next; from simulation v21 (`fight::version::v::BR1_FIXED_MOVIES`) its 360 frames
    /// all the same, black, so the ending's length does not depend on the install's files. A CON file is always in
    /// the export: its epilogue waits.
    pub fn missing(&mut self, g: crate::fight::version::Gates) {
        if self.art.is_some() && self.waiting && !self.loaded {
            log::warn!("{} not found: no epilogue", self.file());
            (self.loaded, self.waiting) = (true, false);
            if g.br1_fixed_movies {
                self.blank = true;
            } else {
                self.t = 0;
            }
        }
    }

    /// The one imported picture an epilogue shows (`br1`), or none (a decoded state's check).
    pub fn valid(&self) -> bool {
        self.art.as_ref().is_none_or(|a| *a == br1_art_file())
    }

    pub fn draw(&self, out: &mut Vec<Prim>) {
        out.push(prim::f4(0, 0, 512, 480, [0, 0, 0]));
        if !self.loaded || self.t <= 0 || self.blank {
            return;
        }
        if self.art.is_some() {
            // 15-bit pages of 256 × 256 from (0, 0): 640 pixels over the 512-wide screen, 480 lines
            for (y, h, ty) in [(0, 256, 0u16), (256, 224, 0x10)] {
                for (k, (x, w, uw)) in [(0, 205, 256), (205, 205, 256), (410, 102, 128)].into_iter().enumerate() {
                    out.push(ft4(x, y, w, h, (0, 0, uw, h), 0x100 | ty | (4 * k as u16), 0).at(Anchor::AMBIENT));
                }
            }
        } else {
            for (k, (x, y, tp)) in [(0, 0, 0x88u16), (256, 0, 0x8A), (0, 240, 0x99), (256, 240, 0x9B)].iter().enumerate() {
                out.push(ft4(*x, *y, 256, 240, (0, 0, 256, 240), *tp, clut(0, 496 + k as u16)).at(Anchor::AMBIENT));
            }
        }
        if self.t < 60 {
            fade(out, 255 - self.t * 255 / 60);
        } else if self.t > 300 {
            fade(out, (self.t - 300) * 255 / 60);
        }
    }
}
