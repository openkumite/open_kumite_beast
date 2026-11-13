//! Recording: the app's always-on `Recorder` of a live session and the matches it saves as files.
//!
//! Engine addition.

use super::inputs::InputWriter;
use super::sync::checkpoint;
use super::{Body, CHECK_EVERY, Header, MAX_CHEATS, MAX_FRAMES, MAX_PREFS, Replay};
use crate::fight::sim::Outcome;
use crate::frontend::options::Prefs;
use crate::frontend::{MatchSpec, TeamSpec};
use crate::session::{Cheat, FrameInput, GameState, STATE_VERSION, SessionStart};

/// The match under way, as the recorder saw it start.
#[derive(Clone, Debug)]
struct Game {
    from: u32,
    fight_from: Option<u32>,
    mode: String,
    spec: Option<MatchSpec>,
    /// a team battle's (engine addition): its sides
    team: Option<TeamSpec>,
    /// (TEAM ARCADE A4) the battle's place in its run
    arcade: Option<super::TeamArcadeInfo>,
    demo: bool,
}

/// The always-on recording of a live session (the app's): every step's input, cheats and preference changes, a
/// checkpoint every `CHECK_EVERY` steps, and the matches' bounds.
pub struct Recorder {
    /// the header without the match part: the start, the costumes, the first preferences
    header: Header,
    sim_version: u32,
    inputs: InputWriter,
    cheats: Vec<(u32, Cheat)>,
    prefs: Vec<(u32, Prefs)>,
    checks: Vec<(u32, u64, u64)>,
    last_prefs: Prefs,
    frames: u32,
    game: Option<Game>,
}

impl Recorder {
    /// A session started from `start` with these preferences (and a direct fight's costumes), at frame 0.
    pub fn new(start: &SessionStart, prefs: Prefs, costume: [u8; 2]) -> Self {
        let header = Header { state_version: STATE_VERSION, start: start.clone(), costume, prefs, ..Default::default() };
        Self {
            header,
            sim_version: start.sim_version,
            inputs: InputWriter::default(),
            cheats: Vec::new(),
            prefs: Vec::new(),
            checks: Vec::new(),
            last_prefs: prefs,
            frames: 0,
            game: None,
        }
    }

    /// Steps recorded.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// After a live step with this input, cheats and preferences, which left `s`. Err (the recording must stop)
    /// when the step does not follow the recording (a state put back by the debug tools) or the recording is full.
    pub fn step(&mut self, i: FrameInput, cheats: &[Cheat], prefs: Prefs, s: &GameState) -> Result<(), String> {
        if s.frame != self.frames.wrapping_add(1) {
            return Err(format!("frame {} does not follow the recording's {}", s.frame, self.frames));
        }
        if self.frames >= MAX_FRAMES || self.cheats.len() + cheats.len() > MAX_CHEATS || self.prefs.len() >= MAX_PREFS {
            return Err("the recording is full".into());
        }
        if prefs != self.last_prefs {
            self.prefs.push((self.frames, prefs));
            self.last_prefs = prefs;
        }
        self.cheats.extend(cheats.iter().map(|&c| (self.frames, c)));
        self.inputs.push(i);
        self.frames += 1;
        if self.frames.is_multiple_of(CHECK_EVERY) {
            self.checks.push(checkpoint(s));
        }
        Ok(())
    }

    /// A match was asked for (`Request::Load`) by the step that left `s`: its VS screen is where watching starts.
    pub fn match_load(&mut self, s: &GameState) {
        let f = s.front.as_ref();
        self.game = Some(Game {
            from: s.frame,
            fight_from: None,
            mode: f.map(|f| f.mode_name().to_string()).unwrap_or_default(),
            spec: s.spec,
            team: s.team_spec,
            arcade: arcade_info(s),
            demo: f.is_some_and(|f| f.demo_running()),
        });
    }

    /// The match's fight started (`Request::Fight`) with the step that left `s`.
    pub fn match_start(&mut self, s: &GameState) {
        let g = self.game.get_or_insert(Game { from: s.frame, fight_from: None, mode: String::new(), spec: None, team: None, arcade: None, demo: false });
        g.fight_from = Some(s.frame);
        g.spec = s.spec;
        g.team = s.team_spec;
        g.arcade = arcade_info(s);
        g.demo |= s.fight.as_ref().is_some_and(|g| g.demo);
        if let Some(f) = &s.front {
            g.mode = f.mode_name().to_string();
        }
    }

    /// The match ended (`Request::FightOver`) with the step that left `s`: the file of the session up to here,
    /// watched from the match's VS screen; None for the attract demo or a match whose start was not seen.
    pub fn match_end(&mut self, s: &GameState, outcome: Outcome, rounds: [u8; 2], demo: bool, time: u64) -> Option<Replay> {
        let g = self.game.take()?;
        if demo || g.demo || g.fight_from.is_none() {
            return None;
        }
        let mut r = self.replay(s, time);
        let h = &mut r.header;
        (h.from, h.fight_from, h.mode, h.spec, h.outcome, h.rounds) = (g.from, g.fight_from, g.mode, g.spec, Some(outcome), rounds);
        Some(r)
    }

    /// The team battle ended (`Request::TeamOver`, engine addition) with the step that left `s`: the file of the session
    /// up to here (FORMAT 2), watched from its loading screen; None for a battle whose start was not seen or a QUIT.
    pub fn team_end(&mut self, s: &GameState, outcome: Outcome, rounds: [u8; 4], time: u64) -> Option<Replay> {
        let g = self.game.take()?;
        if g.fight_from.is_none() || g.team.is_none() || outcome == Outcome::Quit {
            return None;
        }
        let mut r = self.replay_as(s, time, true);
        let h = &mut r.header;
        (h.from, h.fight_from, h.mode, h.team, h.outcome, h.team_rounds) = (g.from, g.fight_from, g.mode, g.team, Some(outcome), rounds);
        h.team_arcade = g.arcade;
        Some(r)
    }

    /// The whole session so far (watched from its start), `s` its state now; `time` the clock (seconds since 1970).
    /// FORMAT 2 when a step used pads 3 / 4, else 1.
    pub fn replay(&self, s: &GameState, time: u64) -> Replay {
        self.replay_as(s, time, self.inputs.uses_ext)
    }

    /// `replay` in FORMAT 2 (`wide`, the team battle's input stream) or 1.
    fn replay_as(&self, s: &GameState, time: u64, wide: bool) -> Replay {
        let mut checks = self.checks.clone();
        if s.frame == self.frames && self.frames > 0 && checks.last().is_none_or(|c| c.0 != self.frames) {
            checks.push(checkpoint(s));
        }
        let header = Header { frames: self.frames, time, ..self.header.clone() };
        let body = Body { inputs: self.inputs.bytes(wide), cheats: self.cheats.clone(), prefs: self.prefs.clone(), checks };
        let format = if wide { super::FORMAT_TEAM } else { 1 };
        Replay { format, sim_version: self.sim_version, engine: crate::base::build::VERSION.to_string(), header, body }
    }
}

/// (TEAM ARCADE A4) The team battle's place in its TEAM ARCADE run (`Front::team_arcade_info`), None outside one.
fn arcade_info(s: &GameState) -> Option<super::TeamArcadeInfo> {
    let (stage, stages, continues) = s.front.as_ref()?.team_arcade_info()?;
    s.team_spec.is_some().then_some(super::TeamArcadeInfo { stage, stages, continues })
}
