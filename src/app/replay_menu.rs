//! The replay list (the mode menu's REPLAY): play, keep, rename, export, delete, import.
//!
//! Engine addition.
//!
//! The replay list (engine addition, game-state plan stage 6b; the mode menu's REPLAY): the stored replays newest
//! first, an entry's actions (PLAY, KEEP / UNKEEP, RENAME, EXPORT, DELETE with YES / NO), IMPORT. Built in the list
//! menus' style (`menus::frame`, the scrolling banner, menu font palettes 0 / 1, the cursor `>`, the select's face
//! icons) with their sounds (0x0A move, 0x0B value, 0x0C confirm, 0x0D back); RENAME cycles letters as the NAME
//! ENTRY does (`ranking`). The list lives outside the game state (the files are the app's): the session waits on
//! the mode menu while it is up.

use super::replay_store::{Item, utc};
use crate::assets::text;
use crate::assets::text::Label::{self, Game, Port};
use crate::fight::input::pad::{DOWN, LEFT, RIGHT, TRIANGLE, UP};
use crate::fight::sim::Outcome;
use crate::frontend::{Pads, stage_name};
use crate::output::font::Fonts;
use crate::output::prim::{Prim, clut, ft4};
use crate::session::replay::MAX_NAME;

/// □ × ○ (the list menus' confirm; START does not confirm in them)
const CONFIRM: u16 = 0xE0;
/// Menu font palettes: highlighted, normal, orange (values).
const HL: u16 = 0;
const NORMAL: u16 = 1;
const ORANGE: u16 = 3;
/// The frame's colour set (`menus::COLOURS`: the brown of MOVIES AND PICTURES).
const STYLE: usize = 6;
/// Banner (the text `replay`): the text width + 64, as the list menus'.
const BANNER_W: i32 = 160;
/// The list's rows: an entry 46 high, IMPORT / EXIT 24; from y = 80 down to the bottom bar.
const TOP_Y: i32 = 80;
const BOTTOM_Y: i32 = 412;
const ENTRY_H: i32 = 46;
const ROW_H: i32 = 24;

/// An entry's actions.
const ACTIONS: [Label; 6] = [Port("PLAY"), Port("KEEP"), Port("RENAME"), Port("EXPORT"), Port("DELETE"), Game("exit")];
const A_PLAY: usize = 0;
const A_KEEP: usize = 1;
const A_RENAME: usize = 2;
const A_EXPORT: usize = 3;
const A_DELETE: usize = 4;

/// What the app must do after a frame of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    None,
    /// back to the mode menu
    Close,
    /// play this replay (its key)
    Play(String),
    /// KEEP set / cleared
    Keep(String, bool),
    Rename(String, String),
    Export(String),
    Delete(String),
    Import,
}

#[derive(Clone, Debug, PartialEq)]
enum View {
    List,
    /// the entry's actions, the cursor on this one
    Actions(usize),
    /// DELETE: YES (true) / NO
    Confirm(bool),
    Rename(NameEdit),
}

pub struct Browser {
    pub items: Vec<Item>,
    /// the row: an entry, then IMPORT, EXIT
    cursor: usize,
    /// the first row shown
    top: usize,
    view: View,
    banner_x: i32,
    frame: u32,
    pads: Pads,
}

impl Browser {
    /// The list of `items` (newest first), the cursor on the newest; `held` = the pads as it opens (the press that
    /// opened it is not a new one).
    pub fn new(items: Vec<Item>, held: [u16; 2]) -> Self {
        let mut pads = Pads::default();
        pads.update(held);
        Self { items, cursor: 0, top: 0, view: View::List, banner_x: 0, frame: 0, pads }
    }

    fn rows(&self) -> usize {
        self.items.len() + 2
    }

    fn import_row(&self) -> usize {
        self.items.len()
    }

    /// The entry under the cursor.
    fn item(&self) -> Option<&Item> {
        self.items.get(self.cursor)
    }

    /// The list again after a change (`keep` = the key the cursor stays on, when it is still there); an entry that
    /// went leaves its actions.
    pub fn refresh(&mut self, items: Vec<Item>, keep: Option<&str>) {
        self.items = items;
        match keep.and_then(|k| self.items.iter().position(|i| i.key == k)) {
            Some(i) => self.cursor = i,
            None => {
                self.cursor = self.cursor.min(self.rows() - 1);
                if self.view != View::List && self.item().is_none() {
                    self.view = View::List;
                }
            }
        }
        self.scroll_to_cursor();
    }

