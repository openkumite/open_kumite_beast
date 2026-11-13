//! The fight's logic state `Fight`: the match with the session's glue around it.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The fight's logic state `Fight` (part of `GameState`, stepped at 60 Hz; drawn by `present::fight_scene`): the
//! match with the session's glue around it (the in-fight CONTROLLER CONFIG, the attract demo's exit, the challenger,
//! the debug setup).

use crate::fight::fight_data::Loaded;
use crate::fight::sim::Match;
use serde::{Deserialize, Serialize};

/// The fight's logic state: the match, the HUD, the attract demo's exit, the in-fight CONTROLLER CONFIG.
/// Stepped by `tick` and the `*_step` methods; its sound calls go to an `output::sound::Queue`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Fight {
    pub m: Match,
    pub hud: crate::fight::hud::Hud,
    /// the front end's mode bytes and ladder for the status labels
    pub labels: crate::fight::hud::LabelInfo,
    /// the attract demo: DEMONSTRATION and the prompt; `demo_fade` = frames of the exit fade (fade (1, 8))
    pub demo: bool,
    /// the stage BGM at the match start: the demo starts it (by the opponent's character) only when opt.20 is set
    pub bgm: bool,
    pub demo_fade: Option<i32>,
    /// the demo's end key: 0x840 (START / ×, the title) or 0x10 (△ / KO / timeout, the ranking)
    pub demo_key: u16,
    /// the in-fight CONTROLLER CONFIG. screen (drawn over the pause)
    pub config: Option<crate::frontend::menus::Menu>,
    /// the config screen's pad edges and auto-repeat
    config_pads: crate::frontend::Pads,
}

impl Fight {
    /// After deserialising: the static data the state refers to, from the fight's `data` (`Match::rebind`).
    pub fn rebind(&mut self, data: &Loaded) -> Result<(), String> {
        // a decoded CONTROLLER CONFIG.: its rows and cursor in range (`Menu::tick` indexes by them)
        if let Some(m) = &self.config {
            if m.kind != crate::frontend::menus::Kind::Controller {
                return Err("fight config".into());
            }
            m.check()?;
        }
        self.m.rebind(data)
    }

    /// The match of `data` with the session's debug setup (OKB_P2, OKB_WATCH, OKB_MODEL, OKB_CONT, OKB_GAUGE,
    /// OKB_P2DMG).
    pub fn new(data: &Loaded, debug: &crate::session::FightDebug) -> Self {
        let mut g = Self {
            m: Match::new(data),
            hud: crate::fight::hud::Hud::new(),
            labels: crate::fight::hud::LabelInfo { m9b: 2, m9c: 1, ..Default::default() },
            demo: false,
            bgm: true,
            demo_fade: None,
            demo_key: 0,
            config: None,
            config_pads: crate::frontend::Pads::default(),
        };
        // player 2 is the CPU unless OKB_P2=human (C toggles it in game)
        g.m.set_cpu(1, !debug.p2_human);
        if debug.watch {
            g.m.set_cpu(0, true); // debug: CPU vs CPU
        }
        if let Some(t) = debug.custom {
            // debug: MODEL TYPE 1 big head, 2 kids, 3 big arms
            let custom = crate::fight::rules::Custom { model: t.model, flags: t.flags, ..crate::fight::rules::Custom::normal() };
            let rules = crate::fight::sim::Rules { need: 2, time_limit: 0xE10, hp_pct: 100, cpu_level: 0x80, custom, practice: t.practice, demo: false };
            g.m.configure(data, rules);
        }
        if debug.cont {
            g.m.round.continue_ok = true; // debug: the continue screen after a lost direct fight
        }
        if let Some(v) = debug.gauge {
            g.m.fighters[0].gauge = v; // debug: start with a beast gauge
        }
        if let Some(v) = debug.p2dmg {
            // debug: P2 starts the first round with this much damage (a quick KO)
            let f = &mut g.m.fighters[1];
            f.o_ac = v.min(f.o_b0 - 1);
            f.o_ae = f.o_ac;
            f.o_aa = f.o_ac;
        }
        g
    }

