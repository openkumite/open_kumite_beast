//! The title and mode select, the boot logos, the attract demo and ranking.
//!
//! State: none of its own: steps the serialised `Front`
//!
//! The title and mode select, the boot logos, the attract demo and
//! the attract ranking.

use super::Tim;
use super::progress::resolve_costumes;
use super::{Event, Front, MODES, MatchSpec, Mode, Screen, mode_half};
use crate::output::layout::{Anchor, tag};
use crate::output::prim::{self, Prim, clut, ft4};
use crate::output::sound::Queue;

impl Front {
    pub(super) fn enter_title(&mut self, audio: &mut Queue) {
        self.title_visits = self.title_visits.saturating_add(1);
        self.screen = Screen::Title { menu: false, timer: 360 };
        audio.stop_music();
        // USATITLE entries 0..3 (8-bit 256×240) with their CLUTs at (0,496..499)
        for (k, pix) in [(512, 0), (640, 0), (576, 256), (704, 256)].iter().enumerate() {
            if let Some(Some(t)) = self.data.title.get(k) {
                self.gfx.vram.load(Tim::Title(k as u8), t, Some((0, 496 + k)), Some(*pix));
            }
        }
        self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        self.play(audio, 0x82, 0);
        self.play(audio, 0x83, 115);
    }

    /// Main mode 5: the auto-save when AUTO SAVE is ON and opt changed, then the title.
    /// The engine's silent saves keep the progress either way.
    #[allow(clippy::wrong_self_convention)] // "go to the title", not a conversion
    pub(super) fn to_title(&mut self, audio: &mut Queue) {
        if self.save.settings.auto_save != 0 && self.save.dirty {
            self.save.dirty = false;
            self.save.card = Some(crate::frontend::memcard::CardDialog::auto());
            self.screen = Screen::AutoSave;
            return;
        }
        self.enter_title(audio);
    }

    /// main mode 2 → 3: the attract loop restarts (s3 = 0) with the SCEA and HUDSON logos.
    pub(super) fn enter_logos(&mut self) {
        self.title_visits = 0;
        self.enter_logo(3);
    }

