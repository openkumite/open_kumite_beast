//! Sound output: VAB banks played by the software mixer through cpal.
//!
//! State: presentation (reads the state, never writes it)
//!
//! Sound: Sony VAB banks (`*.SND`, "pBAV", decoded by `formats::vab`) played by a small software mixer (`mixer.rs`)
//! standing in for the PS1 SPU (24 voices, pitch from the tone's centre note, volume / pan from tone × program).
//! Output through cpal (CoreAudio / WASAPI / ALSA / WebAudio).

mod mixer;

use crate::formats::music::Music;
use crate::formats::vab::Vab;
use crate::frontend::data::FrontData;
use crate::output::sound::{Cmd, Queue};
use mixer::{Env, Mixer, MusicVoice, Voice};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Sound output and the loaded banks (by file name, e.g. "COMMON_1.SND").
pub struct Audio {
    mixer: Arc<Mutex<Mixer>>,
    stream: Option<cpal::Stream>,
    pub banks: HashMap<String, Vab>,
    pub master: f32,
    /// BGM volume relative to `master`
    pub bgm: f32,
    /// the pause lowers the music (XA volume 0x30 / 0x7F)
    pub music_scale: f32,
    /// 0 no SE, 1 every SE, 2 only the sound definitions of priority 0x60
    /// (none of the game's tables has one, so the SE are silent)
    pub se_mode: u16,
    /// debug (OKB_SOUNDLOG=file, desktop only): every sound call written there, one line each
    #[cfg(not(target_arch = "wasm32"))]
    log: Option<std::fs::File>,
    /// no output device: the samples are taken with `render_offline` (`offline`, engine addition)
    offline: bool,
}

impl Audio {
    pub fn new() -> Self {
        Self {
            mixer: Arc::new(Mutex::new(Mixer { rate: 44100.0, ..Default::default() })),
            stream: None,
            banks: HashMap::new(),
            master: 0.6,
            bgm: 0.8,
            music_scale: 1.0,
            se_mode: 1,
            #[cfg(not(target_arch = "wasm32"))]
            log: crate::base::env::var("SOUNDLOG")
                .ok()
                .and_then(|p| std::fs::File::create(&p).map_err(|e| log::warn!("OKB_SOUNDLOG: could not create {p}: {e}")).ok()),
            offline: false,
        }
    }

    /// A mixer without an output device (engine addition, `replay --video`): `start` opens none, the samples are mixed
    /// at `rate` by `render_offline`.
    pub fn offline(rate: f64) -> Self {
        let a = Self { offline: true, ..Self::new() };
        a.mixer.lock().unwrap().rate = rate;
        a
    }

    /// Debug: OKB_SOUNDLOG is on (the steps' `Queue::mark` lines are worth building).
    pub fn logging(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.log.is_some();
        #[cfg(target_arch = "wasm32")]
        return false;
    }

