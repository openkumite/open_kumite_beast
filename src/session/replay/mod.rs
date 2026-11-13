//! Replays (`.okbr`): a session's start and every step's input, re-simulated to watch it.
//!
//! Engine addition.
//! State: the replay file (CBOR header and body, `FORMAT`); not part of `GameState`
//!
//! Replays (`.okbr`, game-state plan stage 6): a session as what it started from (`SessionStart`, the app's
//! preferences, a direct fight's costumes) and every step's input (`FrameInput`, the debug `Cheat`s, the
//! preferences when they changed), re-simulated to watch it. The app records every session (`Recorder`), saves each
//! finished match as a file of the session up to its end (watched from its VS screen: `Header::from`), and plays a
//! file back (`Player`; the headless `simulate` checks one). Checkpoints every 60 steps hold two hashes of the
//! state: `sync_hash` (values every later build computes the same way) and the full state hash (compared only by
//! the build that wrote the file, whose state layout it is).
//!
//! The file: little-endian, the fixed envelope first so that a newer file is refused before its CBOR is read.
//!
//! | bytes | |
//! |---|---|
//! | 4 | `MAGIC` "BR2R" |
//! | 2 | `FORMAT` (u16) |
//! | 4 | `sim_version` (u32, `super::SIM_VERSION` when recorded) |
//! | 1 + n | the engine version (`build::VERSION`): length, UTF-8 |
//! | 4 + n | `Header` as CBOR: length, bytes |
//! | 4 + n | `Body` as CBOR: length, bytes (the input stream, the cheats, the preference changes, the checkpoints) |
//! | 8 | FNV-1a 64 of everything before |
//!
//! CBOR names every field: a field added later takes its `#[serde(default)]` in older files, one removed is
//! ignored; a change those cannot carry bumps `FORMAT`. The input stream is one CBOR byte string of runs, each: a
//! byte `m` (bits 0..4: which of pad 1, pad 2, forced 1, forced 2, flags differ from the run before, which starts
//! as `FrameInput::default()`; bits 5..7: the run's length − 1, or 7 = 8 + a varint that follows), then the changed
//! values as LEB128 varints in that order. FORMAT 2 (`FORMAT_TEAM`, engine addition: the team battle's pads 3 / 4)
//! has the same runs over nine fields (then pad 3, pad 4, forced 3, forced 4) with a u16 `m` (little-endian; bits
//! 0..8 the fields, bits 9..11 the length code, bits 12..15 zero); the rest of the file is FORMAT 1's.
//!
//! Loading never panics or allocates without bound: the size is capped (`MAX_BYTES`), the CBOR is decoded under a
//! memory budget (`limit`), and every collection, id, index and frame is checked after decoding (`Replay::check`).

use super::{Cheat, Entry, FrameInput, SIM_VERSION, SessionStart};
use crate::base::decode_limit;
use crate::base::hash::{FNV_BASIS, fnv1a};
use crate::fight::sim::Outcome;
use crate::frontend::options::Prefs;
use crate::frontend::{MatchSpec, TeamSpec};
use serde::{Deserialize, Serialize};

mod headless;
mod inputs;
mod player;
mod recorder;
mod sync;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

#[cfg(not(target_arch = "wasm32"))]
pub use headless::simulate;
pub use headless::{Headless, Summary, simulate_async};
use inputs::read_run;
pub use player::{Player, Pos};
pub use recorder::Recorder;
pub use sync::sync_hash;