    /// The boot logo n, substate 0: the logo's TIM, pixels at (768, 0), CLUT at (0, 500).
    pub(super) fn enter_logo(&mut self, n: u8) {
        let (tim, src) = if n == 3 { (&self.data.scea, Tim::Scea) } else { (&self.data.hudson, Tim::Hudson) };
        if let Some(t) = tim {
            self.gfx.vram.load(src, t, Some((0, 500)), Some((768, 0)));
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
        self.screen = Screen::Logo { n, k: 0 };
    }

    /// State 0: the attract demo — random characters (0..8) in their first costumes on a random stage
    /// (0..4), both CPU-controlled.
    pub(super) fn enter_demo(&mut self, audio: &mut Queue) -> Event {
        audio.stop_music();
        self.run.mode = Mode::Demo;
        self.run.team_ladder = None;
        self.run.humans = [false, false];
        // engine addition (the import S10): an ARCADE run's imported boss mark ends with the run
        self.run.br1_boss = 0;
        let a = (self.rand() % 9) as u8;
        let b = (self.rand() % 9) as u8;
        let stage = (self.rand() % 5) as u8;
        let mut spec = MatchSpec { chr: [a, b], costume: [0, 0], stage, cpu: [true, true] };
        resolve_costumes(&mut spec);
        self.screen = Screen::DemoLoad { ready: false, spec };
        log::info!("front: attract demo {spec:?}");
        Event::Load(spec)
    }

    /// The boot logo per frame: the logo (8-bit, tpage 0x8C, CLUT (0, 500)) under a full-screen subtract fade;
    /// frames 0 and 239 are black.
    pub(super) fn draw_logo(&self, out: &mut Vec<Prim>, n: u8, k: i32) {
        out.push(prim::f4(0, 0, 512, 480, [0, 0, 0]));
        if k == 0 || k == 239 {
            return;
        }
        let c = clut(0, 500);
        if n == 3 {
            // SCEA: the 512×64 text stored as two 256×64 strips
            out.push(ft4(0, 208, 256, 64, (0, 0, 255, 64), 0x8C, c));
            out.push(ft4(256, 208, 256, 64, (0, 64, 255, 64), 0x8C, c));
        } else {
            // engine addition: the window around the picture gets its ambient fill
            out.push(ft4(0, 0, 512, 480, (0, 0, 255, 240), 0x8C, c).at(Anchor::AMBIENT));
        }
        let s1 = 239 - k;
        let f = if k < 60 {
            255 * (s1 - 180) / 60
        } else if k >= 180 {
            255 - 255 * s1 / 60
        } else {
            0
        };
        if f > 0 {
            // the ambient fill copies the faded picture (the darker bars would reach black first)
            out.push(prim::fade_sub(f as u8).at(if n == 3 { Anchor::CORE } else { Anchor::AMBIENT }));
        }
    }

    /// Title: four 256×240 backdrop sprites, the copyright lines, then PRESS START or the mode spinner.
    /// The attract ranking's text at frame `k` (font 2): block A = RANKING (table 0, one row per
    /// frame), block B = VS RANKING (table 1, all rows, off-screen right); frames 599..726 slide both 4 px left.
    pub(super) fn draw_ranking(&self, out: &mut Vec<Prim>, k: i32) {
        let f = crate::output::font::fonts();
        let start = out.len();
        let a = -4 * (k - 598).clamp(0, 128);
        let b = 512 + a;
        let name = |r: &crate::frontend::options::RankRec| -> String { r.name.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect() };
        // an imported fighter's record (engine addition, the import S10): its name (`roster::name`)
        let chr = |r: &crate::frontend::options::RankRec| crate::assets::roster::name(r.chr);
        f.mid_text(out, crate::assets::text::get("ranking"), 207 + a, 32, 1);
        for (i, r) in self.save.settings.ranking[0].iter().take((k + 1).clamp(0, 5) as usize).enumerate() {
            let y = 96 + 64 * i as i32;
            f.mid_text(out, &format!("{}", i + 1), 56 + a, y, 3);
            f.mid_text(out, crate::assets::text::item("ordinal", i), 72 + a, y, 4);
            f.mid_text(out, &crate::frontend::ranking::time_text(r.value), 128 + a, y, 1);
            f.mid_text(out, chr(r), 272 + a, y, 1);
            f.mid_text(out, &name(r), 416 + a, y, 4);
        }
        f.mid_text(out, crate::assets::text::get("vs_ranking"), 186 + b, 32, 1);
        for (i, r) in self.save.settings.ranking[1].iter().take(5).enumerate() {
            let y = 96 + 64 * i as i32;
            let w = format!("{}", r.c as u16 % 1000);
            f.mid_text(out, &format!("{}", i + 1), 56 + b, y, 3);
            f.mid_text(out, crate::assets::text::item("ordinal", i), 72 + b, y, 4);
            f.mid_text(out, &w, 112 + (3 - w.len() as i32) * 16 + b, y, 4);
            f.mid_text(out, crate::assets::text::get("wins"), 160 + b, y, 1);
            f.mid_text(out, chr(r), 272 + b, y, 1);
            f.mid_text(out, &name(r), 416 + b, y, 4);
        }
        // the 512-wide screen: the sliding blocks are cut at its edges (the core scissor)
        tag(&mut out[start..], Anchor::CLIP);
    }

    /// The demo's stage behind the attract ranking (from frame 5): the free camera, None = black.
    pub fn backdrop_cam(&self) -> Option<&crate::fight::camera::Camera> {
        match self.screen {
            Screen::Ranking { k } | Screen::RankFade { at: k, .. } if k >= 5 => Some(&self.rank_cam),
            _ => None,
        }
    }

    /// The attract ranking is up (the app keeps the demo's stage loaded meanwhile).
    pub fn wants_backdrop(&self) -> bool {
        matches!(self.screen, Screen::Ranking { .. } | Screen::RankFade { .. })
    }

    pub(super) fn draw_title(&self, out: &mut Vec<Prim>, menu: bool) {
        for (k, (x, y, tp)) in [(0, 0, 0x88u16), (256, 0, 0x8A), (0, 240, 0x99), (256, 240, 0x9B)].iter().enumerate() {
            out.push(ft4(*x, *y, 256, 240, (0, 0, 256, 240), *tp, clut(0, 496 + k as u16)).at(Anchor::AMBIENT));
        }
        let cr = clut(496, 492);
        for (x, y, w, u, v) in [
            (116, 392, 152, 16, 184),
            (268, 393, 128, 56, 144),
            (164, 408, 184, 0, 144),
            (165, 424, 58, 64, 168),
            (224, 424, 24, 228, 144),
            (252, 424, 94, 122, 168),
        ] {
            out.push(ft4(x, y, w, 16, (u, v, w, 16), 0x0F, cr));
        }
        if menu {
            let i = self.mode_item();
            out.push(ft4(248, 344, 16, 16, (0, 232, 16, 16), 0x1E, clut(352, 480)));
            out.push(ft4(248, 376, 16, 16, (16, 232, 16, 16), 0x1E, clut(352, 480)));
            crate::output::font::fonts().menu_text(out, MODES[i].text(), 256 - mode_half(i), 360, 0);
        } else if (self.frame >> 4) & 3 != 0 {
            crate::output::font::fonts().small_text(out, crate::assets::text::get("press_start_or_x_button"), 256 - 69, 360, 4);
        }
    }
}
