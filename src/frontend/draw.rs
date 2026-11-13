//! The front end's frame as PS1 primitives (`Front::draw`).
//!
//! State: none of its own: reads the serialised `Front`
//!
//! The front end's frame as PS1 primitives (`draw`): one arm per `Screen`, the larger screens' drawing in their
//! own files.

use super::story_flow::StoryScreen;
use super::{Front, Screen};
use crate::assets::roster::br2_names;
use crate::output::font;
use crate::output::prim::{self, Prim, clut, ft4};

impl Front {
    pub fn draw(&self) -> Vec<Prim> {
        let mut out = Vec::new();
        match &self.screen {
            Screen::Title { menu, .. } => self.draw_title(&mut out, *menu),
            Screen::TitleFade { k } => {
                // a subtract F4 of (8, 8, 8) over the uncleared previous frame, every frame
                self.draw_title(&mut out, false);
                // in the ambient fill's copy too (engine addition)
                out.push(prim::fade_sub((8 * (k + 1)).min(255) as u8).at(crate::output::layout::Anchor::AMBIENT));
            }
            Screen::Logo { n, k } => self.draw_logo(&mut out, *n, *k),
            Screen::Ranking { k } | Screen::RankFade { at: k, .. } => {
                self.draw_ranking(&mut out, *k);
                match self.screen {
                    Screen::RankFade { k, .. } => out.push(prim::fade_sub((8 * (k + 1)).min(255) as u8)),
                    // 48 of every 128 frames
                    _ if (self.frame >> 4) & 3 != 0 && self.frame & 0x40 != 0 => {
                        font::fonts().small_text(&mut out, crate::assets::text::get("press_start_or_x_button"), 187, 420, 4)
                    }
                    _ => {}
                }
            }
            Screen::Select(sel) => {
                self.draw_select(&mut out, sel);
                // engine addition: a tall window's bars above and below get the ambient fill of the whole screen
                for p in &mut out {
                    p.at.ambient = crate::output::layout::Ambient::TopBottom;
                }
            }
            Screen::Vs { spec, .. } => self.draw_vs(&mut out, spec),
            // the team battle's screens (engine addition, `team`)
            Screen::TeamMode { cursor, banner_x } => self.draw_team_mode(&mut out, *cursor, *banner_x),
            Screen::TeamSelect(sel) | Screen::TeamStage { sel, .. } => {
                let stage = if let Screen::TeamStage { row, .. } = self.screen { Some(row) } else { None };
                self.draw_team_select(&mut out, sel, stage);
                for p in &mut out {
                    p.at.ambient = crate::output::layout::Ambient::TopBottom;
                }
            }
            Screen::TeamLoad { spec, .. } => self.draw_team_load(&mut out, spec),
            Screen::TeamFight(_) => {}
            Screen::TeamResult { spec, winner, rounds, cursor, banner_x } => self.draw_team_result(&mut out, spec, *winner, rounds, *cursor, *banner_x),
            Screen::TeamArcadeClear { .. } => self.draw_team_arcade_clear(&mut out),
            Screen::Fight(_) => {}
            // the demo's loading: DEMONSTRATION on black
            Screen::DemoLoad { .. } => font::fonts().small_text(&mut out, crate::assets::text::get("demonstration"), 32, 32, 1),
            Screen::NameEntry(e) => e.draw(font::fonts(), &self.save.settings.ranking[e.table], &br2_names(), &mut out),
            Screen::NameFade { e, k } => {
                e.draw(font::fonts(), &self.save.settings.ranking[e.table], &br2_names(), &mut out);
                out.push(prim::fade_sub((8 * (k + 1)).min(255) as u8));
            }
            Screen::Menu { menu, .. } => {
                menu.draw(font::fonts(), &mut out);
                if let Some(c) = &self.save.card {
                    c.draw(font::fonts(), &mut out);
                }
            }
            // no frame, menu or emblem: the text on black
            Screen::AutoSave => {
                if let Some(c) = &self.save.card {
                    c.draw(font::fonts(), &mut out);
                }
            }
            Screen::Movie { .. } => self.draw_movie(&mut out),
            Screen::Story => match &self.story_screen {
                Some(StoryScreen::Prologue(p)) => p.draw(font::fonts(), &mut out),
                Some(StoryScreen::Pages(p)) => p.draw(font::fonts(), &mut out),
                Some(StoryScreen::Epilogue(e)) => e.draw(&mut out),
                None => {}
            },
            Screen::Picture { ready, .. } => {
                out.push(prim::f4(0, 0, 512, 480, [0, 0, 0]));
                if *ready {
                    // engine addition: the window around the picture gets its ambient fill
                    out.push(ft4(0, 0, 512, 480, (0, 0, 256, 240), 0x88, clut(0, 511)).at(crate::output::layout::Anchor::AMBIENT));
                }
            }
            Screen::Message { text, .. } => {
                let text = text.str();
                let w = font::fonts().menu_width(text);
                font::fonts().menu_text(&mut out, text, 256 - w / 2, 232, 0);
            }
            Screen::StoryGameOver { k, .. } => {
                // message 0x16 alone on black: the big font, palette 0xA, at the fixed (W/2 − 0x63, H/2 − 0x10)
                font::fonts().big_text(&mut out, crate::assets::text::get("game_over"), 256 - 0x63, 480 / 2 - 0x10, 0xA);
                // the fade over the uncleared last frame (as TitleFade)
                if let Some(k) = k {
                    out.push(prim::fade_sub((8 * (k + 1)).min(255) as u8));
                }
            }
        }
        out
    }
}