    /// Debug (`session::Cheat::Break`): wall `side` broken as on a match-deciding KO.
    pub fn break_wall(&mut self, side: usize, data: &Loaded) {
        if side < 4 {
            let w = &mut self.m.world.stage.walls;
            w.model[side] = -1;
            w.hp[side] = 0;
            // (simulation v18 on an imported stage: the imported game's own debris, as a wall the fight breaks)
            match data.br1_stage.as_ref().filter(|_| data.gates().br1_debris) {
                Some(b) => self.m.world.stage.spawn_debris_br1(side, b),
                None => self.m.world.stage.spawn_debris(side, &data.trig.random),
            }
        }
    }

    /// The fighters as the HUD reads them: from simulation v16 an imported fighter's gauge on its own marks
    /// (`fight::br1::bridge::hud_gauge`, engine addition); else the match's.
    fn hud_fighters<'a>(m: &'a Match, data: &Loaded) -> std::borrow::Cow<'a, [crate::fight::fighter::Fighter; 2]> {
        use crate::fight::br1::bridge::hud_gauge;
        let br1 = [0, 1].map(|i| data.gates().br1_hud_gauge && m.br1.as_ref().is_some_and(|b| b.side[i]));
        if br1 == [false; 2] {
            return std::borrow::Cow::Borrowed(&m.fighters);
        }
        let mut fs = m.fighters.clone();
        for (f, _) in fs.iter_mut().zip(br1).filter(|(_, b)| *b) {
            f.gauge = hud_gauge(f.gauge, f.form);
        }
        std::borrow::Cow::Owned(fs)
    }

    /// One 60 Hz game frame.
    pub fn tick(&mut self, data: &Loaded, audio: &mut crate::output::sound::Queue) {
        let new_match = self.m.round.index == 0 && self.m.round.state == 0 && self.m.round.cnt == 0;
        self.m.tick(data);
        // music: the stage BGM starts with each match (loader state 5) and loops (a1 = 1); the round flow's
        // tracks (2 / 3 / 4 / continue / game over) play once (a1 = 0)
        // HUD, built at the first round of a match, skipped while the round flow hides it
        if self.m.round.index == 0 && self.m.round.state == 0 && self.m.round.cnt <= 1 {
            let fs = Self::hud_fighters(&self.m, data);
            self.hud.build(&fs);
        }
        if !self.m.round.hud_hidden {
            let fs = Self::hud_fighters(&self.m, data);
            let r = &self.m.round;
            // an imported fighter's own name (engine addition, the import S9, simulation v19: before it the proxy's,
            // as recorded; its face holds its proxy's character)
            let br1_names = data.gates().br1_names;
            let names = [0, 1].map(|i| data.br1[i].as_ref().filter(|_| br1_names).and_then(|b| crate::assets::roster::br1_name(&b.res.code)));
            self.hud.frame(&fs, r.index, r.need, r.lost, r.timer, false, names);
        }
        if new_match
            && self.bgm
            && let Some((t, m)) = data.music.first()
        {
            audio.play_music(*t, m.clone(), true);
        }
        // pause: XA volume 0x30 while open (play(0x201)), back to 0x7F on close (play(0x202))
        if let Some(low) = self.m.pause.music_low {
            audio.set_music_volume(if low { 0x30 } else { 0x7F });
        }
        if self.m.round.music_stop {
            audio.stop_music();
        }
        if let Some(side) = self.m.round.gover_voice
            && let Some((name, _, vols)) = data.sound_banks.get(5 + side)
        {
            audio.play_tone_in(&format!("slot{}:{name}", 5 + side), 0, vols.first().copied().unwrap_or(0x1FC0), 2, 30);
        }
        if let Some(t) = self.m.round.music.map(|t| match t {
            0x2A => data.music_cont.0,
            0x27 => data.music_cont.1,
            t => t,
        }) && let Some((t, m)) = data.music.iter().find(|(n, _)| *n == t)
        {
            audio.play_music(*t, m.clone(), false);
        }
        // the imported fighters' sounds (engine addition, the import), an imported stage's in a match without
        // an imported fighter (simulation v18: `stage::br1::Counters::sounds`, no queue of the imported game's to take
        // them), then the match's
        let br1_ids = match &self.m.br1 {
            Some(b) => &b.sounds_started,
            None => &self.m.world.stage.br1.sounds,
        };
        play_started(audio, data, &self.m.sounds_started, br1_ids);
    }

    /// The attract demo (attract state 2): START / × (→ title), △, a KO or 1800 frames
    /// (→ ranking) end it: SE, music stopped, then 33 frames of fade over the frozen fight. True while the fade
    /// runs (the fight does not step; the match is over once `m.result` is set).
    pub fn demo_step(&mut self, pads: [u16; 2], front: Option<&crate::frontend::Front>, audio: &mut crate::output::sound::Queue) -> bool {
        if self.demo && self.m.result.is_none() {
            if let Some(k) = self.demo_fade {
                if k + 1 >= 33 {
                    self.m.result = Some(crate::fight::sim::Outcome::Quit);
                } else {
                    self.demo_fade = Some(k + 1);
                }
                return true;
            }
            let ko = self.m.fighters.iter().any(|f| f.o_00 & 0x80 != 0);
            let key = (pads[0] | pads[1]) & 0x850;
            if key != 0 || ko || self.m.frame >= 1800 {
                self.demo_fade = Some(0);
                self.demo_key = if key & 0x840 != 0 { 0x840 } else { 0x10 };
                audio.stop_music();
                if let (Some(f), true) = (front, key != 0) {
                    f.play_se(audio, if key & 0x840 != 0 { 0x0C } else { 0x0D });
                }
            }
        }
        false
    }

    /// pause → CONTROLLER CONFIG.: the button-mapping screen runs over the frozen fight, then the
    /// maps are applied to both fighters; the pause underneath gets no input meanwhile
    pub fn config_step(&mut self, pads: [u16; 2], front: Option<&mut crate::frontend::Front>) {
        if self.m.config_request {
            self.m.config_request = false;
            let (menu, p) = config_open(front.as_deref(), pads);
            (self.config, self.config_pads) = (Some(menu), p);
        }
        if let Some(menu) = &mut self.config {
            if let Some(buttons) = config_tick(menu, &mut self.config_pads, pads, front, &mut self.m.sound_queue) {
                self.m.buttons = buttons;
                self.config = None;
            }
            self.m.pads = [0; 2];
        }
    }

    /// The scripted pads (`session::FrameInput::forced`): OKB_SCRIPT drives pad 1; then the WATCH display pads.
    /// Must run after the CPU masking in `session::step`: the script overrides pad 0 after it (recorded input has to
    /// keep that order).
    pub fn script_step(&mut self, pads: [u16; 2], forced: [Option<u16>; 2], p2dmg: Option<i16>) {
        if let Some(p) = forced[0] {
            self.m.pads[0] = p;
        }
        // debug: OKB_SCRIPT2 drives pad 2 (with OKB_P2=human); OKB_P2DMG=d@frame sets P2's damage at that frame
        if let Some(p) = forced[1] {
            self.m.pads[1] = p;
        }
        if let Some(d) = p2dmg {
            let f = &mut self.m.fighters[1];
            (f.o_ac, f.o_ae, f.o_aa) = (d, d, d);
        }
        self.m.watch_pads = if self.config.is_some() { [0; 2] } else { [forced[0].unwrap_or(pads[0]), pads[1]] };
    }

    /// A second player pressing START during a 1P ladder fight is a challenger.
    pub fn challenger_step(&mut self, pads: [u16; 2], front: Option<&crate::frontend::Front>) {
        if let Some(port) = front.and_then(|f| f.challenger_port())
            && pads[port] & 0x800 != 0
            && self.m.round.state == 4
        {
            self.m.challenge(port);
        }
    }

    pub fn set_move(&mut self, who: usize, n: i16) {
        self.m.fighters[who].move_req = n;
    }

    pub fn toggle_beast(&mut self, who: usize) {
        self.m.fighters[who].form ^= 0x11;
        self.m.fighters[who].move_cur = -1; // restart the current move in the new form
    }

    /// Debug: leave fighter `i` with 1 HP, so the next hit is a KO (a KO only happens through a hit,
    /// which also starts the knockdown).
    pub fn debug_one_hp(&mut self, i: usize) {
        let f = &mut self.m.fighters[i];
        if f.o_ac >= 0 {
            f.o_ac = f.o_b0 - 1;
            f.o_ae = f.o_ae.max(f.o_ac);
        }
    }
}

