//! The setup screen's picture: untextured quads in window pixels for `Renderer::screen` (engine addition).
//!
//! Drawn without any game data: the Android backdrop's look (a radial glow from dark red to near black, three
//! tapered claw marks), the title, then the page (`screen::Page`) in a 640 × 400 canvas centred and scaled to the
//! window, its text in the engine's bitmap font (`output::tiny_font`). `draw` also gives each row's rectangle (window
//! pixels) for mouse clicks and taps.

use super::screen::{Kind, Page, Phase, Setup};
use crate::output::layout::Clip;
use crate::output::psx_quad::PsxQuad;
use crate::output::tiny_font;

/// The canvas the page is laid out in (scaled to fit the window).
const CW: f32 = 640.0;
const CH: f32 = 400.0;
/// The text's right edge (canvas units).
const RIGHT: f32 = 596.0;

/// The glow's colours (centre, 55 %, edge; OkbActivity.Backdrop) and the clear colour beyond it.
const GLOW: [[f32; 3]; 3] = [[76.0, 13.0, 20.0], [26.0, 6.0, 10.0], [6.0, 2.0, 3.0]];
pub const CLEAR: [f64; 3] = [6.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0];

const TITLE: [f32; 3] = [235.0, 64.0, 52.0];
const DIM: [f32; 3] = [170.0, 150.0, 150.0];
const TEXT: [f32; 3] = [225.0, 220.0, 215.0];
const REASON: [f32; 3] = [245.0, 205.0, 120.0];
const WARN: [f32; 3] = [250.0, 190.0, 60.0];
const ERROR: [f32; 3] = [255.0, 120.0, 105.0];
const ROW: [f32; 3] = [200.0, 195.0, 190.0];
const ROW_ON: [f32; 3] = [255.0, 255.0, 255.0];
const BAR: [f32; 3] = [200.0, 50.0, 40.0];

/// The semi-transparency modes used (tpage bits 5-6).
const HALF: u16 = 0;
const ADD: u16 = 1;

struct Canvas {
    out: Vec<PsxQuad>,
    win: [f32; 2],
    /// the canvas's origin and scale (window pixels per canvas unit)
    o: [f32; 2],
    k: f32,
}

impl Canvas {
    fn clip(&self, p: [f32; 2]) -> [f32; 4] {
        [p[0] / self.win[0] * 2.0 - 1.0, 1.0 - p[1] / self.win[1] * 2.0, 0.0, 1.0]
    }

    /// A quad in window pixels, corners TL TR BL BR with their colours, opaque or blended with what is under it
    /// (`HALF`: B/2 + F/2, `ADD`: B + F; the PS1's semi-transparency modes).
    fn quad(&mut self, p: [[f32; 2]; 4], c: [[f32; 3]; 4], blend: Option<u16>) {
        self.out.push(PsxQuad {
            pos: p.map(|p| self.clip(p)),
            uv: [[0.0; 2]; 4],
            rgb: c.map(|c| c.map(|v| v.clamp(0.0, 255.0) as u8)),
            tpage: blend.unwrap_or(0) << 5,
            clip: Clip::No,
            clut: 0,
            textured: false,
            semi: blend.is_some(),
            sheet: None,
        });
    }

    fn rect_px(&mut self, r: [f32; 4], c: [f32; 3], blend: Option<u16>) {
        self.quad([[r[0], r[1]], [r[2], r[1]], [r[0], r[3]], [r[2], r[3]]], [c; 4], blend);
    }

