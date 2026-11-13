//! The overlay drawn over the game: state lines, key help, panels, toasts.
//!
//! The overlay drawn over the game: the state lines and key help (`state_text`, `overlay_prims`), the panels at
//! the right edge (`panel`), the toast, the line wrap (`wrap`); the front end's primitives or the replay list
//! over them, or the NETWORK menu (`front_prims`).

use super::{App, net_menu, replay_menu};
use crate::output::layout::{Anchor, Ax};
use crate::output::sprite;
use crate::platform::support;
use crate::{frontend, output};

/// Split text into lines of at most `n` characters at spaces (the overlay's 8-px font in 512 px); a longer word
/// (a path) is cut into lines of `n`.
pub(super) fn wrap(text: &str, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    for l in text.lines() {
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

/// The front end's screen, or the replay list or the NETWORK menu over it.
pub(super) fn front_prims(browser: &Option<replay_menu::Browser>, net: &Option<net_menu::NetMenu>, f: &frontend::Front) -> Vec<output::prim::Prim> {
    let mut p = Vec::new();
    match (browser, net) {
        (Some(b), _) => b.draw(output::font::fonts(), &mut p),
        (None, Some(m)) => m.draw(output::font::fonts(), &mut p),
        (None, None) => return f.draw(),
    }
    p
}

impl App {
    /// What is running now, for the overlay and the support report.
    pub(super) fn state_text(&self) -> String {
        let mut v =
            vec![format!("{} fps {:.0}{}{}", support::VERSION, self.fps.value, if self.fast { " x4" } else { "" }, if self.paused { " PAUSED" } else { "" })];
        v.extend(self.net_status());
        if let Some(f) = &self.state.front {
            v.push(format!("FRONT {}", f.describe()));
            v.push(format!("settings {:?} (saved at {})", f.save.settings, crate::platform::storage::save_location()));
        }
        if let Some(spec) = &self.state.spec {
            v.push(format!("last match {spec:?}"));
        }
        if let Some(g) = self.game() {
            v.extend(g.debug_lines());
        }
        v.join("\n")
    }

    /// Overlay primitives (512×480 space): state lines on a dimmed band, the key help, the toast.
    pub(super) fn overlay_prims(&self) -> Vec<output::prim::Prim> {
        let mut out = Vec::new();
        if self.overlay {
            let mut text = format!("{}\n{}", self.state_text(), support::HELP);
            if self.tools.is_some() {
                text = format!("{text}\n{}", support::TOOLS_HELP);
            }
            let lines = wrap(&text, 62);
            // darkened band: subtractive blend
            // the band spans the window (its edges on the core border move out), the text at the top-left corner
            out.push(output::prim::f4(0, 0, 512, 8 + 16 * lines.len() as i32, [110, 110, 110]).blend(sprite::Blend::Sub));
            let s0 = out.len();
            for (k, l) in lines.iter().enumerate() {
                output::font::fonts().small_text(&mut out, l, 4, 4 + 16 * k as i32, output::font::pal::GREY);
            }
            output::layout::tag(&mut out[s0..], Anchor::new(Ax::Start, Ax::Start));
        }
        // the performance panel at the window's right edge, under the fight HUD's top rows; the debug tools' under it
        let mut y = 140;
        if self.perf.on && !self.perf.lines.is_empty() {
            y += self.panel(&mut out, &self.perf.lines, y, output::font::pal::GREEN) + 4;
        }
        let tools = self.tool_lines();
        if !tools.is_empty() {
            y += self.panel(&mut out, &tools, y, output::font::pal::YELLOW) + 4;
        }
        let net = self.net_lines();
        if !net.is_empty() {
            self.panel(&mut out, &net, y, output::font::pal::YELLOW);
        }
        self.playback_prims(&mut out);
        if let Some((t, _)) = &self.toast {
            // a long one (a replay's path) on more lines, upwards
            let lines = wrap(t, 62);
            let y0 = 452 - 16 * (lines.len() as i32 - 1);
            let s0 = out.len();
            out.push(output::prim::f4(0, y0, 512, 474 - y0, [110, 110, 110]).blend(sprite::Blend::Sub).at(Anchor::new(Ax::Center, Ax::End)));
            for (k, l) in lines.iter().enumerate() {
                output::font::fonts().small_text(&mut out, l, 8, y0 + 4 + 16 * k as i32, output::font::pal::YELLOW);
            }
            output::layout::tag(&mut out[s0 + 1..], Anchor::new(Ax::Start, Ax::End));
        }
        out
    }

    /// Text lines on a darkened box at the window's right edge from `y` (512×480 space); its height.
    fn panel(&self, out: &mut Vec<output::prim::Prim>, lines: &[String], y: i32, pal: u16) -> i32 {
        let w = lines.iter().map(|l| output::font::fonts().width(&output::font::fonts().small, l)).max().unwrap_or(0) + 8;
        let (x, h) = (512 - 4 - w, 8 + 16 * lines.len() as i32);
        let s0 = out.len();
        out.push(output::prim::f4(x, y, w, h, [110, 110, 110]).blend(sprite::Blend::Sub));
        for (k, l) in lines.iter().enumerate() {
            output::font::fonts().small_text(out, l, x + 4, y + 4 + 16 * k as i32, pal);
        }
        output::layout::tag(&mut out[s0..], Anchor::new(Ax::End, Ax::Start));
        h
    }
}
