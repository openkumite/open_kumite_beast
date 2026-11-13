//! Playback: the `Player` gives a replay's steps and compares its checkpoints; `Pos` is where it is (the app's seek).
//!
//! Engine addition.

use super::Replay;
use super::inputs::read_run;
use super::sync::sync_hash;
use crate::frontend::options::Prefs;
use crate::session::{Cheat, FrameInput, GameState, STATE_VERSION, hash};

/// Where a playback is (`Player::pos`): what the step after it reads; a seek puts one back with the state of its
/// frame (the app's playback keyframes).
#[derive(Clone, Copy, Debug)]
pub struct Pos {
    at: usize,
    cur: FrameInput,
    left: u32,
    frame: u32,
    cheat: usize,
    pref: usize,
    check: usize,
    prefs: Prefs,
    checked: u32,
}

/// A replay played back: the inputs step by step and the checkpoints compared.
pub struct Player {
    pub replay: Replay,
    /// the input stream's position, the run's input and the steps left in it
    at: usize,
    cur: FrameInput,
    left: u32,
    /// steps given so far (the state's frame after the last)
    frame: u32,
    cheat: usize,
    pref: usize,
    check: usize,
    prefs: Prefs,
    /// the full state hashes are compared too (the file is this build's)
    strict: bool,
    checked: u32,
}

impl Player {
    pub fn new(replay: Replay) -> Self {
        let strict = replay.engine == crate::base::build::VERSION && replay.header.state_version == STATE_VERSION;
        let prefs = replay.header.prefs;
        Self { replay, at: 0, cur: FrameInput::default(), left: 0, frame: 0, cheat: 0, pref: 0, check: 0, prefs, strict, checked: 0 }
    }

    /// Steps given so far.
    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// The preferences the next step runs with (the session's first ones before any step).
    pub fn prefs(&self) -> Prefs {
        self.prefs
    }

    /// Whether the full state hashes are compared (a file of this build), not only the sync hashes.
    pub fn strict(&self) -> bool {
        self.strict
    }

    /// Checkpoints matched so far.
    pub fn checked(&self) -> u32 {
        self.checked
    }

    /// The position after the last step given.
    pub fn pos(&self) -> Pos {
        Pos {
            at: self.at,
            cur: self.cur,
            left: self.left,
            frame: self.frame,
            cheat: self.cheat,
            pref: self.pref,
            check: self.check,
            prefs: self.prefs,
            checked: self.checked,
        }
    }

    /// Back (or on) to a position this player gave (`pos`): the next step is the one after it.
    pub fn seek(&mut self, p: Pos) {
        Pos {
            at: self.at,
            cur: self.cur,
            left: self.left,
            frame: self.frame,
            cheat: self.cheat,
            pref: self.pref,
            check: self.check,
            prefs: self.prefs,
            checked: self.checked,
        } = p;
    }

    /// The next step's input; None at the end.
    pub fn peek(&mut self) -> Option<FrameInput> {
        if self.frame >= self.replay.header.frames {
            return None;
        }
        if self.left == 0 {
            // checked when decoded: a stream that ends early only ends the replay
            self.left = read_run(&self.replay.body.inputs, &mut self.at, &mut self.cur, self.replay.wide()).ok().flatten()?;
        }
        Some(self.cur)
    }

    /// The next step's input, cheats and preferences; None at the end.
    pub fn next_step(&mut self) -> Option<(FrameInput, Vec<Cheat>, Prefs)> {
        let i = self.peek()?;
        self.left -= 1;
        let (b, f) = (&self.replay.body, self.frame);
        while let Some(&(at, p)) = b.prefs.get(self.pref)
            && at <= f
        {
            self.prefs = p;
            self.pref += 1;
        }
        let mut cheats = Vec::new();
        while let Some(&(at, c)) = b.cheats.get(self.cheat)
            && at <= f
        {
            cheats.push(c);
            self.cheat += 1;
        }
        self.frame += 1;
        Some((i, cheats, self.prefs))
    }

    /// After the step of the input `next_step` gave, which left `s`: Err when a checkpoint of this frame differs (the
    /// replay is out of sync).
    pub fn check(&mut self, s: &GameState) -> Result<(), String> {
        if s.frame != self.frame {
            return Err(format!("out of sync: the state is at frame {}, the replay at {}", s.frame, self.frame));
        }
        while let Some(&(at, state, sync)) = self.replay.body.checks.get(self.check)
            && at <= self.frame
        {
            self.check += 1;
            if sync_hash(s) != sync {
                return Err(format!("out of sync at frame {at}"));
            }
            if self.strict && hash(s) != state {
                return Err(format!("out of sync at frame {at} (the full state)"));
            }
            self.checked += 1;
        }
        Ok(())
    }
}
