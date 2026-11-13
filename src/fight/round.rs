//! Round and match flow: the round states, the timer, KO and time-over decisions, continue.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! Round / match flow: the round state machine (round state = match +0x92 low byte, frame counter +0x8C).
//! States: 0 intro (entrance poses, first round only), 1 ROUND n / FINAL ROUND / SUDDEN DEATH, 2 GET READY,
//! 3 FIGHT!! (falls into 4), 4 fighting (timer, time-over and KO decisions), 5 / 6 round won by
//! P1 / P2, 7 double KO, 8 draw, 0xB result, 0xC next round, 0xD / 0xE match won by P1 / P2, 0xF continue
//! (with the countdown), 0x10 game over.
//! The challenger interrupt and the pause run in fight/sim/frame.rs (Match::tick). After the match-end pose (360 frames) a new
//! match starts unless the human lost a 1P match with `continue_ok` set: then the continue countdown runs.

use crate::fight::camera::{self, Camera};
use crate::fight::fighter::{F5C, F6C, F58, F60, F64, Fighter};
use serde::{Deserialize, Serialize};

/// The round flow's message of a frame, by what it says; its words are the game's text (`text`), the team
/// battle's and the challenger's the engine's own. Serialised as its text, as the state held it as a string before;
/// decoded by the installed text (`of_text`), and a string that is no message under it (another text installed when it
/// was written, or none now) kept verbatim (`Other`): a state decodes and encodes back to the same bytes whatever text
/// is installed, and its hash with them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Message {
    #[default]
    None,
    Replay,
    /// ROUND n
    Round(i32),
    FinalRound,
    SuddenDeath,
    GetReady,
    Fight,
    TimeOver,
    OutOfRing,
    Perfect,
    Ko,
    /// the team battle's double KO
    Dko,
    Draw,
    YouWin,
    YouLose,
    Winner,
    /// CONTINUE? n
    Continue(i32),
    GameOver,
    /// "Here Comes A Challenger !!!" (engine wording)
    Challenger,
    /// the team battle's (engine addition): DRAW GAME, TEAM n WINS, Pn WINS
    DrawGame,
    TeamWins(i32),
    PlayerWins(i32),
    /// a decoded text that is none of the above under the installed text (a state written with another text installed,
    /// e.g. read by a tool without the game data): kept as it was, shown and serialised unchanged
    Other(OtherText),
}

/// The longest text a decoded `Message::Other` may hold (the game's longest message is 27 bytes).
const MAX_OTHER: usize = 64;

/// A decoded message's text kept verbatim (`Message::Other`): its bytes in place, so decoding leaks nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OtherText {
    len: u8,
    bytes: [u8; MAX_OTHER],
}

impl OtherText {
    /// `s` kept; None when it is longer than `MAX_OTHER` bytes.
    pub fn new(s: &str) -> Option<OtherText> {
        let mut bytes = [0; MAX_OTHER];
        bytes.get_mut(..s.len())?.copy_from_slice(s.as_bytes());
        Some(OtherText { len: s.len() as u8, bytes })
    }

    pub fn as_str(&self) -> &str {
        // built from a str (`new`), cut nowhere else
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or_default()
    }
}

impl std::fmt::Debug for OtherText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_str().fmt(f)
    }
}

impl Message {
    /// What it shows ("" for none).
    pub fn text(self) -> String {
        use crate::assets::text::{fill, get};
        match self {
            Message::None => String::new(),
            Message::Replay => get("replay").into(),
            Message::Round(n) => format!("{} {n}", get("round")),
            Message::FinalRound => get("final_round").into(),
            Message::SuddenDeath => get("sudden_death").into(),
            Message::GetReady => get("get_ready").into(),
            Message::Fight => get("fight").into(),
            Message::TimeOver => get("time_over").into(),
            Message::OutOfRing => get("out_of_ring").into(),
            Message::Perfect => get("perfect").into(),
            Message::Ko => get("k_o").into(),
            Message::Dko => get("d_k_o").into(),
            Message::Draw => get("draw").into(),
            Message::YouWin => get("you_win").into(),
            Message::YouLose => get("you_lose").into(),
            Message::Winner => get("winner").into(),
            Message::Continue(n) => fill(get("continue_n"), &[&n]),
            Message::GameOver => get("game_over").into(),
            Message::Challenger => "Here Comes A Challenger !!!".into(),
            Message::DrawGame => "DRAW GAME".into(),
            Message::TeamWins(n) => format!("TEAM {n} WINS"),
            Message::PlayerWins(n) => format!("P{n} WINS"),
            Message::Other(s) => s.as_str().into(),
        }
    }

