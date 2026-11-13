//! One front-end frame: `Front::tick` and the dispatch over `Screen`.
//!
//! State: none of its own: steps the serialised `Front`
//!
//! One front-end frame: `tick` and the dispatch over `Screen` (`tick_screen`): the title and mode select,
//! the logos, the attract ranking, the VS screen, NAME ENTRY, the messages,
//! STORY's GAME OVER, the menus, the auto-save, movies, story stills and pictures; the larger screens'
//! steps are in their own files (`title`, `select`, `progress`, `movie`, `story_flow`, `menus::flow`).

use super::movie::After;
use super::screen::{EXIT_ITEM, NETWORK_ITEM, REPLAY_ITEM, TEAM_ITEM};
use super::{Event, Front, Mode, Screen, menus, mode_rows};
use crate::fight::input::pad::{CROSS, DOWN, LEFT, RIGHT, START, TRIANGLE, UP};
use crate::output::sound::Queue;

impl Front {
    /// One frame; `files` = the fetched files the app holds (the story cache's bytes); `sim_version` the session's.
    /// `ext`: pads 3 / 4 (engine addition: the team battle's screens read them).
    pub fn tick(&mut self, raw: [u16; 2], ext: [u16; 2], audio: &mut Queue, files: &crate::session::FileCache, sim_version: u32) -> Option<Event> {
        self.sim_version = sim_version;
        self.frame += 1;
        self.pads.update(raw);
        self.ext.update(ext);
        let ev = self.tick_screen(audio, files);
        ev.or_else(|| self.pending_event.take())
    }

    /// The SE mode at the main modes' entries: modes 3..7 (the logos, the opening movie, the title with its auto-save,
    /// the demo, the ranking) opt.20 ? 1 : 2; the list menus (mode 0xB), the games and the endings 1.
    fn se_mode(&self) -> u16 {
        let attract = match &self.screen {
            Screen::Title { .. } | Screen::TitleFade { .. } | Screen::Logo { .. } | Screen::Ranking { .. } | Screen::RankFade { .. } => true,
            Screen::DemoLoad { .. } | Screen::AutoSave | Screen::Movie { then: After::Boot, .. } => true,
            Screen::Fight(_) => self.run.mode == Mode::Demo,
            _ => false,
        };
        if attract && self.save.settings.attract_sound == 0 { 2 } else { 1 }
    }