    /// Debug: a line in the OKB_SOUNDLOG file (the sound calls and the callers' frame marks).
    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    pub fn log(&self, line: impl FnOnce() -> String) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(mut f) = self.log.as_ref() {
            use std::io::Write;
            let _ = writeln!(f, "{}", line());
        }
    }

    /// Open the output device (on the web this must follow a user gesture).
    pub fn start(&mut self) {
        self.log(|| "start".into());
        if self.stream.is_some() || self.offline {
            return;
        }
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let Some(dev) = host.default_output_device() else {
            log::warn!("audio: no output device");
            return;
        };
        let Ok(cfg) = dev.default_output_config() else { return };
        let channels = cfg.channels() as usize;
        self.mixer.lock().unwrap().rate = cfg.sample_rate().0 as f64;
        let mixer = self.mixer.clone();
        let stream = dev.build_output_stream(
            &cfg.config(),
            move |out: &mut [f32], _| {
                if let Ok(mut m) = mixer.lock() {
                    m.render(out, channels);
                }
            },
            |e| log::warn!("audio: {e}"),
            None,
        );
        match stream {
            Ok(s) => {
                let _ = s.play();
                self.stream = Some(s);
            }
            Err(e) => log::warn!("audio: {e}"),
        }
    }

    /// The output device closed (Android: the app lost the focus or went to the background, engine addition): the
    /// mixer keeps its voices and music, `start` opens the device again.
    pub fn suspend(&mut self) {
        self.log(|| "suspend".into());
        self.stream = None;
    }

    /// Mix `n` stereo frames without a device (`offline`, the tests).
    pub fn render_offline(&self, n: usize) -> Vec<f32> {
        let mut out = vec![0.0; n * 2];
        if let Ok(mut m) = self.mixer.lock() {
            m.render(&mut out, 2);
        }
        out
    }

    pub fn load_bank(&mut self, name: &str, bytes: &[u8]) {
        self.log(|| format!("load_bank {name} {}", bytes.len()));
        if let Some(v) = Vab::parse(bytes) {
            self.banks.insert(name.to_string(), v);
        }
    }

    /// The front end's sound banks into the mixer (as `fe:<name>`), before `Front::new`.
    pub fn load_front_banks(&mut self, data: &FrontData) {
        for (name, bytes, _) in &data.banks {
            self.load_bank(&format!("fe:{name}"), bytes);
        }
    }

    /// Play `note` of `program` in `bank`: volume 0..127, pan 0 (left) .. 64 .. 127 (right). Returns the voice id.
    pub fn play(&self, bank: &str, program: usize, note: u8, volume: u8, pan: u8) -> Option<u32> {
        self.log(|| format!("play {bank} {program} {note} {volume} {pan}"));
        let vab = self.banks.get(bank)?;
        let prog = vab.programs.get(program)?;
        let mut m = self.mixer.lock().ok()?;
        let rate = m.rate;
        let mut first = None;
        for t in prog.tones.iter().filter(|t| t.min <= note && note <= t.max) {
            if t.vag == 0 {
                continue;
            }
            let Some(s) = vab.samples.get(t.vag as usize - 1).cloned() else { continue };
            let semis = note as f64 - t.center as f64 + t.fine as f64 / 128.0;
            let step = 44100.0 * 2f64.powf(semis / 12.0) / rate;
            let vol = (t.volume as f32 / 127.0) * (prog.volume as f32 / 127.0) * (volume as f32 / 127.0) * self.master;
            let p = ((t.pan as f32 - 64.0) + (prog.pan as f32 - 64.0) + (pan as f32 - 64.0)).clamp(-64.0, 63.0) / 64.0;
            let gain = [vol * (1.0 - p.max(0.0)), vol * (1.0 + p.min(0.0))];
            m.next_id += 1;
            let id = m.next_id;
            first.get_or_insert(id);
            if m.voices.len() >= 24 {
                m.voices.remove(0); // the SPU has 24 voices: steal the oldest
            }
            m.voices.push(Voice { id, sample: s, pos: 0.0, step, gain, release: false, env: 1.0, adsr: None, key_off_in: -1, tick_acc: 0.0, wait: 0 });
        }
        first
    }

    /// Sound-effect playback as the game's SFX driver does it: linear tone `tone`
    /// of `bank`, pitch fixed by the tone (its lowest key), volume 0..0x3FFF on both sides, ADSR from the tone,
    /// keyed off after `frames_on` frames so the release phase shapes the tail.
    pub fn play_tone(&self, bank: &str, tone: usize, volume: u16, frames_on: u32) -> Option<u32> {
        self.play_tone_in(bank, tone, volume, frames_on, 0)
    }

    /// `play_tone` started `delay` frames from now (the driver's delayed requests, e.g. `play(0x400, 0x83, 115)`).
    pub fn play_tone_in(&self, bank: &str, tone: usize, volume: u16, frames_on: u32, delay: u32) -> Option<u32> {
        self.log(|| format!("tone {bank} {tone} {volume:#x} {frames_on} {delay} se_mode {}", self.se_mode));
        // the SE mode drops the request (every definition has priority 0x40)
        if self.se_mode != 1 {
            return None;
        }
        let vab = self.banks.get(bank)?;
        let t = *vab.tones.get(tone)?;
        if t.vag == 0 {
            return None;
        }
        let s = vab.samples.get(t.vag as usize - 1)?.clone();
        let mut m = self.mixer.lock().ok()?;
        let rate = m.rate;
        let semis = t.min as f64 - t.center as f64 + t.fine as f64 / 128.0;
        let step = 44100.0 * 2f64.powf(semis / 12.0) / rate;
        let vol = (volume as f32 / 0x3FFF as f32) * self.master;
        m.next_id += 1;
        let id = m.next_id;
        if m.voices.len() >= 24 {
            m.voices.remove(0);
        }
        let key_off_in = (frames_on as f64 * rate / 60.0) as i64;
        m.voices.push(Voice {
            id,
            sample: s,
            pos: 0.0,
            step,
            gain: [vol, vol],
            release: false,
            env: 0.0,
            adsr: Some(Env::new(t.adsr1, t.adsr2)),
            key_off_in,
            tick_acc: 0.0,
            wait: (delay as f64 * rate / 60.0) as i64,
        });
        Some(id)
    }

    /// Start a music track (CD-XA stream in the game; the music request (track, a1)), replacing the current one.
    /// `looped` = a1 ≠ 0: state +0xE bit 0x10; at the track's end the stream handler
    /// seeks back to its start, else it pauses the CD and the music stops.
    pub fn play_music(&self, m: Arc<Music>, looped: bool) {
        self.play_music_from(m, looped, 0.0);
    }

    /// `play_music` from `secs` in (a movie's music found playing in a loaded state, the music of a state the debug
    /// tools restored; a looped track wraps; a movie's audio starting after its first frame: negative, silence
    /// until then).
    pub fn play_music_from(&self, m: Arc<Music>, looped: bool, secs: f64) {
        self.log(|| format!("music {} {} looped {looped}", m.pcm.len(), m.rate));
        if let Ok(mut mx) = self.mixer.lock() {
            let n = (m.pcm.len() / 2) as f64;
            let pos = secs * m.rate as f64;
            let pos = if looped && n > 0.0 { pos % n } else { pos.min(n) };
            mx.music = Some(MusicVoice { m, pos, looped, vol: self.master * self.bgm * self.music_scale });
        }
    }

    /// SOUND OPTIONS: MASTER / BGM VOLUME (0..100) and OUTPUT (mono / stereo), applied at once.
    pub fn set_options(&mut self, master: u8, bgm: u8, stereo: bool) {
        self.log(|| format!("options {master} {bgm} {stereo}"));
        self.master = 0.6 * master.min(100) as f32 / 100.0;
        self.bgm = 0.8 * bgm.min(100) as f32 / 60.0; // 60 (the default) = the level used so far
        if let Ok(mut mx) = self.mixer.lock() {
            mx.mono = !stereo;
            let v = self.master * self.bgm;
            if let Some(m) = mx.music.as_mut() {
                m.vol = v;
            }
        }
    }

    pub fn set_music_scale(&mut self, s: f32) {
        self.log(|| format!("music_scale {s}"));
        self.music_scale = s;
        if let Ok(mut mx) = self.mixer.lock() {
            let v = self.master * self.bgm * s;
            if let Some(m) = mx.music.as_mut() {
                m.vol = v;
            }
        }
    }

    /// Sound commands 0x203 and 0x204 (the sound queue's commands 3 and 4, both
    /// the same handler): XA volume 0, CdlPause until accepted, xa.D (playing) = 0, xa.F (track) = 0xFF.
    /// The SE voices go on. 0x203 is sent by the list menus at their entry and exit, 0x204 at the other screens' exits.
    pub fn stop_music(&self) {
        self.log(|| "stop_music".into());
        if let Ok(mut mx) = self.mixer.lock() {
            mx.music = None;
        }
    }

    /// Debug tools (pause, rewind): the output silenced and held where it is, or going on.
    pub fn set_paused(&self, on: bool) {
        self.log(|| format!("paused {on}"));
        if let Ok(mut mx) = self.mixer.lock() {
            mx.paused = on;
        }
    }

    /// Debug tools: every sound effect cut (a restored state's are not the ones playing).
    pub fn stop_voices(&self) {
        self.log(|| "stop_voices".into());
        if let Ok(mut mx) = self.mixer.lock() {
            mx.voices.clear();
        }
    }

    /// Key off: fade the voice out.
    pub fn stop(&self, id: u32) {
        self.log(|| format!("stop {id}"));
        if let Ok(mut m) = self.mixer.lock() {
            for v in m.voices.iter_mut().filter(|v| v.id == id) {
                v.release = true;
                v.key_off_in = 0;
            }
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    /// The queued calls, in order (the queue is left empty).
    pub fn apply(&mut self, q: &mut Queue) {
        for c in q.cmds.drain(..) {
            match c {
                Cmd::Tone { bank, tone, volume, frames_on, delay } => {
                    self.play_tone_in(&bank, tone, volume, frames_on, delay);
                }
                Cmd::Music(_, m, looped) => self.play_music(m, looped),
                Cmd::Requested(..) => {}
                Cmd::StopMusic => self.stop_music(),
                Cmd::Options { master, bgm, stereo } => self.set_options(master, bgm, stereo),
                Cmd::MusicVolume(v) => self.set_music_scale(v as f32 / 0x7F as f32),
                Cmd::SeMode(mode) => self.se_mode = mode,
                Cmd::Mark(line) => self.log(|| line),
            }
        }
    }
}
