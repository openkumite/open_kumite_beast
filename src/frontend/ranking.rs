//! NAME ENTRY and the ranking table.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! NAME ENTRY and the ranking table (main mode 9): after an arcade clear (clear time, only with TIME
//! LIMIT ≥ 60 s and MATCH POINT ≥ 2) or a survival game over (wins), the player's place in the 10-row table, a
//! 3-letter name from the 43-character set (`<` = back, `^` = end; LEFT / RIGHT change the letter, □ × ○ enter
//! it), then the table for 902 frames. Unplaced runs see the table only. Text only: the menu frame (style 0x23),
//! two scrolling banners and menu font 4; the jingles are /SND/ENDING.SND in SE slot 2.

use crate::assets::text;
use crate::frontend::options::RankRec;
use crate::output::font::Fonts;
use crate::output::prim::Prim;
use serde::{Deserialize, Serialize};

/// The 43 letters of a name (the text `letters`; `BACK` and `END` among them); '?' where the game's text has
/// none.
pub fn letters() -> [u8; 43] {
    let t = text::get("letters").as_bytes();
    std::array::from_fn(|k| t.get(k).copied().unwrap_or(b'?'))
}
const BACK: usize = 0x1F; // '<'
const END: usize = 0x20; // '^'

const LEFT: u16 = 0x8000;
const RIGHT: u16 = 0x2000;
const CONFIRM: u16 = 0xE0;

/// The 22 names that become the text `hud`, the text list `censor` (three bytes each: without
/// the game's text none is refused).
const CENSOR: usize = 22;

/// Banners: the text list `screen_title`'s entries 0, 2, 1 with their widths (`screen.title_widths`).
const TOP: usize = 0;
const CONGRATS: usize = 2;
const THANKS: usize = 1;

fn title_width(k: usize) -> i32 {
    crate::assets::game_tables::get().title_width(k)
}

/// The format `"%02u'%02u\"%02u"`: frames → MM'SS"CC, 99'59"99 from 100 minutes.
pub fn time_text(v: i32) -> String {
    let v = v.max(0) as u32;
    let (m, s, cs) = if v / 3600 >= 100 { (99, 59, 99) } else { (v / 3600, v / 60 % 60, v % 60 * 100 / 60) };
    format!("{m:02}'{s:02}\"{cs:02}")
}

/// What one frame of the screen asks of the front end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    Stay,
    /// END: the (censored) name goes into the record and becomes the next default
    Named([u8; 3]),
    /// the table's first frame (state 4, timer 0): music 7 / 0x28 and the ENDING.SND tones
    Table,
    /// state 0xF0
    Exit,
}

