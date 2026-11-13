//! Save states: the runtime snapshot, the `.okbs` file and `rebind`.
//!
//! Engine addition.
//! State: defines the serialised forms of `GameState` (`STATE_VERSION`, `FILE_FORMAT`)
//!
//! Save states: the runtime snapshot (postcard, this build only: `save_state` / `load_state`), the save-state
//! file (`.okbs`, CBOR: `save_file` / `load_file`) and `rebind`, which puts the static data back after either.

use super::start::data_fits;
use super::{Assets, GameState};
use crate::base::decode_limit;
use crate::fight::version::Gates;

/// The runtime snapshot's version (rewind, the rollback test, desync dumps: postcard, the same build only):
/// bumped on any change to what the state serialises. A snapshot of another version or build is refused; the
/// build check is weak for uncommitted changes (every dirty build of a commit is "hash+").
/// 2: `GameState::sim_version`.
/// 3: `GameState::browser`, `Menu::browser`.
/// 4: `Front::all_open`.
/// 5: `Stage::anim`, `Match::stage_rows`, `Round::win_reset`, `Fx::keep_slot`.
/// 6: `GameState::network`, `frontend::Event::Network`.
/// 7: `GameState::android`, `Menu::android`.
/// 8: `Menu::data_row`, `frontend::Event::ChangeData`.
/// 9: `GameState::extra_stage`.
/// 10: `GameState::extra_q3dm6`.
/// 11: `Fighter::head_yaw`, `Fighter::head_pitch`, `Fighter::head_w`.
/// 12: `Match::br1` and the KO replay snapshot's `br1` (the imported fighters, `fight::br1::bridge::Br1Match`).
/// 13: `Stage::br1` (an imported stage's routine counters, `fight::stage::br1::Counters`).
/// 14: `stage::br1::Counters`' `init`, `g50`, `rand`, `sounds` (simulation v18).
/// 15: `GameState::br1`, `frontend::Tim::Br1Select` / `Br1Faces` / `Br1Vs` (the import S9).
/// 16: `Br1Match::ai`, `Br1Match::ai_rnd` (the imported game's AI blocks and their table, simulation v20), the front
/// end's `Run::br1_boss`, `frontend::Tim::Raw`, the arcade epilogue's imported picture (`Epilogue::art`).
/// 17: `Epilogue::blank` (an epilogue whose imported picture did not arrive, simulation v21).
/// 18: `GameState::team` (the team battle, simulation v22).
/// 19: `TeamFight::hud` (the team battle's HUD, `fight::team::TeamHud`).
/// 20: `GameState::team_spec`, `TeamFight::pause`, `TeamOutcome::Quit`, the team battle's screens (`frontend::team`).
/// 21: `TeamFight::br1` (the team battle's imported fighters, `fight::team::br1::TeamBr1`, simulation v23).
/// 22: `GameState::team_config` (the team pause's CONTROLLER CONFIG., `session::TeamConfig`, simulation v24).
/// 23: `TeamOutcome::Draw` (the team battle's DRAW GAME), `TeamFight::locked` a `target::Locks` (simulation v25;
///     a battle without a thrown fighter's lock keeps its bytes).
/// 24: `frontend::Mode::TeamArcade`, the front end's `Run::team_ladder` (TEAM ARCADE A1 / A2, simulation v26; released).
/// 25: TEAM ARCADE A3 / A4: `TeamFight::arcade` (`fight::team::ArcadeRules`), `TeamOutcome::Continue` / `GameOver`,
///     `frontend::Screen::TeamArcadeClear`, `TeamLadder::ending` (a released 24 state must be refused by its version,
///     not decoded with shifted bytes).
pub const STATE_VERSION: u32 = 25;

/// Largest state accepted from bytes (a snapshot or a file; real ones stay under 100 KB).
pub const MAX_STATE_BYTES: usize = 1 << 20;
/// Memory the collections of a decoded state may take (`decode_limit::Budget`; a real fight takes well under 1 MB).
pub(super) const DECODE_BUDGET: usize = 32 << 20;

/// A runtime snapshot: `STATE_VERSION`, the build (`build::VERSION`) and the postcard `GameState`.
pub fn save_state(s: &GameState) -> Vec<u8> {
    postcard::to_allocvec(&(STATE_VERSION, crate::base::build::VERSION, s)).expect("the state serialises")
}

/// A state from `save_state`'s bytes of this build, its static data put back from `a` (`rebind`). Bytes that are
/// not such a state give Err, never a panic or an unbounded allocation: the size is capped, every collection is
/// charged to a budget while decoding (`limit`), and the ids and indices the state uses are checked (`rebind`).
pub fn load_state(b: &[u8], a: &Assets) -> Result<GameState, String> {
    if b.len() > MAX_STATE_BYTES {
        return Err(format!("save state of {} bytes", b.len()));
    }
    let ((version, build), rest): ((u32, String), _) = postcard::take_from_bytes(b).map_err(|e| format!("save state header: {e}"))?;
    if version != STATE_VERSION || build != crate::base::build::VERSION {
        return Err(format!("save state of version {version} build {build}, this is {STATE_VERSION} {}", crate::base::build::VERSION));
    }
    let (decode_limit::Budgeted::<GameState, DECODE_BUDGET>(mut s), rest) = postcard::take_from_bytes(rest).map_err(|e| format!("save state: {e}"))?;
    if !rest.is_empty() {
        return Err(format!("save state: {} bytes after the state", rest.len()));
    }
    rebind(&mut s, a)?;
    Ok(s)
}