    fn tick_screen(&mut self, audio: &mut Queue, files: &crate::session::FileCache) -> Option<Event> {
        audio.set_se_mode(self.se_mode());
        let press = self.pads.any_press();
        match self.screen {
            Screen::Title { menu: false, timer } => {
                self.screen = Screen::Title { menu: false, timer: timer - 1 };
                if press & (START | CROSS) != 0 {
                    self.screen = Screen::Title { menu: true, timer };
                    self.play(audio, 0x0C, 0);
                    return None;
                }
                if timer <= 1 {
                    // state 0xF0: SE 0x204 (stop), the fade to black
                    audio.stop_music();
                    self.screen = Screen::TitleFade { k: 0 };
                }
                None
            }
            Screen::TitleFade { k } => {
                if k + 1 < 33 {
                    self.screen = Screen::TitleFade { k: k + 1 };
                    return None;
                }
                // main mode 5: s3 < 4 → the demo (mode 6), else mode 2 (the logos, s3 = 0)
                if self.title_visits < 4 {
                    return Some(self.enter_demo(audio));
                }
                self.enter_logos();
                None
            }
            Screen::Logo { n, k } => {
                // 240 frames, no skip (the game's skip is a stub); then SE 0x204 and the next logo / the opening movie.
                // Engine addition: START / × skips the logos and the opening movie straight to the title (the movie's
                // own end path: After::Boot → boot_vram + to_title).
                if press & (START | CROSS) != 0 {
                    self.boot_vram();
                    self.to_title(audio);
                    return None;
                }
                if k + 1 < 240 {
                    self.screen = Screen::Logo { n, k: k + 1 };
                    return None;
                }
                if n == 3 {
                    self.enter_logo(1);
                    return None;
                }
                self.boot_vram();
                Some(self.play_movie(0, After::Boot, audio))
            }
            Screen::Ranking { k } => {
                // START / × → the title, △ or the end → the logos (main mode 3, s3 kept)
                if press & 0x840 != 0 {
                    self.play(audio, 0x0C, 0);
                    self.screen = Screen::RankFade { k: 0, to_title: true, at: k };
                    return None;
                }
                if press & 0x10 != 0 || k >= 1199 {
                    if press & 0x10 != 0 {
                        self.play(audio, 0x0D, 0);
                    }
                    self.screen = Screen::RankFade { k: 0, to_title: false, at: k };
                    return None;
                }
                let k = k + 1;
                if k >= 5 {
                    // from frame 5: the camera update (mode 0x1E), then target yaw + 1
                    self.rank_cam.turntable();
                    self.rank_cam.angt[1] += 1;
                }
                self.screen = Screen::Ranking { k };
                None
            }
            Screen::RankFade { k, to_title, at } => {
                // state 0xF0: the 33-frame fade of the ranking, then the title or the logos (main mode 3)
                if k + 1 < 33 {
                    self.screen = Screen::RankFade { k: k + 1, to_title, at };
                    return None;
                }
                audio.stop_music();
                if to_title {
                    self.to_title(audio);
                } else {
                    self.enter_logo(3);
                }
                None
            }
            Screen::DemoLoad { ready, spec } => {
                if ready {
                    self.screen = Screen::Fight(spec);
                    return Some(Event::Fight);
                }
                None
            }
            Screen::Title { menu: true, timer } => {
                if press & TRIANGLE != 0 {
                    self.screen = Screen::Title { menu: false, timer };
                    self.play(audio, 0x0D, 0);
                    return None;
                }
                let rep = self.pads.any_rep();
                let count = mode_rows(self.gates(), self.browser, self.network, self.netplay).len();
                if rep & DOWN != 0 {
                    self.menu_cursor = (self.menu_cursor + 1) % count;
                    self.play(audio, 0x0A, 0);
                } else if rep & UP != 0 {
                    self.menu_cursor = (self.menu_cursor + count - 1) % count;
                    self.play(audio, 0x0A, 0);
                }
                let item = self.mode_item();
                let mode =
                    [Some(Mode::Arcade), Some(Mode::Versus), Some(Mode::Story), Some(Mode::Survival), Some(Mode::Watch), None, None, None, None, None, None]
                        [item];
                for port in 0..2 {
                    if self.pads.press[port] & (START | CROSS) != 0 {
                        match mode {
                            Some(m) => {
                                self.play(audio, 0x80, 0); // TITLE.SND tone 0 while on the title
                                self.start_mode(m, port, audio);
                            }
                            // engine addition: EXIT GAME quits at once (no confirmation)
                            None if item == EXIT_ITEM => {
                                self.play(audio, 0x0C, 0);
                                return Some(Event::Exit);
                            }
                            // engine addition (v2): REPLAY opens the app's replay list in the list menus' frame (the
                            // next emblem, as they do); the mode menu stays up under it
                            None if item == REPLAY_ITEM => {
                                self.play(audio, 0x0C, 0);
                                self.next_emblem();
                                return Some(Event::Replays);
                            }
                            // engine addition (v22): TEAM BATTLE opens its mode choice in the list menus' frame
                            None if item == TEAM_ITEM => {
                                self.play(audio, 0x0C, 0);
                                self.enter_team_mode(port, audio);
                            }
                            // engine addition: NETWORK opens the app's NETWORK menu in the list menus' frame (the next
                            // emblem, as REPLAY), the mode menu staying up under it; in a netplay session it leaves
                            // the session (both sides step the same press)
                            None if item == NETWORK_ITEM => {
                                self.play(audio, 0x0C, 0);
                                self.next_emblem();
                                return Some(Event::Network);
                            }
                            None => {
                                self.play(audio, 0x0C, 0);
                                let k = if item == 5 { menus::Kind::Custom } else { menus::Kind::Options };
                                self.open_menu(k, audio);
                            }
                        }
                        return None;
                    }
                }
                None
            }
            Screen::Select(_) => self.tick_select(audio),
            // the team battle's screens (engine addition, `team`)
            Screen::TeamMode { cursor, banner_x } => self.tick_team_mode(cursor, banner_x, audio),
            Screen::TeamSelect(_) => self.tick_team_select(audio),
            Screen::TeamStage { sel, row } => self.tick_team_stage(sel, row, audio),
            Screen::TeamLoad { t, ready, spec } => self.tick_team_load(t, ready, spec, audio),
            Screen::TeamFight(_) => None,
            Screen::TeamResult { spec, winner, rounds, cursor, banner_x } => self.tick_team_result(spec, winner, rounds, cursor, banner_x, audio),
            Screen::TeamArcadeClear { t } => self.tick_team_arcade_clear(t, audio),
            Screen::Vs { t, ready, spec } => {
                // the PS1 starts the fight as soon as loading is done; keep the announcer audible here
                if ready && t + 1 >= 150 {
                    self.screen = Screen::Fight(spec);
                    self.count_plays(&spec);
                    return Some(Event::Fight);
                }
                self.screen = Screen::Vs { t: t + 1, ready, spec };
                None
            }
            Screen::Fight(_) => None,
            Screen::NameEntry(e) => {
                // the player's pad: P1, or P2 when only P2 plays (M.B4 / M.B6 & 0xF00 == 0xA00)
                let p = usize::from(self.run.humans[1] && !self.run.humans[0]);
                self.tick_name(e, [self.pads.rep[p], self.pads.press[p], press], audio)
            }
            Screen::NameFade { e, k } => {
                if k + 1 < 33 {
                    self.screen = Screen::NameFade { e, k: k + 1 };
                    return None;
                }
                self.to_title(audio);
                None
            }
            Screen::Message { t, text, to_custom } => {
                if t <= 1 || (t < 200 && press & (START | CROSS) != 0) {
                    if let Some(e) = self.run.pending_entry.take() {
                        return self.enter_name(e, audio);
                    }
                    if to_custom {
                        self.open_menu(menus::Kind::Custom, audio);
                    } else if self.run.mode == Mode::TeamArcade {
                        // engine addition (TEAM ARCADE): its GAME OVER and CONGRATULATIONS end on the mode menu
                        self.team_to_menu(audio);
                    } else {
                        self.to_title(audio);
                    }
                } else {
                    self.screen = Screen::Message { t: t - 1, text, to_custom };
                }
                None
            }
            Screen::StoryGameOver { t, k: None } => {
                // until s7 = −1 or a □×○ press (from the first frame; not START), then SE 0x204
                // (music stop) and the fade
                let t = t - 1;
                self.screen = if t < 0 || press & 0xE0 != 0 {
                    audio.stop_music();
                    Screen::StoryGameOver { t, k: Some(0) }
                } else {
                    Screen::StoryGameOver { t, k: None }
                };
                None
            }
            Screen::StoryGameOver { t, k: Some(k) } => {
                // the game loop's sub-state 9: the fade (1, 8) for 33 frames, then main mode 5 (the title)
                if k + 1 < 33 {
                    self.screen = Screen::StoryGameOver { t, k: Some(k + 1) };
                } else {
                    self.to_title(audio);
                }
                None
            }
            Screen::Menu { menu, parent } => self.tick_menu(menu, parent, audio),
            Screen::AutoSave => {
                let Some(mut card) = self.save.card else {
                    self.enter_title(audio);
                    return None;
                };
                let mut sounds = Vec::new();
                let out = card.tick(press, self.pads.any_rep(), &mut self.save.settings, &mut self.save.dirty, &mut self.save.memory_card, &mut sounds);
                for id in sounds {
                    self.play(audio, id, 0);
                }
                self.save.card = Some(card);
                if out != crate::frontend::memcard::Out::Stay {
                    self.save.card = None;
                    self.enter_title(audio);
                }
                None
            }
            Screen::Movie { then, .. } => {
                if self.tick_movie(press, audio) {
                    self.after_movie(then, audio);
                }
                None
            }
            Screen::Story => self.tick_story(press, audio, files),
            Screen::Picture { n, ready, menu, mut parent } => {
                if !ready {
                    return None;
                }
                // LEFT / RIGHT: the previous / next unlocked picture; △ □ × ○ back to the menu
                let mut menu = menu;
                let count = menu.picture_count().max(1);
                let step = if self.pads.any_rep() & RIGHT != 0 {
                    1
                } else if self.pads.any_rep() & LEFT != 0 {
                    count - 1
                } else {
                    0
                };
                if step != 0 {
                    menu.picture = (menu.picture + step) % count;
                    self.play(audio, 0x0B, 0);
                    if let Some(p) = menu.nth_picture(menu.picture) {
                        return self.show_picture(p, menu, parent.take());
                    }
                }
                if press & 0xF0 != 0 {
                    self.play(audio, 0x0D, 0);
                    self.restore_vram();
                    self.screen = Screen::Menu { menu, parent };
                } else {
                    self.screen = Screen::Picture { n, ready, menu, parent };
                }
                None
            }
        }
    }
}