    /// One 60 Hz frame with both pads; the sounds (`play(0x400, id)`) go into `sounds`.
    pub fn tick(&mut self, raw: [u16; 2], sounds: &mut Vec<u16>) -> Cmd {
        self.frame = self.frame.wrapping_add(1);
        self.pads.update(raw);
        self.banner_x -= 4;
        while self.banner_x < -BANNER_W {
            self.banner_x += BANNER_W;
        }
        let (press, rep) = (self.pads.any_press(), self.pads.any_rep());
        let step = |n: usize, c: usize, sounds: &mut Vec<u16>| {
            if rep & DOWN != 0 {
                sounds.push(0x0A);
                (c + 1) % n
            } else if rep & UP != 0 {
                sounds.push(0x0A);
                (c + n - 1) % n
            } else {
                c
            }
        };
        match self.view.clone() {
            View::List => {
                if press & TRIANGLE != 0 {
                    sounds.push(0x0D);
                    return Cmd::Close;
                }
                self.cursor = step(self.rows(), self.cursor, sounds);
                // LEFT / RIGHT: a page up / down (engine addition for long lists)
                if rep & (LEFT | RIGHT) != 0 && !self.items.is_empty() {
                    let c = if rep & RIGHT != 0 { (self.cursor + 5).min(self.rows() - 1) } else { self.cursor.saturating_sub(5) };
                    if c != self.cursor {
                        self.cursor = c;
                        sounds.push(0x0A);
                    }
                }
                self.scroll_to_cursor();
                if press & CONFIRM != 0 {
                    sounds.push(0x0C);
                    if self.cursor == self.import_row() {
                        return Cmd::Import;
                    }
                    if self.cursor > self.import_row() {
                        return Cmd::Close;
                    }
                    self.view = View::Actions(A_PLAY);
                }
                Cmd::None
            }
            View::Actions(a) => {
                let Some(item) = self.item().cloned() else {
                    self.view = View::List;
                    return Cmd::None;
                };
                if press & TRIANGLE != 0 {
                    sounds.push(0x0D);
                    self.view = View::List;
                    return Cmd::None;
                }
                let a = step(ACTIONS.len(), a, sounds);
                self.view = View::Actions(a);
                if press & CONFIRM == 0 {
                    return Cmd::None;
                }
                sounds.push(0x0C);
                match a {
                    A_PLAY => Cmd::Play(item.key),
                    A_KEEP => Cmd::Keep(item.key, !item.header.keep),
                    A_RENAME => {
                        self.view = View::Rename(NameEdit::new(item.header.name.as_deref().unwrap_or("")));
                        Cmd::None
                    }
                    A_EXPORT => Cmd::Export(item.key),
                    A_DELETE => {
                        self.view = View::Confirm(false);
                        Cmd::None
                    }
                    _ => {
                        self.view = View::List;
                        Cmd::None
                    }
                }
            }
            View::Confirm(yes) => {
                let Some(item) = self.item().cloned() else {
                    self.view = View::List;
                    return Cmd::None;
                };
                if press & TRIANGLE != 0 {
                    sounds.push(0x0D);
                    self.view = View::Actions(A_DELETE);
                    return Cmd::None;
                }
                if rep & (LEFT | RIGHT) != 0 {
                    sounds.push(0x0B);
                    self.view = View::Confirm(!yes);
                    return Cmd::None;
                }
                if press & CONFIRM == 0 {
                    return Cmd::None;
                }
                if yes {
                    sounds.push(0x0C);
                    self.view = View::List;
                    Cmd::Delete(item.key)
                } else {
                    sounds.push(0x0D);
                    self.view = View::Actions(A_DELETE);
                    Cmd::None
                }
            }
            View::Rename(mut e) => {
                let Some(item) = self.item().cloned() else {
                    self.view = View::List;
                    return Cmd::None;
                };
                match e.tick(rep, press, sounds) {
                    Some(Some(name)) => {
                        self.view = View::Actions(A_RENAME);
                        Cmd::Rename(item.key, name)
                    }
                    Some(None) => {
                        self.view = View::Actions(A_RENAME);
                        Cmd::None
                    }
                    None => {
                        self.view = View::Rename(e);
                        Cmd::None
                    }
                }
            }
        }
    }