    /// Canvas units → window pixels.
    fn at(&self, x: f32, y: f32) -> [f32; 2] {
        [self.o[0] + x * self.k, self.o[1] + y * self.k]
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 3], blend: Option<u16>) {
        let (a, b) = (self.at(x0, y0), self.at(x1, y1));
        self.rect_px([a[0], a[1], b[0], b[1]], c, blend);
    }

    /// `s` with its top-left at canvas (x, y), each font pixel `px` canvas units (snapped to whole window pixels).
    fn text(&mut self, s: &str, x: f32, y: f32, px: f32, c: [f32; 3]) {
        let p = (px * self.k).round().max(1.0);
        let o = self.at(x, y).map(f32::round);
        for (i, ch) in s.chars().enumerate() {
            let cx = o[0] + (i as u32 * tiny_font::ADVANCE) as f32 * p;
            for (gx, gy, n) in tiny_font::runs(ch) {
                let (x0, y0) = (cx + gx as f32 * p, o[1] + gy as f32 * p);
                self.rect_px([x0, y0, x0 + n as f32 * p, y0 + p], c, None);
            }
        }
    }

    /// The width of `s` in canvas units at `px`.
    fn width(&self, s: &str, px: f32) -> f32 {
        tiny_font::width(s) as f32 * (px * self.k).round().max(1.0) / self.k
    }

    /// How many characters at `px` fit from `x` to the text's right edge (the font's pixels are whole window pixels,
    /// so this depends on the scale).
    fn cols(&self, x: f32, px: f32) -> usize {
        let cell = tiny_font::ADVANCE as f32 * (px * self.k).round().max(1.0) / self.k;
        (((RIGHT - x) / cell) as usize).max(8)
    }

    fn centred(&mut self, s: &str, y: f32, px: f32, c: [f32; 3]) {
        let w = self.width(s, px);
        self.text(s, (CW - w) * 0.5, y, px, c);
    }
}

/// The backdrop over the whole window: the glow (rings of gradient quads) and three claw marks (as OkbActivity.Backdrop
/// draws it).
fn backdrop(c: &mut Canvas) {
    let [w, h] = c.win;
    let (cx, cy, r) = (w * 0.5, h * 0.42, w.max(h) * 0.72);
    let col = |t: f32| {
        let (a, b, f) = if t < 0.55 { (GLOW[0], GLOW[1], t / 0.55) } else { (GLOW[1], GLOW[2], ((t - 0.55) / 0.45).min(1.0)) };
        [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)
    };
    const RINGS: usize = 20;
    const SIDES: usize = 48;
    let pt = |rad: f32, k: usize| {
        let a = k as f32 * std::f32::consts::TAU / SIDES as f32;
        [cx + rad * a.cos(), cy + rad * a.sin()]
    };
    for i in 0..RINGS {
        let (t0, t1) = (i as f32 / RINGS as f32, (i + 1) as f32 / RINGS as f32);
        let (c0, c1) = (col(t0), col(t1));
        for k in 0..SIDES {
            c.quad([pt(r * t0, k), pt(r * t1, k), pt(r * t0, k + 1), pt(r * t1, k + 1)], [c0, c1, c0, c1], None);
        }
    }
    // a slash from the top right down to the left: wide in the middle, pointed at both ends
    for i in 0..3 {
        let f = i as f32;
        let (x0, y0) = (w * (0.70 + 0.07 * f), h * (0.10 + 0.02 * f));
        let (x1, y1) = (w * (0.52 + 0.07 * f), h * (0.92 - 0.03 * f));
        let (mx, my, t) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5, w * 0.012);
        let (a, b) = ([mx + t * 2.2, my + t], [mx - t * 0.6, my - t * 0.4]);
        let bez = |c: [f32; 2], s: f32| {
            let u = 1.0 - s;
            [u * u * x0 + 2.0 * u * s * c[0] + s * s * x1, u * u * y0 + 2.0 * u * s * c[1] + s * s * y1]
        };
        const STEPS: usize = 16;
        for k in 0..STEPS {
            let (s0, s1) = (k as f32 / STEPS as f32, (k + 1) as f32 / STEPS as f32);
            c.quad([bez(a, s0), bez(b, s0), bez(a, s1), bez(b, s1)], [[28.0, 6.0, 6.0]; 4], Some(ADD));
        }
    }
}

