//! `Front`'s checks and keys: decoded-state checks, replay sync values, the screen as text.
//!
//! Engine addition.
//! State: none of its own: reads the serialised `Front`
//!
//! `Front`'s checks and keys (engine additions): `check`, a decoded state's ids and indices in range
//! (`session::load_state`); `sync_key`, the front-end values of a replay's checkpoints; `describe`, the screen as text
//! (tests, logs).

use super::movie::After;
use super::screen::STAGE_COUNT;
use super::story_flow::StoryScreen;
use super::{Event, Front, MODES, Mode, Resident, Screen, Text, Tim, mode_rows};
use crate::assets::roster::{CHARACTERS, name};

impl Front {
    /// A decoded state's ids and indices are in range (`session::load_state`): those that index the roster, the
    /// stages, the movies, the menus, the rankings, the ladder and the story tables.
    pub(super) fn check(&self) -> Result<(), String> {
        let bad = |what: &str| Err(format!("front end: {what}"));
        let chr = |c: usize| c < CHARACTERS.len();
        // engine addition (the import S9, S10): a select cursor may be on the imported row only where the select
        // has it (`br1_row`; the NORMAL and STORY selects' tables have the main game's 11 cells), the last choices and a fight's
        // spec hold an imported fighter only in the modes that have them (`br1_modes`: VS, CUSTOM VS HUMAN, PRACTICE;
        // from v20 ARCADE, SURVIVAL, WATCH, CUSTOM VS COMPUTER (ALL MEMBERS): the ARCADE ladder's imported boss and
        // opponents), the last choices also through the attract demo that may follow (`start_mode` resets them), and a
        // imported stage in any mode with the open stage select (`br1_stages`)
        let (br1_row, br1_modes) = (self.br1_row(), self.br1_modes());
        let cell = |c: u8| chr(c as usize) || (br1_row && crate::assets::roster::is_br1(c));
        let br1_last = br1_modes || (self.br1_stages() && self.run.mode == Mode::Demo);
        let last = |c: u8| chr(c as usize) || (br1_last && crate::assets::roster::is_br1(c));
        let br1_spec = |spec: &super::MatchSpec| {
            let side = spec.chr.iter().any(|&c| crate::assets::roster::is_br1(c));
            let stage = crate::fight::stage::br1::STAGES.iter().any(|s| s.stage == spec.stage);
            if (side && !br1_modes) || (stage && !self.br1_stages()) { bad("match with the imported game's data") } else { Ok(()) }
        };
        let entry = |e: &crate::frontend::ranking::Entry| e.check(self.save.settings.ranking.len());
        // (T5) the team battle's imported cells and big stages only where the session has them (`team_br1`,
        // `team_stages`)
        let team_cells = |c: [u8; 4]| {
            if c.iter().any(|&c| crate::assets::roster::is_br1(c)) && !self.team_br1() { bad("team battle with the imported game's data") } else { Ok(()) }
        };
        let team_spec = |spec: &super::TeamSpec| {
            spec.check()?;
            team_cells(spec.chr)?;
            let big = crate::fight::stage::TEAM_BIG_STAGES.contains(&(spec.stage as usize));
            if big && !self.team_stages().contains(&spec.stage) { bad("team stage") } else { Ok(()) }
        };
        match &self.screen {
            Screen::Select(s) => {
                if !s.p.iter().all(|p| cell(p.cur) && p.costume() < 4) || s.stage > self.stage_count() {
                    return bad("select");
                }
            }
            Screen::Vs { spec, .. } | Screen::DemoLoad { spec, .. } | Screen::Fight(spec) => {
                spec.check()?;
                br1_spec(spec)?;
            }
            Screen::Menu { menu, parent } | Screen::Picture { menu, parent, .. } => {
                menu.check()?;
                if let Some(p) = parent {
                    p.check()?;
                }
            }
            Screen::Movie { then: After::Menu { menu, parent }, .. } => {
                menu.check()?;
                if let Some(p) = parent {
                    p.check()?;
                }
            }
            Screen::NameEntry(e) | Screen::NameFade { e, .. } => entry(e)?,
            // the team battle's (engine addition)
            Screen::TeamMode { cursor, banner_x } if *cursor as usize >= self.team_mode_rows().len() || !super::team::BANNER_X.contains(banner_x) => {
                return bad("team mode");
            }
            Screen::TeamSelect(sel) => {
                sel.check()?;
                team_cells(sel.p.map(|p| p.cur))?;
            }
            Screen::TeamStage { sel, row } => {
                sel.check()?;
                team_cells(sel.p.map(|p| p.cur))?;
                if *row as usize > self.team_stages().len() {
                    return bad("team stage");
                }
            }
            Screen::TeamLoad { spec, .. } | Screen::TeamFight(spec) => team_spec(spec)?,
            Screen::TeamResult { spec, winner, cursor, banner_x, .. } => {
                team_spec(spec)?;
                if *cursor >= 3 || winner.is_some_and(|w| w as usize >= spec.mode.teams()) || !super::team::BANNER_X.contains(banner_x) {
                    return bad("team result");
                }
            }
            _ => {}
        }
        // (T5: the imported row's cells, the big stages' rows)
        team_cells(self.team_last)?;
        if !self.team_last.iter().all(|&c| (c as usize) < crate::assets::roster::SELECTABLE)
            || self.team_port >= 2
            || self.team_stage as usize > self.team_stages().len()
            || self.team_result_row >= 3
        {
            return bad("team battle cursors");
        }
        // engine addition (TEAM ARCADE, simulation v26): a run and its ladder only from v26, the ladder one
        // `TeamLadder::draw` gives with the session's data, and a TEAM ARCADE run has one
        if let Some(l) = &self.run.team_ladder {
            l.check(self.team_br1(), &self.team_stages())?;
        }
        if ((self.run.team_ladder.is_some() || self.run.mode == Mode::TeamArcade) && !self.gates().team_arcade)
            || (self.run.mode == Mode::TeamArcade && self.run.team_ladder.is_none())
        {
            return bad("team arcade");
        }
        // a run's screens (A2): the arcade select of a stage still to play (the humans' slots live, the partner's a human
        // only with its pad; the enemies' CPUs, chosen), the ladder's current battle loading or running, a message
        // (CONGRATULATIONS), the ending (A4); the clear card only in a run
        if let (Some(t), Some(l)) = (self.arcade_team(), &self.run.team_ladder) {
            let ok = match &self.screen {
                Screen::TeamSelect(sel) => {
                    sel.mode == crate::fight::team::TeamMode::Vs2
                        && (l.idx as usize) < l.pairs.len()
                        && (0..crate::fight::team::N)
                            .all(|k| if k & 1 == t { sel.p[k].human == l.slot_port(k).is_some() } else { !sel.p[k].human && sel.p[k].state == 1 })
                }
                Screen::TeamLoad { spec, .. } | Screen::TeamFight(spec) => self.team_arcade_spec() == Some(*spec),
                Screen::Message { to_custom: false, .. } => true,
                // (A4) the ending after the last stage: a human's epilogue, the clear card
                Screen::Story => l.idx as usize == l.pairs.len() && l.ending > 0 && matches!(self.story_screen, Some(StoryScreen::Epilogue(_))),
                Screen::TeamArcadeClear { t } => l.idx as usize == l.pairs.len() && l.ending == 2 && *t < super::team_arcade::CLEAR_FRAMES,
                _ => false,
            };
            if !ok {
                return bad("team arcade screen");
            }
        } else if matches!(self.screen, Screen::TeamArcadeClear { .. }) {
            return bad("team arcade clear outside a run");
        }
        match self.screen {
            Screen::Movie { n, .. } if n >= super::movie::COUNT => return bad("movie"),
            Screen::Movie { then: After::Staff { n }, .. } if n as usize >= super::movie::COUNT => return bad("movie"),
            Screen::Picture { n, .. } if n >= 96 => return bad("picture"),
            _ => {}
        }
        if let Some(e) = &self.run.pending_entry {
            entry(e)?;
        }
        if self.menu_cursor >= mode_rows(self.gates(), self.browser, self.network, self.netplay).len()
            || self.emblem >= 11
            || self.stage_row(self.stage) >= self.stage_count()
            || !self.last.iter().all(|&c| last(c))
            || !self.run.ladder_o.iter().all(|&c| chr(c as usize))
            || !self.run.ladder_s.iter().all(|&s| (s as usize) < STAGE_COUNT)
            || !self.run.human_costume.iter().all(|&c| c < 4)
            || self.run.ending_movie >= super::movie::COUNT
            || self.run.br1_boss > 2
            || (self.run.br1_boss != 0 && !self.has_br1_boss())
        {
            return bad("cursors / ladder");
        }
        if self.story.is_some_and(|r| !r.valid()) {
            return bad("story run");
        }
        match &self.story_screen {
            Some(StoryScreen::Pages(p)) if !p.run.valid() => return bad("story pages"),
            Some(StoryScreen::Epilogue(e)) if !chr(e.chr) || !e.valid() => return bad("epilogue"),
            _ => {}
        }
        // a raw TIM is only the imported game's ending's picture (the import S10)
        let raw_ok = |v: &Resident| v.loads().iter().all(|l| !matches!(&l.tim, Tim::Raw(n) if *n != super::story_screens::br1_art_file()));
        if self.gfx.vram.loads().len() > Resident::MAX
            || self.gfx.vram_saved.as_ref().is_some_and(|v| v.loads().len() > Resident::MAX)
            || !raw_ok(&self.gfx.vram)
            || !self.gfx.vram_saved.as_ref().is_none_or(raw_ok)
        {
            return bad("VRAM loads");
        }
        if let Some(Event::Load(spec)) = &self.pending_event {
            spec.check()?;
            br1_spec(spec)?;
        }
        Ok(())
    }

