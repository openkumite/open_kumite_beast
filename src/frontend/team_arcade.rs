//! TEAM ARCADE's ladder: the pairs and stages of a run, its CPU levels and the partner's pad.
//!
//! Engine addition (TEAM ARCADE A1 .. A4); the draw reuses ARCADE's ladder shuffle and the CPU levels its rank table
//!   and rank rule
//! State: serialised: `TeamLadder` is part of `Front` (`Run::team_ladder`; field changes are `STATE_VERSION` changes)
//!
//! A run (simulation v26, `v::TEAM_ARCADE`) is two players on one team (pad 1's P1 + P3, or pad 2's P2 + P4) through a
//! ladder of 2 VS 2 battles against CPU pairs: four pairs of the nine regular fighters, with the imported game's data a
//! imported pair third on BIG STAGE, and GADO + SHENLONG last on BUILDING1. `begin_team_arcade` draws it once
//! (the mode choice's TEAM ARCADE row, `frontend::team`); the select, the loading screen and the battles' end take
//! their arcade variants from `arcade_team`, and `end_team_arcade` ends a run. A lost battle's CONTINUE countdown runs
//! in the fight (`fight::team::arcade`, A3) and comes back as `Outcome::Continue` / `GameOver`. After the last stage the
//! ending (A4): the humans' epilogues (ARCADE's), the clear card (`Screen::TeamArcadeClear`), CONGRATULATIONS.

use super::progress::rank;
use super::story_flow::StoryScreen;
use super::team::{TeamPlayer, TeamSel, team_spec_of};
use super::{Event, Front, Mode, Screen, TeamSpec, Text};
use crate::assets::roster::{BR1_FIRST, SELECTABLE, is_br1};
use crate::assets::text;
use crate::fight::input::pad::{CROSS, START};
use crate::fight::rules::Outcome;
use crate::fight::team::{N, TeamMode};
use crate::output::sound::Queue;
use serde::{Deserialize, Serialize};

/// The regular fighters the pairs come from: YUGO .. STUN (character codes 0..8; ARCADE's nine shuffled opponents).
const REGULARS: usize = 9;
/// The regular pairs of a ladder (the ninth regular fighter is left out).
const REGULAR_PAIRS: usize = 4;
/// A ladder's length: the regular pairs and the boss pair (`TEAM_ARCADE_MIN_STAGES`), plus the imported pair with
/// its data (`TEAM_ARCADE_MAX_STAGES`); the bounds of `OKB_TEAM_ARCADE` and of a replay header's `team_arcade`.
pub const TEAM_ARCADE_MIN_STAGES: u8 = REGULAR_PAIRS as u8 + 1;
pub const TEAM_ARCADE_MAX_STAGES: u8 = TEAM_ARCADE_MIN_STAGES + 1;
/// The boss pair, last: GADO + SHENLONG (ARCADE's fights 8 / 9, `start_mode`'s 9 / 10).
pub(super) const BOSS_PAIR: [u8; 2] = [9, 10];
/// The boss pair's stage: BUILDING1 (ST7, ARCADE's `ladder_s` stage of Gado's fight).
pub(super) const BOSS_STAGE: u8 = 7;
/// The imported pair's stage: BIG STAGE (`stage::TEAM_BIG_STAGES`).
pub(super) const BR1_STAGE: u8 = crate::fight::stage::TEAM_BIG_STAGES[1] as u8;
/// The imported pair's place: stage 3 (index 2; ARCADE puts an imported player's imported opponents at
/// fights 3 / 6).
const BR1_AT: usize = 2;
/// The rank table's column of the last stage (Gado's fight in ARCADE) and of the CPU partner.
const BOSS_COLUMN: usize = 7;

/// A TEAM ARCADE run's ladder (`Run::team_ladder`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TeamLadder {
    /// the CPU pairs in ladder order (character codes)
    pub(super) pairs: Vec<[u8; 2]>,
    /// each pair's stage
    pub(super) stages: Vec<u8>,
    /// the stage being played (`pairs.len()` after the last)
    pub(super) idx: u8,
    /// the humans' team: 0 P1 + P3 (pad 1 started), 1 P2 + P4 (pad 2 started)
    pub(super) human_team: u8,
    /// the humans' fighters (the starter's, the partner's) and costumes, from the select
    pub(super) chr: [u8; 2],
    pub(super) costume: [u8; 2],
    /// the pad (port 0..3) that plays the partner's slot, or None: a CPU partner (`partner_ports`)
    pub(super) partner_port: Option<u8>,
    /// rounds won and lost over the run (the clear card)
    pub(super) won: u16,
    pub(super) lost: u16,
    /// (A4) the ending's humans done (0..2: the starter, the partner; `team_arcade_next_ending`)
    #[serde(default)]
    pub(super) ending: u8,
}

impl TeamLadder {
    /// The ladder of a run (engine addition): the regular fighters shuffled with ARCADE's nine swaps
    /// (`rand() % 9`) and paired in order; the stages of the regular pairs `pool` shuffled the same way (`rand() % n`)
    /// and taken in order; with `br1` the imported pair (two of MITSUKO / FOX / GREG, the third left out by the
    /// first shuffled code % 3) at stage 3 on BIG STAGE; the boss pair last on BUILDING1. `pool` is the team stages
    /// (`Front::team_stages`) without the boss's and the imported pair's, at least four.
    pub(super) fn draw(rng: &mut u32, pool: &[u8], br1: bool, human_team: u8) -> TeamLadder {
        debug_assert!(pool.len() >= REGULAR_PAIRS, "too few stages for the regular pairs: {pool:?}");
        let mut rand = || crate::base::libc_rand::rand(rng) as u32 as usize;
        let mut o: [u8; REGULARS] = std::array::from_fn(|i| i as u8);
        for k in 0..REGULARS {
            let j = rand() % REGULARS;
            o.swap(k, j);
        }
        let mut s = pool.to_vec();
        for k in 0..s.len() {
            let j = rand() % s.len();
            s.swap(k, j);
        }
        let mut pairs: Vec<[u8; 2]> = (0..REGULAR_PAIRS).map(|k| [o[2 * k], o[2 * k + 1]]).collect();
        let mut stages: Vec<u8> = s[..REGULAR_PAIRS].to_vec();
        if br1 {
            let mut b: Vec<u8> = (BR1_FIRST..BR1_FIRST + 3).collect();
            b.remove(o[0] as usize % 3);
            pairs.insert(BR1_AT, [b[0], b[1]]);
            stages.insert(BR1_AT, BR1_STAGE);
        }
        pairs.push(BOSS_PAIR);
        stages.push(BOSS_STAGE);
        TeamLadder { pairs, stages, idx: 0, human_team, chr: [0; 2], costume: [0; 2], partner_port: None, won: 0, lost: 0, ending: 0 }
    }

    /// The rank table's column of the current stage (the rank table has nine, ARCADE's fights): `idx * 7 / (n − 1)`, so
    /// the first stage is column 0 and the boss pair Gado's column 7 (5 stages: 0 1 3 5 7; 6: 0 1 2 4 5 7).
    pub(super) fn rank_column(&self) -> usize {
        let n = self.pairs.len().max(2);
        (self.idx as usize).min(n - 1) * BOSS_COLUMN / (n - 1)
    }

