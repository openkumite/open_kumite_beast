//! `session::step`: one 60 Hz frame of the session, and `new_state`.
//!
//! State: none of its own: steps the serialised `GameState`
//!
//! `step`: one 60 Hz frame of the session (the fight's or the front end's, the key cheats, the front end's events),
//! and the state a session starts in (`new_state`).

use super::fight::Fight;
use super::start::data_fits;
use super::{Assets, Cheat, FrameInput, GameState, Music, Out, Request, SessionStart};
use crate::frontend::{Event, Front};

/// The state a session starts in: the front end at its boot screen, or the direct fight (its data in `a.fight`).
pub fn new_state(start: &SessionStart, a: &Assets, out: &mut Out) -> GameState {
    let mut s = GameState {
        debug: start.fight,
        sim_version: start.sim_version,
        browser: start.browser,
        network: start.network,
        android: start.android,
        extra_stage: start.extra_stage,
        extra_q3dm6: start.extra_q3dm6,
        br1: start.br1.is_some(),
        netplay: start.netplay,
        ..Default::default()
    };
    s.fight = a.fight.map(|data| {
        data_fits(data, s.sim_version).expect("the direct fight's data");
        Fight::new(data, &start.fight)
    });
    if let Some(data) = a.front {
        let mut f = Front::new(data.clone(), &mut out.audio, start);
        f.set_prefs(a.prefs);
        s.front = Some(f);
    }
    s
}

/// A session that starts in a team battle on `data` (engine addition: the app's debug start `OKB_TEAM`, until the team
/// battle's menus in stage T4): `cpu[k]` a CPU, else fighter `k` plays pad `k`.
pub fn new_team_state(start: &SessionStart, data: &crate::fight::fight_data::Loaded, mode: crate::fight::team::TeamMode, cpu: [bool; 4]) -> GameState {
    GameState {
        sim_version: start.sim_version,
        browser: start.browser,
        network: start.network,
        android: start.android,
        team: Some(Box::new(crate::fight::team::TeamFight::with_cpu(data, mode, cpu))),
        ..Default::default()
    }
}

/// One 60 Hz frame of whatever runs: the fight or the front end.
pub fn step(s: &mut GameState, a: &Assets, i: FrameInput, cheats: &[Cheat], out: &mut Out) {
    s.frame = s.frame.wrapping_add(1);
    if let Some(m) = &mut s.music {
        m.frames = m.frames.saturating_add(1);
    }
    inputs(s, a, i.flags, out);
    // debug: OKB_SOUNDLOG frame mark
    out.audio.mark(|| {
        format!(
            "step fe {} fight {} pads {:04x} {:04x}",
            s.front.as_ref().map_or(0, |f| f.frame_no()),
            s.fight.as_ref().map_or(0, |g| g.m.frame),
            i.pad[0],
            i.pad[1]
        )
    });
    for &c in cheats {
        cheat(s, a, c);
    }
    frame(s, a, i, cheats, out);
    note_music(s, &out.audio);
    if let Some(f) = &mut s.front {
        out.card_dirty |= f.save.memory_card.take_dirty();
    }
}

/// The music calls made into `q`, in order, onto `s.music` (the last one decides: calling it again is the same).
fn note_music(s: &mut GameState, q: &crate::output::sound::Queue) {
    for c in q.music_calls() {
        s.music = c.map(|(track, looped)| Music { track, looped, frames: 0 });
    }
    if let Some(v) = q.music_volumes().last() {
        s.music_volume = v;
    }
}

/// The step's start: what completed since the last step (the card store, the load, the fetch) and the app's
/// preferences. Public for `fetest`, whose loads complete within the frame that asked for them.
pub fn inputs(s: &mut GameState, a: &Assets, flags: u8, out: &mut Out) {
    let Some(f) = &mut s.front else { return };
    if flags & FrameInput::CARD_STORED != 0 {
        f.save.memory_card.stored(flags & FrameInput::CARD_FAILED == 0);
    }
    if flags & FrameInput::LOADED != 0 {
        s.pending = true;
        f.loaded();
    }
    if flags & FrameInput::FETCHED != 0
        && let Some(names) = s.fetching.take()
    {
        let files = names.into_iter().filter_map(|n| a.files.get(&n).map(|b| (n, b.clone()))).collect();
        f.fetched(files, &mut out.audio);
    }
    if f.prefs() != a.prefs {
        f.set_prefs(a.prefs);
    }
    note_music(s, &out.audio);
}