impl Entry {
    /// A decoded state's entry: its table (of `tables`), place, name position and letter in range.
    pub fn check(&self, tables: usize) -> Result<(), String> {
        let ok = self.table < tables && self.rank.is_none_or(|r| r < 10) && self.pos < self.buf.len() && self.cur < 43;
        if ok { Ok(()) } else { Err(format!("name entry table {} rank {:?}", self.table, self.rank)) }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// 0 arcade (time), 2 survival (wins)
    pub table: usize,
    /// the player's place (None: not ranked, the table only)
    pub rank: Option<usize>,
    /// the player's record (the insert's fields), for the VS RANKING update on exit
    pub rec: RankRec,
    /// the name buffer: the letter under the cursor is buf[pos]
    pub buf: [u8; 4],
    pub pos: usize,
    /// letter under the cursor (index into `letters`)
    pub cur: usize,
    /// −1 while entering (state 3), then the table's frames (state 4)
    pub timer: i32,
    /// x of banner slot 0 (top) and slot 1 (bottom)
    pub banner: [i32; 2],
    /// the bottom banner is still CONGRATULATIONS (until the frame after END)
    congrats: bool,
    exit: bool,
    /// M+0xDC: the blink / flicker clock
    pub frame: u32,
}

/// Ascending value, empty slots first / descending +0xE: the place of a new record.
pub fn place(table: &[RankRec; 10], by_time: bool, rec: &RankRec) -> Option<usize> {
    table.iter().position(|r| if by_time { r.value == 0 || rec.value <= r.value } else { rec.e > r.e })
}

/// VS RANKING: descending +0xC, a tie ranks above.
pub fn place_vs(table: &[RankRec; 10], rec: &RankRec) -> Option<usize> {
    table.iter().position(|r| rec.c >= r.c)
}

/// Insert at `rank`, the rest moves down.
pub fn insert(table: &mut [RankRec; 10], rank: usize, rec: RankRec) {
    for k in (rank + 1..10).rev() {
        table[k] = table[k - 1];
    }
    table[rank] = rec;
}

/// BACK's letter (`<`) and DEL become spaces; a listed word (`CENSOR`) becomes the text `hud`.
fn censor(buf: [u8; 4]) -> [u8; 3] {
    let back = letters()[BACK];
    let n = [0, 1, 2].map(|i| if buf[i] == back || buf[i] == 0x7F { b' ' } else { buf[i] });
    if (0..CENSOR).any(|k| text::item("censor", k).as_bytes() == n) {
        let h = text::get("hud").as_bytes();
        std::array::from_fn(|k| h.get(k).copied().unwrap_or(b' '))
    } else {
        n
    }
}

impl Entry {
    /// State 2: a placed record gets the last name entered (pos 3, cursor on END) or "   " (cursor on A); an
    /// unplaced one goes straight to the table (state 4) with an empty name.
    pub fn new(table: usize, rank: Option<usize>, rec: RankRec, last: [u8; 3]) -> Self {
        let named = last[0] != 0;
        let (buf, pos, cur) = match rank {
            None => ([0; 4], 0, 0),
            Some(_) if named => ([last[0], last[1], last[2], 0], 3, END),
            Some(_) => (*b"   \0", 0, 0),
        };
        let timer = if rank.is_some() { -1 } else { 0 };
        Self { table, rank, rec, buf, pos, cur, timer, banner: [0, 512], congrats: rank.is_some(), exit: false, frame: u32::MAX }
    }

    pub fn done(&self) -> bool {
        self.timer >= 0
    }

    /// The entered name (spaces for the not-ranked path's empty buffer).
    pub fn name(&self) -> [u8; 3] {
        [self.buf[0], self.buf[1], self.buf[2]]
    }

    /// One frame of states 3 / 4 for the player's pad (`rep` / `press`) and `any` = both pads' new presses
    /// (M+0x7C). Sounds are `(id, delay)` for `play(0x400, id)`.
    pub fn tick(&mut self, rep: u16, press: u16, any: u16, sounds: &mut Vec<(u16, u32)>) -> Step {
        self.frame = self.frame.wrapping_add(1);
        if self.exit {
            return Step::Exit;
        }
        if self.timer >= 0 && self.congrats {
            self.congrats = false;
            self.banner[1] = 512;
        }
        self.scroll();
        if self.timer < 0 {
            return self.entry(rep, press, sounds);
        }
        // state 4
        let mut step = Step::Stay;
        if self.timer == 0 {
            step = Step::Table;
            sounds.extend([(0x83, 30), (0x84, 35), (0x85, 40)]);
        }
        if self.timer >= 121 && any & 0x8E0 != 0 {
            self.timer = 5940;
        }
        if self.timer >= 901 {
            self.exit = true;
        }
        self.timer += 1;
        step
    }

    /// Top −4, bottom −3 per frame, wrapped by the banner width.
    fn scroll(&mut self) {
        let bw = title_width(if self.congrats { CONGRATS } else { THANKS });
        for (k, (sp, w)) in [(4, title_width(TOP)), (3, bw)].into_iter().enumerate() {
            self.banner[k] -= sp;
            while self.banner[k] < -w {
                self.banner[k] += w;
            }
        }
    }