    /// The slot (fighter index 0..3) of the humans' team: the starter's (`k == 0`) and the partner's (`k == 1`).
    pub(super) fn human_slot(&self, k: usize) -> usize {
        self.human_team as usize + 2 * k
    }

    /// The pads (ports) that may join as the partner: the other of pads 1 / 2, or the partner slot's own pad (pad 3 for
    /// P3, pad 4 for P4); whichever presses START first on the partner's plate (a user decision).
    pub(super) fn partner_ports(&self) -> [u8; 2] {
        [1 - self.human_team, self.human_team + 2]
    }

    /// The pad (port 0..3) that drives slot `k`, or None (a CPU): the starter's pad its own slot, the partner's pad
    /// the partner slot (the front end maps the pads by slot, so a run plays the same from its inputs).
    pub(super) fn slot_port(&self, k: usize) -> Option<u8> {
        if k == self.human_slot(0) {
            Some(self.human_team)
        } else if k == self.human_slot(1) {
            self.partner_port
        } else {
            None
        }
    }

    /// The values a replay's checkpoints hash (`session::replay::sync_hash`).
    pub(super) fn sync_bytes(&self) -> Vec<u8> {
        let mut v = vec![self.idx, self.human_team, self.partner_port.unwrap_or(0xFF), self.pairs.len() as u8];
        v.extend(self.chr);
        v.extend(self.costume);
        v.extend(self.won.to_le_bytes());
        v.extend(self.lost.to_le_bytes());
        v.extend(self.pairs.iter().flatten());
        v.extend(&self.stages);
        v.push(self.ending);
        v
    }

    /// A decoded ladder is one `draw` can give (`Front::check`): 5 stages, or 6 with the imported game's data (`br1`),
    /// the regular pairs from distinct regular fighters on distinct stages of `stages` (`Front::team_stages`), the
    /// imported pair third on BIG STAGE, the boss pair last on BUILDING1; the humans' cells (an imported
    /// one only with `br1`) and costumes, the partner's pad one of `partner_ports`.
    pub(super) fn check(&self, br1: bool, stages: &[u8]) -> Result<(), String> {
        let n = TEAM_ARCADE_MIN_STAGES as usize + br1 as usize;
        let regular = |c: u8| (c as usize) < REGULARS;
        let pairs_ok = self.pairs.len() == n
            && self.stages.len() == n
            && self.pairs.iter().enumerate().all(|(i, p)| match i {
                _ if i == n - 1 => *p == BOSS_PAIR,
                BR1_AT if br1 => p.iter().all(|&c| (BR1_FIRST..BR1_FIRST + 3).contains(&c)) && p[0] != p[1],
                _ => p.iter().all(|&c| regular(c)),
            });
        // the regular pairs' fighters are distinct (the draw pairs nine shuffled codes in order)
        let regulars: Vec<u8> = self.pairs.iter().filter(|p| p.iter().all(|&c| regular(c))).flatten().copied().collect();
        let distinct = regulars.iter().enumerate().all(|(i, c)| !regulars[..i].contains(c));
        let stages_ok = self.stages.iter().enumerate().all(|(i, &s)| {
            stages.contains(&s)
                && !self.stages[..i].contains(&s)
                && match i {
                    _ if i == n - 1 => s == BOSS_STAGE,
                    BR1_AT if br1 => s == BR1_STAGE,
                    _ => s != BOSS_STAGE && s != BR1_STAGE,
                }
        });
        let cell = |c: u8| (c as usize) < SELECTABLE && (br1 || !is_br1(c));
        let humans_ok = self.chr.iter().all(|&c| cell(c)) && self.costume.iter().all(|&c| c < 4);
        let partner_ok = self.partner_port.is_none_or(|p| self.partner_ports().contains(&p));
        if pairs_ok
            && distinct
            && stages_ok
            && humans_ok
            && partner_ok
            && self.idx as usize <= n
            && self.human_team < 2
            && self.ending <= 2
            && (self.ending == 0 || self.idx as usize == n)
        {
            Ok(())
        } else {
            Err("front end: team arcade ladder".into())
        }
    }
}

impl Front {
    /// TEAM ARCADE chosen by pad `port` (0 / 1; engine addition, simulation v26): the run's records reset as
    /// `start_mode`'s (the continues, the play time, the rank at the first column, L1+L2+R1+R2 held: the strongest
    /// CPUs) and the ladder drawn (`TeamLadder::draw`, from the front end's random state).
    pub(super) fn begin_team_arcade(&mut self, port: usize) {
        let port = port.min(1);
        self.run.mode = Mode::TeamArcade;
        self.run.play_time = 0;
        self.run.continues = 0;
        self.run.shoulders = self.pads.held[port] & 0xF == 0xF;
        self.run.rank = rank(self.save.settings.difficulty, 0) as u16 * 16;
        let pool: Vec<u8> = self.team_stages().into_iter().filter(|&s| s != BOSS_STAGE && s != BR1_STAGE).collect();
        let br1 = self.team_br1();
        self.run.team_ladder = Some(TeamLadder::draw(&mut self.rng, &pool, br1, port as u8));
    }

    /// The CPU levels of a TEAM ARCADE battle by slot (engine addition): the enemies' from the rank table's column of
    /// the stage (`TeamLadder::rank_column`) under ARCADE's rank rule (`cpu_level`, which moves the rank), the humans'
    /// team's (a CPU partner's) the boss column's throughout (a user decision); 0xFF with L1+L2+R1+R2.
    pub fn team_arcade_levels(&mut self) -> [u16; 4] {
        let Some(team) = self.run.team_ladder.as_ref().map(|l| l.human_team as usize) else { return [self.team_cpu_level(); 4] };
        let enemy = self.cpu_level();
        let partner = if self.run.shoulders { 0xFF } else { rank(self.save.settings.difficulty, BOSS_COLUMN) as u16 * 16 };
        std::array::from_fn(|k| if k & 1 == team { partner } else { enemy })
    }

    /// The TEAM ARCADE ladder's values a replay's checkpoints hash (`session::replay::sync_hash`; None without a run).
    pub fn team_ladder_sync(&self) -> Option<Vec<u8>> {
        self.run.team_ladder.as_ref().map(TeamLadder::sync_bytes)
    }

    /// The humans' team of a TEAM ARCADE run (None outside one): the team screens' arcade variants and the pads by slot.
    pub fn arcade_team(&self) -> Option<usize> {
        self.run.team_ladder.as_ref().filter(|_| self.run.mode == Mode::TeamArcade).map(|l| l.human_team as usize)
    }

    /// The pad (port) of each slot in a TEAM ARCADE run (`TeamLadder::slot_port`; None outside one: pad k drives slot
    /// k), for the battle's pads and button maps (`session::step`).
    pub fn team_slot_ports(&self) -> Option<[Option<u8>; N]> {
        let l = self.run.team_ladder.as_ref().filter(|_| self.run.mode == Mode::TeamArcade)?;
        Some(std::array::from_fn(|k| l.slot_port(k)))
    }

