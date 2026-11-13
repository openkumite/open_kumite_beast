//! The team battle's round flow (`TeamRound`, engine addition).
//!
//! Engine addition, after the match's round flow (`fight::round`)
//! State: serialised: part of `TeamFight`
//!
//! States as the match's (`fight::round`): 0 the entrance (first round), 1 ROUND n / FINAL ROUND / SUDDEN DEATH,
//! 2 GET READY, 3 FIGHT!! (falls into 4), 4 fighting, 5 a team won the round, 7 D.K.O. (the last fighters went down
//! together), 8 draw (time over with a tie), 0xB the result, 0xC the next round, 0xD the match won. In state 4 a
//! fighter KO'd or rung out (F.0 bit 7) is out until the next round: it lies on the stage (the loser's handling of
//! states 5 / 6) and the others fight on. The round ends when at most one team has a fighter standing, or at time
//! over: the most health % wins (2 VS 2 the team's sum, FREE FOR ALL the highest), a tie is a draw for the tied
//! teams. Wins count per team; the first to `need` (2) wins the match. When several teams reach `need` at once, before
//! simulation v25 a SUDDEN DEATH round follows, from v25 (T7) the match ends in a DRAW GAME (state 0xD without a
//! winner). A TEAM ARCADE battle (simulation v26, `ArcadeRules`) shows YOU WIN / YOU LOSE and, lost or drawn, goes on
//! to 0xF CONTINUE? (the match's countdown, `fight::round::countdown`) and 0x10 GAME OVER. No KO
//! replay, special KO or challenger.

use super::{N, TeamMode};
use crate::fight::fighter::{F5C, F60, Fighter};
use crate::fight::round::Message;
use serde::{Deserialize, Serialize};

/// What the team battle must do after this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    None,
    /// rebuild the fighters for the next round with reset `mode` (2 next round, 3 sudden death)
    NextRound(u8),
    /// the match is over: this team won
    MatchOver(usize),
    /// (T7, simulation v25) the match is over: a DRAW GAME (several teams reached the rounds needed at once)
    MatchDraw,
    /// (TEAM ARCADE A3, simulation v26) START during the CONTINUE countdown
    Continue,
    /// (TEAM ARCADE A3, simulation v26) the GAME OVER screen has ended
    GameOver,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TeamRound {
    pub state: u8,
    pub cnt: i32,
    /// round index (0 = first), −1 = sudden death
    pub index: i8,
    /// rounds a team must win
    pub need: u8,
    /// rounds won by team
    pub wins: [u8; N],
    /// frames left (0xE10 = 60 s), −1 = no limit
    pub timer: i16,
    pub time_limit: i16,
    pub hud_hidden: bool,
    /// the text message of this frame
    pub message: Message,
    /// the fighters out of this round (KO'd or rung out)
    pub out: [bool; N],
    /// the teams that won this round (several on a draw or a D.K.O.)
    pub round_winners: Vec<usize>,
    /// the team that won the match
    pub winner: Option<usize>,
    /// (TEAM ARCADE) the humans' new raw presses of this frame (the countdown's and GAME OVER's input; set by the
    /// battle before `tick`, as the match's `Round::raw_press`)
    #[serde(skip)]
    pub raw_press: u16,
    /// (TEAM ARCADE) the music request of this frame: 3 YOU WIN, 4 YOU LOSE, 0x2A the continue, 0x27 GAME OVER (as the
    /// match's `Round::music`)
    #[serde(skip)]
    pub music: Option<u8>,
    /// (TEAM ARCADE) stop the music this frame
    #[serde(skip)]
    pub music_stop: bool,
}

impl Default for TeamRound {
    fn default() -> Self {
        Self::new()
    }
}

/// A fighter's health in 1/4096 (the match's time-over ratio).
fn ratio(f: &Fighter) -> i32 {
    if f.o_b0 != 0 { ((f.o_b0 as i32 - f.o_ac as i32) << 12) / f.o_b0 as i32 } else { 0 }
}