    /// State 3: the letter under the cursor is part of the name; START does nothing.
    fn entry(&mut self, rep: u16, press: u16, sounds: &mut Vec<(u16, u32)>) -> Step {
        if rep & RIGHT != 0 {
            sounds.push((0x0B, 0));
            self.cur = (self.cur + 1) % 43;
        }
        if rep & LEFT != 0 {
            sounds.push((0x0B, 0));
            self.cur = (self.cur + 42) % 43;
        }
        if self.pos >= 3 {
            // a full name: only BACK or END
            self.cur = self.cur.clamp(BACK, END);
        }
        self.buf[self.pos] = letters()[self.cur];
        if press & CONFIRM == 0 {
            return Step::Stay;
        }
        match self.cur {
            BACK => {
                sounds.push((0x0D, 0));
                if self.pos > 0 {
                    self.buf[self.pos] = b' ';
                    self.pos -= 1;
                }
            }
            END => {
                sounds.push((0x0C, 0));
                self.buf[self.pos] = b' ';
                let n = censor(self.buf);
                self.buf = [n[0], n[1], n[2], 0];
                self.timer = 0;
                return Step::Named(n);
            }
            _ => {
                sounds.push((0x0C, 0));
                self.pos += 1;
                if self.pos == 3 {
                    self.cur = END;
                }
            }
        }
        Step::Stay
    }

    /// Frame (style 0x23) → top banner → title → rows 10..1 → bottom banner.
    pub fn draw(&self, f: &Fonts, table: &[RankRec; 10], names: &[&str; 11], out: &mut Vec<Prim>) {
        let fr = self.frame;
        crate::frontend::menus::frame_with(out, f, 3, false);
        let banner = |out: &mut Vec<Prim>, k: usize, x0: i32, y: i32, pal: u16| {
            crate::frontend::menus::banner(f, out, text::item("screen_title", k), title_width(k), x0, y, pal)
        };
        banner(out, TOP, self.banner[0], 48, 7);
        f.menu_text(out, text::item("screen_title", TOP), 32, 48, 0);
        let survival = self.table == 2;
        // pen y of each row: 96 + 32k, the rank row 8 higher and 40 tall
        for k in (0..10).rev() {
            let r = &table[k];
            let n = format!("{}", k + 1);
            let sx = 32 + f.menu_width(&n);
            // an imported fighter's record (engine addition, the import S10): its name (`roster::name`)
            let chr = names.get(r.chr as usize).copied().unwrap_or_else(|| crate::assets::roster::name(r.chr));
            let value = if survival { format!("{}", (r.e as i32).rem_euclid(1000)) } else { time_text(r.value) };
            let vx = if survival { 112 + (3 - value.len() as i32) * 16 } else { 112 };
            if Some(k) == self.rank {
                let y = 88 + 32 * k as i32;
                let flick = if fr & 4 != 0 { 5 } else { 0 };
                f.menu_text_tall(out, &n, 32, y, 3);
                f.menu_text_tall(out, text::item("ordinal", k), sx, y, 0);
                f.menu_mono(out, &value, vx, y, flick, true);
                if survival {
                    f.menu_text_tall(out, text::get("wins"), 160, y, flick);
                }
                f.menu_text_tall(out, chr, 344, y, 2);
                if self.timer < 0 {
                    // the letters, one per 16-px cell; the cursor's blinks 8 on / 8 off
                    for i in 0..=self.pos.min(3) {
                        let c = self.buf[i];
                        if (i == self.pos && fr & 8 == 0) || c == 0 {
                            continue;
                        }
                        f.menu_text_tall(out, &(c as char).to_string(), 264 + 16 * i as i32, y, 0);
                    }
                } else {
                    let s: String = self.buf.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
                    f.menu_text_tall(out, &s, 264, y, if fr & 4 != 0 { 0 } else { 3 });
                }
            } else {
                let y = 96 + 32 * k as i32;
                f.menu_text(out, &n, 32, y, 3);
                f.menu_text(out, text::item("ordinal", k), sx, y, 0);
                f.menu_mono(out, &value, vx, y, 5, false);
                if survival {
                    f.menu_text(out, text::get("wins"), 160, y, 5);
                }
                let name: String = r.name.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
                f.menu_text(out, &name, 264, y, 0);
                f.menu_text(out, chr, 344, y, 2);
            }
        }
        if self.congrats {
            banner(out, CONGRATS, self.banner[1], 420, 0);
        } else {
            banner(out, THANKS, self.banner[1], 420, 3);
        }
    }
}