/// A replay file's first bytes (a historical format id, kept so that files of earlier builds stay readable; the files are `.okbr`, earlier `.br2r`).
pub const MAGIC: &[u8; 4] = b"BR2R";
/// The file layout's newest version this build reads (the module doc's table): 1, or 2 for a file whose input
/// stream holds pads 3 / 4 (the team battle, engine addition: `FORMAT_TEAM`).
pub const FORMAT: u16 = 2;
/// The input stream with pads 3 / 4 (nine fields and a u16 mask, `inputs`): written only when the header holds a
/// `TeamSpec` or the session used pads 3 / 4; every other file stays FORMAT 1.
pub const FORMAT_TEAM: u16 = 2;
/// Largest file accepted (an hour of play takes a few hundred KB).
pub const MAX_BYTES: usize = 8 << 20;
/// Most steps a recording holds: 24 hours (the recorder stops there).
pub const MAX_FRAMES: u32 = 24 * 60 * 60 * 60;
/// A checkpoint every this many steps (and one at a saved file's end).
pub const CHECK_EVERY: u32 = 60;
/// The longest name a replay can be given (the replay list's RENAME), in characters.
pub const MAX_NAME: usize = 64;
const MAX_CHEATS: usize = 100_000;
const MAX_PREFS: usize = 10_000;
const MAX_CHECKS: usize = (MAX_FRAMES / CHECK_EVERY) as usize + 1024;
/// The engine version and the mode name.
const MAX_TEXT: usize = 128;
/// The memory card file in the start (`platform::storage::read_card`'s cap).
const MAX_CARD: usize = 1 << 20;
/// Memory the decoded header and body may take (`decode_limit::Budget`).
const DECODE_BUDGET: usize = 32 << 20;

/// What a replay is of: the start, the setup the app gave the session, and the match it was saved for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Header {
    /// `super::STATE_VERSION` of the build that recorded it (the full checkpoint hashes are its state's)
    pub state_version: u32,
    /// the session's start (its `sim_version` is the envelope's)
    pub start: SessionStart,
    /// a direct fight's costumes (`fight_data::direct_costume`)
    pub costume: [u8; 2],
    /// the app's preferences at the start (the GRAPHICS screen shows them; changes are in the body)
    pub prefs: Prefs,
    /// when it was saved: seconds since 1970 (UTC)
    pub time: u64,
    /// steps recorded
    pub frames: u32,
    /// the frame watching starts at: the match's VS screen (0: the whole session)
    pub from: u32,
    /// the frame the match's fight started
    pub fight_from: Option<u32>,
    /// the front end's mode (`Front::mode_name`; empty for a direct fight)
    pub mode: String,
    /// the match's sides and stage
    pub spec: Option<MatchSpec>,
    /// how it ended
    pub outcome: Option<Outcome>,
    /// rounds won by P1, P2
    pub rounds: [u8; 2],
    /// a name the player gave it (the replay menu's RENAME)
    pub name: Option<String>,
    /// KEEP: never dropped by the last-20 rotation
    pub keep: bool,
    /// the team battle saved (engine addition, simulation v22; `spec` is None then): its fighters and stage; `outcome`
    /// `Winner(t)` names the team that won (2 VS 2: 0 = P1 + P3, 1 = P2 + P4; FREE FOR ALL: the player)
    /// (not written when None, nor the rounds when all 0: a 1v1 header's bytes stay those of the builds before)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamSpec>,
    /// the rounds each team won (the team battle)
    #[serde(skip_serializing_if = "no_team_rounds")]
    pub team_rounds: [u8; 4],
    /// a TEAM ARCADE battle's place in its run (engine addition, simulation v26, A4; with `team`): the replay list's
    /// STAGE n / CLEAR / CONTINUE / GAME OVER (not written when None; a field older builds ignore)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_arcade: Option<TeamArcadeInfo>,
}

/// A TEAM ARCADE battle's place in its run (`Header::team_arcade`, from `Front::team_arcade_info` as the battle
/// started).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamArcadeInfo {
    /// the ladder's stage (1 ..)
    pub stage: u8,
    /// the ladder's length (5, or 6 with the imported game's data)
    pub stages: u8,
    /// the run's continues before the battle
    pub continues: u32,
}

fn no_team_rounds(r: &[u8; 4]) -> bool {
    *r == [0; 4]
}