/// Save-state files (`.okbs`): `FILE_MAGIC`, the format as u16 LE, then CBOR of `(engine version, GameState)`.
/// CBOR names every field, so a later build reads an older file: a field added since takes its
/// `#[serde(default)]` and a field removed since is ignored. A change those cannot carry (a field's type or
/// meaning) bumps `FILE_FORMAT` and `load_file` migrates from the older formats (none so far). `BR2S` is a
/// historical format id, kept so that files of earlier builds stay readable (the files are `.okbs`, earlier `.br2s`).
pub const FILE_MAGIC: &[u8; 4] = b"BR2S";
pub const FILE_FORMAT: u16 = 1;

/// The state as a save-state file.
pub fn save_file(s: &GameState) -> Vec<u8> {
    let mut b = FILE_MAGIC.to_vec();
    b.extend_from_slice(&FILE_FORMAT.to_le_bytes());
    ciborium::into_writer(&(crate::base::build::VERSION, s), &mut b).expect("the state serialises");
    b
}

/// A save-state file's state and the engine version that wrote it, before `rebind` (Err for anything but a file
/// of a format this build reads; the same caps as `load_state`).
pub fn decode_file(b: &[u8]) -> Result<(GameState, String), String> {
    if b.len() > MAX_STATE_BYTES {
        return Err(format!("save state file of {} bytes", b.len()));
    }
    let (Some(magic), Some(format)) = (b.get(..4), b.get(4..6)) else { return Err("not a save state file".into()) };
    if magic != FILE_MAGIC {
        return Err("not a save state file".into());
    }
    match u16::from_le_bytes([format[0], format[1]]) {
        FILE_FORMAT => {}
        f if f > FILE_FORMAT => return Err(format!("save state file format {f} is newer than this build reads ({FILE_FORMAT})")),
        f => return Err(format!("unknown save state file format {f}")),
    }
    let mut body = &b[6..];
    let (engine, decode_limit::Budgeted::<GameState, DECODE_BUDGET>(s)) = ciborium::from_reader(&mut body).map_err(|e| format!("save state file: {e}"))?;
    if !body.is_empty() {
        return Err(format!("save state file: {} bytes after the state", body.len()));
    }
    Ok((s, engine))
}

/// A save-state file's state, its static data put back from `a`. A file of another engine version loads (the
/// simulation may differ from there on: logged).
pub fn load_file(b: &[u8], a: &Assets) -> Result<GameState, String> {
    let (mut s, engine) = decode_file(b)?;
    if engine != crate::base::build::VERSION {
        log::warn!("save state file of engine {engine}, this is {}", crate::base::build::VERSION);
    }
    rebind(&mut s, a)?;
    Ok(s)
}

/// After deserialising: the static data the state refers to, from `a` (the front end's files, the fight's and the
/// backdrop's data, the app's preferences). Err when `a` lacks some of it (e.g. a movie's stream not fetched
/// again) or it does not match the state.
pub fn rebind(s: &mut GameState, a: &Assets) -> Result<(), String> {
    // a match and a team battle share the fight data (`Assets::fight`): never both at once
    if s.fight.is_some() && s.team.is_some() {
        return Err("a match and a team battle".into());
    }
    if let Some(spec) = &s.spec {
        spec.check()?;
    }
    if let Some(spec) = &s.team_spec {
        spec.check()?;
    }
    let extras = s.extras();
    if let Some(f) = &mut s.front {
        f.set_sim_version(s.sim_version);
        f.set_browser(s.browser);
        f.set_network(s.network);
        f.set_extras(extras);
        f.set_android(s.android);
        f.set_br1(s.br1);
        f.set_netplay(s.netplay);
        f.rebind(a.front.ok_or("no front-end data")?, a.files, a.prefs)?;
    }
    // the fight data's clips and skeleton math are those of one simulation version (`Loaded::portable`)
    if let Some(g) = &mut s.fight {
        let d = a.fight.ok_or("no fight data")?;
        data_fits(d, s.sim_version).and_then(|_| g.rebind(d)).map_err(|e| format!("fight: {e}"))?;
    }
    if let Some(g) = &mut s.backdrop {
        let d = a.backdrop.ok_or("no backdrop data")?;
        data_fits(d, s.sim_version).and_then(|_| g.rebind(d)).map_err(|e| format!("backdrop: {e}"))?;
    }
    if let Some(t) = &mut s.team {
        let d = a.fight.ok_or("no team battle data")?;
        if !Gates::of(s.sim_version).team {
            return Err(format!("team battle: simulation version {}", s.sim_version));
        }
        data_fits(d, s.sim_version).and_then(|_| t.rebind(d)).map_err(|e| format!("team battle: {e}"))?;
        // (TEAM ARCADE, v26) a battle of the front end has its rules exactly in a run, for the run's humans' team (`sim`'s
        // battles have no front end)
        if let Some(f) = &s.front
            && t.arcade.as_ref().map(|r| r.human_team as usize) != f.arcade_team()
        {
            return Err("team battle: arcade rules".into());
        }
    }
    // the team pause's CONTROLLER CONFIG. (simulation v24): only over a team battle, its rows and cursor in range
    if let Some(c) = &s.team_config {
        if s.team.is_none() || !Gates::of(s.sim_version).team_config || c.menu.kind != crate::frontend::menus::Kind::Controller {
            return Err("team config".into());
        }
        c.menu.check()?;
    }
    Ok(())
}