/// A key cheat at the step's start (the env ones go in later, in `frame`).
fn cheat(s: &mut GameState, a: &Assets, c: Cheat) {
    if c == Cheat::Unlock {
        if let Some(f) = &mut s.front {
            f.unlock_all();
        }
        return;
    }
    let Some(g) = &mut s.fight else { return };
    match c {
        Cheat::OneHp(i) => g.debug_one_hp(i),
        Cheat::Timer => {
            if g.m.round.timer > 60 {
                g.m.round.timer = 60;
            }
        }
        Cheat::Gauges => {
            for f in g.m.fighters.iter_mut() {
                f.gauge = 0x3BFF;
            }
        }
        Cheat::NextMove => {
            let last = a.fight.map_or(0, |d| d.fighters[0].ani.entries.len() as i16 - 1);
            g.set_move(0, (g.m.fighters[0].move_cur + 1).min(last));
        }
        Cheat::PrevMove => g.set_move(0, (g.m.fighters[0].move_cur - 1).max(0)),
        Cheat::Beast(i) => g.toggle_beast(i),
        Cheat::Cpu => {
            let on = !g.m.cpu[1];
            g.m.set_cpu(1, on);
        }
        Cheat::Unlock | Cheat::P2Dmg(_) | Cheat::ForceMove(_) | Cheat::Break(_) => {}
    }
}

/// The fight's frame (CPU masking, the attract demo's exit, the in-fight config, the scripted pads, a challenger,
/// the tick, the match end) or else the front end's.
fn frame(s: &mut GameState, a: &Assets, i: FrameInput, cheats: &[Cheat], out: &mut Out) {
    let pads = i.pad;
    if let Some(t) = &mut s.team {
        team_frame(t, &mut s.team_config, s.front.as_mut(), a, i, out);
        if let (Some(r), true) = (t.result, s.front.is_some()) {
            end_team(s, a, r, std::array::from_fn(|k| i.pad4(k)), out);
        }
        return;
    }
    if let Some(fight) = &mut s.fight {
        let data = a.fight.expect("the fight's data");
        for k in 0..2 {
            fight.m.pads[k] = if fight.m.cpu[k] { 0 } else { pads[k] }; // the CPU routine drives its side
        }
        if fight.demo_step(pads, s.front.as_ref(), &mut out.audio) {
            if let (Some(w), true) = (fight.m.result, s.front.is_some()) {
                let key = fight.demo_key;
                end_match(s, a, w, [key, 0], true, out);
            }
            return;
        }
        fight.config_step(pads, s.front.as_mut());
        let p2dmg = cheats.iter().find_map(|c| if let Cheat::P2Dmg(d) = *c { Some(d) } else { None });
        fight.script_step(pads, [i.forced(0), i.forced(1)], p2dmg);
        fight.challenger_step(pads, s.front.as_ref());
        for &c in cheats {
            match c {
                // debug: P1's move request (a move of its table: a replay's value is not trusted)
                Cheat::ForceMove(mv) if data.br1[0].is_none() && (-1..data.fighters[0].ani.entries.len() as i64).contains(&(mv as i64)) => {
                    fight.m.force_move = Some((0, mv))
                }
                // an imported game P1: a state of its table (the import S7)
                Cheat::ForceMove(mv) if data.br1[0].is_some() && (-1..crate::formats::br1_anm::STATES as i64).contains(&(mv as i64)) => {
                    fight.m.force_move = Some((0, mv))
                }
                Cheat::Break(side) => fight.break_wall(side, data),
                _ => {}
            }
        }
        fight.tick(data, &mut out.audio);
        out.vib = Some(fight.m.vib);
        if let (Some(w), Some(f)) = (fight.m.result, &mut s.front) {
            f.add_play_time(fight.m.play_frames);
            end_match(s, a, w, pads, false, out);
        }
        return;
    }
    if let Some(f) = &mut s.front {
        let mut pads = pads;
        for (k, pad) in pads.iter_mut().enumerate() {
            if let Some(p) = i.forced(k) {
                *pad = p;
            }
        }
        let ext = [i.pad4(2), i.pad4(3)];
        let ev = f.tick(pads, ext, &mut out.audio, a.files, s.sim_version);
        handle(s, a, ev, out);
        // the attract ranking keeps the demo's stage up
        if s.front.as_ref().is_some_and(|f| !f.wants_backdrop()) {
            s.backdrop = None;
        }
    }
}