/// The sound effects a fight frame started, played: the imported game's `br1_ids` (engine addition, the import: its
/// fighters', or an imported stage's without them; id = slot·64 + tone in the imported game's slots, the
/// volume the slot's record's, `Loaded::br1_banks`; a slot without a bank plays nothing), then the main game's
/// `br2_ids` (id = slot·64 + tone, the volume from the slot's definition table); each keyed off after 2
/// frames.
pub(super) fn play_started(audio: &mut crate::output::sound::Queue, data: &Loaded, br2_ids: &[i16], br1_ids: &[i16]) {
    for &id in br1_ids {
        let (slot, tone) = ((id >> 6) as usize, (id & 63) as usize);
        let vol = data.br1_banks.get(slot).and_then(|(_, _, v)| v.get(tone).copied()).unwrap_or(0x1800);
        audio.play_tone(&format!("br1slot{slot}"), tone, vol, 2);
    }
    for &id in br2_ids {
        let (slot, tone) = ((id >> 6) as usize, (id & 63) as usize);
        if let Some((name, _, vols)) = data.sound_banks.get(slot) {
            let vol = vols.get(tone).copied().unwrap_or(0x1800);
            audio.play_tone(&format!("slot{slot}:{name}"), tone, vol, 2);
        }
    }
}