    /// The message whose text `s` is (`text`); None for any other.
    pub fn of_text(s: &str) -> Option<Message> {
        use Message as M;
        let fixed = [
            M::None,
            M::Replay,
            M::FinalRound,
            M::SuddenDeath,
            M::GetReady,
            M::Fight,
            M::TimeOver,
            M::OutOfRing,
            M::Perfect,
            M::Ko,
            M::Dko,
            M::Draw,
            M::YouWin,
            M::YouLose,
            M::Winner,
            M::GameOver,
            M::Challenger,
            M::DrawGame,
        ];
        if let Some(m) = fixed.into_iter().find(|m| m.text() == s) {
            return Some(m);
        }
        // a numbered one: its text with the number in place
        [M::Round(0), M::Continue(0), M::TeamWins(0), M::PlayerWins(0)].into_iter().find_map(|m| {
            let blank = m.text();
            let (head, tail) = blank.split_once('0')?;
            let n: i32 = s.strip_prefix(head)?.strip_suffix(tail)?.parse().ok()?;
            let m = match m {
                M::Round(_) => M::Round(n),
                M::Continue(_) => M::Continue(n),
                M::TeamWins(_) => M::TeamWins(n),
                _ => M::PlayerWins(n),
            };
            (m.text() == s).then_some(m)
        })
    }

    pub fn is_none(self) -> bool {
        self == Message::None
    }
}

impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

impl Serialize for Message {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text())
    }
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let other = OtherText::new(&s).ok_or_else(|| serde::de::Error::custom("round message too long"))?;
        Ok(Message::of_text(&s).unwrap_or(Message::Other(other)))
    }
}

/// What the match must do after this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    None,
    /// rebuild the fighters for the next round (loader state 6) with reset `mode` (match +0x9A: 0 first round,
    /// 2 next round, 3 sudden death)
    NextRound(u8),
    /// start a new match
    NewMatch,
    /// continue accepted (START during the countdown)
    Continue,
    /// the match-end reset (round state 0xD / 0xE at cnt 0, simulation version 6): effects, fades
    /// and both fighters (the fighter reset, mode 4), then `win_tail`
    WinReset,
    /// the game-over screen has ended
    GameOver,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Round {
    pub state: u8,
    pub cnt: i32,
    /// match +0x99: round index (0 = first), −1 = sudden death
    pub index: i8,
    /// rounds a fighter must win (opt +4 = MATCH POINT + 1)
    pub need: u8,
    /// fighter +0xE4: rounds lost in this match
    pub lost: [u8; 2],
    /// match +0x94: frames left (0xE10 = 60 s), −1 = no limit
    pub timer: i16,
    pub time_limit: i16,
    /// match +0xB0: HUD bars hidden
    pub hud_hidden: bool,
    /// the text message of this frame
    pub message: Message,
    pub winner: Option<usize>,
    /// music request of this frame (a music track): 3 = match won, 4 = YOU LOSE
    pub music: Option<u8>,
    /// set by the match when the KO replay has played to its end (replay mode 4)
    pub replay_done: bool,
    /// a lost 1P match goes to the continue screen (arcade / story; set by the front end)
    pub continue_ok: bool,
    /// new raw presses of both ports this frame (START is not part of the fighters' logical pads)
    pub raw_press: [u16; 2],
    /// match +0xB2: continue countdown frames 0..999
    b2: u16,
    /// match +0x8E / +0x90: game-over frame counters
    go_cnt: i32,
    go_wait: i32,
    /// the side that lost the match (continue / game over)
    pub loser: usize,
    /// stop the music this frame (`play(0x204)`)
    pub music_stop: bool,
    /// play the game-over voice of this side (GOVER_cc tone 0, 30 frames late)
    pub gover_voice: Option<usize>,
    /// PRACTICE: the practice flow replaces the round flow
    pub practice: bool,
    /// the attract demo: no round flow at all (the demo steps the fight without it)
    pub demo: bool,
    /// the special KO (recorder +0xA): stage in the low byte, the M.9E value of replay #2 above
    pub special: u16,
    /// recorder +0xC (frames of the stage) and +0xE (the timer kept over the replays)
    special_c: i16,
    special_timer: i16,
    special_pending: u16,
    /// M.9E, the frame-persistence mode the round sets (2 = white, 1 = dark trails)
    pub m9e: u8,
    /// a replay command for the recorder (fight/sim/mod.rs), taken once
    pub replay_cmd: Option<ReplayCmd>,
    /// the match-end reset runs (`Request::WinReset`; simulation version 6, set by the match every round)
    pub win_reset: bool,
}