/// The team battle's frame (engine addition, simulation v22): pad k (the scripted one where forced; pads 3 / 4
/// `FrameInput::ext`) drives fighter k unless it is a CPU, the humans' pads the pause; the tick, the sounds started
/// (slot·64 + tone, slots 7 / 8 bank 1's character sounds), the pause's music volume. From simulation v24 (T6) the
/// pause's CONTROLLER CONFIG. (`TeamConfig`, as a match's `Fight::config_step`): opened right after the tick that
/// chose it, it takes pads 1 / 2 while the pause underneath gets no input, and on leaving gives P1 / P2 the maps
/// (and the save the settings). In a TEAM ARCADE run (simulation v26) the pads drive the slots by `slot_ports` and
/// pads 1 / 2's maps follow them.
fn team_frame(
    t: &mut crate::fight::team::TeamFight,
    config: &mut Option<super::state::TeamConfig>,
    mut front: Option<&mut Front>,
    a: &Assets,
    i: FrameInput,
    out: &mut Out,
) {
    let data = a.fight.expect("the team battle's data");
    // (TEAM ARCADE, simulation v26) the pads by slot: a slot's pad, none for a CPU's
    let ports = slot_ports(front.as_deref());
    for k in 0..crate::fight::team::N {
        t.raw_pads[k] = ports[k].map_or(0, |p| i.pad4(p));
        t.pads[k] = if t.cpu[k] { 0 } else { t.raw_pads[k] };
    }
    if let Some(c) = config {
        if let Some(buttons) = super::fight::config_tick(&mut c.menu, &mut c.pads, [i.pad4(0), i.pad4(1)], front.as_deref_mut(), &mut t.sound_queue) {
            // pads 1 / 2's maps to their slots
            for k in 0..crate::fight::team::N {
                if let Some(p @ 0..=1) = ports[k] {
                    t.buttons[k] = buttons[p];
                }
            }
            *config = None;
        }
        t.raw_pads = [0; crate::fight::team::N];
        t.pads = [0; crate::fight::team::N];
    }
    let new_match = t.round.index == 0 && t.round.state == 0 && t.round.cnt == 0;
    let fighting = t.round.state == 4 && t.result.is_none();
    t.tick(data);
    t.hud_step();
    // (TEAM ARCADE A4) the run's play time: each frame the battle ran in round state 4, not paused (as the match's
    // `Match::play_frames`, handed to the front end at once: the clear card's time)
    if fighting
        && !t.pause.frozen()
        && let Some(f) = front.as_deref_mut().filter(|f| f.arcade_team().is_some())
    {
        f.add_play_time(1);
    }
    // (the match opens its screen before its tick, `Fight::config_step`; the team battle right after the tick that
    // chose it)
    if std::mem::take(&mut t.config_request) {
        let (menu, pads) = super::fight::config_open(front.as_deref(), [i.pad4(0), i.pad4(1)]);
        *config = Some(super::state::TeamConfig { menu, pads });
    }
    // the pause: XA volume 0x30 while open, back to 0x7F on close (as `Fight::tick`)
    if let Some(low) = t.pause.music_low {
        out.audio.set_music_volume(if low { 0x30 } else { 0x7F });
    }
    // the stage's BGM with the match, looped (as `Fight::tick`)
    if new_match && let Some((track, m)) = data.music.first() {
        out.audio.play_music(*track, m.clone(), true);
    }
    // (TEAM ARCADE) the round flow's tracks, once, as a match's (`Fight::tick`): YOU WIN / YOU LOSE, the continue's and
    // GAME OVER's (the stage's `music_cont`)
    if t.round.music_stop {
        out.audio.stop_music();
    }
    if let Some(track) = t.round.music.map(|m| match m {
        0x2A => data.music_cont.0,
        0x27 => data.music_cont.1,
        m => m,
    }) && let Some((track, m)) = data.music.iter().find(|(n, _)| *n == track)
    {
        out.audio.play_music(*track, m.clone(), false);
    }
    // (T5) the imported fighters' sounds (slots 5 / 6 bank 1's characters), BIG STAGE's without them (as
    // `Fight::tick`)
    let br1_ids = match &t.br1 {
        Some(b) => &b.sounds_started,
        None => &t.world.stage.br1.sounds,
    };
    super::fight::play_started(&mut out.audio, data, &t.sounds_started, br1_ids);
}