    /// The select's cursors and the last choices (tests: a decoded state's `check`).
    #[cfg(test)]
    pub(crate) fn set_select_cursors(&mut self, cur: [u8; 2], last: [u8; 2]) {
        if let Screen::Select(s) = &mut self.screen {
            s.p[0].cur = cur[0];
            s.p[1].cur = cur[1];
        }
        self.last = last;
    }

    /// A fight in progress on `spec`, or (`pending`) its load requested (tests: a decoded state's `check`).
    #[cfg(test)]
    pub(crate) fn set_fight(&mut self, spec: super::MatchSpec, pending: bool) {
        if pending {
            self.pending_event = Some(Event::Load(spec));
        } else {
            self.screen = Screen::Fight(spec);
        }
    }

    /// An ARCADE run's imported boss mark `v` (`Run::br1_boss`; tests: a run left at the boss or after it).
    #[cfg(test)]
    pub(crate) fn set_br1_boss(&mut self, v: u8) {
        self.run.br1_boss = v;
    }

    /// The values a replay's checkpoints hash besides the fight's (`session::replay::sync_hash`; part of its
    /// file format 1, so the numbers are fixed: a new screen or mode takes a new one): the frame, the random state,
    /// the ladder's fight index, the play time, the screen (a fixed id and up to three of its values: the select's
    /// players and stage, the menu and its cursor, the movie, …), the mode menu's cursor, the mode and the stage.
    pub fn sync_key(&self) -> [u32; 12] {
        let (id, a, b, c) = match &self.screen {
            Screen::Title { menu, .. } => (1, *menu as u32, 0, 0),
            Screen::TitleFade { .. } => (2, 0, 0, 0),
            Screen::Logo { n, .. } => (3, *n as u32, 0, 0),
            Screen::Ranking { .. } => (4, 0, 0, 0),
            Screen::RankFade { to_title, .. } => (5, *to_title as u32, 0, 0),
            Screen::Select(s) => {
                let [p1, p2] = s.sync_key();
                (6, p1, p2, s.stage as u32)
            }
            Screen::Vs { ready, .. } => (7, *ready as u32, 0, 0),
            Screen::DemoLoad { ready, .. } => (8, *ready as u32, 0, 0),
            Screen::Fight(_) => (9, 0, 0, 0),
            Screen::Message { text, to_custom, .. } => (10, matches!(text, Text::GameOver) as u32, *to_custom as u32, 0),
            Screen::StoryGameOver { k, .. } => (11, k.is_some() as u32, 0, 0),
            Screen::Menu { menu, parent } => (12, menu.kind.sync_id(), menu.cursor as u32, parent.map_or(0, |p| p.kind.sync_id())),
            Screen::NameEntry(_) => (13, 0, 0, 0),
            Screen::NameFade { .. } => (14, 0, 0, 0),
            Screen::Movie { n, .. } => (15, *n as u32, 0, 0),
            Screen::Story => (16, 0, 0, 0),
            Screen::Picture { n, ready, .. } => (17, *n as u32, *ready as u32, 0),
            Screen::AutoSave => (18, 0, 0, 0),
            // the team battle's (engine addition, simulation v22): its select's four players folded into two values
            Screen::TeamMode { cursor, .. } => (19, *cursor as u32, 0, 0),
            Screen::TeamSelect(s) => {
                let k = s.sync_key();
                (20, k[0] ^ k[2].rotate_left(4), k[1] ^ k[3].rotate_left(4), s.mode as u32)
            }
            Screen::TeamStage { row, .. } => (21, *row as u32, 0, 0),
            Screen::TeamLoad { ready, .. } => (22, *ready as u32, 0, 0),
            Screen::TeamFight(_) => (23, 0, 0, 0),
            Screen::TeamResult { winner, cursor, .. } => (24, winner.map_or(0xFF, |w| w as u32), *cursor as u32, 0),
            Screen::TeamArcadeClear { t } => (25, *t, 0, 0),
        };
        let mode = match self.run.mode {
            Mode::Arcade => 1,
            Mode::Versus => 2,
            Mode::Watch => 3,
            Mode::Survival => 4,
            Mode::Story => 5,
            Mode::Demo => 6,
            Mode::Custom(n) => 0x10 + n as u32,
            Mode::Team => 0x20,
            Mode::TeamArcade => 0x21,
        };
        [self.frame, self.rng, self.run.a2 as u32, self.run.play_time, id, a, b, c, self.menu_cursor as u32, mode, self.stage as u32, self.stage_random as u32]
    }