    /// The arcade select as it opens (2 VS 2): the starter's slot a human, the partner's a human if its pad has joined
    /// (else a CPU, picked by the starter), the enemies' chosen (the ladder's, never a turn); the cursors `p`'s.
    pub(super) fn arcade_sel(&self, mut p: [TeamPlayer; N]) -> TeamSel {
        if let Some(l) = &self.run.team_ladder {
            for (k, pl) in p.iter_mut().enumerate() {
                *pl = TeamPlayer { cur: pl.cur, state: (k & 1 != l.human_team as usize) as u8, human: l.slot_port(k).is_some(), costume: 0, flash: 0 };
            }
        }
        TeamSel::new(TeamMode::Vs2, p)
    }

    /// A partner pad's START (`press`: the four pads' new presses) while the partner is a CPU: that pad plays the
    /// partner's slot from now on (`TeamLadder::partner_port`; the first of `partner_ports` on a tie). The slot, or None.
    pub(super) fn arcade_join(&mut self, press: [u16; N]) -> Option<usize> {
        let l = self.run.team_ladder.as_mut().filter(|l| l.partner_port.is_none())?;
        let port = l.partner_ports().into_iter().find(|&p| press[p as usize] & START != 0)?;
        l.partner_port = Some(port);
        log::info!("front: team arcade partner joined on pad {}", port + 1);
        Some(l.human_slot(1))
    }

    /// The ladder's current battle: the humans' picks (`chr` / `costume`) in their slots, the stage's pair in the
    /// enemies' (first costumes), its stage, the CPUs (the slots without a pad), the duplicate rule (`team_spec_of`).
    pub(super) fn team_arcade_spec(&self) -> Option<TeamSpec> {
        let l = self.run.team_ladder.as_ref()?;
        let i = l.idx as usize;
        let (pair, stage) = (*l.pairs.get(i)?, *l.stages.get(i)?);
        let ours = |k: usize| k & 1 == l.human_team as usize;
        let chr = std::array::from_fn(|k| if ours(k) { l.chr[k >> 1] } else { pair[k >> 1] });
        let costume = std::array::from_fn(|k| if ours(k) { l.costume[k >> 1] } else { 0 });
        let cpu = std::array::from_fn(|k| l.slot_port(k).is_none());
        Some(team_spec_of(TeamMode::Vs2, chr, costume, stage, cpu))
    }

    /// TeamLoad's header in a TEAM ARCADE run: STAGE n, the last FINAL STAGE.
    pub(super) fn team_arcade_header(&self) -> Option<String> {
        self.arcade_team()?;
        let l = self.run.team_ladder.as_ref()?;
        Some(if l.idx as usize + 1 >= l.pairs.len() { text::get("final_stage").into() } else { text::fill(text::get("stage_n"), &[&(l.idx + 1)]) })
    }

    /// A TEAM ARCADE run ends (back to the mode choice or the mode menu): the ladder cleared and the mode the team
    /// battle's again at once, so a `Mode::TeamArcade` front end always has its ladder (`check`).
    pub(super) fn end_team_arcade(&mut self) {
        if self.run.mode == Mode::TeamArcade {
            self.run.mode = Mode::Team;
        }
        self.run.team_ladder = None;
    }

    /// A TEAM ARCADE battle ended (`team_over`): the rounds counted (`won` / `lost`); a win the next stage's loading
    /// screen, after the last the ending (A4: `team_arcade_next_ending`); a loss or a DRAW GAME went through the
    /// battle's countdown (A3): `Continue` the arcade select at the same stage, `GameOver` the mode menu (also any other
    /// end without the humans' win: an arcade battle has none); QUIT the mode menu.
    pub(super) fn team_arcade_over(&mut self, outcome: Outcome, rounds: [u8; N], audio: &mut Queue) -> Option<Event> {
        let l = self.run.team_ladder.as_mut()?;
        let team = l.human_team as usize;
        if outcome == Outcome::Quit {
            self.team_to_menu(audio);
            return None;
        }
        l.won = l.won.saturating_add(rounds[team] as u16);
        l.lost = l.lost.saturating_add(rounds[1 - team] as u16);
        log::info!("front: team arcade stage {} over: {outcome:?} ({rounds:?}, rounds {} / {})", l.idx + 1, l.won, l.lost);
        match outcome {
            Outcome::Continue => {
                // (A3) START in the countdown: as ARCADE's continue (`match_over`), the select again at
                // the same stage, the cursors on the humans' fighters (a CPU partner picked again, a partner pad may
                // join); the humans choose their fighters and costumes anew, as ARCADE's select after a continue
                let chr = l.chr;
                self.play(audio, 0x0C, 0);
                self.run.continues += 1;
                self.run.rank = self.run.rank.saturating_sub(20) | 0x100;
                let mut p = [TeamPlayer::default(); N];
                for (k, pl) in p.iter_mut().enumerate() {
                    pl.cur = if k & 1 == team { chr[k >> 1] } else { self.team_last[k] };
                }
                self.enter_team_select(self.arcade_sel(p), audio);
                None
            }
            Outcome::Winner(w) if w == team => {
                l.idx += 1;
                if (l.idx as usize) < l.pairs.len() {
                    let spec = self.team_arcade_spec()?;
                    return Some(self.enter_team_load(spec, audio));
                }
                // (A4) the last stage won: the ending
                self.team_arcade_next_ending();
                None
            }
            // (A3) the countdown ran out: the battle has shown GAME OVER, the run ends on the mode menu (an arcade
            // battle the humans did not win always ends through the countdown: `Continue` or `GameOver`)
            _ => {
                self.team_to_menu(audio);
                None
            }
        }
    }

    /// (A4) The ending's next screen, after the last stage's win and after each epilogue: the epilogue of the next
    /// human in slot order (the starter, then a partner who plays by a pad; a fighter shown once) whose fighter has
    /// one (`team_epilogue`), else the clear card (the resident VRAM back first, as STORY's fights).
    pub(super) fn team_arcade_next_ending(&mut self) {
        let Some(l) = self.run.team_ladder.as_mut() else { return };
        while l.ending < 2 {
            let k = l.ending as usize;
            l.ending += 1;
            let human = k == 0 || l.partner_port.is_some();
            if !human || (k == 1 && l.chr[1] == l.chr[0]) {
                continue;
            }
            let c = l.chr[k];
            match team_epilogue(c) {
                Some(e) => {
                    log::info!("front: team arcade ending: {}'s epilogue", crate::assets::roster::name(c));
                    self.story_screen = Some(StoryScreen::Epilogue(e));
                    self.screen = Screen::Story;
                    return;
                }
                None => log::info!("front: team arcade ending: no epilogue picture for {}", crate::assets::roster::name(c)),
            }
        }
        self.story_screen = None;
        self.boot_vram();
        self.screen = Screen::TeamArcadeClear { t: 0 };
    }

    /// (A4) The clear card (`Screen::TeamArcadeClear`): `CLEAR_FRAMES` frames, or from `CLEAR_SKIP` on until a human's
    /// START / ×; then CONGRATULATIONS, which ends on the mode menu (`tick`).
    pub(super) fn tick_team_arcade_clear(&mut self, t: u32, audio: &mut Queue) -> Option<Event> {
        let press = self.press4().0;
        let humans = self.team_slot_ports().map_or(press[0], |p| p.iter().flatten().fold(0, |a, &port| a | press[port as usize]));
        if t + 1 >= CLEAR_FRAMES || (t >= CLEAR_SKIP && humans & (START | CROSS) != 0) {
            self.play(audio, 0x0C, 0);
            self.screen = Screen::Message { t: 300, text: Text::Congratulations, to_custom: false };
        } else {
            self.screen = Screen::TeamArcadeClear { t: t + 1 };
        }
        None
    }

