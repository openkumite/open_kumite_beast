//! The logic's sound commands (`Cmd`, `Queue`), applied by the app after each step.
//!
//! State: none (a step's output, never serialised)
//!
//! The game logic's sound commands (`Cmd`, `Queue`): what the fight and the front end ask of the mixer in a step,
//! applied by the app right after it (`audio::Audio::apply`).

use crate::formats::music::Music;
use std::sync::Arc;

/// A sound call of the game logic, in the order made.
pub enum Cmd {
    Tone {
        bank: String,
        tone: usize,
        volume: u16,
        frames_on: u32,
        delay: u32,
    },
    /// XA track `.0` (the state's `session::Music`), looped when `.2`
    Music(u8, Arc<Music>, bool),
    /// the front end asked for XA track `.0` (`frontend::Event::Music`): the app loads and plays it, the state
    /// records it here in the order of the calls
    Requested(u8, bool),
    StopMusic,
    Options {
        master: u8,
        bgm: u8,
        stereo: bool,
    },
    /// the XA volume (0x7F full; the pause's 0x30)
    MusicVolume(u8),
    SeMode(u16),
    /// debug: a OKB_SOUNDLOG line (the step's frame mark, `Queue::mark`)
    Mark(String),
}

/// The game logic's sound output: the fight and the front end write their calls here (same names as on `Audio`)
/// and the app applies them to the mixer right after the step (`Audio::apply`), in the same order.
#[derive(Default)]
pub struct Queue {
    pub(crate) cmds: Vec<Cmd>,
    /// debug: `mark` lines are kept (the app's OKB_SOUNDLOG is on)
    marks: bool,
}

impl Queue {
    /// An empty queue; `marks` = keep the `mark` lines (`Audio::logging`).
    pub fn new(marks: bool) -> Self {
        Self { cmds: Vec::new(), marks }
    }

    /// Debug: a OKB_SOUNDLOG line at this point of the calls (built only when the log is on).
    pub fn mark(&mut self, line: impl FnOnce() -> String) {
        if self.marks {
            self.cmds.push(Cmd::Mark(line()));
        }
    }

    pub fn play_tone(&mut self, bank: &str, tone: usize, volume: u16, frames_on: u32) {
        self.play_tone_in(bank, tone, volume, frames_on, 0);
    }

    pub fn play_tone_in(&mut self, bank: &str, tone: usize, volume: u16, frames_on: u32, delay: u32) {
        self.cmds.push(Cmd::Tone { bank: bank.to_string(), tone, volume, frames_on, delay });
    }

    /// XA track `track` (decoded: `m`).
    pub fn play_music(&mut self, track: u8, m: Arc<Music>, looped: bool) {
        self.cmds.push(Cmd::Music(track, m, looped));
    }

    /// XA track `track` asked for, played by the app once loaded (`session::Request::Music`).
    pub fn request_music(&mut self, track: u8, looped: bool) {
        self.cmds.push(Cmd::Requested(track, looped));
    }

    /// The music calls in order: `Some((track, looped))` a start, None a stop.
    pub fn music_calls(&self) -> impl Iterator<Item = Option<(u8, bool)>> + '_ {
        self.cmds.iter().filter_map(|c| match c {
            Cmd::Music(t, _, looped) | Cmd::Requested(t, looped) => Some(Some((*t, *looped))),
            Cmd::StopMusic => Some(None),
            _ => None,
        })
    }

    pub fn stop_music(&mut self) {
        self.cmds.push(Cmd::StopMusic);
    }

    pub fn set_options(&mut self, master: u8, bgm: u8, stereo: bool) {
        self.cmds.push(Cmd::Options { master, bgm, stereo });
    }

    /// The XA volume, 0x7F = full (the pause: 0x30, play(0x201) / play(0x202)).
    pub fn set_music_volume(&mut self, v: u8) {
        self.cmds.push(Cmd::MusicVolume(v));
    }

    /// The XA volume calls in order.
    pub fn music_volumes(&self) -> impl Iterator<Item = u8> + '_ {
        self.cmds.iter().filter_map(|c| if let Cmd::MusicVolume(v) = c { Some(*v) } else { None })
    }

    /// `Audio::se_mode` from here on.
    pub fn set_se_mode(&mut self, mode: u16) {
        self.cmds.push(Cmd::SeMode(mode));
    }
}