/// `s` in lines of at most `n` characters (at spaces; a longer word, a path, cut).
pub fn wrap(s: &str, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    for l in s.lines() {
        let mut cur = String::new();
        for w in l.split(' ') {
            let w: Vec<char> = w.chars().collect();
            for part in w.chunks(n.max(1)) {
                let part: String = part.iter().collect();
                if !cur.is_empty() && cur.chars().count() + 1 + part.chars().count() > n {
                    out.push(std::mem::take(&mut cur));
                }
                if !cur.is_empty() {
                    cur.push(' ');
                }
                cur.push_str(&part);
            }
        }
        out.push(cur);
    }
    out
}

/// A row's label cut to `n` characters, from the left (a path keeps its end).
fn fit(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        return s.to_string();
    }
    let keep: String = c[c.len() - (n - 3)..].iter().collect();
    format!("...{keep}")
}

/// A picked source as the player knows it: a path, or (Android) a picked tree's URI as its folder
/// (`content://…/tree/primary%3ADownload%2Fbr2_export` → `Download/br2_export`).
fn shown(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    // a tree (`…/tree/<id>`) or a document (`…/document/<id>`): its id, the last of them
    let Some(tree) = s.strip_prefix("content://").and_then(|r| r.rsplit_once("/document/").or_else(|| r.rsplit_once("/tree/"))).map(|(_, t)| t) else {
        return s;
    };
    let b = tree.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let hex = |k: usize| b.get(k).and_then(|&c| (c as char).to_digit(16));
        match (b[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    let t = String::from_utf8_lossy(&out).to_string();
    t.strip_prefix("primary:").map(str::to_string).unwrap_or(t)
}

fn mb(b: u64) -> String {
    format!("{:.0} MB", b as f64 / 1e6)
}

/// The screen as quads for a window of `win` pixels, and the shown rows' rectangles (row, window pixels).
pub fn draw(s: &Setup, win: [f32; 2], version: &str, caret: bool) -> (Vec<PsxQuad>, Vec<Hit>) {
    let k = (win[0] / CW).min(win[1] / CH).max(0.01);
    let mut c = Canvas { out: Vec::new(), win, o: [(win[0] - CW * k) * 0.5, (win[1] - CH * k) * 0.5], k };
    backdrop(&mut c);
    c.centred("OPENKUMITEBEAST", 18.0, 4.0, TITLE);
    c.centred(&format!("UNOFFICIAL ENGINE  {version}"), 66.0, 1.5, DIM);
    c.centred("Game data not included: it is made from your own disc.", 84.0, 1.5, DIM);
    // the page's panel: the backdrop darkened to a quarter
    for _ in 0..2 {
        c.rect(30.0, 106.0, 610.0, 392.0, [0.0, 0.0, 0.0], Some(HALF));
    }
    c.rect(30.0, 106.0, 610.0, 107.0, [90.0, 24.0, 24.0], None);
    let (px, lh, x) = (2.0, 22.0, 44.0);
    let (cols, row_cols) = (c.cols(x, px), c.cols(x + 14.0, px));
    let mut y = 116.0;
    let line = |c: &mut Canvas, t: &str, col: [f32; 3], y: &mut f32| {
        for l in wrap(t, cols) {
            c.text(&l, x, *y, px, col);
            *y += lh;
        }
    };
    // the optional installs' states (a damaged one in the warning colour)
    let extra_notes = |c: &mut Canvas, y: &mut f32| {
        if let Some(v) = &s.br1 {
            *y += 6.0;
            line(c, &v.note(), if v.usable() || *v == crate::assets::manifest::Br1Verdict::Absent { DIM } else { WARN }, y);
        }
        if let Some(v) = &s.q3 {
            line(c, &v.note(), if v.usable() || *v == crate::assets::extras::Q3Verdict::Absent { DIM } else { WARN }, y);
        }
    };
    match &s.page {
        Page::Intro if s.kind == Kind::Br1 => {
            line(&mut c, "ADD THE BLOODY ROAR 1 DISC (OPTIONAL)", REASON, &mut y);
            y += 6.0;
            line(
                &mut c,
                "Its data is converted from your Bloody Roar (USA) disc and kept beside the game's. Without it the game plays as before.",
                TEXT,
                &mut y,
            );
        }
        Page::Intro if s.kind == Kind::Q3 => {
            line(&mut c, "ADD QUAKE III ARENA MAPS (OPTIONAL)", REASON, &mut y);
            y += 6.0;
            line(
                &mut c,
                "The extra stages Q3DM17 and Q3DM6 are Quake III Arena's maps: they are converted from the .pk3 files of your own copy (its baseq3 folder) and kept beside the game's data.",
                TEXT,
                &mut y,
            );
        }
        Page::Intro => {
            line(&mut c, &s.reason, REASON, &mut y);
            y += 6.0;
            // opened from the game (its data installed): new data replaces it
            let needs = if s.leave == Some(super::screen::Act::Close) {
                "New game data replaces the installed one."
            } else {
                "The game needs its data, converted from your Bloody Roar II (USA) disc."
            };
            line(&mut c, needs, TEXT, &mut y);
            extra_notes(&mut c, &mut y);
        }
        Page::Done => {
            line(&mut c, "THE GAME DATA IS INSTALLED.", REASON, &mut y);
            y += 6.0;
            line(
                &mut c,
                "You can also add your Bloody Roar (USA) disc and Quake III Arena's maps now (optional; later: OPTIONS, CHANGE GAME DATA).",
                TEXT,
                &mut y,
            );
            extra_notes(&mut c, &mut y);
        }
        Page::Remove(_) if s.removing => line(&mut c, "REMOVING...", TEXT, &mut y),
        Page::Remove(_) if s.remove_error.is_some() => line(&mut c, s.remove_error.as_deref().unwrap_or_default(), ERROR, &mut y),
        Page::Remove(k) => {
            line(&mut c, &format!("REMOVE THE {}?", k.data().to_uppercase()), REASON, &mut y);
            y += 6.0;
            let again = if *k == Kind::Q3 { "You can add the maps again later." } else { "You can add the disc again later." };
            line(&mut c, &format!("Its converted files are deleted; the game's own data stays. {again}"), TEXT, &mut y);
        }
        Page::Pick if s.kind == Kind::Q3 => {
            let how = if s.disc {
                if cfg!(target_arch = "wasm32") {
                    "Choose Quake III Arena's baseq3 folder (or the game's folder holding it): its .pk3 files are converted here."
                } else if cfg!(target_os = "android") {
                    "Copy Quake III Arena's baseq3 folder (its .pk3 files) to this device, then choose it."
                } else {
                    "Drop Quake III Arena's baseq3 folder (or the game's folder, or a .pk3 file) onto this window, type its path or BROWSE."
                }
            } else {
                "Choose a folder of Quake III Arena maps this game converted (its manifest.json, or a folder with a q3 folder in it)."
            };
            line(&mut c, how, TEXT, &mut y);
        }
        Page::Pick if s.kind == Kind::Br1 => {
            let how = if s.disc {
                if cfg!(target_os = "android") {
                    "Choose the disc image of your Bloody Roar (USA) disc: a .chd (with its CD audio) or the Track 1 .bin (raw, 2352-byte sectors)."
                } else if cfg!(target_arch = "wasm32") {
                    "Choose the disc image of your Bloody Roar (USA) disc: the .cue with both .bin files at once, or a .chd or .iso file."
                } else {
                    "Drop the Bloody Roar (USA) disc image (.cue, .bin, .chd or .iso) onto this window, choose one below, type its path or BROWSE."
                }
            } else {
                "Choose the folder of converted Bloody Roar 1 data (its manifest.json, or a folder with a br1 folder in it)."
            };
            line(&mut c, how, TEXT, &mut y);
        }
        Page::Pick => {
            let how = if s.disc {
                if cfg!(any(target_os = "android", target_arch = "wasm32")) {
                    "Choose the disc image of your Bloody Roar II (USA) disc: a .bin (raw, 2352-byte sectors), .chd or .iso file."
                } else {
                    "Drop the disc image (.cue, .bin, .chd or .iso) onto this window, choose one below, type its path or BROWSE."
                }
            } else if cfg!(target_arch = "wasm32") {
                "Choose the folder of converted game data: the browser copies it into this site's own storage."
            } else if cfg!(target_os = "android") {
                "Copy the folder of converted game data to this device (for example into Download), then choose it."
            } else {
                "Drop the folder of converted game data onto this window, choose one below, type its path or BROWSE."
            };
            line(&mut c, how, TEXT, &mut y);
            if s.listed().is_empty() && s.path_row {
                let where_ = if cfg!(target_os = "macos") { "beside the game" } else { "Downloads, Desktop or beside the game" };
                line(&mut c, &format!("(none found in {where_})"), DIM, &mut y);
            }
        }
        Page::Checking(p) => {
            line(&mut c, "CHECKING...", TEXT, &mut y);
            line(&mut c, &p.display().to_string(), DIM, &mut y);
        }
        Page::Confirm(f) => {
            // one line: a long path keeps its end
            line(&mut c, &fit(&shown(&f.path), cols), REASON, &mut y);
            line(&mut c, &f.what, TEXT, &mut y);
            line(&mut c, &format!("It takes about {} of disk space.", mb(f.bytes)), TEXT, &mut y);
            if let Some(w) = &f.warn {
                line(&mut c, w, WARN, &mut y);
            }
        }
        Page::Working => {
            let p = &s.progress;
            let what = match p.phase {
                Phase::Reading => "READING",
                Phase::Copying => "COPYING",
                Phase::Converting => "CONVERTING",
                Phase::Verifying => "VERIFYING",
                Phase::Installing => "INSTALLING",
            };
            // a conversion counts its steps, the rest bytes
            let (done, total) = if p.phase == Phase::Converting { (format!("step {}", p.done), format!("{}", p.total)) } else { (mb(p.done), mb(p.total)) };
            let disc = match s.kind {
                Kind::Br1 => " (BLOODY ROAR 1)",
                Kind::Q3 => " (QUAKE III)",
                Kind::Br2 => "",
            };
            line(&mut c, &format!("{what}{disc}  {done} of {total}"), TEXT, &mut y);
            let f = if p.total > 0 { (p.done as f32 / p.total as f32).clamp(0.0, 1.0) } else { 0.0 };
            c.rect(x, y, RIGHT, y + 16.0, [40.0, 20.0, 20.0], None);
            c.rect(x, y, x + (RIGHT - x) * f, y + 16.0, BAR, None);
            y += 26.0;
            line(&mut c, &fit(&p.file, cols), DIM, &mut y);
        }
        Page::Error { msg, .. } => line(&mut c, msg, ERROR, &mut y),
    }
    // the rows under the text; as many as fit above the hint line, scrolled to keep the cursor in view
    let mut hits = Vec::new();
    let rows = s.rows();
    // under the text, but always room for two rows above the hint (a long text is overdrawn rather than the rows lost)
    let rh = 24.0;
    let top = (y.max(200.0) + 8.0).min(368.0 - rh * rows.len().min(2) as f32);
    let fits = (((368.0 - top) / rh) as usize).max(1);
    let first = s.cursor.saturating_sub(fits - 1).min(rows.len().saturating_sub(fits));
    let mut ry = top;
    for (i, (label, _)) in rows.iter().enumerate().skip(first).take(fits) {
        let on = i == s.cursor;
        if on {
            c.rect(x - 6.0, ry - 4.0, RIGHT, ry + rh - 4.0, [120.0, 28.0, 28.0], None);
        }
        let mut t = fit(label, row_cols - 1);
        if on && caret && s.typing() {
            t.push('_');
        }
        let (more_up, more_down) = (i == first && first > 0, i + 1 == first + fits && i + 1 < rows.len());
        c.text(
            if on {
                ">"
            } else if more_up {
                "^"
            } else if more_down {
                "v"
            } else {
                " "
            },
            x - 2.0,
            ry,
            px,
            ROW_ON,
        );
        c.text(&t, x + 14.0, ry, px, if on { ROW_ON } else { ROW });
        let (a, b) = (c.at(x - 6.0, ry - 4.0), c.at(RIGHT, ry + rh - 4.0));
        // a row marked `^` / `v` scrolls when clicked or tapped (it is half the way to the rows out of view)
        let scroll = if on {
            0
        } else if more_up {
            -1
        } else if more_down {
            1
        } else {
            0
        };
        hits.push(Hit { row: i, rect: [a[0], a[1], b[0], b[1]], scroll });
        ry += rh;
    }
    let hint = match s.page {
        Page::Checking(_) => "Checking the folder's files...",
        Page::Working if cfg!(target_os = "android") => "Tap CANCEL (or Back) to stop it; the old data stays.",
        Page::Working => "Esc or CANCEL stops it; the old data stays.",
        _ if s.typing() => "Type a path, Enter to use it. Up / Down for the other rows.",
        _ if cfg!(target_os = "android") => "Tap a row, or use a gamepad. Back goes back.",
        _ if cfg!(target_arch = "wasm32") => "Click a row, or use the arrows and Enter, or a gamepad.",
        _ => "Up / Down and Enter (or a pad), or click. Esc goes back.",
    };
    c.text(hint, x, 372.0, 1.5, DIM);
    (c.out, hits)
}

/// A row as drawn: its index, its rectangle in window pixels, and whether a click or tap on it scrolls (-1 the row
/// marked `^`: the one above comes into view; 1 the row marked `v`) instead of choosing it (0).
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub row: usize,
    pub rect: [f32; 4],
    pub scroll: i8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_have_hit_boxes_inside_the_window() {
        let mut s = Setup::new("No game data yet.".into(), true, Some(super::super::screen::Act::Quit));
        for win in [[1280.0, 720.0], [800.0, 1280.0], [320.0, 200.0]] {
            let (q, hits) = draw(&s, win, "v", true);
            assert!(q.len() > 1000);
            assert_eq!(hits.len(), s.rows().len());
            for Hit { rect: h, .. } in &hits {
                assert!(h[0] >= 0.0 && h[2] <= win[0] && h[1] >= 0.0 && h[3] <= win[1] && h[1] < h[3]);
            }
        }
        s.found = (0..10).map(|k| format!("/a/very/long/path/to/some/export/number/{k}/br2_export").into()).collect();
        s.open_pick();
        let (_, hits) = draw(&s, [1280.0, 720.0], "v", true);
        // the rows that fit, scrolled to the cursor; the last one shown marked: it scrolls
        assert!(!hits.is_empty() && hits.len() < s.rows().len() && hits[0].row == 0);
        assert_eq!((hits[0].scroll, hits.last().unwrap().scroll), (0, 1));
        assert!(hits[1..hits.len() - 1].iter().all(|h| h.scroll == 0));
        s.cursor = s.rows().len() - 1;
        let (_, hits) = draw(&s, [1280.0, 720.0], "v", true);
        assert_eq!((hits.last().unwrap().row, hits.last().unwrap().scroll), (s.cursor, 0));
        assert_eq!(hits[0].scroll, -1);
        // the cursor on the marked row: no mark there, it chooses
        let k = hits[0].row;
        s.cursor = k;
        let (_, hits) = draw(&s, [1280.0, 720.0], "v", true);
        assert_eq!(hits.iter().find(|h| h.row == k).map(|h| h.scroll), Some(0));
    }

    #[test]
    fn labels_fit() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("/a/b/c/d/e/f/g", 8), "...e/f/g");
        assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
        let tree = "content://com.android.externalstorage.documents/tree/primary%3ADownload%2Fbr2_export";
        assert_eq!(shown(tree.as_ref()), "Download/br2_export");
        assert_eq!(shown("/a/b".as_ref()), "/a/b");
        let doc = "content://com.android.externalstorage.documents/document/primary%3ADownload%2Fbr2.bin";
        assert_eq!(shown(doc.as_ref()), "Download/br2.bin");
    }
}