/// The steps: the input stream and the sparse events.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Body {
    /// the `FrameInput` of every step, run-length coded (the module doc)
    #[serde(with = "crate::base::serde_util::blob")]
    pub inputs: Vec<u8>,
    /// (step, cheat): applied by the step from that frame, in order
    pub cheats: Vec<(u32, Cheat)>,
    /// (step, preferences): from the step from that frame on
    pub prefs: Vec<(u32, Prefs)>,
    /// (frame, state hash, sync hash) after the step that reached that frame
    pub checks: Vec<(u32, u64, u64)>,
}

/// A replay file's content.
#[derive(Clone, Debug)]
pub struct Replay {
    /// the file's `FORMAT`: 1, or `FORMAT_TEAM` (the input stream with pads 3 / 4)
    pub format: u16,
    /// the simulation version recorded with (`super::SIM_VERSION`; the logic branches on it)
    pub sim_version: u32,
    /// the build that recorded it (`build::VERSION`)
    pub engine: String,
    pub header: Header,
    pub body: Body,
}

impl Replay {
    /// The session's start, running the recorded simulation version.
    pub fn start(&self) -> SessionStart {
        SessionStart { sim_version: self.sim_version, ..self.header.start.clone() }
    }

    /// The file's bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&self.format.to_le_bytes());
        b.extend_from_slice(&self.sim_version.to_le_bytes());
        let engine = &self.engine.as_bytes()[..self.engine.floor_char_boundary(MAX_TEXT)];
        b.push(engine.len() as u8);
        b.extend_from_slice(engine);
        for part in [cbor(&self.header), cbor(&self.body)] {
            b.extend_from_slice(&(part.len() as u32).to_le_bytes());
            b.extend_from_slice(&part);
        }
        let sum = fnv1a(FNV_BASIS, &b);
        b.extend_from_slice(&sum.to_le_bytes());
        b
    }

    /// A file's replay: Err (never a panic) for anything but a valid file this build plays; a newer one is
    /// refused with a message saying so.
    pub fn decode(b: &[u8]) -> Result<Self, String> {
        let (format, sim_version, engine, header, rest) = envelope(b)?;
        let (body, rest) = part::<Body>(rest, "body")?;
        if !rest.is_empty() {
            return Err(format!("{} bytes after the replay", rest.len()));
        }
        let r = Replay { format, sim_version, engine, header, body };
        r.check()?;
        Ok(r)
    }

    /// A file's header only (a list of replays), with the same checks of the envelope and the header.
    pub fn decode_header(b: &[u8]) -> Result<(u32, String, Header), String> {
        let (_, sim_version, engine, header, _) = envelope(b)?;
        check_header(&header, sim_version)?;
        Ok((sim_version, engine, header))
    }

    /// The input stream is FORMAT 2's (pads 3 / 4).
    pub fn wide(&self) -> bool {
        self.format == FORMAT_TEAM
    }

    /// Everything a step and the menu would trust: the ids, indices, lengths and frames.
    fn check(&self) -> Result<(), String> {
        let h = &self.header;
        check_header(h, self.sim_version)?;
        let mut frames = 0u64;
        let (mut at, mut cur) = (0, FrameInput::default());
        while let Some(n) = read_run(&self.body.inputs, &mut at, &mut cur, self.wide())? {
            frames += n as u64;
            if frames > h.frames as u64 {
                return Err(format!("the input stream holds more than the {} steps", h.frames));
            }
        }
        if frames != h.frames as u64 {
            return Err(format!("the input stream holds {frames} of the {} steps", h.frames));
        }
        let b = &self.body;
        if b.cheats.len() > MAX_CHEATS || b.prefs.len() > MAX_PREFS || b.checks.len() > MAX_CHECKS {
            return Err("too many events".into());
        }
        if !b.cheats.windows(2).all(|w| w[0].0 <= w[1].0) || b.cheats.last().is_some_and(|c| c.0 >= h.frames) {
            return Err("cheats out of order".into());
        }
        if let Some(c) = b.cheats.iter().find(|(_, c)| !cheat_ok(*c)) {
            return Err(format!("cheat {c:?}"));
        }
        if !b.prefs.windows(2).all(|w| w[0].0 < w[1].0) || b.prefs.last().is_some_and(|p| p.0 >= h.frames) {
            return Err("preference changes out of order".into());
        }
        if let Some(p) = b.prefs.iter().find(|(_, p)| !prefs_ok(p)) {
            return Err(format!("preferences {p:?}"));
        }
        if !b.checks.windows(2).all(|w| w[0].0 < w[1].0) || b.checks.first().is_some_and(|c| c.0 == 0) || b.checks.last().is_some_and(|c| c.0 > h.frames) {
            return Err("checkpoints out of order".into());
        }
        Ok(())
    }
}