/// The special KO's recorder calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayCmd {
    /// Keep the KO moment, play back from the middle snapshot
    Start,
    /// Play back the same snapshot again
    Restart,
    /// Back to the KO moment (the camera stays)
    Restore,
}

/// Once at the KO: the special-KO kind (0 = none; 0x201 white, 0x101 dark trails, 1 normal background).
fn special_ko_kind(w: &Fighter, l: &Fighter) -> u16 {
    let f5c = w.flags[F5C];
    if f5c & 8 == 0 {
        return 0; // the winner's attack did not connect
    }
    let mut a = 0;
    // the last hit of a chained string: a chain follow-up with no further link open or queued
    if f5c & 0x40 != 0 && w.flags[F6C] & 0x200_0000 == 0 && w.chain_sel < 0 {
        a = 1;
        if w.form & 1 != 0 {
            a |= 0x200;
        }
    }
    if w.flags[F64] & 0x2000 != 0 {
        a = 1;
        if w.form & 1 != 0 {
            a |= 0x100;
        }
    }
    if l.flags[F58] & 0x400_0000 != 0 {
        a = 1; // the loser was blown away
    }
    a
}

impl Default for Round {
    fn default() -> Self {
        Self::new()
    }
}

impl Round {
    pub fn new() -> Self {
        Self {
            state: 0,
            cnt: 0,
            index: 0,
            need: 2,
            lost: [0; 2],
            timer: 0xE10,
            time_limit: 0xE10,
            hud_hidden: true,
            message: Message::None,
            winner: None,
            music: None,
            replay_done: false,
            continue_ok: false,
            raw_press: [0; 2],
            b2: 0,
            go_cnt: 0,
            go_wait: 0,
            loser: 1,
            music_stop: false,
            gover_voice: None,
            practice: false,
            demo: false,
            special: 0,
            special_c: 0,
            special_timer: 0,
            special_pending: 0,
            m9e: 0,
            replay_cmd: None,
            win_reset: false,
        }
    }

    /// Round variables at the start of every round.
    pub fn start(&mut self) {
        self.state = 0;
        self.cnt = 0;
        self.hud_hidden = true;
        self.timer = self.time_limit;
        self.winner = None;
        self.message = Message::None;
        self.special = 0;
        self.special_pending = 0;
        self.m9e = 0;
        self.replay_cmd = None;
    }

