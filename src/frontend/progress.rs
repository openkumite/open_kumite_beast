//! The arcade ladder, the fight rules, the routing after a fight, PLAY DATA and NAME ENTRY.
//!
//! State: none of its own: updates the serialised `Front`
//!
//! The arcade ladder, the fight rules, the routing after a fight,
//! PLAY DATA and NAME ENTRY.

use super::Tim;
use super::movie::After;
use super::story_flow::StoryScreen;
use super::{Event, Front, MatchSpec, Mode, Screen, Text};
use crate::fight::sim::{Outcome, Rules};
use crate::frontend::menus;
use crate::output::sound::Queue;

/// The arcade rank table (`progress.rank`): DIFFICULTY × fight index → CPU level / 16.
pub(super) fn rank(difficulty: u8, fight: usize) -> u8 {
    crate::assets::game_tables::get().rank[difficulty.min(7) as usize][fight]
}

/// Engine addition (the import S10): the ARCADE run's imported boss, "URIKO (BR1)" (`roster::BR1_CODES` URI),
/// on her own stage (the imported game's G.94 8, URI00: its arcade's last stage).
const URIKO_BR1: u8 = crate::assets::roster::BR1_FIRST + 3;
const URIKO_STAGE: u8 = crate::fight::stage::br1::STAGES[8].stage;

impl Front {
    /// Fight rules from the options and the CPU level.
    pub fn rules(&mut self) -> Rules {
        let s = self.save.settings;
        let mut r = Rules {
            need: s.match_point.min(3) + 1,
            time_limit: Rules::TIME[s.time_limit.min(4) as usize],
            hp_pct: Rules::HP_PCT[s.attack_level.min(7) as usize],
            cpu_level: self.cpu_level(),
            custom: if matches!(self.run.mode, Mode::Custom(_)) { s.custom } else { crate::frontend::options::Custom::normal() },
            practice: self.run.mode == Mode::Custom(3),
            demo: self.run.mode == Mode::Demo,
        };
        match self.run.mode {
            Mode::Survival => {
                r.need = 1;
                r.time_limit = 0xE10;
            }
            Mode::Demo => r.need = 1,             // M.99 = 2*need - 2: one deciding round
            Mode::Custom(3) => r.time_limit = -1, // practice: no time limit (fight/round.rs practice_tick)
            _ => {}
        }
        r
    }

    /// The CPU level of this fight from the rank table and the arcade rank M.A0.
    pub(super) fn cpu_level(&mut self) -> u16 {
        if self.run.mode == Mode::Demo {
            return 0xFF; // M.A0 = 0xFF, the rank untouched
        }
        // engine addition (TEAM ARCADE, simulation v26): the ladder stage's column (`TeamLadder::rank_column`)
        let col = match (&self.run.team_ladder, self.run.mode) {
            (Some(l), Mode::TeamArcade) => l.rank_column(),
            _ => (self.run.a2.max(0) as usize).min(8),
        };
        let a = rank(self.save.settings.difficulty, col) as i32 * 16;
        if self.run.shoulders {
            return 0xFF;
        }
        let v = match self.run.mode {
            Mode::Watch => 0xFF,
            Mode::Survival => (self.run.a2 * 12).min(0xFF),
            _ => {
                let mut v = self.run.rank as i32;
                if v & 0x100 != 0 {
                    v &= 0xFF;
                    v = v.max(a - 0x32);
                } else {
                    v = v.max(a);
                }
                v.min(a + 0x1F).clamp(0, 0xFF)
            }
        };
        self.run.rank = v as u16;
        v as u16
    }