/// The loser's handling of the match's states 5 / 6 for a fighter that is out: locked, and once its fall is over
/// (F5C bit 28) the lying move 0x2E.
fn lie(f: &mut Fighter) {
    f.flags[F60] |= 0x8002_0000;
    if f.flags[F5C] & 0x1000_0000 != 0 {
        f.flags[F60] = (f.flags[F60] & 2) | 0x0081_0005;
        f.move_req = 0x2E;
    }
}

impl TeamRound {
    pub fn new() -> Self {
        Self {
            state: 0,
            cnt: 0,
            index: 0,
            need: 2,
            wins: [0; N],
            timer: 0xE10,
            time_limit: 0xE10,
            hud_hidden: true,
            message: Message::None,
            out: [false; N],
            round_winners: Vec::new(),
            winner: None,
            raw_press: 0,
            music: None,
            music_stop: false,
        }
    }

    /// The round variables at every round start.
    pub fn start(&mut self) {
        self.state = 0;
        self.cnt = 0;
        self.hud_hidden = true;
        self.timer = self.time_limit;
        self.message = Message::None;
        self.out = [false; N];
        self.round_winners.clear();
        self.winner = None;
    }

    pub fn new_match(&mut self) {
        self.index = 0;
        self.wins = [0; N];
        self.start();
    }

    /// A decoded round's state and team numbers are ones `tick` handles: a known state, the round's winners (at
    /// least one in state 5) and the match's winner teams of `mode`; `need` 1..=4 (the HUD's lamps); no wins for a
    /// team the mode does not have, none past `need` before SUDDEN DEATH; the round index −1 (SUDDEN DEATH) or at
    /// most `teams · (need − 1)` (every round gives someone a win, so a team has `need` by then).
    pub fn check(&self, mode: TeamMode) -> Result<(), String> {
        let teams = mode.teams();
        let max_index = teams as i32 * (self.need as i32 - 1);
        if !matches!(self.state, 0..=5 | 7 | 8 | 0xB..=0xD | 0xF | 0x10)
            || !(1..=4).contains(&self.need)
            || self.round_winners.iter().any(|&t| t >= teams)
            || self.round_winners.len() > teams
            || (self.state == 5 && self.round_winners.is_empty())
            || self.winner.is_some_and(|t| t >= teams)
            || self.wins[teams..].iter().any(|&w| w != 0)
            || (self.index >= 0 && self.wins.iter().any(|&w| w > self.need))
            || !(self.index == -1 || (0..=max_index).contains(&(self.index as i32)))
            // the counters `tick` steps without a wrap: cnt at most the entrance's skip value, the timers from −0x100
            // (`step_timer` turns −0x101 into −1)
            || !(0..=0x1734).contains(&self.cnt)
            || self.timer < -0x100
            || self.time_limit < -0x100
        {
            return Err("round".into());
        }
        Ok(())
    }

    fn set(&mut self, s: u8) {
        self.cnt = 0;
        self.state = s;
    }

    fn step_timer(&mut self) {
        if self.timer == 0 {
            return;
        }
        self.timer -= 1;
        if self.timer < -0x100 {
            self.timer = -1;
        }
    }

    /// The teams with a fighter standing.
    fn alive_teams(&self, mode: TeamMode) -> Vec<usize> {
        let mut t: Vec<usize> = (0..N).filter(|&k| !self.out[k]).map(|k| mode.team(k)).collect();
        t.sort_unstable();
        t.dedup();
        t
    }

    /// Time over: the teams with the most health % (2 VS 2 the sum of the team's fighters, FREE FOR ALL each
    /// fighter's; a fighter that is out counts 0).
    fn time_over_winners(&self, fs: &[Fighter; N], mode: TeamMode) -> Vec<usize> {
        let mut score = vec![0i32; mode.teams()];
        for k in 0..N {
            if !self.out[k] {
                score[mode.team(k)] += ratio(&fs[k]);
            }
        }
        let best = score.iter().copied().max().unwrap_or(0);
        (0..mode.teams()).filter(|&t| score[t] == best).collect()
    }

