//! The software mixer standing in for the PS1 SPU.
//!
//! State: presentation (reads the state, never writes it)
//!
//! The software mixer standing in for the PS1 SPU: 24 voices with the SPU's ADSR envelope, the music stream, the
//! mono output select; rendered by `Audio`'s cpal callback.

use crate::formats::music::Music;
use crate::formats::vab::Sample;
use std::sync::Arc;

/// SPU ADSR envelope (rates as in the SPU: 7-bit rate = shift << 2 | step), ticked at 44.1 kHz.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Attack,
    Decay,
    Sustain,
    Release,
    Off,
}

pub(super) struct Env {
    phase: Phase,
    level: i32,
    counter: i32,
    adsr1: u16,
    adsr2: u16,
}

impl Env {
    pub(super) fn new(adsr1: u16, adsr2: u16) -> Self {
        Self { phase: Phase::Attack, level: 0, counter: 0, adsr1, adsr2 }
    }

    fn tick(&mut self) {
        let (a1, a2) = (self.adsr1 as i32, self.adsr2 as i32);
        let (rate, exp, dec) = match self.phase {
            Phase::Attack => ((a1 >> 8) & 0x7F, a1 & 0x8000 != 0, false),
            Phase::Decay => (((a1 >> 4) & 0xF) << 2, true, true),
            Phase::Sustain => ((a2 >> 6) & 0x7F, a2 & 0x8000 != 0, a2 & 0x4000 != 0),
            Phase::Release => ((a2 & 0x1F) << 2, a2 & 0x20 != 0, true),
            Phase::Off => return,
        };
        let shift = rate >> 2;
        let st = rate & 3;
        let mut cycles = 1i32 << (shift - 11).max(0);
        let mut step = if dec { -8 + st } else { 7 - st } << (11 - shift).max(0);
        if exp && !dec && self.level > 0x6000 {
            cycles *= 4;
        }
        if exp && dec {
            step = (step * self.level) >> 15;
        }
        self.counter += 1;
        if self.counter < cycles {
            return;
        }
        self.counter = 0;
        self.level = (self.level + step).clamp(0, 0x7FFF);
        match self.phase {
            Phase::Attack if self.level >= 0x7FFF => self.phase = Phase::Decay,
            Phase::Decay if self.level <= ((a1 & 0xF) + 1) * 0x800 => self.phase = Phase::Sustain,
            Phase::Release if self.level == 0 => self.phase = Phase::Off,
            _ => {}
        }
    }
}

pub(super) struct Voice {
    pub(super) id: u32,
    pub(super) sample: Arc<Sample>,
    pub(super) pos: f64,
    pub(super) step: f64,
    pub(super) gain: [f32; 2],
    pub(super) release: bool,
    pub(super) env: f32,
    /// SPU envelope (None = the simple fade used by `play`)
    pub(super) adsr: Option<Env>,
    /// output samples until key-off
    pub(super) key_off_in: i64,
    /// fractional SPU ticks (44.1 kHz) per output sample
    pub(super) tick_acc: f64,
    /// output samples of silence before the voice starts (delayed requests of the sound driver)
    pub(super) wait: i64,
}

pub(super) struct MusicVoice {
    pub(super) m: Arc<Music>,
    pub(super) pos: f64,
    pub(super) looped: bool,
    pub(super) vol: f32,
}

#[derive(Default)]
pub(super) struct Mixer {
    pub(super) voices: Vec<Voice>,
    pub(super) rate: f64,
    pub(super) next_id: u32,
    pub(super) music: Option<MusicVoice>,
    /// OUTPUT SELECT = MONO: both channels get the average
    pub(super) mono: bool,
    /// debug tools (pause, rewind): silence, and neither the music nor the voices go on
    pub(super) paused: bool,
}

impl Mixer {
    pub(super) fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        if self.paused {
            return;
        }
        if let Some(mv) = self.music.as_mut() {
            let step = mv.m.rate as f64 / self.rate;
            let n = mv.m.pcm.len() / 2;
            // the 16-bit samples to -1..1, with the voice's volume
            let g = mv.vol / 32768.0;
            for frame in out.chunks_mut(channels) {
                // before the start (a movie's audio that begins after its first frame, `VramView::step`): silence
                if mv.pos < 0.0 {
                    mv.pos += step;
                    continue;
                }
                let mut i = mv.pos as usize;
                if i >= n {
                    if !mv.looped || n == 0 {
                        break;
                    }
                    mv.pos -= n as f64;
                    i = mv.pos as usize;
                }
                frame[0] += mv.m.pcm[2 * i] as f32 * g;
                if channels > 1 {
                    frame[1] += mv.m.pcm[2 * i + 1] as f32 * g;
                }
                mv.pos += step;
            }
        }
        for v in &mut self.voices {
            for frame in out.chunks_mut(channels) {
                if v.wait > 0 {
                    v.wait -= 1;
                    continue;
                }
                let i = v.pos as usize;
                let s = match v.sample.pcm.get(i) {
                    Some(&s) => s as f32 / 32768.0,
                    None => match v.sample.looped {
                        Some(l) if l < v.sample.pcm.len() => {
                            v.pos = l as f64 + (v.pos - v.sample.pcm.len() as f64);
                            v.sample.pcm[l] as f32 / 32768.0
                        }
                        _ => {
                            v.env = 0.0;
                            break;
                        }
                    },
                };
                if let Some(e) = v.adsr.as_mut() {
                    if v.key_off_in == 0 && e.phase != Phase::Release && e.phase != Phase::Off {
                        e.phase = Phase::Release;
                        e.counter = 0;
                    }
                    v.key_off_in -= 1;
                    v.tick_acc += 44100.0 / self.rate;
                    while v.tick_acc >= 1.0 {
                        v.tick_acc -= 1.0;
                        e.tick();
                    }
                    if e.phase == Phase::Off {
                        v.env = 0.0;
                        break;
                    }
                    v.env = e.level as f32 / 32767.0;
                } else if v.release {
                    v.env -= 0.002;
                    if v.env <= 0.0 {
                        v.env = 0.0;
                        break;
                    }
                }
                frame[0] += s * v.gain[0] * v.env;
                if channels > 1 {
                    frame[1] += s * v.gain[1] * v.env;
                }
                v.pos += v.step;
            }
        }
        self.voices.retain(|v| v.env > 0.0 || v.wait > 0);
        if self.mono && channels > 1 {
            for f in out.chunks_mut(channels) {
                let m = (f[0] + f[1]) * 0.5;
                f[0] = m;
                f[1] = m;
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_before_its_start_is_silent() {
        let m = Arc::new(Music { pcm: vec![16384, -16384, 8192, 8192], rate: 100 });
        let mut mx = Mixer { rate: 100.0, music: Some(MusicVoice { m, pos: -2.0, looped: false, vol: 1.0 }), ..Default::default() };
        let mut out = [1.0; 10];
        mx.render(&mut out, 2);
        assert_eq!(out, [0.0, 0.0, 0.0, 0.0, 0.5, -0.5, 0.25, 0.25, 0.0, 0.0]);
    }
}