    /// Mode chosen and a port joined (the ladder).
    pub(super) fn start_mode(&mut self, mode: Mode, port: usize, audio: &mut Queue) {
        self.run.mode = mode;
        // engine addition (TEAM ARCADE): another mode ends a run
        self.run.team_ladder = None;
        // the players' records reset with a new game
        self.run.streak = [0; 2];
        self.run.best = [0; 2];
        self.run.champ = [false; 2];
        self.run.dropped = [false; 2];
        self.run.play_time = 0;
        self.run.humans = match mode {
            Mode::Arcade | Mode::Survival | Mode::Story | Mode::Custom(1..) => [port == 0, port == 1],
            Mode::Versus | Mode::Custom(_) | Mode::Team | Mode::TeamArcade => [true, true],
            Mode::Watch | Mode::Demo => [false, false],
        };
        self.run.shoulders = self.pads.held[port] & 0xF == 0xF;
        self.run.rank = rank(self.save.settings.difficulty, 0) as u16 * 16;
        self.last = [0, 2];
        self.run.continues = 0;
        self.run.a2 = 0;
        self.run.br1_boss = 0;
        let mut o: [u8; 11] = std::array::from_fn(|i| i as u8);
        let mut s: [u8; 11] = std::array::from_fn(|i| (i % 9) as u8);
        for k in 0..9 {
            let j = (self.rand() % 9) as usize;
            o.swap(k, j);
        }
        if mode == Mode::Arcade {
            (o[7], o[8], o[9], o[10]) = (9, 10, o[7], o[8]);
            s.swap(4, 5);
            for k in 0..5 {
                let j = (self.rand() % 5) as usize;
                s.swap(k, j);
            }
        } else {
            for v in s.iter_mut() {
                *v = (self.rng >> 16) as u8 & 7;
                self.rand();
            }
        }
        self.run.ladder_o = o;
        self.run.ladder_s = s;
        if mode == Mode::Story {
            // the STORY movie (skippable once seen), then the select
            let ev = self.play_movie(1, After::Select, audio);
            self.pending_event = Some(ev);
            return;
        }
        self.enter_select(audio);
    }

    /// The ladder opponent of fight M.A2 (survival from 11 on: random).
    pub(super) fn opponent(&mut self) -> u8 {
        let i = self.run.a2 as usize;
        // engine addition (the import S10): the ARCADE run's imported boss, and an imported player's
        // imported opponents
        if self.run.br1_boss == 1 {
            return URIKO_BR1;
        }
        if self.run.mode == Mode::Survival && i >= 11 {
            let c = (self.rand() % 11) as u8;
            self.run.ladder_o[i % 11] = c;
            self.run.ladder_s[i % 11] = (self.rand() & 7) as u8;
        }
        self.br1_ladder(i).unwrap_or(self.run.ladder_o[i % 11])
    }

    /// Engine addition (the import S10, simulation v20): in a
    /// imported player's ARCADE, fights 3 and 6 are two other imported fighters (of MITSUKO, FOX and GREG: the
    /// two the player is not; Uriko's two by the ladder's first slot), in the order the ladder's second slot gives; no
    /// draw of its own (the ladder of `start_mode`, shuffled before the select, decides).
    fn br1_ladder(&self, i: usize) -> Option<u8> {
        let h = if self.run.humans[0] { 0 } else { 1 };
        let me = self.last[h];
        if !self.has_br1_boss() || !crate::assets::roster::is_br1(me) || !matches!(i, 2 | 5) {
            return None;
        }
        let first = crate::assets::roster::BR1_FIRST;
        let mut others: Vec<u8> = (first..first + 3).filter(|&c| c != me).collect();
        if others.len() == 3 {
            others.remove(self.run.ladder_o[0] as usize % 3);
        }
        let k = (i == 5) as usize ^ (self.run.ladder_o[1] & 1) as usize;
        others.get(k).copied()
    }

    /// NAME ENTRY state 2: the record's place in ranking `table` (None = not in the top 10: the table only); a
    /// placed record is inserted with the entry's default name.
    pub(super) fn rank_entry(&mut self, table: usize, by_time: bool, rec: crate::frontend::options::RankRec) -> crate::frontend::ranking::Entry {
        let rank = crate::frontend::ranking::place(&self.save.settings.ranking[table], by_time, &rec);
        let e = crate::frontend::ranking::Entry::new(table, rank, rec, self.save.settings.last_name);
        if let Some(r) = rank {
            crate::frontend::ranking::insert(&mut self.save.settings.ranking[table], r, crate::frontend::options::RankRec { name: e.name(), ..rec });
        }
        e
    }