    /// The rows from `top` that fit: the cursor's always among them.
    fn scroll_to_cursor(&mut self) {
        self.top = self.top.min(self.cursor);
        while self.bottom_of(self.top, self.cursor) > BOTTOM_Y {
            self.top += 1;
        }
    }

    fn row_h(&self, r: usize) -> i32 {
        if r < self.items.len() { ENTRY_H } else { ROW_H }
    }

    /// The bottom y of row `last` when the list starts with row `top`.
    fn bottom_of(&self, top: usize, last: usize) -> i32 {
        TOP_Y + (top..=last).map(|r| self.row_h(r)).sum::<i32>()
    }

    /// The menu screen: the frame, the banner, then the list, the actions, the question or the name entry.
    pub fn draw(&self, f: &Fonts, out: &mut Vec<Prim>) {
        crate::frontend::menus::frame(out, f, STYLE);
        crate::frontend::menus::banner(f, out, text::get("replay"), BANNER_W, self.banner_x, 48, 7);
        let title = match &self.view {
            View::Rename(_) => "RENAME",
            _ => text::get("replay"),
        };
        f.menu_text(out, title, 48, 48, HL);
        match &self.view {
            View::List => self.draw_list(f, out),
            View::Actions(a) => {
                if let Some(item) = self.item() {
                    draw_entry(f, out, item, TOP_Y, true);
                }
                let mut y = TOP_Y + ENTRY_H + 16;
                for (k, label) in ACTIONS.iter().enumerate() {
                    let label = if k == A_KEEP && self.item().is_some_and(|i| i.header.keep) { "UNKEEP" } else { label.text() };
                    let hl = k == *a;
                    if hl {
                        f.menu_text(out, ">", 32, y, HL);
                    }
                    f.menu_text(out, label, 48, y, if hl { HL } else { NORMAL });
                    y += ROW_H;
                }
            }
            View::Confirm(yes) => {
                if let Some(item) = self.item() {
                    draw_entry(f, out, item, TOP_Y, true);
                }
                // as the MEMORY CARD questions: the text, then YES / NO (the chosen one in palette 0)
                f.menu_text(out, "DELETE THIS REPLAY ?", 48, 208, HL);
                f.menu_text(out, text::get("yes"), 80, 240, if *yes { HL } else { NORMAL });
                f.menu_text(out, text::get("no"), 176, 240, if *yes { NORMAL } else { HL });
            }
            View::Rename(e) => {
                if let Some(item) = self.item() {
                    draw_entry(f, out, item, TOP_Y, false);
                }
                e.draw(f, out, self.frame);
            }
        }
    }

    fn draw_list(&self, f: &Fonts, out: &mut Vec<Prim>) {
        if self.items.is_empty() {
            f.menu_text(out, "NO REPLAYS YET", 48, TOP_Y, NORMAL);
            f.menu_small(out, "EVERY FINISHED MATCH IS SAVED HERE.", 48, TOP_Y + 24, NORMAL);
        }
        let mut y = TOP_Y + if self.items.is_empty() { 48 } else { 0 };
        for r in self.top..self.rows() {
            let h = self.row_h(r);
            if y + h > BOTTOM_Y {
                break;
            }
            let hl = r == self.cursor;
            if hl {
                f.menu_text(out, ">", 16, y, HL);
            }
            match self.items.get(r) {
                Some(item) => draw_entry(f, out, item, y, hl),
                None => f.menu_text(out, if r == self.import_row() { "IMPORT" } else { text::get("exit") }, 32, y, if hl { HL } else { NORMAL }),
            }
            y += h;
        }
        // more above / below: the mode spinner's arrows
        if self.top > 0 {
            out.push(ft4(472, 80, 16, 16, (0, 232, 16, 16), 0x1E, clut(352, 480)));
        }
        if self.bottom_of(self.top, self.rows() - 1) > BOTTOM_Y {
            out.push(ft4(472, 392, 16, 16, (16, 232, 16, 16), 0x1E, clut(352, 480)));
        }
    }
}

