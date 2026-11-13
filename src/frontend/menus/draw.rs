//! Drawing the list menus: the frame, the scrolling banner, the rows, PLAY DATA.
//!
//! State: none of its own: reads the serialised `Menu`
//!
//! Drawing the list menus: the menu screen frame (`frame`, `frame_with`, `COLOURS`), the scrolling
//! banner, the rows with their labels and values, the PLAY DATA pages.

use super::{HL, Kind, Menu, NORMAL, PICTURE_TOTAL};
use crate::assets::text;
use crate::assets::text::Label::{self, Game, Port};
use crate::frontend::options::Aspect;
use crate::output::font::Fonts;
use crate::output::layout::{Anchor, Ax, Clip, REACH};
use crate::output::prim::{self, Prim, clut, ft4};

/// A screen title's text and banner width (the text list `screen_title`, `screen.title_widths`).
fn title(k: usize) -> (&'static str, i32) {
    (text::item("screen_title", k), crate::assets::game_tables::get().title_width(k))
}

impl Menu {
    /// Banner text and width: the screen titles 3 (OPTIONS), 6 (SOUND OPTIONS), 9 (CUSTOM) …
    pub(super) fn banner(&self) -> (&'static str, i32) {
        match self.kind {
            Kind::Options => title(3),
            Kind::Sound => title(6),
            Kind::Custom => title(9),
            Kind::Controller => title(5),
            Kind::Movies => title(7),
            Kind::PlayData => title(8),
            Kind::MemCard => title(4),
            // engine addition: the text width (120 px in the menu font) + 64, as SOUND OPTIONS
            Kind::Graphics => ("GRAPHICS", 184),
        }
    }

    /// Colour set of the frame: OPTIONS 0 (red), CUSTOM 1 (green), SOUND OPTIONS 2 (blue), MEMORY CARD 7 (teal).
    fn style(&self) -> usize {
        match self.kind {
            Kind::Options => 0,
            Kind::Custom => 1,
            Kind::Sound => 2,
            Kind::Controller => 4,
            Kind::Movies => 6,
            Kind::PlayData => 5,
            Kind::MemCard => 7,
            // engine addition: the grey set none of the game's menus uses
            Kind::Graphics => 3,
        }
    }

    fn valx(&self) -> i32 {
        match self.kind {
            Kind::Options => 352,
            Kind::Custom => 320,
            Kind::Sound => 312,
            Kind::Controller => 112,
            Kind::Movies => 320,
            Kind::PlayData => 192,
            Kind::MemCard => 352,
            Kind::Graphics => 288,
        }
    }

    pub fn draw(&self, f: &Fonts, out: &mut Vec<Prim>) {
        if self.kind == Kind::Controller {
            frame_with(out, f, self.style(), false);
            let (name, bw) = self.banner();
            banner(f, out, name, bw, self.banner_x, 48, 7);
            f.menu_text(out, name, 48, 48, HL);
            self.draw_controller(f, out);
            // legend: "DEFAULT L1+R1   EXIT START"
            f.menu_small(out, text::get("default"), 152, 424, 0);
            f.menu_icons(out, "hi", 220, 420, 8);
            f.menu_small(out, "+", 245, 424, 0);
            f.menu_icons(out, "de", 252, 420, 8);
            f.menu_small(out, text::get("exit"), 288, 424, 0);
            out.push(ft4(328, 420, 24, 16, (224, 216, 24, 16), 0x1E, clut(352, 488)));
            return;
        }
        frame(out, f, self.style());
        // header: the scrolling banner (pal 7, blended) under the title (pal 0) at (48, 48)
        let (name, bw) = self.banner();
        banner(f, out, name, bw, self.banner_x, 48, 7);
        f.menu_text(out, name, 48, 48, HL);
        let valx = self.valx();
        let mut y = 80;
        if self.kind == Kind::PlayData {
            self.draw_play_data(f, out);
            y = 352;
        }
        for i in 1..=self.count() {
            if !self.enabled(i) {
                continue;
            }
            let hl = i == self.cursor;
            let gs = self.kind == Kind::Custom && i == 1;
            // GAME START uses pal 3 (highlighted) / 4; the others 0 / 1
            let pal = if gs {
                if hl { 3 } else { 4 }
            } else if hl {
                HL
            } else {
                NORMAL
            };
            if hl {
                f.menu_text(out, ">", 32, y, HL);
            }
            f.menu_text(out, self.label(i).text(), 48, y, pal);
            self.value(f, out, i, valx, y, pal);
            // engine: OPTIONS' 15 rows (GRAPHICS added) at 22 lines so EXIT stays above the bottom bar; 16 (CHANGE GAME
            // DATA, v9) at 21
            y += if gs {
                44
            } else if self.kind == Kind::Options {
                if self.data_row { 21 } else { 22 }
            } else {
                24
            };
        }
    }