    /// One frame. `sounds` gets (id, delay) for the SFX queue; `pressed` = new buttons of the human pads;
    /// `g` the rules of the battle's simulation version (`version::v::SUDDEN_DEATH_DRAW`, `version::v::DRAW_GAME`);
    /// `arcade` a TEAM ARCADE battle's rules (YOU WIN / YOU LOSE, the continue countdown and GAME OVER; v26).
    pub fn tick(
        &mut self,
        fs: &mut [Fighter; N],
        mode: TeamMode,
        sounds: &mut Vec<(i16, i16)>,
        pressed: u16,
        g: crate::fight::version::Gates,
        arcade: Option<&mut super::ArcadeRules>,
    ) -> Request {
        self.message = Message::None;
        self.music = None;
        self.music_stop = false;
        let ann = |s: &mut Vec<(i16, i16)>, id: i16, d: [i16; 3]| {
            for k in 0..3 {
                s.push((id + k as i16, d[k]));
            }
        };
        match self.state {
            0 => {
                if self.index != 0 {
                    self.set(1);
                    return self.tick(fs, mode, sounds, pressed, g, arcade);
                }
                if self.cnt >= 10 && pressed & 0x8E8 != 0 {
                    // START / □ / × / ○ / R1 skips the entrance (each faces where it was placed)
                    self.cnt = 0x1734;
                    for f in fs.iter_mut() {
                        f.yaw = f.yaw_base;
                    }
                }
                if self.cnt == 60 {
                    for f in fs.iter_mut() {
                        f.move_req = 0x29;
                    }
                }
                if self.cnt < 160 {
                    self.cnt += 1;
                } else {
                    self.set(1);
                }
            }
            1 => {
                self.hud_hidden = false;
                self.message = if self.index < 0 {
                    Message::SuddenDeath
                } else if (self.index as i32 + 1) < 2 * self.need as i32 - 1 {
                    Message::Round(self.index as i32 + 1)
                } else {
                    Message::FinalRound
                };
                if self.cnt == 0 {
                    for f in fs.iter_mut() {
                        if f.o_01 != 0 {
                            f.o_01 |= 0x80;
                        }
                        f.flags[F60] = 0x0001_8004;
                    }
                }
                if self.cnt < 90 {
                    self.cnt += 1;
                } else {
                    self.set(2);
                    ann(sounds, 0x64, [0, 6, 12]);
                }
            }
            2 => {
                self.message = Message::GetReady;
                if self.cnt < 60 {
                    self.cnt += 1;
                } else {
                    for f in fs.iter_mut() {
                        if f.o_01 != 0 {
                            f.o_01 &= 0x7F;
                        }
                    }
                    self.set(3);
                    ann(sounds, 0x67, [0, 6, 12]);
                }
            }
            3 | 4 => {
                if self.state == 3 {
                    self.message = Message::Fight;
                    if self.cnt == 0 {
                        for f in fs.iter_mut() {
                            f.flags[F60] = 0;
                        }
                    } else if self.cnt >= 25 {
                        self.set(4);
                    }
                    if self.state == 3 {
                        self.cnt += 1;
                    }
                }
                self.step_timer();
                let before = self.alive_teams(mode);
                for k in 0..N {
                    if !self.out[k] && fs[k].o_00 & 0x80 != 0 {
                        // KO'd or rung out: out until the next round (the KO's sound, the round flow's at a KO)
                        self.out[k] = true;
                        sounds.push((0x5F, 0));
                    }
                }
                for k in (0..N).filter(|&k| self.out[k]) {
                    lie(&mut fs[k]);
                }
                let alive = self.alive_teams(mode);
                if alive.len() <= 1 || self.timer == 0 {
                    for k in (0..N).filter(|&k| !self.out[k]) {
                        if fs[k].o_00 == 1 {
                            fs[k].o_00 = 2;
                        }
                    }
                    if self.timer == 0 && alive.len() > 1 {
                        // (T6, simulation v24) SUDDEN DEATH starts every fighter with no health left (reset mode 3:
                        // "the next hit decides"), so what a time over finds is only the point or two some fighters won
                        // back without a hit: a DRAW for every team still standing; before v24 the
                        // most health % won it
                        self.round_winners = if self.index < 0 && g.sudden_death_draw { alive } else { self.time_over_winners(fs, mode) };
                        self.set(if self.round_winners.len() == 1 { 5 } else { 8 });
                    } else if alive.len() == 1 {
                        if self.timer == 0 {
                            self.timer = 1;
                        }
                        self.round_winners = alive;
                        self.set(5);
                    } else {
                        // the last ones went down in the same frame: a D.K.O. for their teams
                        if self.timer == 0 {
                            self.timer = 1;
                        }
                        self.round_winners = before;
                        self.set(7);
                    }
                }
            }
            5 => {
                let w = self.round_winners[0];
                if self.cnt == 0 {
                    for f in fs.iter_mut() {
                        if f.o_01 != 0 {
                            f.o_01 |= 0x80;
                        }
                    }
                    self.wins[w] = self.wins[w].saturating_add(1);
                }
                if self.timer == 0 {
                    self.message = Message::TimeOver;
                    if self.cnt == 0 {
                        ann(sounds, 0x6D, [20, 24, 28]);
                    }
                } else if self.cnt >= 2 {
                    self.message = Message::Ko;
                }
                if self.cnt == 1 && self.timer != 0 {
                    ann(sounds, 0x6A, [20, 26, 32]);
                }
                for k in 0..N {
                    if self.out[k] || mode.team(k) != w {
                        lie(&mut fs[k]);
                    } else {
                        fs[k].flags[F60] |= 0x80_0000;
                    }
                }
                self.cnt += 1;
                if self.cnt >= 120 {
                    self.set(0xB);
                }
            }
            7 | 8 => {
                self.message = if self.state == 7 { Message::Dko } else { Message::Draw };
                if self.timer == 0 && self.cnt == 0 {
                    ann(sounds, 0x6D, [20, 24, 28]);
                }
                for f in fs.iter_mut() {
                    f.flags[F60] |= 0xC082_0004;
                    if f.flags[F5C] & 0x1000_0000 != 0 {
                        f.flags[F60] = (f.flags[F60] & 2) | 0x4081_0005;
                        f.move_req = 0x2E;
                    }
                }
                if self.cnt == 0 {
                    for f in fs.iter_mut() {
                        if f.o_01 != 0 {
                            f.o_01 |= 0x80;
                        }
                    }
                    // a draw for those tied: each gets the round (the match's draw takes one from both sides)
                    for &t in &self.round_winners {
                        self.wins[t] = self.wins[t].saturating_add(1);
                    }
                }
                self.cnt += 1;
                if self.cnt >= 60 {
                    self.set(0xB);
                }
            }
            0xB => {
                self.hud_hidden = true;
                let best = self.wins.iter().copied().max().unwrap_or(0);
                let top: Vec<usize> = (0..mode.teams()).filter(|&t| self.wins[t] == best).collect();
                if best >= self.need && top.len() == 1 {
                    self.winner = Some(top[0]);
                    self.set(0xD);
                } else if best >= self.need && g.draw_game {
                    // (T7, simulation v25) several teams reached the rounds needed at once (a D.K.O. or a time-over tie
                    // in the deciding round): the match is a DRAW GAME (state 0xD without a winner), no SUDDEN DEATH
                    self.winner = None;
                    self.set(0xD);
                } else {
                    if best >= self.need {
                        self.index = -1;
                    }
                    self.state = 0xC;
                }
            }
            0xC => {
                if self.cnt >= 30 {
                    for f in fs.iter_mut() {
                        f.flags[F60] |= 0x8002_8000;
                    }
                }
                if self.cnt < 90 {
                    self.cnt += 1;
                } else {
                    if self.index >= 0 {
                        self.index += 1;
                    }
                    self.cnt = 0;
                    return Request::NextRound(if self.index < 0 { 3 } else { 2 });
                }
            }
            0xD => {
                // (T7, simulation v25) no winner: the DRAW GAME, every fighter still standing locked without a pose
                let draw = self.winner.is_none() && g.draw_game;
                let w = self.winner.unwrap_or(0);
                // (TEAM ARCADE) the humans' team lost (a DRAW GAME counts as a loss)
                let lost = arcade.as_ref().map(|a| a.lost(self.winner.filter(|_| !draw)));
                self.message = match mode {
                    _ if draw => Message::DrawGame,
                    // (TEAM ARCADE) the match's messages against the CPU
                    _ if lost.is_some() => {
                        if lost == Some(true) {
                            Message::YouLose
                        } else {
                            Message::YouWin
                        }
                    }
                    TeamMode::Vs2 => Message::TeamWins(w as i32 + 1),
                    TeamMode::Ffa => Message::PlayerWins(w as i32 + 1),
                };
                if self.cnt == 0 {
                    // (TEAM ARCADE) the match's tracks: 3 won, 4 YOU LOSE; YOU WIN announced
                    match lost {
                        Some(true) => self.music = Some(4),
                        Some(false) => {
                            self.music = Some(3);
                            ann(sounds, 0x61, [20, 26, 32]);
                        }
                        None => {}
                    }
                }
                for k in 0..N {
                    let f = &mut fs[k];
                    if draw && !self.out[k] {
                        f.flags[F60] |= 0x40A1_0005;
                    } else if !self.out[k] && mode.team(k) == w {
                        // the winners' pose (the match's: stand at cnt 0, the win pose at cnt 2)
                        match self.cnt {
                            0 => f.move_req = 0x28,
                            2 => f.move_req = 0x2C,
                            _ => {}
                        }
                        f.flags[F60] |= 0x40A1_0005;
                    } else {
                        f.flags[F60] |= 0x40A1_0007;
                        f.o_c0 = -1;
                    }
                }
                if self.cnt >= 10 && pressed & 0x800 != 0 {
                    self.cnt = 360;
                }
                if self.cnt >= 360 {
                    // (TEAM ARCADE) a lost battle → the continue countdown (as the match's)
                    if let Some(a) = arcade.filter(|_| lost == Some(true)) {
                        a.b2 = 0;
                        a.go_cnt = 0;
                        self.set(0xF);
                        return Request::None;
                    }
                    return if draw { Request::MatchDraw } else { Request::MatchOver(w) };
                }
                self.cnt += 1;
            }
            // (TEAM ARCADE) CONTINUE? 9..0 (the match's): the winners hidden, the losers in the continue pose,
            // either human's START continues
            0xF => {
                let Some(a) = arcade else { return Request::None };
                self.hud_hidden = true;
                for (k, f) in fs.iter_mut().enumerate() {
                    if mode.team(k) != a.human_team as usize {
                        f.flags[F60] |= 2;
                        continue;
                    }
                    f.flags[F60] &= !2;
                    if f.flags[F5C] & 0x1000_0000 != 0 || f.move_cur == 0x2E {
                        f.flags[F60] = 0x4081_0005;
                        f.move_req = 0x2D;
                    }
                }
                if self.cnt == 1 {
                    self.music = Some(0x2A);
                }
                if a.b2 % 100 == 4 {
                    sounds.push((0x11, 0)); // countdown tick (COMMON_2 tone 17)
                }
                self.cnt += 1;
                match crate::fight::round::countdown(&mut a.b2, self.raw_press) {
                    1 => {
                        self.cnt = 0;
                        return Request::Continue;
                    }
                    -1 => {
                        self.music_stop = true;
                        a.go_cnt = 0;
                        self.set(0x10);
                    }
                    _ => self.message = Message::Continue(9 - a.b2 as i32 / 100),
                }
            }
            // (TEAM ARCADE) GAME OVER (the match's; `cnt` its wait +0x90): 900 frames, a press skips them
            0x10 => {
                let Some(a) = arcade else { return Request::None };
                self.hud_hidden = true;
                if self.raw_press & 0x8E8 != 0 {
                    a.go_cnt = 0x1734;
                }
                self.cnt = (self.cnt + 1).min(4);
                if self.cnt < 4 {
                    return Request::None;
                }
                if a.go_cnt == 0 {
                    self.music = Some(0x27);
                }
                self.message = Message::GameOver;
                a.go_cnt += 1;
                if a.go_cnt >= 900 {
                    a.go_cnt = 0;
                    self.music_stop = true;
                    return Request::GameOver;
                }
            }
            _ => {}
        }
        Request::None
    }
}