    /// Round state 0xD / 0xE after the cnt-0 reset: the winner's and loser's moves, the
    /// camera, the F60 locks; at cnt 2 the win pose.
    pub fn win_tail(&mut self, fs: &mut [Fighter; 2], cam: &mut Camera, pressed: u16) {
        let (w, l) = if self.state == 0xD { (0, 1) } else { (1, 0) };
        if self.cnt == 0 {
            fs[w].move_req = 0x28;
            fs[l].move_req = if self.timer != 0 { 0x67 } else { 0x2D };
            cam.snap = -1;
            cam.set_mode(camera::MOVE);
        } else if self.cnt == 2 {
            if self.win_reset {
                // the loser's current key ends on the next frame (S.C2 += F.88 − S.CA, S.C8 = S.CA = F.88)
                let (a, step) = (&mut fs[l].anim, fs[l].step);
                a.c2 = a.c2.wrapping_add(step.wrapping_sub(a.left));
                (a.c8, a.left) = (step, step);
            }
            fs[w].move_req = 0x2C; // win pose (its script drives the camera)
        }
        fs[w].flags[F60] |= 0x40A1_0005;
        fs[l].flags[F60] |= 0x40A1_0007;
        fs[l].o_c0 = -1;
        if self.cnt >= 10 && pressed & 0x800 != 0 {
            self.cnt = 0x1734;
        }
        self.cnt += 1;
    }