    /// PLAY DATA code 0x100: the rankings (row format 0x12), the per-character PLAY DATA table or
    /// the VS DIAGRAM, in 240-line coordinates doubled.
    fn draw_play_data(&self, f: &Fonts, out: &mut Vec<Prim>) {
        use crate::assets::roster::br2_name;
        let s = &self.s;
        // fixed-pitch text
        let mono = |out: &mut Vec<Prim>, t: &str, x: i32, y: i32, pal: u16, big: bool| {
            let pitch = if big { 15 } else { 8 };
            for (i, c) in t.chars().enumerate() {
                if c != ' ' {
                    let g = c.to_string();
                    if big {
                        f.menu_text(out, &g, x + pitch * i as i32, y, pal);
                    } else {
                        f.menu_small(out, &g, x + pitch * i as i32, y, pal);
                    }
                }
            }
        };
        let pct = |a: u32, b: u32| {
            let v = a * 1000 / b.max(1);
            format!("{:3}.{}%", v / 10, v % 10)
        };
        match self.display {
            0..=2 => {
                for (k, r) in s.ranking[self.display as usize].iter().enumerate() {
                    let y = 96 + 24 * k as i32;
                    let n = format!("{}", k + 1);
                    f.menu_text(out, &n, 32, y, 3);
                    f.menu_text(out, text::item("ordinal", k), 32 + f.menu_width(&n), y, 0);
                    let v = r.value.max(0) as u32;
                    let (m, sec, cs) = if v / 3600 >= 100 { (99, 59, 99) } else { (v / 3600, v / 60 % 60, v % 60 * 100 / 60) };
                    mono(out, &format!("{m:02}'{sec:02}\"{cs:02}"), 112, y, 5, true);
                    let name: String = r.name.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
                    f.menu_text(out, &name, 264, y, 0);
                    f.menu_text(out, crate::assets::roster::name(r.chr), 344, y, 2);
                }
            }
            3 => {
                mono(out, text::get("play_play_vs_com_vs_hum"), 104, 96, 0, false);
                mono(out, text::get("counts_rate_win_rate_win_rate"), 96, 112, 0, false);
                let total: u32 = s.stats.iter().map(|r| r.plays as u32).sum::<u32>().max(1);
                let mut order: Vec<usize> = (0..11).collect();
                order.sort_by(|&a, &b| s.stats[b].plays.cmp(&s.stats[a].plays));
                for (row, &c) in order.iter().enumerate() {
                    let r = &s.stats[c];
                    let y = 144 + 16 * row as i32;
                    mono(out, &format!("{:>8}", br2_name(c)), 16, y, 0, false);
                    let pal = |n: u16| if n == 0 { 1 } else { 0 };
                    mono(out, &format!("{:5}", r.plays), 96, y, pal(r.plays), false);
                    mono(out, &pct(r.plays as u32, total), 160, y, pal(r.plays), false);
                    mono(out, &pct(r.com_wins as u32, r.com_games as u32), 224, y, pal(r.com_games), false);
                    mono(out, &pct(r.hum_wins as u32, r.hum_games as u32), 288, y, pal(r.hum_games), false);
                }
            }
            _ => {
                // 10-point match-ups between human players: round(100 a / (a + b) / 10), the pair kept at 10
                let points = |i: usize, j: usize| -> (u32, u32) {
                    let (a, b) = (s.stats[i].vs[j] as u32, s.stats[j].vs[i] as u32);
                    let n = (a + b).max(1);
                    let (mut p, q) = ((a * 100 / n + 5) / 10, (b * 100 / n + 5) / 10);
                    if p + q > 10 && p < q {
                        p -= 1;
                    }
                    (p, a + b)
                };
                let totals: Vec<u32> = (0..11).map(|i| (0..11).filter(|&j| j != i).map(|j| points(i, j).0).sum()).collect();
                let mut order: Vec<usize> = (0..11).collect();
                order.sort_by(|&a, &b| totals[b].cmp(&totals[a]));
                for (col, &c) in order.iter().enumerate() {
                    f.menu_small(out, &br2_name(c).chars().take(3).collect::<String>(), 96 + 32 * col as i32, 96, 0);
                }
                f.menu_small(out, text::get("total"), 448, 96, 0);
                for (row, &i) in order.iter().enumerate() {
                    let y = 144 + 16 * row as i32;
                    mono(out, &format!("{:>8}", br2_name(i)), 16, y, 0, false);
                    for (col, &j) in order.iter().enumerate() {
                        let x = 96 + 32 * col as i32;
                        if i == j {
                            f.menu_small(out, "-", x + 10, y, 1);
                        } else {
                            let (p, games) = points(i, j);
                            mono(out, &format!("{p:2}"), x + 4, y, if games == 0 { 1 } else { 0 }, false);
                        }
                    }
                    mono(out, &format!("{:3}", totals[i]), 456, y, 3, false);
                }
            }
        }
    }

