//! The VS screen with the announcer.
//!
//! State: none of its own: steps the serialised `Front`

use super::Tim;
use super::{Event, Front, MatchSpec, Mode, Screen};
use crate::assets::text;
use crate::output::prim::{Prim, clut, ft4};
use crate::output::sound::Queue;

impl Front {
    /// VS backgrounds (SBxx 8-bit 192×256: P1 → (512,0) CLUT (0,501), P2 → (608,0) CLUT (0,502),
    /// Stun on the right uses the mirrored SB0B) and the announcer; loading starts at once.
    pub(super) fn enter_vs(&mut self, spec: MatchSpec, audio: &mut Queue) -> Event {
        audio.stop_music();
        for side in 0..2 {
            let c = spec.chr[side] as usize;
            // engine addition (the import S9): an imported cell's picture in the SB's place and format
            // (`assets::br1_art`)
            if let Some(k) = spec.chr[side].checked_sub(crate::assets::roster::BR1_FIRST) {
                if let Some(t) = self.data.br1.vs.get(k as usize) {
                    self.gfx.vram.load(Tim::Br1Vs(k), t, Some((0, 501 + side)), Some((512 + 96 * side, 0)));
                }
                continue;
            }
            let k = if side == 1 && c == 8 { 11 } else { c };
            if let Some(t) = self.data.sb.get(k).filter(|t| !t.is_empty()) {
                self.gfx.vram.load(Tim::Sb(k as u8), t, Some((0, 501 + side)), Some((512 + 96 * side, 0)));
            }
        }
        // the VS pages of SEL00.DAT (background bands, plates, name logos, VS mark, CLUTs)
        for (k, at) in [(0x31usize, (768, 256)), (0x32, (640, 256)), (0x30, (0, 480)), (0x34, (704, 0)), (0x35, (704, 256))] {
            if let Some(Some(t)) = self.data.sel00.get(k) {
                self.gfx.vram.load(Tim::Sel00(k as u8), t, None, Some(at));
            }
        }
        self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        self.screen = Screen::Vs { t: 0, ready: false, spec };
        log::info!("front: VS {spec:?}");
        // announcer (CHASEL_2, ids 0x80 + tone; three echoes at +0 / +7 / +14)
        let mut t = 0;
        for c in spec.chr.map(|c| c as u32) {
            // engine addition (the import S9): no name call for an imported fighter (the main game's announcer
            // has none)
            if crate::assets::roster::is_br1(c as u8) {
                continue;
            }
            for e in 0..3 {
                self.play(audio, (0x80 + 6 * c + e) as u16, t + 7 * e);
            }
            t += crate::assets::game_tables::get().announcer_delays[c as usize].0;
            let k = if c == 10 { 2 } else { c };
            for e in 0..3 {
                self.play(audio, (0x83 + 6 * k + e) as u16, t + 7 * e);
            }
            t += crate::assets::game_tables::get().announcer_delays[c as usize].1;
        }
        Event::Load(spec)
    }