    /// (A4) The clear card: TEAM ARCADE CLEAR, the humans' team (a CPU partner marked COM), the clear time (the run's
    /// `play_time`: frames of round state 4, MM'SS"CC as the ranking's), the rounds won and lost, the continues.
    pub(super) fn draw_team_arcade_clear(&self, out: &mut Vec<crate::output::prim::Prim>) {
        let Some(l) = self.run.team_ladder.as_ref() else { return };
        let f = crate::output::font::fonts();
        super::menus::frame_with(out, f, super::team::STYLE, false);
        let title = "TEAM ARCADE CLEAR";
        f.menu_text(out, title, 256 - f.menu_width(title) / 2, 64, 0);
        for k in 0..2 {
            let slot = l.human_slot(k);
            let y = 120 + 40 * k as i32;
            let who = format!("{}{}", if l.slot_port(slot).is_none() { "COM " } else { "" }, crate::assets::roster::name(l.chr[k]));
            f.mid_text(out, &format!("P{}", slot + 1), 96, y, crate::fight::team::hud::colour(TeamMode::Vs2, slot));
            f.menu_text(out, &who, 144, y + 6, 1);
        }
        let rows = [
            ("TIME", crate::frontend::ranking::time_text(self.run.play_time.min(i32::MAX as u32) as i32)),
            ("ROUNDS WON", l.won.to_string()),
            ("ROUNDS LOST", l.lost.to_string()),
            ("CONTINUES", self.run.continues.to_string()),
        ];
        for (k, (label, value)) in rows.iter().enumerate() {
            let y = 236 + 32 * k as i32;
            f.menu_text(out, label, 96, y, 4);
            f.menu_text(out, value, 416 - f.menu_width(value), y, 0);
        }
    }

    /// (A4) Debug OKB_TEAM_ARCADE (`session::Entry::TeamArcade`): a pad-1 run (drawn as the mode choice draws it) at
    /// stage `stage`'s (1 .., the last at most) loading screen with the humans' fighters `chr` (the main game's,
    /// clamped) in first costumes, the partner pad 2 (`partner`) or a CPU.
    pub(super) fn debug_team_arcade(&mut self, stage: u8, chr: [u8; 2], partner: bool, audio: &mut Queue) {
        self.enter_team_mode(0, audio);
        self.begin_team_arcade(0);
        let chr = chr.map(|c| c.min(crate::assets::roster::CHARACTERS.len() as u8 - 1));
        let Some(l) = self.run.team_ladder.as_mut() else { return };
        l.idx = stage.saturating_sub(1).min(l.pairs.len() as u8 - 1);
        (l.chr, l.partner_port) = (chr, partner.then_some(1));
        log::info!("front: debug team arcade from stage {} ({:?})", l.idx + 1, l.pairs[l.idx as usize]);
        if let Some(spec) = self.team_arcade_spec() {
            self.pending_event = Some(self.enter_team_load(spec, audio));
        }
    }

    /// (A4) A TEAM ARCADE battle's place in its run for the replay's header (`session::replay::TeamArcadeInfo`): the
    /// stage (1 ..), the ladder's length and the continues so far; None outside a run.
    pub fn team_arcade_info(&self) -> Option<(u8, u8, u32)> {
        self.arcade_team()?;
        let l = self.run.team_ladder.as_ref()?;
        Some((l.idx + 1, l.pairs.len() as u8, self.run.continues))
    }
}

/// (A4) The clear card's frames, and the frame from which a press skips it.
pub(super) const CLEAR_FRAMES: u32 = 600;
const CLEAR_SKIP: u32 = 60;