/// The CONTROLLER CONFIG. screen opened over a paused fight on the front end's settings (the defaults
/// with the game data's button maps without a front end), its pad edges started from `pads`.
pub(super) fn config_open(front: Option<&crate::frontend::Front>, pads: [u16; 2]) -> (crate::frontend::menus::Menu, crate::frontend::Pads) {
    let s = front.map(|f| f.save.settings).unwrap_or_else(|| {
        let mut s = crate::frontend::options::Settings::default();
        s.resolve_buttons();
        s
    });
    let mut p = crate::frontend::Pads::default();
    p.update(pads);
    (crate::frontend::menus::Menu::new(crate::frontend::menus::Kind::Controller, s), p)
}

/// One frame of the CONTROLLER CONFIG. screen `menu` on pads 1 / 2 (`pads`, their edges and auto-repeat in `p`), its
/// sounds into the fight's queue. On leaving, the maps it returns (for the fight's P1 / P2) go into the
/// front end's settings and the memory card's save.
pub(super) fn config_tick(
    menu: &mut crate::frontend::menus::Menu,
    p: &mut crate::frontend::Pads,
    pads: [u16; 2],
    front: Option<&mut crate::frontend::Front>,
    queue: &mut Vec<(i16, i16)>,
) -> Option<[[u16; 8]; 2]> {
    p.update(pads);
    let mut snd = Vec::new();
    let act = menu.tick(p.any_press(), p.any_rep(), p.press, p.rep, p.held, &mut snd);
    queue.extend(snd.into_iter().map(|id| (id as i16, 0)));
    if act != crate::frontend::menus::Action::Leave {
        return None;
    }
    let buttons = menu.s.buttons;
    if let Some(f) = front {
        f.save.dirty |= f.save.settings.buttons != buttons;
        f.save.settings.buttons = buttons;
        f.save.settings.save(&mut f.save.memory_card);
    }
    Some(buttons)
}