/// An entry at `y` (46 high): the two sides' face icons, the name (or "P1 VS P2") and KEEP, then the date, mode and
/// stage, then the result and the fight's length in the 8×8 menu font.
fn draw_entry(f: &Fonts, out: &mut Vec<Prim>, item: &Item, y: i32, hl: bool) {
    let h = &item.header;
    let pal = if hl { HL } else { NORMAL };
    let sides = h.spec.map(|m| m.chr.map(|c| Some(crate::assets::roster::name(c)).filter(|n| !n.is_empty()).unwrap_or("?")));
    if let Some(m) = h.spec {
        // the select's face icons (INIT 24, CLUT row 480 + c from SE00), at about half size; none for an imported
        // fighter (INIT has no imported faces)
        for (k, c) in m.chr.iter().enumerate().filter(|(_, c)| !crate::assets::roster::is_br1(**c)) {
            let c = (*c as i32).min(10);
            out.push(ft4(32 + 34 * k as i32, y + 4, 32, 36, ((c & 3) * 64, (c >> 2) * 64 + 1, 56, 63), 0x9D, clut(0, 480 + c as u16)));
        }
    }
    // a team battle (engine addition): its four faces smaller, "2 VS 2" / "FREE FOR ALL" and the four names' codes
    if let Some(t) = h.team {
        for (k, c) in t.chr.iter().enumerate().filter(|(_, c)| !crate::assets::roster::is_br1(**c)) {
            let c = (*c as i32).min(10);
            let (x, yy) = (32 + 34 * (k as i32 & 1), y + 2 + 22 * (k as i32 >> 1));
            out.push(ft4(x, yy, 32, 20, ((c & 3) * 64, (c >> 2) * 64 + 1, 56, 63), 0x9D, clut(0, 480 + c as u16)));
        }
    }
    let vs = match (sides, h.team) {
        (Some([a, b]), _) => format!("{a} VS {b}"),
        (None, Some(t)) => {
            let m = if t.mode == crate::fight::team::TeamMode::Vs2 { "2 VS 2" } else { "FREE FOR ALL" };
            format!("{m} {}", t.chr.map(crate::assets::roster::code).join(" "))
        }
        _ => "SESSION".into(),
    };
    let title = h.name.clone().unwrap_or_else(|| vs.clone());
    f.menu_text(out, &fit(f, &title, if h.keep { 296 } else { 360 }), 104, y, pal);
    if h.keep {
        f.menu_text(out, "KEEP", 416, y, ORANGE);
    }
    let (yr, mo, d, hh, mi, _) = utc(h.time);
    let stage = h.spec.map(|m| m.stage).or(h.team.map(|t| t.stage)).map(stage_name).unwrap_or("");
    let line2 = format!("{yr:04}-{mo:02}-{d:02} {hh:02}:{mi:02} UTC  {}  {stage}", mode_label(&h.mode));
    f.menu_small(out, &line2, 104, y + 20, pal);
    let len = h.frames.saturating_sub(h.fight_from.unwrap_or(h.from)) / 60;
    let result = match (h.team, h.team_arcade) {
        (Some(t), Some(a)) => team_arcade_label(a, h.outcome, &h.team_rounds[..t.mode.teams()]),
        (Some(t), None) => team_result_label(t.mode, h.outcome, h.team_rounds),
        _ => result_label(h.outcome, h.rounds),
    };
    let mut line3 = format!("{result}  {}:{:02}", len / 60, len % 60);
    if h.name.is_some() {
        line3 = format!("{vs}  {line3}");
    }
    f.menu_small(out, &line3, 104, y + 32, pal);
}

/// The header's mode, short enough for the line (CUSTOM's long banners, which start with it, as CUSTOM: the text list
/// `modes` entry 5).
fn mode_label(mode: &str) -> &str {
    let custom = text::item("modes", 5);
    if mode.starts_with(custom) {
        custom
    } else if mode.is_empty() {
        "FIGHT"
    } else {
        mode
    }
}

/// The winner and the rounds won: "P1 WIN 2-1".
fn result_label(o: Option<Outcome>, r: [u8; 2]) -> String {
    let side = match o {
        Some(Outcome::Winner(w)) => format!("P{} WIN", w + 1),
        Some(Outcome::Quit) => text::get("quit").into(),
        Some(Outcome::Challenger(_)) => text::get("challenger").into(),
        // a human lost (CONTINUE / GAME OVER): the side with more rounds
        Some(_) if r[0] != r[1] => format!("P{} WIN", if r[0] > r[1] { 1 } else { 2 }),
        _ => text::get("draw").into(),
    };
    format!("{side} {}-{}", r[0], r[1])
}