fn cbor<T: Serialize>(x: &T) -> Vec<u8> {
    let mut b = Vec::new();
    ciborium::into_writer(x, &mut b).expect("the replay serialises");
    b
}

/// The envelope (magic, format, version, engine, checksum) and the header; the bytes after the header.
fn envelope(b: &[u8]) -> Result<(u16, u32, String, Header, &[u8]), String> {
    if b.len() > MAX_BYTES {
        return Err(format!("a replay of {} bytes (at most {MAX_BYTES})", b.len()));
    }
    if b.get(..4) != Some(MAGIC) {
        return Err("not a replay file".into());
    }
    let (Some(format), Some(sim)) = (b.get(4..6), b.get(6..10)) else { return Err("not a replay file".into()) };
    let format = match u16::from_le_bytes([format[0], format[1]]) {
        f @ (1 | FORMAT_TEAM) => f,
        f if f > FORMAT => return Err(format!("replay file format {f} is newer than this build reads ({FORMAT}): update the game")),
        f => return Err(format!("unknown replay file format {f}")),
    };
    let sim_version = u32::from_le_bytes([sim[0], sim[1], sim[2], sim[3]]);
    if sim_version > SIM_VERSION {
        return Err(format!("recorded with a newer version (v{sim_version}), update the game"));
    }
    if sim_version == 0 {
        return Err("simulation version 0".into());
    }
    let Some(body) = b.len().checked_sub(8).map(|n| &b[..n]) else { return Err("the replay is truncated".into()) };
    let sum = u64::from_le_bytes(b[body.len()..].try_into().expect("8 bytes"));
    if fnv1a(FNV_BASIS, body) != sum {
        return Err("the replay is damaged (checksum)".into());
    }
    let n = *body.get(10).ok_or("the replay is truncated")? as usize;
    let engine = body.get(11..11 + n).ok_or("the replay is truncated")?;
    let engine = std::str::from_utf8(engine).map_err(|_| "the engine version is not text")?.to_string();
    let (header, rest) = part::<Header>(&body[11 + n..], "header")?;
    // pads 3 / 4 from the team battle's simulation version on
    if format == FORMAT_TEAM && !crate::fight::version::Gates::of(sim_version).team {
        return Err(format!("replay file format {format} of simulation v{sim_version}"));
    }
    Ok((format, sim_version, engine, header, rest))
}

/// A length-prefixed CBOR part; the bytes after it.
fn part<'a, T: serde::de::DeserializeOwned>(b: &'a [u8], what: &str) -> Result<(T, &'a [u8]), String> {
    let n = b.get(..4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize).ok_or("the replay is truncated")?;
    let mut data = b.get(4..).and_then(|x| x.get(..n)).ok_or("the replay is truncated")?;
    let decode_limit::Budgeted::<T, DECODE_BUDGET>(x) = ciborium::from_reader(&mut data).map_err(|e| format!("replay {what}: {e}"))?;
    if !data.is_empty() {
        return Err(format!("replay {what}: {} bytes after it", data.len()));
    }
    Ok((x, &b[4 + n..]))
}