    pub(super) fn label(&self, i: usize) -> Label {
        let i = if self.kind == Kind::Options { self.options_item(i) } else { i };
        let l: &[Label] = match self.kind {
            Kind::Options => &[
                Game("screen_title.4"),
                Game("screen_title.5"),
                Game("difficulty"),
                Game("attack_level"),
                Game("time_limit"),
                Game("match_point"),
                Game("character_select"),
                Game("display_type"),
                Game("screen_title.6"),
                Port("GRAPHICS"),
                Game("screen_title.7"),
                Game("screen_title.8"),
                Game("vibration"),
                Game("default"),
                Game("exit"),
                Port("CHANGE GAME DATA"),
            ],
            Kind::Sound => &[
                Game("output_select"),
                Game("bgm_select"),
                Game("master_volume"),
                Game("bgm_volume"),
                Game("bgm_player"),
                Game("se_type"),
                Game("se_player"),
                Game("default"),
                Game("exit"),
            ],
            Kind::Controller => &[Port(""); 8],
            Kind::Movies => &[Game("movies"), Game("pictures"), Game("exit")],
            Kind::PlayData => &[Game("display"), Game("exit")],
            Kind::MemCard => &[Game("auto_save"), Game("load_from_memory_card_slot_1"), Game("save_to_memory_card_slot_1"), Game("exit")],
            Kind::Graphics => &[
                Port("SCREEN MODE"),
                Port("WINDOW"),
                Port("WINDOW SIZE"),
                Port("VSYNC"),
                Port("FRAME CAP"),
                Port("RENDER QUALITY"),
                Port("HUD SIZE"),
                Port("PERF OVERLAY"),
                Game("default"),
                Game("exit"),
            ],
            Kind::Custom => &[
                Game("game_start"),
                Game("model_type"),
                Game("blood_effect"),
                Game("any_cancel_point"),
                Game("charge_speed"),
                Game("recover_speed"),
                Game("wall_display"),
                Game("afterimage_mode"),
                Game("side_step"),
                Game("default"),
                Game("exit"),
            ],
        };
        l[i - 1]
    }