/// The pad (port) of each team battle slot: pad k slot k, in a TEAM ARCADE run the front end's (`Front::team_slot_ports`:
/// the starter's and the partner's pads in the humans' slots, none in the others).
fn slot_ports(front: Option<&Front>) -> [Option<usize>; crate::fight::team::N] {
    match front.and_then(|f| f.team_slot_ports()) {
        Some(p) => p.map(|p| p.map(usize::from)),
        None => std::array::from_fn(Some),
    }
}

/// The fight's result to the front end (`Front::match_over`); `backdrop`: the attract demo's end, whose fight stays
/// as the ranking's backdrop when the front end goes there.
pub fn end_match(s: &mut GameState, a: &Assets, w: crate::fight::sim::Outcome, held: [u16; 2], backdrop: bool, out: &mut Out) {
    let g = s.fight.take();
    let Some(f) = &mut s.front else { return };
    let ev = f.match_over(w, held, &mut out.audio);
    let keep = backdrop && f.wants_backdrop();
    // the rounds each side won: the other side's lost rounds
    let (lost, demo) = g.as_ref().map_or(([0; 2], false), |g| (g.m.round.lost, g.demo));
    if keep {
        s.backdrop = g;
    }
    out.requests.push(Request::FightOver { backdrop: keep, outcome: w, rounds: [lost[1], lost[0]], demo });
    handle(s, a, ev, out);
    note_music(s, &out.audio);
}

/// The team battle's result to the front end (`Front::team_over`, engine addition; public for `fetest`, whose battles do
/// not run): its result screen, or the mode menu after a QUIT; the app drops its data and view (`Request::TeamOver`).
/// `held`: the four pads' input on the battle's last frame (each pad, also a TEAM ARCADE run's pads that play no slot;
/// zero without a battle, as `fetest`'s).
pub fn end_team(s: &mut GameState, a: &Assets, r: crate::fight::team::TeamOutcome, held: [u16; crate::fight::team::N], out: &mut Out) {
    let t = s.team.take();
    s.team_config = None;
    let Some(f) = &mut s.front else { return };
    let rounds = t.as_ref().map_or([0; 4], |t| t.round.wins);
    let outcome = match r {
        crate::fight::team::TeamOutcome::Winner(w) => crate::fight::sim::Outcome::Winner(w),
        crate::fight::team::TeamOutcome::Quit => crate::fight::sim::Outcome::Quit,
        crate::fight::team::TeamOutcome::Draw => crate::fight::sim::Outcome::Draw,
        crate::fight::team::TeamOutcome::Continue => crate::fight::sim::Outcome::Continue,
        crate::fight::team::TeamOutcome::GameOver => crate::fight::sim::Outcome::GameOver,
    };
    let ev = f.team_over(outcome, rounds, held, &mut out.audio);
    out.requests.push(Request::TeamOver { outcome, rounds });
    handle(s, a, ev, out);
    note_music(s, &out.audio);
}