    /// VS screen (primitives as dumped from the game under emulation): in the 512×240 mode, so
    /// every y is doubled here. OT 22 background bands, 20 the SB portraits (P2 mirrored), 16 plates and name
    /// logos, 15 the stage banner and the VS mark.
    pub(super) fn draw_vs(&self, out: &mut Vec<Prim>, spec: &MatchSpec) {
        let mut v: Vec<Prim> = Vec::new();
        let mirror = |p: &mut Prim, x0: f32, x1: f32| {
            for k in 0..4 {
                p.xy[k][0] = x0 + x1 - p.xy[k][0];
            }
        };
        for k in 0..8 {
            vs_band(&mut v, k);
        }
        // engine addition: the bands (mirrored halves) repeat out to the window edges, above and below too
        crate::output::layout::mirror_tiles(&mut v, 0, [512.0, 240.0], [2, 1]);
        // portraits: P1 (512,0) 8-bit, P2 (608,0) across pages 0x88 / 0x8A, mirrored
        v.push(ft4(33, 56, 191, 127, (0, 0, 191, 255), 0x88, clut(0, 501)));
        let mut a = ft4(288, 56, 128, 127, (0, 0, 127, 255), 0x8A, clut(0, 502));
        mirror(&mut a, 288.0, 416.0);
        v.push(a);
        let mut b = ft4(416, 56, 64, 127, (192, 0, 63, 255), 0x88, clut(0, 502));
        mirror(&mut b, 416.0, 480.0);
        v.push(b);
        // name plates and logos: logos centred on (128, 192) / (384, 192)
        const LOGO: [(i32, i32, i32, i32, u16); 11] = [
            (0, 160, 104, 48, 0x1C),
            (0, 96, 112, 48, 0x1A),
            (104, 160, 112, 48, 0x1C),
            (144, 48, 111, 48, 0x1A),
            (0, 48, 144, 48, 0x1A),
            (0, 0, 200, 48, 0x1A),
            (0, 208, 176, 47, 0x1C),
            (112, 96, 136, 48, 0x1A),
            (0, 144, 104, 48, 0x1A),
            (104, 144, 112, 48, 0x1A),
            (0, 112, 208, 48, 0x1C),
        ];
        for side in [1usize, 0] {
            let cx = if side == 0 { 128 } else { 384 };
            v.push(ft4(cx - 98, 184, 196, 16, (0, 192, 196, 32), 0x1A, clut(480, if side == 0 { 500 } else { 505 })));
            // engine addition (the import S9): an imported fighter's name in the menu font on its plate (the
            // main game's name logos have none), drawn as the stage banner is
            if crate::assets::roster::is_br1(spec.chr[side]) {
                let (f, name) = (crate::output::font::fonts(), crate::assets::roster::name(spec.chr[side]));
                f.menu_text(&mut v, name, cx - f.menu_width(name) / 2, 184, 5);
                continue;
            }
            let (u, vv, w, h, tp) = LOGO[(spec.chr[side] as usize).min(10)];
            v.push(ft4(cx - w / 2, 180, w, h / 2, (u, vv, w, h), tp, clut(480, 505)));
        }
        // the stage banner: font 4, palette 4, centred at y = 240 / 16
        if let Some((text, w)) = self.vs_banner() {
            crate::output::font::fonts().menu_text(&mut v, &text, (512 - w) / 2, 15, 4);
        }
        v.push(ft4(220, 126, 72, 32, (0, 0, 72, 64), 0x1C, clut(496, 502)));
        for mut p in v {
            for k in 0..4 {
                p.xy[k][1] *= 2.0;
            }
            out.push(p);
        }
    }

    /// The banner text of the VS screen by game mode and fight number (with the widths it centres).
    pub(super) fn vs_banner(&self) -> Option<(String, i32)> {
        let n = self.run.a2 + 1;
        Some(match self.run.mode {
            Mode::Arcade if n == 9 => (text::get("special_stage").into(), 176),
            Mode::Arcade if n >= 8 => (text::get("final_stage").into(), 144),
            Mode::Arcade => (text::fill(text::get("stage_n"), &[&n]), 96),
            Mode::Story if n >= 8 => (text::get("final_fight").into(), 144),
            Mode::Custom(3) => (text::item("screen_title", 23).into(), 112),
            Mode::Demo => return None,
            _ => (text::fill(text::get("fight_n"), &[&n]), 96),
        })
    }
}

/// A VS background band × 2 halves, the right one mirrored: 60-line strips of pages 0x0B / 0x1B
/// squeezed to 30 (240-line units).
pub(super) fn vs_band(v: &mut Vec<Prim>, k: usize) {
    const BANDS: [(i32, i32, i32, u16); 8] = [
        (0, 30, 0, 0x0B),
        (30, 30, 60, 0x0B),
        (60, 30, 120, 0x0B),
        (90, 30, 180, 0x0B),
        (120, 30, 0, 0x1B),
        (150, 30, 60, 0x1B),
        (179, 32, 120, 0x1B),
        (210, 30, 180, 0x1B),
    ];
    let (y, h, vv, tp) = BANDS[k];
    v.push(ft4(0, y, 256, h, (0, vv, 255, 60), tp, clut(432, 480)));
    let mut r = ft4(256, y, 256, h, (0, vv, 255, 60), tp, clut(432, 480));
    for c in 0..4 {
        r.xy[c][0] = 768.0 - r.xy[c][0];
    }
    v.push(r);
}