    fn value(&self, f: &Fonts, out: &mut Vec<Prim>, i: usize, x: i32, y: i32, pal: u16) {
        let i = if self.kind == Kind::Options { self.options_item(i) } else { i };
        let s = &self.s;
        let onoff = |v: bool| text::get(if v { "on" } else { "off" });
        let num = |v: u8| format!("{}", v + 1);
        let text: String = match (self.kind, i) {
            (Kind::Options, 3) => num(s.difficulty),
            (Kind::Options, 4) => num(s.attack_level),
            (Kind::Options, 5) => {
                if s.time_limit < 4 {
                    f.menu_small(out, "sec.", x + 32, y + 7, pal);
                }
                ["20", "40", "60", "90", "----"][s.time_limit.min(4) as usize].into()
            }
            (Kind::Options, 6) => num(s.match_point),
            (Kind::Options, 7) => text::get(["normal", "quick"][s.char_select.min(1) as usize]).into(),
            (Kind::Options, 8) => text::get(["normal", "wide_1", "wide_2", "wide_3"][s.display.min(3) as usize]).into(),
            (Kind::Options, 13) => onoff(s.vibration != 0).into(),
            (Kind::Graphics, 1) => if self.g.aspect == Aspect::Four3 { "CLASSIC" } else { "FULL" }.into(),
            (Kind::Graphics, 2) => if self.g.fullscreen { "FULLSCREEN" } else { "WINDOW" }.into(),
            (Kind::Graphics, 3) => {
                // the preset chosen; a window larger than the monitor is made the monitor's size instead
                let (w, h) = self.g.window_size();
                format!("{w}X{h}")
            }
            (Kind::Graphics, 4) => onoff(self.g.vsync).into(),
            (Kind::Graphics, 5) => if self.g.frame_cap { "60" } else { "UNLIMITED" }.into(),
            (Kind::Graphics, 6) => format!("{}%", (self.g.render_scale() * 100.0) as u32),
            (Kind::Graphics, 7) => [text::get("small"), text::get("normal"), "LARGE"][(self.g.hud_size as usize).min(2)].into(),
            (Kind::Graphics, 8) => onoff(self.g.perf).into(),
            (Kind::Sound, 1) => text::get(["mono", "stereo"][s.output.min(1) as usize]).into(),
            (Kind::Sound, 2) => [text::get("console"), text::item("modes", 0)][s.bgm_select.min(1) as usize].into(),
            (Kind::Sound, 3) | (Kind::Sound, 4) => {
                let (v, cols) = if i == 3 {
                    (s.master, [[0x60, 0x54, 0], [0x40, 0x38, 0], [0xFF, 0xE0, 0]])
                } else {
                    (s.bgm_volume, [[0, 0x60, 0x54], [0, 0x40, 0x38], [0, 0xFF, 0xE0]])
                };
                out.push(prim::f4(x + 32, y, 100, 16, [0, 0, 0]));
                let mut bar = prim::g4(x + 32, y, v as i32, 16, [0, 0, 0], cols[1]);
                bar.rgb = [[0, 0, 0], cols[0], cols[1], cols[2]];
                out.push(bar);
                format!("{v}")
            }
            (Kind::Sound, 5) => text::fill(text::get("no_n"), &[&(self.bgm_no + 1)]),
            (Kind::Sound, 6) => text::item("se_banks", self.se_bank).into(),
            (Kind::Sound, 7) => text::fill(text::get("no_n"), &[&(self.se_no + 1)]),
            (Kind::Custom, 1) => {
                if s.custom.game == 2 {
                    f.menu_text(out, text::get("all_members"), x, y + 22, pal);
                }
                [text::get("vs_human"), text::get("vs_computer"), text::get("vs_computer"), text::item("screen_title", 23)][s.custom.game.min(3) as usize]
                    .into()
            }
            (Kind::Custom, 2) => text::get(["normal", "big_head", "kids", "big_arms"][s.custom.model.min(3) as usize]).into(),
            (Kind::Custom, 3) => onoff(s.custom.blood != 0).into(),
            (Kind::Custom, 4) => onoff(s.custom.flags >> 13 & 1 != 0).into(),
            (Kind::Custom, 5) | (Kind::Custom, 6) => {
                f.menu_small(out, "X", x, y + 7, pal);
                let v = if i == 5 { s.custom.charge } else { s.custom.recover };
                f.menu_text(out, &format!("{v}"), x + 10, y, pal);
                return;
            }
            (Kind::Custom, 7) => onoff(s.custom.flags >> 6 & 1 == 0).into(),
            (Kind::Custom, 8) => onoff(s.custom.flags >> 11 & 1 != 0).into(),
            (Kind::Custom, 9) => onoff(s.custom.flags >> 10 & 1 != 0).into(),
            (Kind::Movies, 1) => text::item("movies", self.movie).into(),
            (Kind::MemCard, 1) => onoff(s.auto_save != 0).into(),
            (Kind::PlayData, 1) => text::item("displays", self.display as usize).into(),
            (Kind::Movies, 2) => {
                let n = self.picture_count();
                if n == 0 { "0/0(0%)".into() } else { format!("{}/{}({}%)", self.picture + 1, n, n * 100 / PICTURE_TOTAL) }
            }
            _ => return,
        };
        f.menu_text(out, &text, x, y, pal);
    }
}