/// A team battle's winner and the rounds each team won: "TEAM 1 WIN 2-1", "P3 WIN 2-1-0-1" (engine addition).
fn team_result_label(mode: crate::fight::team::TeamMode, o: Option<Outcome>, r: [u8; 4]) -> String {
    let side = match (o, mode) {
        (Some(Outcome::Winner(w)), crate::fight::team::TeamMode::Vs2) => format!("TEAM {} WIN", w + 1),
        (Some(Outcome::Winner(w)), _) => format!("P{} WIN", w + 1),
        (Some(Outcome::Draw), _) => text::get("draw").into(),
        // (TEAM ARCADE) a loss's end: the countdown's START or its end
        (Some(Outcome::Continue), _) => text::get("continue").into(),
        (Some(Outcome::GameOver), _) => text::get("game_over").into(),
        _ => text::get("quit").into(),
    };
    let rounds: Vec<String> = r[..mode.teams()].iter().map(|n| n.to_string()).collect();
    format!("{side} {}", rounds.join("-"))
}

/// A TEAM ARCADE battle's line (engine addition, A4): its stage and how it ended, the rounds by team: "STAGE 2/5 WIN 2-1",
/// "STAGE 3/5 CONTINUE 0-2", "STAGE 3/5 GAME OVER 1-2", the last stage won "CLEAR 2-0"; the continues before it.
fn team_arcade_label(a: crate::session::replay::TeamArcadeInfo, o: Option<Outcome>, r: &[u8]) -> String {
    let stage = format!("{} {}/{}", text::get("stage"), a.stage, a.stages);
    let end = match o {
        Some(Outcome::Winner(_)) if a.stage == a.stages => text::get("clear").to_string(),
        Some(Outcome::Winner(_)) => format!("{stage} WIN"),
        Some(Outcome::Continue) => format!("{stage} {}", text::get("continue")),
        Some(Outcome::GameOver) => format!("{stage} {}", text::get("game_over")),
        _ => format!("{stage} {}", text::get("quit")),
    };
    let rounds: Vec<String> = r.iter().map(|n| n.to_string()).collect();
    let cont = if a.continues > 0 { format!("  CONT {}", a.continues) } else { String::new() };
    format!("{end} {}{cont}", rounds.join("-"))
}

/// `s` cut to `w` pixels of the menu font ("..." at the end when cut).
fn fit(f: &Fonts, s: &str, w: i32) -> String {
    if f.menu_width(s) <= w {
        return s.to_string();
    }
    let mut t: String = s.to_string();
    while !t.is_empty() && f.menu_width(&format!("{t}...")) > w {
        t.pop();
    }
    format!("{t}...")
}

// ------------------------------------------------------------------ RENAME

/// RENAME's characters: the NAME ENTRY's 43 (`ranking::letters`: `<` back, `^` end) and a space.
pub fn set() -> [u8; 44] {
    let l = crate::frontend::ranking::letters();
    std::array::from_fn(|k| l.get(k).copied().unwrap_or(b' '))
}
const BACK: usize = 0x1F;
const END: usize = 0x20;

/// A name typed as on the NAME ENTRY screen (its state 3): LEFT / RIGHT change the letter under the cursor,
/// □ × ○ enter it (`<` removes the last, `^` ends), up to `MAX_NAME` characters; △ cancels (engine addition).
#[derive(Clone, Debug, PartialEq)]
pub struct NameEdit {
    pub buf: Vec<u8>,
    /// the letter under the cursor (`SET` index)
    pub cur: usize,
}

impl NameEdit {
    /// Editing `name` (its characters of `SET`, upper-cased): the cursor on END when there is one, else on A.
    pub fn new(name: &str) -> Self {
        let buf: Vec<u8> =
            name.bytes().map(|c| c.to_ascii_uppercase()).filter(|c| set().contains(c) && ![set()[BACK], set()[END]].contains(c)).take(MAX_NAME).collect();
        let cur = if buf.is_empty() { 0 } else { END };
        Self { buf, cur }
    }