    /// NAME ENTRY (main mode 9): the next P00 emblem for the frame, ENDING.SND in slot 2, music only
    /// when ranked (music (opt.D ? 9 : 0x29, 1)); state 2 falls through into the first entry frame.
    pub(super) fn enter_name(&mut self, e: crate::frontend::ranking::Entry, audio: &mut Queue) -> Option<Event> {
        if let Some(Some(t)) = self.data.p00.get(self.emblem) {
            self.gfx.vram.load(Tim::P00(self.emblem as u8), t, None, Some((960, 256)));
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
        self.emblem = (self.emblem + 1) % 11;
        let ev = self.tick_name(e, [0; 3], audio);
        if e.rank.is_some() {
            audio.stop_music();
            return Some(Event::Music(Some((if self.save.settings.bgm_select != 0 { 9 } else { 0x29 }, true))));
        }
        ev
    }

    /// One frame of NAME ENTRY; `pad` = the player's rep and press words and both pads' presses (M+0x7C).
    pub(super) fn tick_name(&mut self, mut e: crate::frontend::ranking::Entry, pad: [u16; 3], audio: &mut Queue) -> Option<Event> {
        use crate::frontend::ranking::Step;
        let mut snd = Vec::new();
        let step = e.tick(pad[0], pad[1], pad[2], &mut snd);
        self.screen = Screen::NameEntry(e);
        for (id, delay) in snd {
            self.play(audio, id, delay);
        }
        match step {
            Step::Stay => None,
            Step::Named(name) => {
                // the name goes into the record and becomes the next default (opt.0..2); saved
                if let Some(r) = e.rank {
                    self.save.settings.ranking[e.table][r].name = name;
                }
                self.save.settings.last_name = name;
                self.save.dirty = true;
                self.save.settings.save(&mut self.save.memory_card);
                None
            }
            // state 4's first frame: music (opt.D ? 7 : 0x28, 0)
            Step::Table => Some(Event::Music(Some((if self.save.settings.bgm_select != 0 { 7 } else { 0x28 }, false)))),
            Step::Exit => {
                // arcade / versus: the VS RANKING by the best streak, named "???" when not ranked
                if matches!(self.run.mode, Mode::Arcade | Mode::Versus) {
                    let name = if e.buf[0] == 0 { *b"???" } else { e.name() };
                    let rec = crate::frontend::options::RankRec { name, ..e.rec };
                    if let Some(r) = crate::frontend::ranking::place_vs(&self.save.settings.ranking[1], &rec) {
                        crate::frontend::ranking::insert(&mut self.save.settings.ranking[1], r, rec);
                    }
                    self.save.settings.save(&mut self.save.memory_card);
                }
                // a1 = 1 sets the save's dirty flag (save+0x410), so the next title entry
                // auto-saves (the fights' calls pass 0 and do nothing)
                self.save.dirty = true;
                // SE 0x204 (stop), the fade to black
                audio.stop_music();
                self.screen = Screen::NameFade { e, k: 0 };
                None
            }
        }
    }

    /// PLAY DATA is kept for human players, not in WATCH or PRACTICE (by mode).
    fn stats_kept(&self) -> bool {
        !matches!(self.run.mode, Mode::Watch | Mode::Demo | Mode::Custom(3))
    }

    /// At the fight start: a play for each human player's character.
    pub(super) fn count_plays(&mut self, spec: &MatchSpec) {
        if !self.stats_kept() {
            return;
        }
        for side in 0..2 {
            // an imported fighter (engine addition, the import S9) has no PLAY DATA of its own
            if self.run.humans[side] && !crate::assets::roster::is_br1(spec.chr[side]) {
                let r = &mut self.save.settings.stats[spec.chr[side] as usize % 11];
                r.plays = r.plays.saturating_add(1);
            }
        }
    }

    /// At the match end (win / loss) for the human players; wins against a human also
    /// count in the VS DIAGRAM row. Saved with the next save.
    fn record_result(&mut self, spec: &MatchSpec, outcome: Outcome) {
        // the winner's streak grows only against a human; the champion flag moves
        if let Outcome::Winner(w) = outcome {
            if self.run.humans[1 - w] {
                self.run.streak[w] = self.run.streak[w].saturating_add(1);
            }
            self.run.best[w] = self.run.best[w].max(self.run.streak[w]);
            self.run.streak[1 - w] = 0;
            self.run.champ = [w == 0, w == 1];
        }
        if !self.stats_kept() {
            return;
        }
        let winner = match outcome {
            Outcome::Winner(w) => w,
            // the human lost to the CPU
            Outcome::Continue | Outcome::GameOver => usize::from(!self.run.humans[1] || self.run.humans[0]),
            _ => return,
        };
        for side in 0..2 {
            if !self.run.humans[side] || crate::assets::roster::is_br1(spec.chr[side]) {
                continue;
            }
            let (me, opp) = (spec.chr[side] as usize % 11, spec.chr[1 - side] as usize % 11);
            let opp_cpu = !self.run.humans[1 - side];
            let r = &mut self.save.settings.stats[me];
            let inc = |v: &mut u16| *v = v.saturating_add(1);
            if side == winner {
                inc(&mut r.wins);
                if opp_cpu {
                    inc(&mut r.com_wins);
                    inc(&mut r.com_games);
                } else {
                    inc(&mut r.hum_wins);
                    inc(&mut r.hum_games);
                    if !crate::assets::roster::is_br1(spec.chr[1 - side]) {
                        inc(&mut r.vs[opp]);
                    }
                }
            } else if opp_cpu {
                inc(&mut r.com_games);
            } else {
                inc(&mut r.hum_games);
            }
        }
        self.save.dirty = true;
        self.save.settings.save(&mut self.save.memory_card);
    }

    /// A second player may join this fight (1P ladder modes with one human): the idle port.
    pub fn challenger_port(&self) -> Option<usize> {
        let humans = self.run.humans.iter().filter(|h| **h).count();
        (matches!(self.run.mode, Mode::Arcade | Mode::Survival | Mode::Story) && humans == 1).then(|| if self.run.humans[0] { 1 } else { 0 })
    }

    /// A lost 1P match goes to the continue countdown (arcade and story; not on the extra stage, not survival).
    pub fn continue_allowed(&self) -> bool {
        self.run.mode == Mode::Arcade && self.run.a2 < 8
    }

    /// Match over (the routing, and the continue / game-over states of the round flow).
    pub fn match_over(&mut self, outcome: Outcome, held: [u16; 2], audio: &mut Queue) -> Option<Event> {
        let Screen::Fight(spec) = self.screen else { return None };
        audio.stop_music();
        self.boot_vram();
        if self.run.mode == Mode::Demo {
            // the attract demo's states 0xF0 / 0xF1: START / × → the title; △, a KO or the timeout → main
            // mode 6 goes on to the attract ranking
            // (the SE was played when the demo ended, before its fade)
            if (held[0] | held[1]) & 0x840 != 0 {
                self.to_title(audio);
                return None;
            }
            self.screen = Screen::Ranking { k: 0 };
            // the turntable (camera mode 0x1E)
            self.rank_cam = crate::fight::camera::Camera { mode: 0x1E, ..Default::default() };
            // if opt.20, music (opt.D ? 9 : 0x29, 1), looped
            return (self.save.settings.attract_sound != 0).then_some(Event::Music(Some((if self.save.settings.bgm_select != 0 { 9 } else { 0x29 }, true))));
        }
        self.record_result(&spec, outcome);
        if self.run.mode == Mode::Story && !matches!(outcome, Outcome::Quit | Outcome::Challenger(_)) {
            // the page player (1): the win or lose pages
            let h = if self.run.humans[0] { 0 } else { 1 };
            let won = matches!(outcome, Outcome::Winner(w) if w == h);
            if let Some(run) = self.story {
                self.story_screen = Some(StoryScreen::Pages(crate::frontend::story_screens::Pages::after(run, won)));
                self.screen = Screen::Story;
                return None;
            }
        }
        let winner = match outcome {
            Outcome::Continue => {
                // continue accepted → the select again, same ladder slot
                self.play(audio, 0x0C, 0);
                self.run.continues += 1;
                self.run.rank = self.run.rank.saturating_sub(20) | 0x100;
                self.enter_select(audio);
                return None;
            }
            Outcome::GameOver => {
                // NAME ENTRY mode 3: survival ranks by the wins
                if self.run.mode == Mode::Survival {
                    let h = if self.run.humans[0] { 0 } else { 1 };
                    let rec =
                        crate::frontend::options::RankRec { name: [b' '; 3], chr: spec.chr[h], value: self.run.play_time as i32, c: 0, e: self.run.a2 as i16 };
                    let e = self.rank_entry(2, false, rec);
                    return self.enter_name(e, audio);
                }
                self.to_title(audio);
                return None;
            }
            // pause → QUIT → YES: m.1 = 9, all players dropped, back to the title (a team battle's DRAW never
            // ends a match: `team_over` takes it)
            Outcome::Quit | Outcome::Draw => {
                self.to_title(audio);
                return None;
            }
            Outcome::Challenger(_) => {
                // both players choose (opt.23 = 1: back to the select); the ladder slot is kept
                self.run.humans = [true, true];
                self.enter_select(audio);
                return None;
            }
            Outcome::Winner(w) => w,
        };
        // a challenger match (both human in a ladder mode): the loser is out, the winner continues the ladder
        if self.run.mode.ladder() && self.run.humans == [true, true] {
            self.run.humans = [winner == 0, winner == 1];
            self.run.dropped[1 - winner] = true;
            self.last[winner] = spec.chr[winner];
            self.run.human_costume[winner] = spec.costume[winner];
            return Some(self.next_fight(audio));
        }
        if self.run.mode.ladder() && self.run.humans.iter().any(|&h| h) {
            // the extra stage ends the game whoever wins
            let extra = self.run.mode == Mode::Arcade && self.run.a2 >= 8;
            // engine addition (the import S10): the imported boss's fight (after the last one, a2 >= 8) ends the
            // game whoever wins, as the extra stage, and is not a stage of the ladder
            let boss = self.run.br1_boss == 1;
            if self.run.humans[winner] || extra {
                if self.run.humans[winner] && !boss {
                    self.run.a2 += 1;
                }
                let len = match self.run.mode {
                    Mode::Survival => i32::MAX,
                    Mode::Custom(_) => 11, // ALL MEMBERS: opt.10 = 11
                    _ if self.run.continues == 0 => 9,
                    _ => 8,
                };
                if self.run.a2 >= len || extra {
                    // engine addition (the import S10, simulation v20; a user decision): the main game's last fight won
                    // → the imported boss, "URIKO (BR1)", before the ending
                    if self.run.br1_boss == 0 && self.has_br1_boss() && self.run.humans[winner] {
                        self.run.br1_boss = 1;
                        return Some(self.next_fight(audio));
                    }
                    if boss {
                        self.run.br1_boss = 2;
                    }
                    // ending: unlocks, epilogue, movie
                    let h = if self.run.humans[0] { 0 } else { 1 };
                    let c = spec.chr[h];
                    let br1 = crate::assets::roster::is_br1(c);
                    if self.run.mode == Mode::Arcade {
                        self.save.settings.characters |= 0x200 | if self.run.a2 >= 9 { 0x400 } else { 0 };
                        // an imported fighter has no fourth costume or clear mark of its own (engine addition)
                        if !br1 {
                            self.save.settings.costume4 |= 1 << c;
                            self.save.settings.cleared |= 1 << (c + 16);
                        }
                        self.save.dirty = true;
                        self.save.settings.save(&mut self.save.memory_card);
                    }
                    // NAME ENTRY mode 0: the clear time enters the arcade ranking (TIME LIMIT >= 60 s, MATCH POINT >= 2);
                    // otherwise the table only
                    if self.run.mode == Mode::Arcade {
                        let rec = crate::frontend::options::RankRec {
                            name: [b' '; 3],
                            chr: c,
                            value: self.run.play_time as i32,
                            c: self.run.best[h] as i16,
                            e: self.run.a2 as i16,
                        };
                        self.run.pending_entry = Some(if self.save.settings.time_limit >= 2 && self.save.settings.match_point >= 1 {
                            self.rank_entry(0, true, rec)
                        } else {
                            crate::frontend::ranking::Entry::new(0, None, rec, self.save.settings.last_name)
                        });
                    }
                    if self.run.mode == Mode::Arcade && br1 {
                        // engine addition (the import S10; user decisions): an imported fighter's ending is its
                        // ending movie and staff roll from the imported game (`movie::br1_ending`); Uriko's, who has none, a
                        // picture of the imported game's gallery (URIKO 1, ART124) in the epilogue's place, then STAFF ROLL A;
                        // then CONGRATULATIONS and the NAME ENTRY as the main game's
                        let k = c - crate::assets::roster::BR1_FIRST;
                        if let Some((ending, staff)) = super::movie::br1_ending(k) {
                            // its fetch from the next frame (as STORY's movie: the fight's end takes no event)
                            let ev = self.play_movie(ending, super::movie::After::Staff { n: staff as u8 }, audio);
                            self.pending_event = Some(ev);
                            return None;
                        }
                        self.run.ending_movie = super::movie::BR1_STAFF_A;
                        self.story_screen = Some(StoryScreen::Epilogue(crate::frontend::story_screens::Epilogue::br1(crate::frontend::story_screens::BR1_ART)));
                        self.screen = Screen::Story;
                        return None;
                    }
                    if self.run.mode == Mode::Arcade {
                        // the epilogue illustration, then ENDING1 (extra stage cleared) / ENDING2
                        self.run.ending_movie = if self.run.a2 >= 9 { 2 } else { 3 };
                        self.story_screen = Some(StoryScreen::Epilogue(crate::frontend::story_screens::Epilogue::new(c as usize)));
                        self.screen = Screen::Story;
                        return None;
                    }
                    self.screen = Screen::Message { t: 300, text: Text::Congratulations, to_custom: matches!(self.run.mode, Mode::Custom(_)) };
                    return None;
                }
                return Some(self.next_fight(audio));
            }
            // CUSTOM ALL MEMBERS: the CPU won → back to the CUSTOM menu
            if matches!(self.run.mode, Mode::Custom(_)) {
                self.open_menu(menus::Kind::Custom, audio);
                return None;
            }
            // survival (and anything without a continue): game over
            self.screen = Screen::Message { t: 240, text: Text::GameOver, to_custom: false };
            return None;
        }
        // VS / WATCH: any shoulder button held → instant rematch, else back to the select
        if held.iter().any(|h| h & 0xF != 0) {
            return Some(self.enter_vs(spec, audio));
        }
        self.enter_select(audio);
        None
    }

    /// Next ladder fight (M.1 = 2): no select, the CPU takes the next opponent.
    fn next_fight(&mut self, audio: &mut Queue) -> Event {
        let h = if self.run.humans[0] { 0 } else { 1 };
        let mut spec = MatchSpec { chr: [0; 2], costume: [0; 2], stage: 0, cpu: [!self.run.humans[0], !self.run.humans[1]] };
        spec.chr[h] = self.last[h];
        spec.costume[h] = self.run.human_costume[h];
        spec.chr[1 - h] = self.opponent();
        self.finish_spec(&mut spec);
        self.enter_vs(spec, audio)
    }

    /// Costumes of CPU slots and the same-character rule, stage of the ladder or the stage select.
    pub(super) fn finish_spec(&mut self, spec: &mut MatchSpec) {
        if self.run.mode.ladder() {
            for i in 0..2 {
                if spec.cpu[i] {
                    spec.costume[i] = if i == 0 { 1 } else { 0 };
                }
            }
            let i = self.run.a2 as usize;
            spec.stage = if self.run.mode == Mode::Survival { self.run.ladder_s[i % 8] } else { self.run.ladder_s[i % 11] };
            if self.run.br1_boss == 1 {
                spec.stage = URIKO_STAGE;
            }
        } else if self.stage_random {
            // RANDOM = rand × 9 >> 8, one of the stages; the open select's over its 13 (never an extra
            // stage)
            spec.stage = (((self.rand() & 0xFF) * self.random_count() as u32) >> 8) as u8;
        } else {
            spec.stage = self.stage;
        }
        if self.run.mode == Mode::Story {
            spec.costume = [0, 0];
        }
        resolve_costumes(spec);
    }
}

/// The same character in the same costume on both sides — the CPU side (P2 between humans) takes
/// costume 0, or 1 when it was 0.
pub(super) fn resolve_costumes(spec: &mut MatchSpec) {
    if spec.chr[0] == spec.chr[1] && spec.costume[0] == spec.costume[1] {
        let k = if spec.cpu[0] && !spec.cpu[1] { 0 } else { 1 };
        spec.costume[k] = if spec.costume[k] != 0 { 0 } else { 1 };
    }
}