/// (A4) Fighter `c`'s epilogue in the TEAM ARCADE ending: ARCADE's illustration (`Epilogue::new`) for a
/// main-game fighter; for URIKO (BR1) the imported picture ARCADE shows in her epilogue's place
/// (`Epilogue::br1`); MITSUKO, FOX and GREG have none (their ARCADE endings are the imported game's movies, which this short
/// ending does not play): None.
fn team_epilogue(c: u8) -> Option<crate::frontend::story_screens::Epilogue> {
    use crate::frontend::story_screens::{BR1_ART, Epilogue};
    match c {
        _ if (c as usize) < crate::assets::roster::CHARACTERS.len() => Some(Epilogue::new(c as usize)),
        _ if c == BR1_FIRST + 3 => Some(Epilogue::br1(BR1_ART)),
        _ => None,
    }
}
#[cfg(test)]
impl Front {
    /// A pad-1 TEAM ARCADE run (YUGO + ALICE, the partner on pad `partner` or a CPU) at its first battle's loading
    /// screen; its spec (tests: `session`'s pads by slot).
    pub(crate) fn set_team_arcade_load(&mut self, partner: Option<u8>) -> TeamSpec {
        self.set_sim_version(crate::fight::version::v::TEAM_ARCADE);
        self.enter_team_mode(0, &mut Queue::default());
        self.begin_team_arcade(0);
        let l = self.run.team_ladder.as_mut().expect("a ladder");
        (l.chr, l.partner_port) = ([0, 1], partner);
        let spec = self.team_arcade_spec().expect("a battle");
        self.screen = Screen::TeamLoad { t: 0, ready: false, spec };
        spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POOL: [u8; 4] = [0, 2, 5, 8];
    const POOL_Q3: [u8; 5] = [0, 2, 5, 8, 14];

    fn members(l: &TeamLadder) -> Vec<u8> {
        l.pairs.iter().flatten().copied().collect()
    }

    /// The same seed draws the same ladder, other seeds others.
    #[test]
    fn draw_is_deterministic() {
        let draw = |seed: u32| {
            let mut r = seed;
            (TeamLadder::draw(&mut r, &POOL_Q3, true, 0), r)
        };
        assert_eq!(draw(7), draw(7));
        let distinct: std::collections::BTreeSet<Vec<u8>> = (0..64).map(|s| members(&draw(s).0)).collect();
        assert!(distinct.len() > 32, "{} distinct ladders of 64 seeds", distinct.len());
    }

    /// Every pair member appears once, the boss pair is last on BUILDING1, no stage repeats, and the imported pair
    /// is third on BIG STAGE only with the imported game's data.
    #[test]
    fn draw_has_no_repeats_and_the_boss_last() {
        for seed in 0..500u32 {
            for (pool, br1) in [(&POOL[..], false), (&POOL[..], true), (&POOL_Q3[..], false), (&POOL_Q3[..], true)] {
                let mut r = seed.wrapping_mul(0x9E37_79B9);
                let l = TeamLadder::draw(&mut r, pool, br1, (seed & 1) as u8);
                let n = 5 + br1 as usize;
                assert_eq!((l.pairs.len(), l.stages.len()), (n, n));
                let m = members(&l);
                assert!(m.iter().enumerate().all(|(i, c)| !m[..i].contains(c)), "a repeated fighter: {:?}", l.pairs);
                assert!(l.stages.iter().enumerate().all(|(i, s)| !l.stages[..i].contains(s)), "a repeated stage: {:?}", l.stages);
                assert_eq!((l.pairs[n - 1], l.stages[n - 1]), (BOSS_PAIR, BOSS_STAGE));
                assert_eq!(m.iter().filter(|&&c| is_br1(c)).count(), if br1 { 2 } else { 0 });
                if br1 {
                    assert!(l.pairs[BR1_AT].iter().all(|&c| is_br1(c) && c != BR1_FIRST + 3), "{:?}", l.pairs);
                    assert_eq!(l.stages[BR1_AT], BR1_STAGE);
                }
                assert!(l.stages.iter().all(|s| pool.contains(s) || *s == BOSS_STAGE || *s == BR1_STAGE));
                assert_eq!(l.check(br1, &[0, 2, 5, 7, 8, 14, 25]), Ok(()));
                assert!(l.check(!br1, &[0, 2, 5, 7, 8, 14, 25]).is_err());
            }
        }
    }

    /// The rank table's column by stage: the first stage column 0, the boss Gado's column 7.
    #[test]
    fn rank_columns() {
        let cols = |br1: bool| {
            let mut l = TeamLadder::draw(&mut 1, &POOL, br1, 0);
            (0..l.pairs.len() as u8)
                .map(|i| {
                    l.idx = i;
                    l.rank_column()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(cols(false), [0, 1, 3, 5, 7]);
        assert_eq!(cols(true), [0, 1, 2, 4, 5, 7]);
    }

    /// The partner is the other of pads 1 / 2 or the partner slot's own pad; the pads drive the humans' slots.
    #[test]
    fn partner_ports_and_slots() {
        let mut l = TeamLadder::draw(&mut 1, &POOL, false, 0);
        assert_eq!(l.partner_ports(), [1, 2]);
        assert_eq!((0..4).map(|k| l.slot_port(k)).collect::<Vec<_>>(), [Some(0), None, None, None]);
        l.partner_port = Some(1);
        assert_eq!((0..4).map(|k| l.slot_port(k)).collect::<Vec<_>>(), [Some(0), None, Some(1), None]);
        assert_eq!(l.check(false, &[0, 2, 5, 7, 8]), Ok(()));
        l.partner_port = Some(3);
        assert!(l.check(false, &[0, 2, 5, 7, 8]).is_err());
        let mut l = TeamLadder::draw(&mut 1, &POOL, false, 1);
        assert_eq!(l.partner_ports(), [0, 3]);
        l.partner_port = Some(3);
        assert_eq!((0..4).map(|k| l.slot_port(k)).collect::<Vec<_>>(), [None, Some(1), None, Some(3)]);
    }
}

#[cfg(test)]
mod front_tests {
    use super::*;
    use crate::output::sound::Queue;

    /// The front end at the title (None without the export: skipped).
    fn front() -> Option<Front> {
        let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
        if !std::path::Path::new(&root).join("manifest.json").exists() {
            eprintln!("skipped: no okb_export");
            return None;
        }
        let data = pollster::block_on(crate::frontend::load(&crate::assets::Assets::new(root))).expect("front data");
        Some(Front::new(std::sync::Arc::new(data), &mut Queue::default(), &Default::default()))
    }

    /// A run's ladder from the session's data passes `check`: five stages on the enlarged rings, six with the
    /// imported game's data (BIG STAGE third), Q3DM6 among the regular pairs' stages with its files; refused before v26 and with
    /// other data than it was drawn with.
    #[test]
    fn begin_draws_a_checked_ladder() {
        let Some(mut f) = front() else { return };
        f.begin_team_arcade(0);
        f.screen = Screen::TeamSelect(f.arcade_sel([TeamPlayer::default(); N]));
        assert_eq!(f.run.mode, Mode::TeamArcade);
        let l = f.run.team_ladder.clone().expect("a ladder");
        assert_eq!((l.pairs.len(), l.human_team, l.stages[4]), (5, 0, BOSS_STAGE));
        assert!(l.stages.iter().all(|s| crate::fight::stage::TEAM_STAGES.contains(&(*s as usize))));
        assert_eq!(f.check(), Ok(()));
        f.set_br1(true);
        assert!(f.check().is_err(), "a ladder without the imported pair in a session with its data");
        f.set_extras(1 << crate::fight::stage::extra::Q3DM6.index());
        let mut seen_q3 = false;
        for _ in 0..20 {
            f.begin_team_arcade(1);
            f.screen = Screen::TeamSelect(f.arcade_sel([TeamPlayer::default(); N]));
            let l = f.run.team_ladder.clone().expect("a ladder");
            assert_eq!((l.pairs.len(), l.human_team, l.stages[BR1_AT]), (6, 1, BR1_STAGE));
            seen_q3 |= l.stages.contains(&crate::fight::stage::extra::Q3DM6.stage);
            assert_eq!(f.check(), Ok(()));
        }
        assert!(seen_q3, "Q3DM6 drawn for a regular pair");
        // the ladder rides the snapshot (postcard) and the `.okbs` file (CBOR)
        let l = f.run.team_ladder.clone();
        let back: Front = postcard::from_bytes(&postcard::to_allocvec(&f).expect("serialises")).expect("decodes");
        assert_eq!((back.run.mode, &back.run.team_ladder), (Mode::TeamArcade, &l));
        let mut cbor = Vec::new();
        ciborium::into_writer(&f, &mut cbor).expect("serialises");
        let back: Front = ciborium::from_reader(&cbor[..]).expect("decodes");
        assert_eq!((back.run.mode, &back.run.team_ladder), (Mode::TeamArcade, &l));
        f.set_sim_version(crate::fight::version::v::TEAM_ARCADE - 1);
        assert!(f.check().is_err(), "TEAM ARCADE before v26");
        f.set_sim_version(crate::fight::version::v::TEAM_ARCADE);
        f.run.team_ladder = None;
        assert!(f.check().is_err(), "a TEAM ARCADE run without its ladder");
    }

    /// The enemies' level follows the stage's rank column under ARCADE's rank rule; the humans' team (a CPU partner)
    /// plays at the boss column's throughout.
    #[test]
    fn levels_by_stage() {
        let Some(mut f) = front() else { return };
        f.begin_team_arcade(1);
        let d = f.save.settings.difficulty.min(7) as usize;
        let (first, boss) = (rank(d as u8, 0) as u16 * 16, rank(d as u8, BOSS_COLUMN) as u16 * 16);
        assert_eq!(f.team_arcade_levels(), [first, boss, first, boss]);
        f.run.team_ladder.as_mut().expect("a ladder").idx = 4;
        assert_eq!(f.team_arcade_levels(), [boss, boss, boss, boss]);
        assert_eq!(f.run.rank, boss);
    }

    use crate::fight::input::pad::{CROSS, DOWN, SQUARE, TRIANGLE};
    use crate::fight::version::v;

    /// One front-end frame with pads 1 / 2 `raw` and pads 3 / 4 `ext` at simulation v26.
    fn tick(f: &mut Front, raw: [u16; 2], ext: [u16; 2]) -> Option<Event> {
        f.tick(raw, ext, &mut Queue::default(), &crate::session::FileCache::default(), v::TEAM_ARCADE)
    }

    /// TEAM BATTLE chosen by pad `port`, then (DOWN × 2, ×) its third row at simulation `ver`.
    fn third_row(f: &mut Front, port: usize, ver: u32) {
        f.set_sim_version(ver);
        f.enter_team_mode(port, &mut Queue::default());
        let mut pads = [0u16; 2];
        for p in [DOWN, 0, DOWN, 0, CROSS, 0] {
            pads[port] = p;
            f.tick(pads, [0; 2], &mut Queue::default(), &crate::session::FileCache::default(), ver);
        }
    }

    /// The mode choice has 2 VS 2 / FREE FOR ALL / EXIT to v25 (DOWN × 2 reaches EXIT, as old replays do) and TEAM
    /// ARCADE third from v26; its cursor's bound is the version's rows.
    #[test]
    fn mode_rows_by_version() {
        let Some(mut f) = front() else { return };
        third_row(&mut f, 0, v::TEAM_ARCADE - 1);
        assert!(matches!(f.screen, Screen::Title { menu: true, .. }), "{}", f.describe());
        assert_eq!((f.run.mode, f.run.team_ladder.is_none()), (Mode::Team, true));
        f.enter_team_mode(0, &mut Queue::default());
        f.screen = Screen::TeamMode { cursor: 3, banner_x: 0 };
        assert!(f.check().is_err(), "a fourth row before v26");
        f.set_sim_version(v::TEAM_ARCADE);
        assert_eq!(f.check(), Ok(()));
        third_row(&mut f, 0, v::TEAM_ARCADE);
        assert!(matches!(f.screen, Screen::TeamSelect(_)), "{}", f.describe());
        assert_eq!((f.run.mode, f.arcade_team()), (Mode::TeamArcade, Some(0)));
        assert_eq!(f.check(), Ok(()));
    }

    /// The arcade select: only the humans' slots are live (the enemies' chosen CPUs); the partner joins from either of
    /// its pads (pad 2 or pad 3 for a pad-1 run), not from another; △ makes it leave; the starter's △ ends the run.
    /// Once both have chosen, the ladder's first battle loads (no stage panel) with the picks and the CPUs by pad.
    #[test]
    fn arcade_select_and_partner() {
        let Some(mut f) = front() else { return };
        third_row(&mut f, 0, v::TEAM_ARCADE);
        let sel = |f: &Front| if let Screen::TeamSelect(s) = f.screen { s } else { panic!("{}", f.describe()) };
        let s = sel(&f);
        assert_eq!(s.p.map(|p| (p.human, p.state)), [(true, 0), (false, 1), (false, 0), (false, 1)]);
        let partner = |f: &Front| f.run.team_ladder.as_ref().expect("a ladder").partner_port;
        // pad 4 is not a partner pad of a pad-1 run
        tick(&mut f, [0; 2], [0, START]);
        assert_eq!((sel(&f).p[2].human, partner(&f)), (false, None));
        // pad 2 joins, leaves with △; pad 3 joins
        tick(&mut f, [0, START], [0; 2]);
        assert_eq!((sel(&f).p[2].human, partner(&f)), (true, Some(1)));
        assert_eq!(f.check(), Ok(()));
        tick(&mut f, [0, TRIANGLE], [0; 2]);
        assert_eq!((sel(&f).p[2].human, partner(&f)), (false, None));
        tick(&mut f, [0; 2], [START, 0]);
        assert_eq!((sel(&f).p[2].human, partner(&f)), (true, Some(2)));
        // an enemy slot played by a pad is not an arcade select
        let mut bad = f.clone();
        if let Screen::TeamSelect(s) = &mut bad.screen {
            s.p[1].human = true;
        }
        assert!(bad.check().is_err());
        // nor is a select after the last stage (no battle to load)
        let mut bad = f.clone();
        let l = bad.run.team_ladder.as_mut().expect("a ladder");
        l.idx = l.pairs.len() as u8;
        assert!(bad.check().is_err());
        // pad 1 □ (costume 0), pad 3 × (costume 1): the first battle
        tick(&mut f, [SQUARE, 0], [CROSS, 0]);
        let mut ev = None;
        for _ in 0..10 {
            ev = ev.or(tick(&mut f, [0; 2], [0; 2]));
        }
        let Some(Event::LoadTeam(spec)) = ev else { panic!("no battle: {}", f.describe()) };
        let l = f.run.team_ladder.clone().expect("a ladder");
        assert_eq!(l.chr, [spec.chr[0], spec.chr[2]]);
        assert_eq!(l.costume, [0, 1]);
        assert_eq!(([spec.chr[1], spec.chr[3]], spec.stage, spec.cpu), (l.pairs[0], l.stages[0], [false, true, false, true]));
        assert!(matches!(f.screen, Screen::TeamLoad { .. }));
        assert_eq!(f.team_arcade_header(), Some(crate::assets::text::fill(crate::assets::text::get("stage_n"), &[&1])));
        assert_eq!(f.check(), Ok(()));
        // a pad-2 run: the partner is pad 1 or pad 4; the starter's △ ends the run (back to the TEAM ARCADE row)
        third_row(&mut f, 1, v::TEAM_ARCADE);
        assert_eq!(f.arcade_team(), Some(1));
        tick(&mut f, [0; 2], [START, 0]);
        assert_eq!(partner(&f), None);
        tick(&mut f, [START, 0], [0; 2]);
        assert_eq!((sel(&f).p[3].human, partner(&f)), (true, Some(0)));
        tick(&mut f, [0, TRIANGLE], [0; 2]);
        assert!(matches!(f.screen, Screen::TeamMode { cursor: super::super::team::ARCADE_ROW, .. }), "{}", f.describe());
        assert_eq!((f.run.mode, f.run.team_ladder.is_none()), (Mode::Team, true));
        assert_eq!(f.check(), Ok(()));
    }

    /// A run at its stage `idx`'s battle (YUGO + ALICE, a CPU partner).
    fn at_battle(f: &mut Front, idx: u8) -> TeamSpec {
        f.set_sim_version(v::TEAM_ARCADE);
        f.enter_team_mode(0, &mut Queue::default());
        f.begin_team_arcade(0);
        let l = f.run.team_ladder.as_mut().expect("a ladder");
        (l.chr, l.idx) = ([0, 1], idx);
        let spec = f.team_arcade_spec().expect("a battle");
        f.screen = Screen::TeamFight(spec);
        assert_eq!(f.check(), Ok(()));
        spec
    }

    /// A won battle loads the next stage's (its pair and stage, the humans' picks); the last one opens the ending (A4);
    /// any other end without the humans' win and QUIT the mode menu at once, the run ended. The rounds are counted.
    #[test]
    fn battle_over() {
        let Some(mut f) = front() else { return };
        let over = |f: &mut Front, o: Outcome, rounds: [u8; 4]| f.team_over(o, rounds, [0; 4], &mut Queue::default());
        at_battle(&mut f, 0);
        let Some(Event::LoadTeam(spec)) = over(&mut f, Outcome::Winner(0), [2, 1, 0, 0]) else { panic!("{}", f.describe()) };
        let l = f.run.team_ladder.clone().expect("a ladder");
        assert_eq!((l.idx, l.won, l.lost), (1, 2, 1));
        assert_eq!(([spec.chr[0], spec.chr[2]], [spec.chr[1], spec.chr[3]], spec.stage), ([0, 1], l.pairs[1], l.stages[1]));
        assert_eq!(f.team_arcade_header(), Some(crate::assets::text::fill(crate::assets::text::get("stage_n"), &[&2])));
        assert_eq!(f.check(), Ok(()));
        // the loading screen's spec must be the ladder's battle
        let mut bad = f.clone();
        bad.run.team_ladder.as_mut().expect("a ladder").idx = 2;
        assert!(bad.check().is_err());
        // (A4) any other end without the humans' win (an arcade battle ends a loss through its countdown) ends the run
        for o in [Outcome::Winner(1), Outcome::Draw] {
            at_battle(&mut f, 1);
            assert!(over(&mut f, o, [1, 2, 0, 0]).is_none());
            assert!(matches!(f.screen, Screen::Title { menu: true, .. }), "{}", f.describe());
            assert_eq!((f.run.mode, f.run.team_ladder.is_none()), (Mode::Team, true));
            assert_eq!(f.check(), Ok(()));
        }
        let n = f.run.team_ladder.as_ref().map_or(5, |l| l.pairs.len()) as u8;
        at_battle(&mut f, n - 1);
        assert_eq!(f.team_arcade_header().as_deref(), Some(crate::assets::text::get("final_stage")));
        assert!(over(&mut f, Outcome::Winner(0), [2, 0, 0, 0]).is_none());
        // (A4) the ending: the starter's epilogue
        assert!(matches!(f.screen, Screen::Story), "{}", f.describe());
        assert_eq!(f.describe().split(' ').take(2).collect::<Vec<_>>(), ["epilogue", "0"]);
        assert_eq!(f.check(), Ok(()));
        at_battle(&mut f, 0);
        assert!(over(&mut f, Outcome::Quit, [0; 4]).is_none());
        assert!(matches!(f.screen, Screen::Title { menu: true, .. }), "{}", f.describe());
        assert_eq!((f.run.mode, f.run.team_ladder.is_none()), (Mode::Team, true));
    }

    /// (A3) The battle's countdown: a continue opens the arcade select at the same stage (the cursors on the humans'
    /// fighters, a CPU partner picked again, the continue counted, the rank −20 with bit 8 as ARCADE's), where pad 2
    /// may join and both choose anew → the same stage loads with the partner a human; GAME OVER ends the run on the
    /// mode menu at once (the battle has shown it).
    #[test]
    fn continue_and_game_over() {
        let Some(mut f) = front() else { return };
        let over = |f: &mut Front, o: Outcome, rounds: [u8; 4]| f.team_over(o, rounds, [0; 4], &mut Queue::default());
        let spec = at_battle(&mut f, 1);
        f.run.rank = 0x50;
        assert!(over(&mut f, Outcome::Continue, [0, 2, 0, 0]).is_none());
        let Screen::TeamSelect(sel) = f.screen else { panic!("{}", f.describe()) };
        assert_eq!((sel.p[0].cur, sel.p[2].cur, sel.p[0].human, sel.p[2].human), (0, 1, true, false));
        // the humans' slots choosing, the enemies' chosen
        assert_eq!(sel.p.map(|p| p.state), [0, 1, 0, 1]);
        let l = f.run.team_ladder.clone().expect("a ladder");
        assert_eq!((l.idx, l.won, l.lost, f.run.continues, f.run.rank), (1, 0, 2, 1, (0x50 - 20) | 0x100));
        assert_eq!(f.check(), Ok(()));
        // pad 2 joins as the partner; pad 1 takes YUGO in □'s costume, pad 2 ALICE in ×'s
        for (raw, ext) in [([0, START], [0, 0]), ([0, 0], [0, 0]), ([SQUARE, CROSS], [0, 0]), ([0, 0], [0, 0])] {
            assert!(tick(&mut f, raw, ext).is_none(), "{}", f.describe());
        }
        let mut ev = None;
        for _ in 0..20 {
            ev = tick(&mut f, [0; 2], [0; 2]);
            if ev.is_some() {
                break;
            }
        }
        let Some(Event::LoadTeam(again)) = ev else { panic!("{}", f.describe()) };
        assert_eq!((again.stage, [again.chr[1], again.chr[3]], again.cpu), (spec.stage, [spec.chr[1], spec.chr[3]], [false, true, false, true]));
        assert_eq!((again.chr[0], again.chr[2], again.costume[0], again.costume[2]), (0, 1, 0, 1));
        assert_eq!(f.run.team_ladder.as_ref().and_then(|l| l.partner_port), Some(1));
        assert_eq!(f.check(), Ok(()));
        // the countdown ran out
        at_battle(&mut f, 1);
        assert!(over(&mut f, Outcome::GameOver, [0, 2, 0, 0]).is_none());
        assert!(matches!(f.screen, Screen::Title { menu: true, .. }), "{}", f.describe());
        assert_eq!((f.run.mode, f.run.team_ladder.is_none()), (Mode::Team, true));
        assert_eq!(f.check(), Ok(()));
    }

    /// A run's loading screens keep the enemies' pairs out of the select's cursors: after a run reached the boss pair
    /// (locked on a fresh save, `SessionStart::locked`) and lost, TEAM BATTLE's 2 VS 2 select opens on unlocked fighters only.
    #[test]
    fn enemies_not_remembered() {
        let Some(mut f) = front() else { return };
        (f.save.all_open, f.save.unlock_override) = (false, false);
        assert_eq!(f.chars() & (1 << BOSS_PAIR[0] | 1 << BOSS_PAIR[1]), 0, "a fresh save");
        at_battle(&mut f, 0);
        let n = f.run.team_ladder.as_ref().expect("a ladder").pairs.len() as u8;
        at_battle(&mut f, n - 2);
        let Some(Event::LoadTeam(spec)) = f.team_over(Outcome::Winner(0), [2, 0, 0, 0], [0; 4], &mut Queue::default()) else { panic!("{}", f.describe()) };
        assert_eq!([spec.chr[1], spec.chr[3]], BOSS_PAIR);
        f.screen = Screen::TeamFight(spec);
        assert!(f.team_over(Outcome::Winner(1), [0, 2, 0, 0], [0; 4], &mut Queue::default()).is_none());
        for _ in 0..300 {
            tick(&mut f, [0; 2], [0; 2]);
        }
        assert!(matches!(f.screen, Screen::Title { menu: true, .. }), "{}", f.describe());
        let chars = f.chars();
        let unlocked = |c: u8| chars >> c & 1 != 0;
        assert!(f.team_last.iter().all(|&c| unlocked(c)), "{:?}", f.team_last);
        f.enter_team_mode(0, &mut Queue::default());
        tick(&mut f, [CROSS, 0], [0; 2]);
        let Screen::TeamSelect(sel) = f.screen else { panic!("{}", f.describe()) };
        assert!(sel.p.iter().all(|p| unlocked(p.cur)), "{:?}", sel.p.map(|p| p.cur));
    }

    /// One frame as the session runs it, a fetch served from the export at once (`tick`'s files are the front end's).
    fn tick_fetching(f: &mut Front, raw: [u16; 2], ext: [u16; 2]) {
        if let Some(Event::Fetch(names)) = tick(f, raw, ext) {
            let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
            let a = crate::assets::Assets::new(root);
            let files = names.into_iter().filter_map(|n| Some((n.clone(), pollster::block_on(a.load(&n)).ok()?.into()))).collect();
            f.fetched(files, &mut Queue::default());
        }
    }

    /// (A4) The ending after the last stage: the epilogue of each human's fighter in slot order (the starter's, then a
    /// partner pad's; a CPU partner's none; a fighter shown once; MITSUKO / FOX / GREG have no picture), then the clear
    /// card (the time, the rounds, the continues; a press skips it after its first second), then CONGRATULATIONS and
    /// the mode menu with the run ended. Each screen passes `check`, the epilogue and the card also as decoded states.
    #[test]
    fn ending() {
        let Some(mut f) = front() else { return };
        let roundtrip = |f: &Front| {
            let back: Front = postcard::from_bytes(&postcard::to_allocvec(f).expect("serialises")).expect("decodes");
            assert_eq!((back.describe(), back.check()), (f.describe(), Ok(())));
        };
        let last = |f: &mut Front, chr: [u8; 2], partner: Option<u8>| {
            let n = f.run.team_ladder.as_ref().map_or(5, |l| l.pairs.len()) as u8;
            at_battle(f, n - 1);
            let l = f.run.team_ladder.as_mut().expect("a ladder");
            (l.chr, l.partner_port, l.won, l.lost) = (chr, partner, 9, 3);
            f.run.continues = 2;
            f.run.play_time = 3600 * 7 + 60 * 5 + 30;
            let spec = f.team_arcade_spec().expect("a battle");
            f.screen = Screen::TeamFight(spec);
            assert!(f.team_over(Outcome::Winner(0), [2, 1, 0, 0], [0; 4], &mut Queue::default()).is_none());
        };
        let epilogue = |f: &Front| match (&f.screen, &f.story_screen) {
            (Screen::Story, Some(StoryScreen::Epilogue(e))) => Some(e.chr),
            _ => None,
        };
        // YUGO and a partner on pad 2 with ALICE: both epilogues, 360 frames each once loaded
        last(&mut f, [0, 1], Some(1));
        let l = f.run.team_ladder.clone().expect("a ladder");
        assert_eq!((l.idx as usize, l.won, l.lost, l.ending), (l.pairs.len(), 11, 4, 1));
        let mut seen = vec![epilogue(&f)];
        assert_eq!(f.check(), Ok(()));
        let mut frames = 0;
        while matches!(f.screen, Screen::Story) && frames < 1000 {
            tick_fetching(&mut f, [0; 2], [0; 2]);
            frames += 1;
            if epilogue(&f).is_some() && seen.last() != Some(&epilogue(&f)) {
                seen.push(epilogue(&f));
            }
            if frames == 100 {
                assert_eq!(f.check(), Ok(()));
                roundtrip(&f);
            }
        }
        assert_eq!(seen, [Some(0), Some(1)]);
        assert!((2 * 360..2 * 370).contains(&frames), "{frames} frames of epilogues");
        // the clear card
        assert!(matches!(f.screen, Screen::TeamArcadeClear { t: 0 }), "{}", f.describe());
        assert_eq!(f.check(), Ok(()));
        roundtrip(&f);
        let mut prims = Vec::new();
        f.draw_team_arcade_clear(&mut prims);
        assert!(!prims.is_empty());
        // START in its first second does not skip it, later it does
        for _ in 0..30 {
            tick(&mut f, [START, 0], [0; 2]);
            tick(&mut f, [0; 2], [0; 2]);
        }
        assert!(matches!(f.screen, Screen::TeamArcadeClear { t: 60 }), "{}", f.describe());
        tick(&mut f, [0, START], [0; 2]);
        assert!(matches!(f.screen, Screen::Message { text: Text::Congratulations, .. }), "{}", f.describe());
        assert_eq!(f.check(), Ok(()));
        for _ in 0..300 {
            tick(&mut f, [0; 2], [0; 2]);
        }
        assert!(matches!(f.screen, Screen::Title { menu: true, .. }), "{}", f.describe());
        assert_eq!((f.run.mode, f.run.team_ladder.is_none()), (Mode::Team, true));
        // a CPU partner: the starter's epilogue only; the same fighter twice: once; the card runs out by itself
        for (chr, partner) in [([2, 3], None), ([4, 4], Some(2))] {
            last(&mut f, chr, partner);
            assert_eq!(epilogue(&f), Some(chr[0] as usize));
            f.story_screen = None;
            f.team_arcade_next_ending();
            assert!(matches!(f.screen, Screen::TeamArcadeClear { t: 0 }), "{}", f.describe());
            for _ in 0..CLEAR_FRAMES - 1 {
                tick(&mut f, [0; 2], [0; 2]);
            }
            assert!(matches!(f.screen, Screen::TeamArcadeClear { .. }), "{}", f.describe());
            tick(&mut f, [0; 2], [0; 2]);
            assert!(matches!(f.screen, Screen::Message { text: Text::Congratulations, .. }), "{}", f.describe());
        }
        // The imported game's fighters: URIKO (BR1)'s picture, none for MITSUKO / FOX / GREG
        assert!(team_epilogue(BR1_FIRST + 3).is_some_and(|e| e.valid() && e.chr == 0));
        assert!((BR1_FIRST..BR1_FIRST + 3).all(|c| team_epilogue(c).is_none()));
        // the ending's screens only after the last stage
        last(&mut f, [0, 1], None);
        let mut bad = f.clone();
        bad.run.team_ladder.as_mut().expect("a ladder").idx -= 1;
        assert!(bad.check().is_err());
        let mut bad = f.clone();
        bad.screen = Screen::TeamArcadeClear { t: 0 };
        assert!(bad.check().is_err(), "the clear card before the epilogues");
        bad.end_team_arcade();
        assert!(bad.check().is_err(), "the clear card outside a run");
    }

    /// (A4) The debug start OKB_TEAM_ARCADE (`Entry::TeamArcade`): from v26 a pad-1 run at the stage's loading screen
    /// (the last at most) with the humans' fighters and the partner on pad 2, its load asked for on the first frame;
    /// before v26 the title.
    #[test]
    fn debug_start() {
        let Some(f) = front() else { return };
        let start = |sim_version| crate::session::SessionStart {
            entry: crate::session::Entry::TeamArcade { stage: 6, chr: [1, 6], partner: true },
            sim_version,
            ..Default::default()
        };
        let mut f = Front::new(f.data.clone(), &mut Queue::default(), &start(v::TEAM_ARCADE));
        let Screen::TeamLoad { spec, .. } = f.screen else { panic!("{}", f.describe()) };
        assert_eq!(f.team_arcade_header().as_deref(), Some(crate::assets::text::get("final_stage")));
        assert_eq!(([spec.chr[0], spec.chr[2]], [spec.chr[1], spec.chr[3]], spec.cpu), ([1, 6], BOSS_PAIR, [false, true, false, true]));
        assert_eq!(f.team_slot_ports(), Some([Some(0), None, Some(1), None]));
        assert_eq!(f.check(), Ok(()));
        assert!(matches!(tick(&mut f, [0; 2], [0; 2]), Some(Event::LoadTeam(s)) if s == spec));
        let f = Front::new(f.data.clone(), &mut Queue::default(), &start(v::TEAM_ARCADE - 1));
        assert!(f.run.team_ladder.is_none() && !matches!(f.screen, Screen::TeamLoad { .. }), "{}", f.describe());
    }
}