    /// One frame: Some(Some(name)) at END, Some(None) on △.
    pub fn tick(&mut self, rep: u16, press: u16, sounds: &mut Vec<u16>) -> Option<Option<String>> {
        if press & TRIANGLE != 0 {
            sounds.push(0x0D);
            return Some(None);
        }
        if rep & RIGHT != 0 {
            sounds.push(0x0B);
            self.cur = (self.cur + 1) % set().len();
        }
        if rep & LEFT != 0 {
            sounds.push(0x0B);
            self.cur = (self.cur + set().len() - 1) % set().len();
        }
        if self.buf.len() >= MAX_NAME {
            // a full name: only BACK or END
            self.cur = self.cur.clamp(BACK, END);
        }
        if press & CONFIRM == 0 {
            return None;
        }
        match self.cur {
            BACK => {
                sounds.push(0x0D);
                self.buf.pop();
            }
            END => {
                sounds.push(0x0C);
                return Some(Some(String::from_utf8_lossy(&self.buf).trim().to_string()));
            }
            c => {
                sounds.push(0x0C);
                self.buf.push(set()[c]);
                if self.buf.len() >= MAX_NAME {
                    self.cur = END;
                }
            }
        }
        None
    }

    /// The name (its last 26 characters) in the tall menu font with the letter under the cursor blinking after it
    /// (8 frames on, 8 off, as the NAME ENTRY's), and the keys.
    fn draw(&self, f: &Fonts, out: &mut Vec<Prim>, frame: u32) {
        f.menu_text(out, "NAME", 48, 176, ORANGE);
        let shown = &self.buf[self.buf.len().saturating_sub(26)..];
        let mut x = 48;
        for &c in shown {
            if c != b' ' {
                f.menu_text_tall(out, &(c as char).to_string(), x, 208, HL);
            }
            x += 16;
        }
        if frame & 8 == 0 {
            let c = if set()[self.cur] == b' ' { '_' } else { set()[self.cur] as char };
            f.menu_text_tall(out, &c.to_string(), x, 208, 5);
        }
        f.menu_small(out, &format!("{}/{MAX_NAME}", self.buf.len()), 48, 256, NORMAL);
        f.menu_small(out, "LEFT/RIGHT: LETTER   OK: ENTER IT", 48, 288, NORMAL);
        f.menu_small(out, "<: DELETE   ^: END   TRIANGLE: CANCEL", 48, 302, NORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::replay::Header;

    const OK: u16 = 0x40;

    fn item(key: &str, time: u64, keep: bool) -> Item {
        Item { key: key.into(), header: Header { time, keep, ..Default::default() }, written: 0 }
    }

    /// Frames of the list with these pads (each press then a release).
    fn press(b: &mut Browser, keys: &[u16]) -> Vec<Cmd> {
        let mut out = Vec::new();
        for &k in keys {
            out.push(b.tick([k, 0], &mut Vec::new()));
            out.push(b.tick([0, 0], &mut Vec::new()));
        }
        out.into_iter().filter(|c| *c != Cmd::None).collect()
    }

    /// RENAME (with the test data's letters, data-gated): letters cycled and entered, `<` deletes, `^` ends; △ cancels;
    /// the cap holds.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn name_editing() {
        if crate::formats::tables::export_tables().is_none() {
            eprintln!("skipped: no okb_export");
            return;
        }
        let mut sounds = Vec::new();
        let mut e = NameEdit::new("");
        assert_eq!(e.cur, 0);
        // B, A, <, then C: "BC"
        e.tick(RIGHT, 0, &mut sounds);
        assert_eq!(e.tick(0, OK, &mut sounds), None);
        e.tick(LEFT, 0, &mut sounds);
        e.tick(0, OK, &mut sounds);
        assert_eq!(e.buf, b"BA");
        e.cur = BACK;
        e.tick(0, OK, &mut sounds);
        e.cur = 2;
        e.tick(0, OK, &mut sounds);
        e.cur = set().len() - 1;
        e.tick(0, OK, &mut sounds);
        e.cur = END;
        assert_eq!(e.tick(0, OK, &mut sounds), Some(Some("BC".into())));
        assert_eq!(sounds, [0x0B, 0x0C, 0x0B, 0x0C, 0x0D, 0x0C, 0x0C, 0x0C]);
        // wrapping LEFT from A reaches the space, the last
        let mut e = NameEdit::new("");
        e.tick(LEFT, 0, &mut sounds);
        assert_eq!(set()[e.cur], b' ');
        assert_eq!(e.tick(0, TRIANGLE, &mut sounds), Some(None));
        // an existing name: upper-cased, foreign characters dropped, the cursor on END
        let e = NameEdit::new("Best ko é<^!");
        assert_eq!((e.buf.as_slice(), e.cur), (&b"BEST KO !"[..], END));
        // full: only BACK / END, and the cursor goes to END
        let mut e = NameEdit::new(&"A".repeat(MAX_NAME - 1));
        e.cur = 0;
        e.tick(0, OK, &mut sounds);
        assert_eq!((e.buf.len(), e.cur), (MAX_NAME, END));
        e.tick(RIGHT, 0, &mut sounds);
        assert_eq!(e.cur, END);
        e.tick(LEFT, 0, &mut sounds);
        assert_eq!(e.cur, BACK);
        e.tick(0, OK, &mut sounds);
        assert_eq!(e.buf.len(), MAX_NAME - 1);
        assert_eq!(NameEdit::new(&"Z".repeat(200)).buf.len(), MAX_NAME);
    }

    /// The list's moves, an entry's actions, DELETE's question, IMPORT and EXIT.
    #[test]
    fn list_actions() {
        let items = vec![item("c", 30, false), item("b", 20, true), item("a", 10, false)];
        let mut b = Browser::new(items, [OK, 0]);
        // the held × that opened it is not a press
        assert_eq!(b.tick([OK, 0], &mut Vec::new()), Cmd::None);
        assert_eq!(b.view, View::List);
        // DOWN, ×: the actions of "b"; DOWN ×: UNKEEP
        assert_eq!(press(&mut b, &[DOWN, OK, DOWN, OK]), [Cmd::Keep("b".into(), false)]);
        // DOWN ×3 to DELETE, ×: the question (NO first); × = NO, back on DELETE
        assert_eq!(press(&mut b, &[DOWN, DOWN, DOWN, OK, OK]), []);
        assert_eq!(b.view, View::Actions(A_DELETE));
        assert_eq!(press(&mut b, &[OK, RIGHT, OK]), [Cmd::Delete("b".into())]);
        assert_eq!(b.view, View::List);
        b.refresh(vec![item("c", 30, false), item("a", 10, false)], None);
        assert_eq!(b.cursor, 1);
        // UP to "c": PLAY
        assert_eq!(press(&mut b, &[UP, OK, OK]), [Cmd::Play("c".into())]);
        // RENAME: ×, then END at once with the empty name
        b.view = View::Actions(A_RENAME);
        assert_eq!(press(&mut b, &[OK]), []);
        assert!(matches!(b.view, View::Rename(_)));
        if let View::Rename(e) = &mut b.view {
            e.cur = END;
        }
        assert_eq!(press(&mut b, &[OK]), [Cmd::Rename("c".into(), String::new())]);
        // △ twice: the list, then closed; IMPORT and EXIT rows after the entries (UP from the first wraps to EXIT)
        assert_eq!(press(&mut b, &[TRIANGLE]), []);
        assert_eq!(press(&mut b, &[UP, OK]), [Cmd::Close]);
        assert_eq!(press(&mut b, &[UP, OK]), [Cmd::Import]);
        assert_eq!(press(&mut b, &[TRIANGLE]), [Cmd::Close]);
    }

    /// A long list scrolls: the cursor's row always fits above the bottom bar.
    #[test]
    fn scrolling() {
        let items: Vec<Item> = (0..25).map(|k| item(&format!("{k}"), 100 - k, false)).collect();
        let mut b = Browser::new(items, [0; 2]);
        for _ in 0..26 {
            press(&mut b, &[DOWN]);
            assert!(b.top <= b.cursor && b.bottom_of(b.top, b.cursor) <= BOTTOM_Y, "{} {}", b.top, b.cursor);
        }
        assert_eq!(b.cursor, 26);
        press(&mut b, &[DOWN]);
        assert_eq!((b.cursor, b.top), (0, 0));
        press(&mut b, &[RIGHT]);
        assert_eq!(b.cursor, 5);
    }

    /// Data-gated: without the game's text every label is the placeholder.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn labels() {
        if crate::formats::tables::export_tables().is_none() {
            eprintln!("skipped: no okb_export");
            return;
        }
        assert_eq!(result_label(Some(Outcome::Winner(1)), [1, 2]), "P2 WIN 1-2");
        assert_eq!(result_label(Some(Outcome::GameOver), [0, 2]), "P2 WIN 0-2");
        assert_eq!(result_label(None, [1, 1]), format!("{} 1-1", text::get("draw")));
        assert_eq!(mode_label(text::item("screen_title", 22)), text::item("modes", 5));
    }

