//! STORY mode screens, MOVIES AND PICTURES and the files fetched for them.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)

use super::movie::{After, MoviePlay};
use super::{Event, Front, MatchSpec, Screen};
use crate::frontend::menus::Menu;
use crate::output::sound::Queue;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// STORY mode screens.
#[derive(Clone, Serialize, Deserialize)]
pub(super) enum StoryScreen {
    Prologue(crate::frontend::story_screens::Prologue),
    Pages(crate::frontend::story_screens::Pages),
    Epilogue(crate::frontend::story_screens::Epilogue),
}

impl Front {
    /// PICTURES: load picture `n` (file 0x118 + n/4, entry n%4: an 8-bit 256×240 TIM).
    pub(super) fn show_picture(&mut self, n: usize, menu: Menu, parent: Option<Menu>) -> Option<Event> {
        self.screen = Screen::Picture { n, ready: false, menu, parent };
        Some(Event::Fetch(vec![format!("engine/sty/PIC{:02}.DAT", n / 4)]))
    }

    /// Files asked for with `Event::Fetch` arrived.
    pub fn fetched(&mut self, files: Vec<(String, Arc<[u8]>)>, audio: &mut Queue) {
        match self.screen {
            Screen::Movie { .. } => {
                // the stream's frame table only (its frames and music are the presentation's: `VramView`)
                let movie = files.into_iter().find(|f| f.0.ends_with(".bsf")).and_then(|f| crate::formats::movie::Movie::parse(f.1));
                let Screen::Movie { n, then } = self.screen else { return };
                // engine addition (simulation v21): an imported movie plays its fixed length, black without its
                // stream (`movie::BR1_MOVIE_END`); before, the stream's own frames, none without it
                let fixed = super::movie::br1_end(n, self.gates());
                if movie.is_none() {
                    log::warn!("movie {} not found", super::movie::name(n));
                    if fixed.is_none() {
                        self.after_movie(then, audio);
                        return;
                    }
                }
                if self.gfx.vram_saved.is_none() {
                    self.gfx.vram_saved = Some(self.gfx.vram.clone());
                }
                // the menu plays them skippable; elsewhere once seen (opt.28)
                let skippable = matches!(then, After::Menu { .. } | After::Boot) || self.save.settings.movies >> n & 1 != 0;
                if !matches!(then, After::Boot) {
                    self.save.settings.movies |= 1 << n;
                }
                let mut mp = match &movie {
                    Some(m) => MoviePlay::new(m, super::movie::last(n), skippable),
                    None => MoviePlay::black(0, skippable),
                };
                if let Some(end) = fixed {
                    mp.end = end;
                }
                self.movie = Some(mp);
            }
            Screen::Story => {
                let pics = &mut self.save.settings.pictures;
                for (name, bytes) in files {
                    if self.file_cache.len() > 24 {
                        self.file_cache.clear();
                    }
                    self.file_cache.insert(name.clone());
                    match &mut self.story_screen {
                        Some(StoryScreen::Prologue(p)) => p.fetched(&mut self.gfx.vram, &bytes, pics),
                        Some(StoryScreen::Pages(p)) => p.fetched(&name, &bytes, &mut self.gfx.vram),
                        Some(StoryScreen::Epilogue(e)) => e.fetched(&mut self.gfx.vram, &bytes),
                        None => {}
                    }
                }
                let g = self.gates();
                if let Some(StoryScreen::Epilogue(e)) = &mut self.story_screen {
                    e.missing(g);
                }
                self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
            }
            Screen::Picture { n, menu, parent, .. } => {
                let tim = files.first().and_then(|f| Some((f.0.clone(), crate::formats::container::read(&f.1).into_iter().nth(n % 4).flatten()?)));
                let Some((name, tim)) = tim else {
                    log::warn!("picture {n} not found");
                    self.screen = Screen::Menu { menu, parent };
                    return;
                };
                if self.gfx.vram_saved.is_none() {
                    self.gfx.vram_saved = Some(self.gfx.vram.clone());
                }
                // pixels at (512, 0), CLUT at (0, 511)
                self.gfx.vram.load(crate::frontend::Tim::File(name, (n % 4) as u8), &tim, Some((0, 511)), Some((512, 0)));
                self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
                self.screen = Screen::Picture { n, ready: true, menu, parent };
            }
            _ => {}
        }
    }