    /// Current screen and ladder position (tests, logs).
    pub fn describe(&self) -> String {
        let s = match self.screen {
            Screen::Title { menu: false, .. } => "title".to_string(),
            Screen::TitleFade { k } => format!("title fade {k}"),
            Screen::Logo { n, k } => format!("logo {} frame {k}", if n == 3 { "SCEA" } else { "HUDSON" }),
            Screen::Ranking { k } => format!("attract ranking {k}"),
            Screen::RankFade { k, .. } => format!("ranking fade {k}"),
            Screen::NameFade { k, .. } => format!("name entry fade {k}"),
            Screen::Title { menu: true, .. } => format!("mode menu ({})", MODES[self.mode_item()].text()),
            Screen::Select(s) => {
                format!("select P1 {} P2 {} stage {}", name(s.p[0].cur), name(s.p[1].cur), self.row_label(s.stage).0)
            }
            Screen::Vs { spec, ready, .. } => format!("VS {} vs {} ready {ready}", name(spec.chr[0]), name(spec.chr[1])),
            Screen::DemoLoad { spec, ready } => format!("demo {} vs {} ready {ready}", name(spec.chr[0]), name(spec.chr[1])),
            Screen::Fight(spec) => format!("fight {:?}", spec),
            Screen::Message { text, .. } => text.str().to_string(),
            Screen::StoryGameOver { t, k } => format!("story game over {t} fade {k:?}"),
            Screen::AutoSave => "auto-save".into(),
            Screen::NameEntry(e) => format!("name entry table {} rank {:?} name {:?} timer {}", e.table, e.rank, String::from_utf8_lossy(&e.buf[..3]), e.timer),
            Screen::Menu { menu, .. } => format!("menu {:?} item {}", menu.kind, menu.cursor),
            Screen::Movie { n, .. } => format!("movie {} frame {:?}", super::movie::name(n), self.movie.as_ref().filter(|m| m.t > 0).map(|m| m.t)),
            Screen::Story => match &self.story_screen {
                Some(StoryScreen::Prologue(_)) => "story prologue".into(),
                Some(StoryScreen::Pages(p)) => format!("story pages fight {} continues {} {}", p.run.fight, p.run.continues, p.state()),
                Some(StoryScreen::Epilogue(e)) => format!("epilogue {}", e.chr),
                None => "story".into(),
            },
            Screen::Picture { n, ready, .. } => format!("picture {n} ready {ready}"),
            Screen::TeamMode { cursor, .. } => format!("team mode {cursor}"),
            Screen::TeamSelect(s) | Screen::TeamStage { sel: s, .. } => {
                let p: Vec<String> = s.p.iter().map(|p| format!("{}{}", name(p.cur), if p.human { "" } else { "(COM)" })).collect();
                let stage = if let Screen::TeamStage { row, .. } = self.screen { format!(" stage row {row}") } else { String::new() };
                format!("team select {:?} {}{stage}", s.mode, p.join(" "))
            }
            Screen::TeamLoad { spec, ready, .. } => format!("team load {:?} ready {ready}", spec.chr.map(name)),
            Screen::TeamFight(spec) => format!("team fight {spec:?}"),
            Screen::TeamResult { winner, cursor, .. } => format!("team result winner {winner:?} item {cursor}"),
            Screen::TeamArcadeClear { t } => format!("team arcade clear {t}"),
        };
        format!("{s} [{:?} fight {} continues {} unlocked {:#x}]", self.run.mode, self.run.a2, self.run.continues, self.chars())
    }
}