/// A session started from `start` plays here only over the same data of the imported game (engine addition, the import
/// S7): `install` = this game data's imported-game content id (`assets::manifest::br1_install_id`).
pub fn br1_fits(start: &crate::session::SessionStart, install: Option<u32>) -> Result<(), String> {
    match start.br1 {
        Some(id) if install != Some(id) => Err(format!(
            "needs the Bloody Roar 1 disc data it was recorded with (content id {id:08x}; this game data has {})",
            install.map_or("none".to_string(), |i| format!("{i:08x}"))
        )),
        _ => Ok(()),
    }
}

/// A session started from `start` was recorded over other game data than this one's (engine addition:
/// `SessionStart::content`; `install` = this data's content id, `assets::manifest::install_id`): the warning to log (it
/// plays anyway, and may go its own way where the data differs); None when the ids are the same, or when the start has
/// none (recorded before the id).
pub fn content_warning(start: &crate::session::SessionStart, install: Option<u32>) -> Option<String> {
    let id = start.content?;
    (install != Some(id)).then(|| {
        format!(
            "recorded over other game data (content id {id:08x}; this game data has {}): it may not play as recorded",
            install.map_or("none".to_string(), |i| format!("{i:08x}"))
        )
    })
}

/// `h` of a file or netplay start of simulation `sim_version` (the envelope's; the start's own is not used).
pub(crate) fn check_header(h: &Header, sim_version: u32) -> Result<(), String> {
    let s = &h.start;
    let g = crate::fight::version::Gates::of(sim_version);
    if s.card.as_ref().is_some_and(|c| c.len() > MAX_CARD) {
        return Err("memory card file too large".into());
    }
    // an imported fighter from simulation v15 on, in a session with the imported game's data (the import S7)
    let br1 = |c: &str| crate::fight::fight_data::is_br1(c) && g.br1_fighters && s.br1.is_some();
    let chr = |c: &str| crate::assets::roster::CHARACTERS.contains(&c) || br1(c);
    match &s.entry {
        Entry::DirectFight { p1, p2, stage } => {
            // the extra stages from their simulation versions (`Extra::sim`: Q3DM17 v10, Q3DM6 v12)
            let extra = crate::fight::stage::extra::by_code(stage, g).is_some();
            // The imported game's stages from simulation v16, in a session with the imported game's data (the import S8)
            let br1_stage = crate::fight::stage::br1::by_code(stage, g).is_some() && s.br1.is_some();
            let st = crate::fight::fight_data::STAGE_CODES.contains(&stage.as_str()) || extra || br1_stage;
            if !chr(p1) || !chr(p2) || !st {
                return Err(format!("direct fight {p1} {p2} {stage}"));
            }
        }
        Entry::Epilogue { chr, .. } if *chr >= crate::assets::roster::CHARACTERS.len() => return Err(format!("epilogue {chr}")),
        // debug OKB_TEAM_ARCADE (TEAM ARCADE A4, simulation v26): a ladder's stage, the main game's fighters
        Entry::TeamArcade { stage, chr, .. }
            if !g.team_arcade
                || !(1..=crate::frontend::TEAM_ARCADE_MAX_STAGES).contains(stage)
                || chr.iter().any(|&c| c as usize >= crate::assets::roster::CHARACTERS.len()) =>
        {
            return Err(format!("team arcade start {stage} {chr:?}"));
        }
        // debug OKB_NAMETEST: arcade (0) clear time in frames, survival (3) wins (an i16 of the ranking)
        Entry::NameTest { mode, value, .. } if !matches!((*mode, *value), (0, 0..=0x7FFF_FFFF) | (3, 0..=0x7FFF)) => {
            return Err(format!("name entry test {mode}:{value}"));
        }
        _ => {}
    }
    // the debug fight setup (OKB_GAUGE, OKB_P2DMG, OKB_MODEL): the values the env hooks are meant for
    let d = &s.fight;
    if d.gauge.is_some_and(|g| !(0..=0x3BFF).contains(&g)) || d.p2dmg.is_some_and(|x| x < 0) || d.custom.is_some_and(|c| c.model > 3) {
        return Err(format!("debug fight setup {d:?}"));
    }
    if !h.costume.iter().all(|&c| c < 4) || !prefs_ok(&h.prefs) {
        return Err("costumes or preferences".into());
    }
    if h.frames > MAX_FRAMES || h.from > h.frames || h.fight_from.is_some_and(|f| f < h.from || f > h.frames) {
        return Err(format!("frames {} from {} fight {:?}", h.frames, h.from, h.fight_from));
    }
    if let Some(spec) = &h.spec {
        spec.check()?;
        // the select's imported cells and stages from simulation v19, in a session with the imported game's data (the imported
        // import S9)
        let new = g.br1_select && s.br1.is_some();
        if !new && (spec.chr.iter().any(|&c| crate::assets::roster::is_br1(c)) || crate::fight::stage::br1::by_stage(spec.stage as usize).is_some()) {
            return Err(format!("match {spec:?}"));
        }
    }
    // the team battle (engine addition) from its simulation version on: its sides, and a winner among its teams
    let teams = match &h.team {
        Some(t) if g.team && h.spec.is_none() => {
            t.check()?;
            // (T5) its imported fighters and the big stages from simulation v23, the imported game's (fighters, BIG
            // STAGE) in a session with the imported game's data
            let br1 = t.chr.iter().any(|&c| crate::assets::roster::is_br1(c)) || crate::fight::stage::br1::by_stage(t.stage as usize).is_some();
            let big = crate::fight::stage::TEAM_BIG_STAGES.contains(&(t.stage as usize));
            if (big && !g.team_big_stages) || (br1 && !(g.team_br1 && s.br1.is_some())) {
                return Err(format!("team battle {t:?} of simulation v{sim_version}"));
            }
            t.mode.teams()
        }
        Some(t) => return Err(format!("team battle {t:?} of simulation v{sim_version}")),
        None => 2,
    };
    if matches!(h.outcome, Some(Outcome::Winner(w) | Outcome::Challenger(w)) if w >= teams)
        || (h.team.is_some() && matches!(h.outcome, Some(Outcome::Challenger(_))))
        // (T7) a DRAW GAME is a team battle's, from simulation v25
        || (matches!(h.outcome, Some(Outcome::Draw)) && (h.team.is_none() || !g.draw_game))
    {
        return Err(format!("outcome {:?}", h.outcome));
    }
    // (TEAM ARCADE A4, simulation v26) a run's place only with its 2 VS 2 battle (an A3 build's run has none: its
    // continues and GAME OVERs stay readable)
    if let Some(a) = &h.team_arcade {
        let ok = g.team_arcade
            && h.team.is_some_and(|t| t.mode == crate::fight::team::TeamMode::Vs2)
            && (crate::frontend::TEAM_ARCADE_MIN_STAGES..=crate::frontend::TEAM_ARCADE_MAX_STAGES).contains(&a.stages)
            && (1..=a.stages).contains(&a.stage)
            && a.continues <= MAX_FRAMES;
        if !ok {
            return Err(format!("team arcade {a:?}"));
        }
    }
    if h.mode.len() > MAX_TEXT || h.name.as_ref().is_some_and(|n| n.chars().count() > MAX_NAME) {
        return Err("mode or name too long".into());
    }
    Ok(())
}

/// A cheat's fighter / wall / damage in range (`ForceMove` is checked where it applies: the move table is the
/// fight's).
fn cheat_ok(c: Cheat) -> bool {
    match c {
        Cheat::OneHp(i) | Cheat::Beast(i) => i < 2,
        Cheat::Break(side) => side < 4,
        Cheat::P2Dmg(d) => d >= 0,
        _ => true,
    }
}

fn prefs_ok(p: &Prefs) -> bool {
    use crate::frontend::options::{HUD_SCALES, RENDER_SCALES, WINDOW_SIZES};
    (p.window as usize) < WINDOW_SIZES.len() && (p.render_scale as usize) < RENDER_SCALES.len() && (p.hud_size as usize) < HUD_SCALES.len()
}