    /// STORY: the next fight of the run (the page player's section end, mode 0 / CONTINUE YES).
    fn story_fight(&mut self, audio: &mut Queue) -> Event {
        let run = self.story.expect("story run");
        let (opp, stage) = run.match_up();
        let h = if self.run.humans[0] { 0 } else { 1 };
        let mut chr = [opp; 2];
        chr[h] = run.chr as u8;
        let mut cpu = [true; 2];
        cpu[h] = false;
        let mut costume = [0, 0];
        if chr[0] == chr[1] {
            costume[1 - h] = 1;
        }
        self.run.a2 = run.fight as i32;
        self.story_screen = None;
        self.boot_vram();
        self.enter_vs(MatchSpec { chr, costume, stage, cpu }, audio)
    }

    /// One frame of the STORY screens.
    pub(super) fn tick_story(&mut self, press: u16, audio: &mut Queue, files: &crate::session::FileCache) -> Option<Event> {
        use crate::frontend::story_screens::Out;
        let cache = crate::session::Cached { names: &self.file_cache, files };
        let mut sounds = Vec::new();
        let pics = &mut self.save.settings.pictures;
        let out = match &mut self.story_screen {
            Some(StoryScreen::Prologue(p)) => p.tick(press, pics, &mut sounds),
            Some(StoryScreen::Pages(p)) => {
                let h = if self.run.humans[0] { 0 } else { 1 };
                let o = p.tick(press, self.pads.held[h], &mut self.gfx.vram, pics, &mut sounds, &cache);
                self.story = Some(p.run);
                o
            }
            Some(StoryScreen::Epilogue(e)) => e.tick(),
            None => Out::Done,
        };
        self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        for id in sounds {
            self.play(audio, id, 0);
        }
        match out {
            Out::None => None,
            Out::Fetch(name) => match cache.get(&name).cloned() {
                Some(bytes) => {
                    self.fetched(vec![(name, bytes)], audio);
                    None
                }
                None => Some(Event::Fetch(vec![name])),
            },
            Out::Fight => Some(self.story_fight(audio)),
            Out::Done => match self.story_screen.take() {
                Some(StoryScreen::Prologue(_)) => {
                    let run = self.story.expect("story run");
                    self.story_screen = Some(StoryScreen::Pages(crate::frontend::story_screens::Pages::start(run)));
                    None
                }
                // engine addition (TEAM ARCADE A4): a human's epilogue → the next one, or the clear card
                _ if self.run.mode == super::Mode::TeamArcade => {
                    self.team_arcade_next_ending();
                    None
                }
                _ => {
                    // the arcade epilogue → the ending movie, then CONGRATULATIONS
                    let n = self.run.ending_movie;
                    Some(self.play_movie(n, After::Congrats { to_custom: false }, audio))
                }
            },
            Out::Ending => {
                // story: cleared bit, all eleven → CUSTOM item 0x4000; ENDING1 without continues
                let run = self.story.expect("story run");
                self.save.settings.cleared |= 1 << run.chr;
                if self.save.settings.cleared & 0x7FF == 0x7FF {
                    self.save.settings.custom_unlock |= 0x4000;
                }
                // CUSTOM unlocks at the story clear (bit 0, bit 15 without continues)
                self.save.settings.custom_unlock |= 1 | if run.continues == 0 { 0x8000 } else { 0 };
                self.save.dirty = true;
                self.save.settings.save(&mut self.save.memory_card);
                self.story_screen = None;
                Some(self.play_movie(if run.fight == -2 { 2 } else { 3 }, After::Title, audio))
            }
            Out::GameOver => {
                // BGM SELECT CONSOLE → track 0x27 for 900 frames, ARCADE → track 6 for 420, played once
                self.story_screen = None;
                self.boot_vram();
                let console = self.save.settings.bgm_select == 0;
                self.screen = Screen::StoryGameOver { t: if console { 900 } else { 420 }, k: None };
                Some(Event::Music(Some((if console { 0x27 } else { 6 }, false))))
            }
        }
    }

    pub(super) fn restore_vram(&mut self) {
        if let Some(v) = self.gfx.vram_saved.take() {
            self.gfx.vram = v;
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
    }
}