    /// (TEAM ARCADE A4) A team battle's line names a continue and GAME OVER; a TEAM ARCADE battle's its stage, CLEAR
    /// for the last one won, and the continues before it. Data-gated as `labels`.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn team_arcade_labels() {
        if crate::formats::tables::export_tables().is_none() {
            eprintln!("skipped: no okb_export");
            return;
        }
        use crate::fight::team::TeamMode;
        use crate::session::replay::TeamArcadeInfo;
        let (stage, cont, over) = (text::get("stage"), text::get("continue"), text::get("game_over"));
        assert_eq!(team_result_label(TeamMode::Vs2, Some(Outcome::Continue), [0, 2, 0, 0]), format!("{cont} 0-2"));
        assert_eq!(team_result_label(TeamMode::Vs2, Some(Outcome::GameOver), [1, 2, 0, 0]), format!("{over} 1-2"));
        let a = TeamArcadeInfo { stage: 3, stages: 5, continues: 0 };
        assert_eq!(team_arcade_label(a, Some(Outcome::Winner(0)), &[2, 1]), format!("{stage} 3/5 WIN 2-1"));
        assert_eq!(team_arcade_label(a, Some(Outcome::Continue), &[0, 2]), format!("{stage} 3/5 {cont} 0-2"));
        assert_eq!(team_arcade_label(a, Some(Outcome::GameOver), &[1, 2]), format!("{stage} 3/5 {over} 1-2"));
        let last = TeamArcadeInfo { stage: 6, stages: 6, continues: 2 };
        assert_eq!(team_arcade_label(last, Some(Outcome::Winner(1)), &[0, 2]), format!("{} 0-2  CONT 2", text::get("clear")));
    }

    /// (TEAM ARCADE A4) The list with TEAM ARCADE entries (`fixtures/team_arcade_v26.okbr`'s header, and the same as a
    /// continue, a GAME OVER and a clear) drawn over the front end's VRAM by the windowless renderer to
    /// `$SHOTDIR/replay_list_team_arcade.png` (run by hand: `cargo test --release team_arcade_list_shot -- --ignored`).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore]
    fn team_arcade_list_shot() {
        use crate::session::replay::{Replay, TeamArcadeInfo};
        let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
        let data = std::sync::Arc::new(pollster::block_on(crate::frontend::load(&crate::assets::Assets::new(root))).expect("front data"));
        let front = crate::frontend::Front::new(data.clone(), &mut Default::default(), &Default::default());
        let r = Replay::decode(include_bytes!("../session/fixtures/team_arcade_v26.okbr")).expect("decodes");
        let h = r.header;
        let at = |stage, continues, outcome| Header { team_arcade: Some(TeamArcadeInfo { stage, stages: 5, continues }), outcome: Some(outcome), ..h.clone() };
        let items = [h.clone(), at(1, 0, Outcome::Continue), at(3, 2, Outcome::GameOver), at(5, 1, Outcome::Winner(0))]
            .into_iter()
            .enumerate()
            .map(|(k, header)| Item { key: format!("{k}"), header, written: 0 })
            .collect();
        let b = Browser::new(items, [0; 2]);
        let mut prims = Vec::new();
        b.draw(crate::output::font::fonts(), &mut prims);
        let mut r = pollster::block_on(crate::render::Renderer::offscreen(640, 480, 2)).expect("a renderer");
        r.set_fit(crate::output::layout::Fit::Classic);
        let mut vram = crate::present::front_vram::VramView::default();
        vram.sync(&front, &data, &Default::default());
        r.upload_vram(&vram.vram.words);
        let quads = crate::output::layout::to_quads(&prims, r.viewport());
        r.frame_capture(&[], &quads, &[], [0.0, 0.0, 0.0], false);
        let [w, h] = r.viewport().win;
        let path = std::path::Path::new(&std::env::var("SHOTDIR").unwrap_or_else(|_| ".".into())).join("replay_list_team_arcade.png");
        let mut sink = |px: &[u8]| {
            let rgba: Vec<u8> = px.as_chunks::<4>().0.iter().flat_map(|c| [c[2], c[1], c[0], 255]).collect();
            image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
        };
        r.take_frames(0, &mut sink).expect("saved");
    }
}