/// A front-end event: the state's part here, the app's part as a `Request`.
fn handle(s: &mut GameState, a: &Assets, ev: Option<Event>, out: &mut Out) {
    match ev {
        Some(Event::Load(spec)) => {
            s.spec = Some(spec);
            s.team_spec = None;
            s.pending = false;
            out.requests.push(Request::Load(spec));
        }
        Some(Event::LoadTeam(spec)) => {
            s.team_spec = Some(spec);
            s.spec = None;
            s.pending = false;
            out.requests.push(Request::LoadTeam(spec));
        }
        Some(Event::Music(m)) => {
            match m {
                Some((t, looped)) => out.audio.request_music(t, looped),
                None => out.audio.stop_music(),
            }
            out.requests.push(Request::Music(m));
        }
        Some(Event::Fetch(names)) => {
            s.fetching = Some(names.clone());
            out.requests.push(Request::Fetch(names));
        }
        Some(Event::Prefs(p)) => out.requests.push(Request::Prefs(p)),
        Some(Event::Exit) => out.requests.push(Request::Exit),
        Some(Event::Replays) => out.requests.push(Request::Replays),
        Some(Event::Network) => out.requests.push(Request::Network),
        Some(Event::ChangeData) => out.requests.push(Request::ChangeData),
        Some(Event::Fight) => {
            start_fight(s, a);
            out.requests.push(Request::Fight);
        }
        None => {}
    }
}

/// The loaded match starts with the front end's sides and rules (a team battle: `start_team`).
fn start_fight(s: &mut GameState, a: &Assets) {
    if s.team_spec.is_some() {
        start_team(s, a);
        return;
    }
    let (Some(data), Some(spec), true) = (a.fight, s.spec, std::mem::take(&mut s.pending)) else { return };
    data_fits(data, s.sim_version).expect("the loaded fight's data");
    let mut fight = Fight::new(data, &s.debug);
    for i in 0..2 {
        fight.m.set_cpu(i, spec.cpu[i]);
    }
    if let Some(f) = &mut s.front {
        let rules = f.rules();
        log::info!("fight rules {rules:?}");
        fight.m.configure(data, rules);
        fight.m.buttons = f.buttons();
        fight.labels = f.label_info();
        fight.demo = f.demo_running();
        fight.bgm = !fight.demo || f.save.settings.attract_sound != 0;
    }
    fight.m.auto_rematch = false;
    fight.m.vibration = s.front.as_ref().is_some_and(|f| f.save.settings.vibration != 0);
    fight.m.round.continue_ok = s.front.as_ref().is_some_and(|f| f.continue_allowed());
    s.fight = Some(fight);
}

/// The loaded team battle starts (engine addition, simulation v22): the spec's CPUs, the front end's CPU level and pads
/// 1 / 2's button maps in their slots (pads 3 / 4 the default map; TEAM ARCADE: the pads by slot, `slot_ports`). A
/// TEAM ARCADE partner who joined on the loading screen is a human by its pad (the load is the same).
fn start_team(s: &mut GameState, a: &Assets) {
    let (Some(data), Some(mut spec), true) = (a.fight, s.team_spec, std::mem::take(&mut s.pending)) else { return };
    data_fits(data, s.sim_version).expect("the loaded team battle's data");
    let mut buttons = [crate::assets::game_tables::default_buttons(); crate::fight::team::N];
    let mut level = 0x80;
    let mut arcade = None;
    let ports = slot_ports(s.front.as_ref());
    if let Some(f) = &mut s.front {
        let b = f.buttons();
        for k in 0..crate::fight::team::N {
            if let Some(p @ 0..=1) = ports[k] {
                buttons[k] = b[p];
            }
        }
        level = f.team_cpu_level();
        // (TEAM ARCADE) the CPUs: the slots without a pad (a partner who joined on the loading screen has one); the
        // rules: each fighter's level (once per battle: the enemies' moves the run's rank) and the continue (unlimited)
        if f.team_slot_ports().is_some() {
            spec.cpu = ports.map(|p| p.is_none());
            s.team_spec = Some(spec);
            arcade = f.arcade_team().map(|team| crate::fight::team::ArcadeRules::new(team, f.team_arcade_levels()));
        }
    }
    log::info!("team battle {spec:?} CPU level {level:#x} arcade {arcade:?}");
    s.team = Some(Box::new(crate::fight::team::TeamFight::with_setup(data, spec.mode, spec.cpu, level, buttons, arcade)));
    s.team_config = None;
}