/// Colour sets of the menu frame: TL, TR, BR of the backdrop and the bottom bar.
const COLOURS: [[[u8; 3]; 4]; 8] = [
    [[32, 0, 0], [64, 0, 0], [12, 0, 0], [48, 0, 0]],
    [[0, 16, 0], [0, 48, 0], [0, 0, 0], [0, 48, 0]],
    [[0, 0, 48], [0, 0, 96], [0, 0, 16], [0, 0, 64]],
    [[32, 32, 32], [64, 64, 64], [16, 16, 16], [48, 48, 48]],
    [[38, 28, 18], [58, 48, 38], [28, 18, 8], [48, 38, 28]],
    [[32, 0, 32], [64, 0, 64], [12, 0, 12], [48, 0, 48]],
    [[24, 12, 0], [48, 24, 0], [8, 4, 0], [32, 16, 0]],
    [[15, 24, 24], [28, 48, 48], [5, 8, 8], [20, 32, 32]],
];

/// A scrolling banner: `text` repeated every `bw` from `x0` (−bw..0) on; the engine draws the repeats
/// out to `layout::REACH` beyond the core on both sides, so a wide window shows it edge to edge (the core scissor or
/// the window edge cuts the rest).
pub fn banner(f: &Fonts, out: &mut Vec<Prim>, text: &str, bw: i32, x0: i32, y: i32, pal: u16) {
    let bw = bw.max(1);
    let mut x = x0 - (REACH + bw - 1) / bw * bw;
    while x < 512 + REACH {
        let s0 = out.len();
        f.menu_text(out, text, x, y, pal);
        if x < x0 {
            // a text wider than its repeat overlaps the next one: the added copies left of the first one
            // are cut at the core's edge, which shows that one as the game does
            for p in &mut out[s0..] {
                p.at.clip = Clip::Outside;
            }
        }
        x += bw;
    }
}

/// The menu screen frame (layout 0, colour set `c`), as captured in 480i: gradient, the beast emblem
/// (P00 picture at (960,256), 50 % blended, tinted CLUT (352, 496+c)), the additive bottom bar, the main-game logo,
/// the metal header bar and the "OK □×○  EXIT △" legend.
pub fn frame(out: &mut Vec<Prim>, f: &Fonts, c: usize) {
    frame_with(out, f, c, true);
}

/// The frame with or without the "OK □×○  EXIT △" legend.
pub fn frame_with(out: &mut Vec<Prim>, f: &Fonts, c: usize, legend: bool) {
    let [a, b, cc, d] = COLOURS[c];
    let mut bg = prim::g4(0, 0, 512, 480, a, [0, 0, 0]);
    bg.rgb = [a, b, [0, 0, 0], cc];
    out.push(bg);
    // engine addition: the emblem and the logo keep to the right edge, the header bar spans the window
    let mut em = ft4(257, 161, 255, 255, (0, 0, 255, 255), 0x1F, clut(352, 496 + c as u16)).at(Anchor::new(Ax::End, Ax::Center));
    em.semi = true;
    out.push(em);
    let mut bar = prim::g4(0, 416, 512, 24, d, d);
    bar.rgb = [d, [0, 0, 0], d, [0, 0, 0]];
    bar.semi = true;
    bar.tpage = 0x20;
    out.push(bar);
    out.push(ft4(368, 396, 127, 40, (128, 152, 127, 40), 0x1E, clut(352, 504 + c as u16)).at(Anchor::new(Ax::End, Ax::Center)));
    for (k, row) in [491u16, 490, 489, 489].iter().enumerate() {
        let mut p = ft4(128 * k as i32, 40, 128, 32, (128, 128, 127, 24), 0x1E, clut(352, *row)).at(Anchor::new(Ax::Stretch, Ax::Center));
        if k == 3 {
            for q in 0..4 {
                p.uv[q][0] = 383.0 - p.uv[q][0];
            }
        }
        out.push(p);
    }
    if !legend {
        return;
    }
    let bc = clut(352, 488);
    for (x, u) in [(328, 0), (248, 16), (232, 32), (216, 48)] {
        out.push(ft4(x, 420, 16, 16, (u, 216, 16, 16), 0x1E, bc));
    }
    f.menu_small(out, text::get("exit"), 288, 424, 0);
    f.menu_small(out, text::get("ok"), 192, 424, 0);
}