    pub fn new_match(&mut self) {
        self.index = 0;
        self.lost = [0; 2];
        self.start();
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

    /// One frame of the special KO — 10 live frames, REPLAY #1 (camera 0xE), REPLAY #2 (camera 0xF,
    /// M.9E from its 3rd frame), then back to the KO moment.
    fn special_step(&mut self, fs: &[Fighter; 2], sounds: &mut Vec<(i16, i16)>, cam: &mut Camera) {
        let stage = self.special & 0xFF;
        let both_up = fs[0].o_00 == 1 && fs[1].o_00 == 1;
        match stage {
            1 => {
                self.special_timer = self.timer;
                self.special_c = 0;
                self.special = (self.special & !0xFF) | 2;
            }
            2 => {
                self.special_c += 1;
                if self.special_c >= 10 {
                    self.special_c = 0;
                    self.special = (self.special & !0xFF) | 3;
                    self.m9e = 0;
                }
            }
            3 | 4 => {
                self.message = Message::Replay;
                if both_up {
                    self.step_timer();
                }
                if self.special_c == 0 {
                    cam.set_mode(if stage == 3 { 0xE } else { 0xF });
                    self.replay_cmd = Some(if stage == 3 { ReplayCmd::Start } else { ReplayCmd::Restart });
                    self.replay_done = false;
                }
                if stage == 3 && self.special_c == 1 {
                    self.m9e = 0;
                }
                if stage == 4 && self.special_c == 2 {
                    self.m9e = (self.special >> 8) as u8;
                }
                self.special_c += 1;
                if self.replay_done && self.special_c > 1 {
                    self.replay_done = false;
                    sounds.push((if stage == 3 { 0x5F } else { 0x60 }, 0));
                    self.special_c = 0;
                    self.special = (self.special & !0xFF) | (stage + 1);
                    if stage == 3 {
                        self.m9e = 0;
                    }
                }
            }
            _ => {
                // stage 5: the KO moment again; M.9E stays until cnt 120
                cam.snap = -3;
                self.replay_cmd = Some(ReplayCmd::Restore);
                self.special = 0;
                self.timer = self.special_timer;
            }
        }
    }

    /// WATCH mode's camera buttons (every frame of state 4); `p` = new presses of both pads.
    /// ○ × □: the fight camera; R1: the next TV preset; SELECT: the HUD on / off.
    pub fn watch_camera(&mut self, cam: &mut Camera, p: u16) {
        const NEXT: [i16; 8] = [4, 7, 5, 6, 1, 3, 0, 2]; // the game's jump table
        if self.state != 4 {
            return;
        }
        if p & 0xE0 != 0 {
            cam.set_mode(camera::MOVE);
        } else if p & 0x08 != 0 {
            if let Some(&n) = NEXT.get(cam.preset as usize) {
                cam.preset = n;
            }
            cam.set_mode(0x20);
        }
        // SELECT shows / hides the HUD (M.B0 ^= 1; the next round start shows it again)
        if p & 0x100 != 0 {
            self.hud_hidden = !self.hud_hidden;
        }
    }

    /// HUD seconds: −1 = no limit.
    pub fn seconds(&self) -> i32 {
        if self.timer < 0 { -1 } else { self.timer as i32 / 60 }
    }

    /// One frame. `sounds` gets (id, delay) for the SFX queue; `pressed` = new buttons of both pads.
    pub fn tick(&mut self, fs: &mut [Fighter; 2], sounds: &mut Vec<(i16, i16)>, cam: &mut Camera, pressed: u16) -> Request {
        self.message = Message::None;
        self.music = None;
        self.music_stop = false;
        self.gover_voice = None;
        if self.practice {
            return self.practice_tick(fs, cam);
        }
        if self.demo {
            // no messages, HUD, timer or KO handling; the camera snaps once, F60 is cleared on the 2nd frame
            self.hud_hidden = true;
            self.timer = -1;
            match self.state {
                // the camera is the demo's own pick (fight/sim/frame.rs demo_camera)
                0 => self.state = 1,
                1 => {
                    fs[0].flags[F60] = 0;
                    fs[1].flags[F60] = 0;
                    self.state = 4;
                }
                _ => {}
            }
            return Request::None;
        }
        let ann = |s: &mut Vec<(i16, i16)>, id: i16, d: [i16; 3]| {
            for k in 0..3 {
                s.push((id + k as i16, d[k]));
            }
        };
        match self.state {
            0 => {
                if self.index != 0 {
                    cam.snap = -1;
                    cam.set_mode(camera::MOVE);
                    self.set(1);
                    return self.tick(fs, sounds, cam, pressed);
                }
                if self.cnt == 0 {
                    cam.snap = -1;
                    cam.set_mode(camera::MOVE);
                }
                if self.cnt >= 10 && pressed & 0x8E8 != 0 {
                    // START / □ / × / ○ / R1 skips the entrance
                    self.cnt = 0x1734;
                    fs[0].yaw = 0x400;
                    fs[0].yaw_base = 0x400;
                    fs[1].yaw = -0x400;
                    fs[1].yaw_base = -0x400;
                }
                if self.cnt == 60 {
                    fs[0].move_req = 0x29;
                    fs[1].move_req = 0x29;
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
                        fs[0].flags[F60] = 0;
                        fs[1].flags[F60] = 0;
                    } else if self.cnt >= 25 {
                        self.set(4);
                    }
                    if self.state == 3 {
                        self.cnt += 1;
                    }
                }
                self.step_timer();
                if self.timer == 0 {
                    for f in fs.iter_mut() {
                        if f.o_00 == 1 {
                            f.o_00 = 2;
                        }
                    }
                    let ratio = |f: &Fighter| if f.o_b0 != 0 { ((f.o_b0 as i32 - f.o_ac as i32) << 12) / f.o_b0 as i32 } else { 0 };
                    let (r1, r2) = (ratio(&fs[0]), ratio(&fs[1]));
                    self.set(if r1 == r2 {
                        8
                    } else if r2 < r1 {
                        5
                    } else {
                        6
                    });
                }
                let (k1, k2) = ((fs[0].o_00 as i8) < 0, (fs[1].o_00 as i8) < 0);
                if k1 || k2 {
                    if self.timer == 0 {
                        self.timer = 1;
                    }
                    if k1 && k2 {
                        self.set(7);
                    } else {
                        let w = if k2 { 0 } else { 1 };
                        if fs[w].o_00 == 1 {
                            fs[w].o_00 = 2;
                        }
                        // the special-KO check reads the fighters at the start of the next frame (the game's round flow runs
                        // before the fighters, the engine's after them): these are those flags
                        self.special_pending = special_ko_kind(&fs[w], &fs[1 - w]);
                        self.set(if w == 0 { 5 } else { 6 });
                    }
                }
            }
            5 | 6 => {
                let (w, l) = if self.state == 5 { (0, 1) } else { (1, 0) };
                self.winner = Some(w);
                if self.cnt == 0 {
                    for f in fs.iter_mut() {
                        if f.o_01 != 0 {
                            f.o_01 |= 0x80;
                        }
                    }
                }
                if self.timer == 0 && self.special == 0 {
                    self.message = Message::TimeOver;
                    if self.cnt == 0 {
                        ann(sounds, 0x6D, [20, 24, 28]);
                        self.lost[l] += 1;
                    }
                    fs[w].flags[F60] |= 0x80_0000;
                    fs[l].flags[F60] |= 0x80_0000;
                    self.cnt += 1;
                } else {
                    let perfect = fs[w].o_ac == 0;
                    if fs[l].o_00 == 0x81 {
                        if self.cnt >= 2 {
                            self.message = Message::OutOfRing;
                        }
                    } else if self.cnt >= 2 {
                        self.message = if perfect { Message::Perfect } else { Message::Ko };
                    }
                    if self.cnt == 0 {
                        if fs[l].o_00 != 0x81 {
                            self.special = self.special_pending;
                        }
                        sounds.push((0x5F, 0));
                        self.lost[l] += 1;
                        self.cnt += 1;
                    }
                    if self.special != 0 {
                        // the special KO instead of the K.O. announcer and the frame count (cnt stays 1)
                        self.special_step(fs, sounds, cam);
                    } else {
                        fs[w].flags[F60] |= 0x80_0000;
                        fs[l].flags[F60] |= 0x80_0000;
                        if self.cnt == 1 && fs[l].o_00 != 0x81 {
                            ann(sounds, if perfect { 0x70 } else { 0x6A }, [20, 26, 32]);
                        }
                        self.cnt += 1;
                    }
                }
                fs[l].flags[F60] |= 0x8002_0000;
                if self.special == 0 && fs[l].flags[F5C] & 0x1000_0000 != 0 {
                    fs[l].flags[F60] = (fs[l].flags[F60] & 2) | 0x0081_0005;
                    fs[l].move_req = 0x2E;
                }
                if self.cnt >= 120 {
                    self.m9e = 0;
                    // a real KO that decides the match is replayed (states 9 / 10)
                    if self.timer != 0 && fs[l].o_00 != 0x81 && self.lost[l] >= self.need {
                        self.replay_done = false;
                        self.set(if w == 0 { 9 } else { 10 });
                    } else {
                        self.set(0xB);
                    }
                }
            }
            9 | 10 => {
                // final KO replay (the match re-simulates from a snapshot with the logged pads)
                self.message = Message::Replay;
                self.hud_hidden = true;
                self.step_timer();
                for f in fs.iter_mut() {
                    f.flags[F60] |= 0x8002_0000;
                }
                if self.cnt == 0 {
                    // REPLAY_1P / REPLAY_2P on the winner
                    cam.set_mode(if self.state == 9 { 10 } else { 11 });
                }
                let skip = self.cnt >= 20 && pressed & 0x800 != 0;
                self.cnt += 1;
                if self.replay_done || skip {
                    self.replay_done = false;
                    cam.snap = -3;
                    cam.set_mode(camera::MOVE);
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
                    if self.index >= 0 {
                        self.lost[0] += 1;
                        self.lost[1] += 1;
                    } else if (fs[0].o_01 != 0) == (fs[1].o_01 != 0) {
                        self.lost[1] += 1; // (win streaks / challenger decide in the game)
                    } else if fs[0].o_01 != 0 {
                        self.lost[1] += 1;
                    } else {
                        self.lost[0] += 1;
                    }
                }
                self.cnt += 1;
                if self.cnt >= 60 {
                    self.set(0xB);
                }
            }
            0xB => {
                self.hud_hidden = true;
                let (l1, l2, need) = (self.lost[0], self.lost[1], self.need);
                if l2 >= need && (l1 < need || l1 < l2) {
                    cam.set_mode(camera::MOVE);
                    self.set(0xD);
                } else if l1 >= need && (l2 < need || l2 < l1) {
                    cam.set_mode(camera::MOVE);
                    self.set(0xE);
                } else {
                    if l1 >= need && l2 >= need {
                        self.index = -1;
                    }
                    self.state = 0xC;
                }
            }
            0xC => {
                if self.cnt >= 30 {
                    fs[0].flags[F60] |= 0x8002_8000;
                    fs[1].flags[F60] |= 0x8002_8000;
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
            0xD | 0xE => {
                let (w, l) = if self.state == 0xD { (0, 1) } else { (1, 0) };
                self.winner = Some(w);
                if self.cnt >= 360 {
                    self.cnt = 0;
                    // a human beaten by the CPU in a 1P mode → continue (participation 0x600)
                    if self.continue_ok && fs[l].o_01 == 0 && fs[w].o_01 != 0 {
                        self.loser = l;
                        self.b2 = 0;
                        self.set(0xF);
                        return Request::None;
                    }
                    return Request::NewMatch;
                }
                fs[w].flags[F60] |= 0x8002_0000;
                fs[l].flags[F60] |= 0x8002_0000;
                let vs_cpu = (fs[0].o_01 != 0) != (fs[1].o_01 != 0);
                if vs_cpu {
                    if fs[w].o_01 != 0 {
                        self.message = Message::YouLose;
                        if self.cnt == 0 {
                            self.music = Some(4);
                        }
                    } else {
                        self.message = Message::YouWin;
                        if self.cnt == 0 {
                            self.music = Some(3);
                            ann(sounds, 0x61, [20, 26, 32]);
                        }
                    }
                } else {
                    self.message = Message::Winner;
                    if self.cnt == 0 {
                        self.music = Some(3);
                    }
                }
                if self.cnt == 0 && self.win_reset {
                    // the reset, then the rest of the frame (`win_tail`)
                    return Request::WinReset;
                }
                self.win_tail(fs, cam, pressed);
            }
            0xF => {
                self.hud_hidden = true;
                let (l, w) = (self.loser, 1 - self.loser);
                // the winner is hidden, the camera looks down on the loser (LOSE_1P / LOSE_2P)
                fs[w].flags[F60] |= 2;
                fs[l].flags[F60] &= !2;
                if self.cnt == 0 {
                    cam.set_mode(6 + l as i16);
                }
                if fs[l].flags[F5C] & 0x1000_0000 != 0 || fs[l].move_cur == 0x2E {
                    fs[l].flags[F60] = 0x4081_0005;
                    fs[l].move_req = 0x2D;
                }
                if self.cnt == 1 {
                    self.music = Some(0x2A);
                }
                if self.b2 % 100 == 4 {
                    sounds.push((0x11, 0)); // countdown tick (COMMON_2 tone 17)
                }
                self.cnt += 1;
                match countdown(&mut self.b2, self.raw_press[l]) {
                    1 => {
                        self.cnt = 0;
                        return Request::Continue;
                    }
                    -1 => {
                        self.music_stop = true;
                        self.go_cnt = 0;
                        self.go_wait = 0;
                        self.set(0x10);
                    }
                    _ => self.message = Message::Continue(9 - self.b2 as i32 / 100),
                }
            }
            0x10 => {
                self.hud_hidden = true;
                let l = self.loser;
                if (self.raw_press[0] | self.raw_press[1]) & 0x8E8 != 0 {
                    self.go_cnt = 0x1734;
                }
                self.go_wait += 1;
                if self.go_wait < 4 {
                    return Request::None;
                }
                if self.go_cnt == 0 {
                    cam.set_mode(6 + l as i16);
                    self.music = Some(0x27);
                    if !fs[l].beast() {
                        self.gover_voice = Some(l);
                    }
                }
                self.message = Message::GameOver;
                self.go_cnt += 1;
                if self.go_cnt >= 900 {
                    self.go_cnt = 0;
                    self.music_stop = true;
                    return Request::GameOver;
                }
            }
            _ => {}
        }
        Request::None
    }

    /// The practice flow: states 0..3 one frame each (camera snap, fighters released, recording),
    /// then 4 = free fighting with no time limit and no KO; SELECT resets positions, health and gauge.
    fn practice_tick(&mut self, fs: &mut [Fighter; 2], cam: &mut Camera) -> Request {
        // the round init hides the HUD (M.B0 = 1) and the practice flow never shows it: no bars, timer,
        // gauges or combo counter; only the PRACTICE MODE label
        self.hud_hidden = true;
        self.timer = -1;
        match self.state {
            0 => {
                cam.snap = -1;
                cam.set_mode(camera::MOVE);
                self.state = 1;
            }
            1 | 2 => self.state += 1,
            3 => {
                fs[0].flags[F60] = 0;
                fs[1].flags[F60] = 0;
                self.state = 4;
            }
            _ => {
                if (self.raw_press[0] | self.raw_press[1]) & 0x100 != 0 {
                    return Request::NextRound(0);
                }
            }
        }
        Request::None
    }
}

/// The continue countdown of the losing port on its frame counter `b2` (match +0xB2, 0..999; the team
/// battle's TEAM ARCADE countdown too, `fight::team::arcade`). 1 = START (continue), −1 = time is up. One digit per
/// 100 frames; □ × ○ R1 after frame 200 jump to the next hundred.
pub fn countdown(b2: &mut u16, press: u16) -> i32 {
    if *b2 < 999 {
        *b2 += 1;
    }
    if press & 0x800 != 0 {
        return 1;
    }
    if *b2 < 4 {
        return 0;
    }
    if *b2 >= 200 && press & 0xE8 != 0 {
        *b2 = (*b2 + 100) / 100 * 100;
    }
    if *b2 < 999 { 0 } else { -1 }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{Message, OtherText};

    /// With the test data's text (data-gated) every message is serialised as its text (the string the state held before:
    /// the replay fixtures' state hashes check that the bytes are the same) and decodes back to itself; an unknown string
    /// is refused.
    #[test]
    fn messages_serialise_as_their_text() {
        if crate::formats::tables::export_tables().is_none() {
            eprintln!("skipped: no okb_export");
            return;
        }
        use Message as M;
        let all = [
            M::None,
            M::Replay,
            M::Round(2),
            M::FinalRound,
            M::SuddenDeath,
            M::GetReady,
            M::Fight,
            M::TimeOver,
            M::OutOfRing,
            M::Perfect,
            M::Ko,
            M::Dko,
            M::Draw,
            M::YouWin,
            M::YouLose,
            M::Winner,
            M::Continue(7),
            M::GameOver,
            M::Challenger,
            M::DrawGame,
            M::TeamWins(2),
            M::PlayerWins(3),
        ];
        for m in all {
            let bytes = postcard::to_allocvec(&m).unwrap();
            assert_eq!(bytes, postcard::to_allocvec(&m.text()).unwrap(), "{m:?}");
            assert_eq!(postcard::from_bytes::<Message>(&bytes).unwrap(), m, "{m:?}");
        }
    }

    /// A string that is no message under the installed text (whatever is installed) decodes as `Other` and encodes back
    /// to the same bytes; a longer one than any message is refused.
    #[test]
    fn unknown_messages_kept_verbatim() {
        let bytes = postcard::to_allocvec("A MESSAGE OF ANOTHER TEXT").unwrap();
        let m = postcard::from_bytes::<Message>(&bytes).unwrap();
        assert_eq!(m, Message::Other(OtherText::new("A MESSAGE OF ANOTHER TEXT").unwrap()));
        assert_eq!(m.text(), "A MESSAGE OF ANOTHER TEXT");
        assert_eq!(postcard::to_allocvec(&m).unwrap(), bytes);
        assert!(postcard::from_bytes::<Message>(&postcard::to_allocvec(&"X".repeat(64)).unwrap()).is_ok());
        assert!(postcard::from_bytes::<Message>(&postcard::to_allocvec(&"X".repeat(65)).unwrap()).is_err());
    }
}
