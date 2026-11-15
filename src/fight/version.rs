//! The simulation versions (engine addition): `SIM_VERSION` with its changelog, one constant per rule a version
//! introduced (`v`) and the rules of one version as flags (`Gates`).
//!
//! State: none (`Gates` is derived from a session's `sim_version`, never serialised)
//!
//! Every comparison of a simulation version with a rule's first version is here: the logic asks `Gates` (held on
//! `fight_data::Loaded::gates`; the front end and the session build it with `Gates::of`), so a rule has one name and
//! one place. `tests/version_gates.rs` refuses a raw `sim_version` comparison elsewhere; `tests::gates_by_version`
//! holds every version's flags against a checked-in table (a new version appends a row; an old row never changes).

/// The simulation's version: what a replay (`session::replay`) was recorded with, carried by the state
/// (`session::GameState::sim_version`). The rule for all later work: every change to the game logic that alters the
/// simulation (the same start and inputs give other states) bumps it, and the old behaviour stays available behind a
/// rule of `v` asked through `Gates` (the replay's recorded version selects it; new sessions run the newest), so older
/// replays keep playing correctly. A replay recorded with a newer version than this build is refused.
/// v1: the first replays (game-state stage 6).
/// v2: REPLAY on the mode menu, before OPTIONS (engine addition, stage 6b): the menu's rows and cursor indices after
/// CUSTOM move down by one (`frontend::screen::mode_rows`; `v::REPLAY_ROW`).
/// v3: cross-platform determinism (stage 7): the fight's skeleton, spring chains and clips through
/// `crate::base::portable` (f64 glam, libm's trig) instead of the platform's f32 math (`Loaded::portable`;
/// `v::PORTABLE_MATH`).
/// v4: the beast form's end in the move decision starts its burst (op17 sub F) and plays its sound
/// (`v::REVERT_BURST`), and that sound (the request (0x400, 7, 0)) is SE 7 on every path (`move_script::revert_sound`;
/// `v::REVERT_SOUND`).
/// v5: every character and fourth costume selectable, and ST9..STC on the stage select (rows 9..12, RANDOM 13, which
/// draws from all 13), unless the session is `SessionStart::locked` (engine default; the game gates the characters
/// and costumes by progression and never offers ST9..STC: `frontend::Save::all_open`; `v::ALL_OPEN`).
/// v6: the stage routine's counters are fight state (reset every round by the stage init table, frozen while the stage is not
/// drawn, rewound by the KO replay; `stage_anim`; `v::STAGE_STATE`); the match-end reset of round state 0xD / 0xE
/// (effects, fades, both fighters by the fighter reset, mode 4; the loser's key cut at cnt 2: `Match::win_reset`,
/// `Round::win_tail`; `v::WIN_RESET`); an effect task that spawns while it runs keeps its slot
/// (`hit_effects::Fx::keep_slot`: emitters, spark trails and ring bursts run to their end, e.g. the beast revert's seven
/// pink sparks; `v::KEEP_SLOT`).
/// v7: the extra stages ST9..STC get bounds fitted to their rooms (engine addition, `fight::stage::EXTRA_BOUNDS`)
/// instead of the game's ±4096 box without walls (`v::EXTRA_BOUNDS`); the
/// sparks' arena follows the stage's ring (`v::SPARKS_ARENA`).
/// v8: NETWORK on the mode menu after REPLAY when the session is `SessionStart::network` (engine addition, the app's
/// NETWORK menu: OPTIONS and EXIT GAME move down by one; `frontend::screen::mode_rows`; `v::NETWORK_ROW`).
/// v9: CHANGE GAME DATA on OPTIONS after GRAPHICS (engine addition, the app's setup screen: MOVIES AND PICTURES ..
/// EXIT move down by one; `frontend::menus::Menu::data_row`; `v::DATA_ROW`).
/// v10: the extra stage Q3DM17 on the stage select after ST9..STC when the session has its files
/// (`SessionStart::extra_stage`; engine addition, `fight::stage::extra`: RANDOM moves down by one and still draws from the
/// 13; its fight runs as STB with a ring of its own, `fight::stage::Stage::new`; `v::Q3DM17`).
/// v11: the extra stage Q3DM17 at 7 game units per Quake unit instead of 8 (`Gates::q3_scale`: a Quake III
/// player model as tall as a main-game fighter): its ring 1120 each way instead of 1280, its ground outside 8960 below
/// instead of 10240 (`v::Q3_SCALE`).
/// v12: the extra stage Q3DM6 on the stage select after Q3DM17 (or in its place) when the session has its files
/// (`SessionStart::extra_q3dm6`; engine addition, `fight::stage::extra::Q3DM6`: RANDOM moves down by one more and still
/// draws from the 13; its fight runs as STB in a walled ring of its own, `fight::stage::Stage::new`; `v::Q3DM6`).
/// v13: the head turn toward the opponent (the end of the script step): node 2's
/// local matrix turned by RotMatrix(S+0x108, −S+0x106, 0) while F5C & 0x10000 (`move_script::head_turn`,
/// `Fighter::head_yaw` / `head_pitch`); the head's children (and so the skeleton the logic reads) move with it
/// (`v::HEAD_TURN`).
/// v14: Q3DM17's teleporter stands solid in its ring (engine addition, `fight::stage::extra::Extra::solids`: its
/// footprint measured in the map, a constant table): the fighters' body spheres are pushed out of it as the ring
/// walls push them in (`fight::physics::solids`); before, they walked through it (`v::EXTRA_SOLIDS`).
/// v15: the imported game's fighters in a match (engine addition, the import S7, `v::BR1_FIGHTERS`): a
/// match may hold them (`fight_data::load_match`, `fight::sim::Match::br1`), their moves inside the main game's
/// rules (`fight::sim::br1`); a match of main-game fighters runs as before.
/// v16: the imported game's stages in a direct fight (engine addition, the import S8, `fight::stage::br1`; `v::BR1_STAGES`):
/// their own ring and walls from the imported game's static tables (`fight::stage::Stage::br1`), imported fighters reading
/// their stage byte and ring records there; the HUD's gauge of an imported fighter marks its 0xF00 (`fight::hud`;
/// `v::BR1_HUD_GAUGE`).
/// v17: on an imported stage the stage sound (op 0x26 sub 7) is queued (`v::BR1_STAGE_SOUND`,
/// `fight::sim::br1`; engine addition, the import S8), on Uriko's stage by its counter G.4A
/// (`fight::stage::br1::Counters`, `fight::stage::Stage::br1`); v16 matches run as before.
/// v18: on an imported stage a broken wall falls into the imported game's own debris (its
/// pieces, bounces or splashes and its stage sound countdown, random bytes on the stage's own counter) in place of
/// the main game's; Uriko's stage runs its wall events (the walls shatter at G.4A 15 / 30 / 45 / 60 when
/// on the camera's far side, rise again from 0x4B.. with the stage sound (1); G.42 bit 2 taken as set)
/// (`v::BR1_DEBRIS`); op 0x17 sub 2's case 10 queues its sound 0x5F (`v::BR1_CALL_SOUND`); v17 matches
/// run as before.
/// v19: the imported game's fighters and stages on the menus when the session has its data (`SessionStart::br1`; engine
/// addition, the import S9, `v::BR1_SELECT`): the select's row of four imported cells under
/// the main game's grid in VS, CUSTOM VS HUMAN and PRACTICE (the bottom row's DOWN leads there), the stage select's
/// eleven imported stages after the extra stages (RANDOM moves down by eleven and still draws from the 13); a
/// imported fighter's own name on the HUD (`v::BR1_NAMES`); an imported loser's key cut at the match end as
/// the match end cuts a main-game loser's (`Match::br1_key_cut`; `v::BR1_KEY_CUT`).
/// v20: the imported game's CPU AI (engine addition, the import S10, `v::BR1_AI`): a CPU-controlled
/// imported fighter plays by its own AI (`fight::br1::ai`; before, its pad was 0), its random table
/// shuffled at every round start by a draw of the fighters' stream G; with the imported game's data the select's
/// imported row also in ARCADE (the NORMAL select's third row), SURVIVAL, WATCH, CUSTOM VS COMPUTER (ALL MEMBERS), ARCADE's
/// extra boss "URIKO (BR1)" after the main game's last fight, an imported fighter's ladder with two imported
/// opponents and the imported game's ending (`frontend::progress`; `v::BR1_CPU_MODES`).
/// v21: an imported ending's length no longer depends on the install's movie and picture files (engine addition, the
/// the import S11, `v::BR1_FIXED_MOVIES`): each movie of the imported game plays the frames of the disc's stream
/// (`frontend::movie::BR1_MOVIE_END`), black when its stream is missing (before: its stream's own frames, none
/// without it); the epilogue without its picture runs its 360 frames black (before: none).
/// v22: the team battle (engine addition, `v::TEAM`): four main-game fighters
/// on an enlarged open ring, 2 VS 2 or FREE FOR ALL (`fight::team::TeamFight`, `GameState::team`), with its
/// presentation (T3) and its menus, pads 3 / 4 and replay format 2 (T4), which came under v22 too (v22 was not
/// released between T2 and T4); a match runs as before.
/// v23: the team battle with imported fighters and on the big stages BIG STAGE and Q3DM6 (T5,
/// `v::TEAM_BR1`, `v::TEAM_BIG_STAGES`: the team select's imported row, the stage panel's big stages); a v22 team
/// battle and a match run as before.
/// v24: the team battle's T6: the pause's CONTROLLER CONFIG. opens the button-mapping screen (`session::TeamConfig`;
/// offered with a human on pad 1 or 2; `v::TEAM_CONFIG`) and a SUDDEN DEATH time over is a DRAW (`TeamRound::tick`;
/// `v::SUDDEN_DEATH_DRAW`); a v22 / v23 team battle and a match run as before.
/// v25: the team battle's T7: unbreakable walls round the enlarged rings (no ring-outs; `v::TEAM_WALLS`), a match
/// DRAW when several teams reach the rounds needed at once (no SUDDEN DEATH; `v::DRAW_GAME`), a main-game
/// knock-down launches an imported fighter (also in a 1v1 mixed match: `v::BR1_LAUNCH`), the team battle's
/// imported fighters' walls by their spheres' radius (`v::BR1_WALL_RADIUS`), a thrown fighter kept on its thrower
/// (`v::THROWN_LOCK`), a posed throw victim kept inside the walls (`v::POSED_INSIDE`), Q3DM6's chamfered corners
/// (`v::CHAMFERS`); a v22..v24 team battle and every match before v25 run as before.
/// v26: TEAM ARCADE (engine addition, `v::TEAM_ARCADE`): two players on one team through a
/// ladder of 2 VS 2 battles against CPU pairs, entered by a third row of the team battle's mode choice; every
/// session before it, and every match and team battle of v26 outside the new mode, run as before.
/// v27: the spring chains (hair, ears, tails) as the game steps them (`v::SPRING_REST`, `fight::anim::spring`): a
/// link's rest frame is aimed at its descriptor's direction, its swing is limited there (±p5, not twice that, and not
/// around the animated direction), the direction is not pushed by it every frame, and a link follows its parent link's
/// stepped rotation; every session before v27 keeps the earlier model.
pub const SIM_VERSION: u32 = 27;

/// The first simulation version of each rule (the changelog on `SIM_VERSION`), named for the rule. Two rules of one
/// version are two constants.
pub mod v {
    /// REPLAY on the mode menu (`frontend::screen::mode_rows`).
    pub const REPLAY_ROW: u32 = 2;
    /// the logic's skeleton, spring and clip math through `crate::base::portable` (`Loaded::portable`).
    pub const PORTABLE_MATH: u32 = 3;
    /// the move decision's beast-form end queues its sound and burst before the script's (`duel::fighter`).
    pub const REVERT_BURST: u32 = 4;
    /// the beast revert's sound is SE 7 on every path (`move_script::revert_sound`).
    pub const REVERT_SOUND: u32 = 4;
    /// every character, fourth costume and ST9..STC open unless the session is locked (`frontend::Save::all_open`).
    pub const ALL_OPEN: u32 = 5;
    /// the stage routine's counters are fight state (`stage_anim`, `Match::stage_view`).
    pub const STAGE_STATE: u32 = 6;
    /// the match-end reset of round state 0xD / 0xE (`Round::win_reset`).
    pub const WIN_RESET: u32 = 6;
    /// an effect task spawning while it runs keeps its slot (`hit_effects::Fx::keep_slot`).
    pub const KEEP_SLOT: u32 = 6;
    /// the extra stages ST9..STC's engine-made bounds (`fight::stage::EXTRA_BOUNDS`).
    pub const EXTRA_BOUNDS: u32 = 7;
    /// the sparks' arena is the stage's ring (`Match::new`).
    pub const SPARKS_ARENA: u32 = 7;
    /// NETWORK on the mode menu (`frontend::screen::mode_rows`).
    pub const NETWORK_ROW: u32 = 8;
    /// CHANGE GAME DATA on OPTIONS (`frontend::menus::Menu::data_row`).
    pub const DATA_ROW: u32 = 9;
    /// the extra stage Q3DM17 (`fight::stage::extra::Extra::sim`).
    pub const Q3DM17: u32 = 10;
    /// 7 game units per Quake unit instead of 8 (`Gates::q3_scale`).
    pub const Q3_SCALE: u32 = 11;
    /// the extra stage Q3DM6 (`fight::stage::extra::Extra::sim`).
    pub const Q3DM6: u32 = 12;
    /// the head turn toward the opponent (`move_script::head_turn`).
    pub const HEAD_TURN: u32 = 13;
    /// an extra stage's solid parts (`fight::stage::extra::Extra::solids`, `physics::solids`).
    pub const EXTRA_SOLIDS: u32 = 14;
    /// Imported fighters in a match (the import S7).
    pub const BR1_FIGHTERS: u32 = 15;
    /// The imported game's stages (`fight::stage::br1`, the import S8).
    pub const BR1_STAGES: u32 = 16;
    /// the HUD shows an imported fighter's gauge on its own marks (`fight::br1::bridge::hud_gauge`).
    pub const BR1_HUD_GAUGE: u32 = 16;
    /// the stage sound queued on an imported stage.
    pub const BR1_STAGE_SOUND: u32 = 17;
    /// an imported stage's walls fall into the imported game's own debris, Uriko's stage
    /// runs its wall events.
    pub const BR1_DEBRIS: u32 = 18;
    /// op 0x17 sub 2's case 10 queues its sound 0x5F.
    pub const BR1_CALL_SOUND: u32 = 18;
    /// the front end offers the imported game's fighters and stages (the select's row, the stage select's; the import S9).
    pub const BR1_SELECT: u32 = 19;
    /// an imported fighter's own name on the HUD (before: its proxy's).
    pub const BR1_NAMES: u32 = 19;
    /// the match end cuts an imported loser's key as it cuts a main-game one's (`Match::br1_key_cut`).
    pub const BR1_KEY_CUT: u32 = 19;
    /// The imported game's CPU AI (`fight::br1::ai`, the import S10): a CPU fighter's pad from its AI block, the AI's
    /// random table shuffled at every round start; before, a CPU fighter's pad was 0.
    pub const BR1_AI: u32 = 20;
    /// the select's imported row also in the modes against the CPU (ARCADE, SURVIVAL, WATCH, CUSTOM VS COMPUTER).
    pub const BR1_CPU_MODES: u32 = 20;
    /// An imported movie's fixed length and the epilogue's 360 frames without its picture
    /// (`frontend::movie::BR1_MOVIE_END`).
    pub const BR1_FIXED_MOVIES: u32 = 21;
    /// the team battle: TEAM BATTLE on the mode menu, pads 3 / 4, replay format 2.
    pub const TEAM: u32 = 22;
    /// Imported fighters in the team battle (T5).
    pub const TEAM_BR1: u32 = 23;
    /// the team battle's big stages BIG STAGE and Q3DM6 (T5, `fight::stage::TEAM_BIG_STAGES`).
    pub const TEAM_BIG_STAGES: u32 = 23;
    /// the team pause's CONTROLLER CONFIG. (T6, `session::TeamConfig`; before, the item played its sound and did
    /// nothing).
    pub const TEAM_CONFIG: u32 = 24;
    /// a SUDDEN DEATH time over is a DRAW (T6, `TeamRound::tick`).
    pub const SUDDEN_DEATH_DRAW: u32 = 24;
    /// unbreakable walls round the team battle's enlarged rings (T7, `fight::stage::Stage::team`; no ring-outs).
    pub const TEAM_WALLS: u32 = 25;
    /// the team battle's DRAW GAME when several teams reach the rounds needed at once (T7; no SUDDEN DEATH).
    pub const DRAW_GAME: u32 = 25;
    /// a main-game knock-down launches an imported fighter (T7, `fight::br1::bridge::launch`; a match's mixed
    /// pair too).
    pub const BR1_LAUNCH: u32 = 25;
    /// the team battle's imported cores' walls take their spheres' radius as their margin (T7).
    pub const BR1_WALL_RADIUS: u32 = 25;
    /// a thrown fighter keeps its thrower after the release until it is in a state both games share (T7,
    /// `fight::team::target::thrown`).
    pub const THROWN_LOCK: u32 = 25;
    /// a posed throw victim's own body back inside the walls and chamfers (T7).
    pub const POSED_INSIDE: u32 = 25;
    /// Q3DM6's chamfered corners are walls (T7, `fight::stage::extra::Extra::chamfers`, `physics::chamfers`).
    pub const CHAMFERS: u32 = 25;
    /// TEAM ARCADE: the team battle's co-op ladder against CPU pairs (`frontend::team_arcade`).
    pub const TEAM_ARCADE: u32 = 26;
    /// the spring chains' rest frame and swing limits as the game's chain step (`fight::anim::spring::step`).
    pub const SPRING_REST: u32 = 27;
}

/// The rules of one simulation version (`Gates::of`): a flag per rule of `v` (the same name, lower case), methods for
/// the ones that are not a flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gates {
    /// the version (only for `reaches` and the methods: a rule is asked by its flag)
    v: u32,
    pub replay_row: bool,
    pub portable_math: bool,
    pub revert_burst: bool,
    pub revert_sound: bool,
    pub all_open: bool,
    pub stage_state: bool,
    pub win_reset: bool,
    pub keep_slot: bool,
    pub extra_bounds: bool,
    pub sparks_arena: bool,
    pub network_row: bool,
    pub data_row: bool,
    pub head_turn: bool,
    pub extra_solids: bool,
    pub br1_fighters: bool,
    pub br1_stages: bool,
    pub br1_hud_gauge: bool,
    pub br1_stage_sound: bool,
    pub br1_debris: bool,
    pub br1_call_sound: bool,
    pub br1_select: bool,
    pub br1_names: bool,
    pub br1_key_cut: bool,
    pub br1_ai: bool,
    pub br1_cpu_modes: bool,
    pub br1_fixed_movies: bool,
    pub team: bool,
    pub team_br1: bool,
    pub team_big_stages: bool,
    pub team_config: bool,
    pub sudden_death_draw: bool,
    pub team_walls: bool,
    pub draw_game: bool,
    pub br1_launch: bool,
    pub br1_wall_radius: bool,
    pub thrown_lock: bool,
    pub posed_inside: bool,
    pub chamfers: bool,
    pub team_arcade: bool,
    pub spring_rest: bool,
}

impl Gates {
    /// The rules of simulation version `ver`.
    pub const fn of(ver: u32) -> Gates {
        Gates {
            v: ver,
            replay_row: ver >= v::REPLAY_ROW,
            portable_math: ver >= v::PORTABLE_MATH,
            revert_burst: ver >= v::REVERT_BURST,
            revert_sound: ver >= v::REVERT_SOUND,
            all_open: ver >= v::ALL_OPEN,
            stage_state: ver >= v::STAGE_STATE,
            win_reset: ver >= v::WIN_RESET,
            keep_slot: ver >= v::KEEP_SLOT,
            extra_bounds: ver >= v::EXTRA_BOUNDS,
            sparks_arena: ver >= v::SPARKS_ARENA,
            network_row: ver >= v::NETWORK_ROW,
            data_row: ver >= v::DATA_ROW,
            head_turn: ver >= v::HEAD_TURN,
            extra_solids: ver >= v::EXTRA_SOLIDS,
            br1_fighters: ver >= v::BR1_FIGHTERS,
            br1_stages: ver >= v::BR1_STAGES,
            br1_hud_gauge: ver >= v::BR1_HUD_GAUGE,
            br1_stage_sound: ver >= v::BR1_STAGE_SOUND,
            br1_debris: ver >= v::BR1_DEBRIS,
            br1_call_sound: ver >= v::BR1_CALL_SOUND,
            br1_select: ver >= v::BR1_SELECT,
            br1_names: ver >= v::BR1_NAMES,
            br1_key_cut: ver >= v::BR1_KEY_CUT,
            br1_ai: ver >= v::BR1_AI,
            br1_cpu_modes: ver >= v::BR1_CPU_MODES,
            br1_fixed_movies: ver >= v::BR1_FIXED_MOVIES,
            team: ver >= v::TEAM,
            team_br1: ver >= v::TEAM_BR1,
            team_big_stages: ver >= v::TEAM_BIG_STAGES,
            team_config: ver >= v::TEAM_CONFIG,
            sudden_death_draw: ver >= v::SUDDEN_DEATH_DRAW,
            team_walls: ver >= v::TEAM_WALLS,
            draw_game: ver >= v::DRAW_GAME,
            br1_launch: ver >= v::BR1_LAUNCH,
            br1_wall_radius: ver >= v::BR1_WALL_RADIUS,
            thrown_lock: ver >= v::THROWN_LOCK,
            posed_inside: ver >= v::POSED_INSIDE,
            chamfers: ver >= v::CHAMFERS,
            team_arcade: ver >= v::TEAM_ARCADE,
            spring_rest: ver >= v::SPRING_REST,
        }
    }

    /// Game units per Quake unit on the extra stages: 8 before `v::Q3_SCALE`, then 7 (a Quake III player's height is
    /// a main-game fighter's).
    pub const fn q3_scale(self) -> i32 {
        if self.v < v::Q3_SCALE { 8 } else { 7 }
    }

    /// Whether this version has reached `first`, a table's own first version (`fight::stage::extra::Extra::sim`: one
    /// of `v`'s constants). The logic asks a flag; this is for a table that names its rows' versions.
    pub const fn reaches(self, first: u32) -> bool {
        self.v >= first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What each version 1..=`SIM_VERSION` turns on: the flags of `Gates` that are set from it on (in field order),
    /// `q3_scale` where it changes, and the extra stages it reaches (`stages=`, `stage::extra::STAGES` by `reaches`). A new version appends its row, a new rule goes into its version's row; an old row
    /// never changes (that version's replays would play differently).
    const TABLE: &str = "\
v1: q3_scale=8
v2: replay_row
v3: portable_math
v4: revert_burst revert_sound
v5: all_open
v6: stage_state win_reset keep_slot
v7: extra_bounds sparks_arena
v8: network_row
v9: data_row
v10: stages=Q3DM17
v11: q3_scale=7
v12: stages=Q3DM6
v13: head_turn
v14: extra_solids
v15: br1_fighters
v16: br1_stages br1_hud_gauge
v17: br1_stage_sound
v18: br1_debris br1_call_sound
v19: br1_select br1_names br1_key_cut
v20: br1_ai br1_cpu_modes
v21: br1_fixed_movies
v22: team
v23: team_br1 team_big_stages
v24: team_config sudden_death_draw
v25: team_walls draw_game br1_launch br1_wall_radius thrown_lock posed_inside chamfers
v26: team_arcade
v27: spring_rest
";

    fn flags(g: Gates) -> [(&'static str, bool); 40] {
        [
            ("replay_row", g.replay_row),
            ("portable_math", g.portable_math),
            ("revert_burst", g.revert_burst),
            ("revert_sound", g.revert_sound),
            ("all_open", g.all_open),
            ("stage_state", g.stage_state),
            ("win_reset", g.win_reset),
            ("keep_slot", g.keep_slot),
            ("extra_bounds", g.extra_bounds),
            ("sparks_arena", g.sparks_arena),
            ("network_row", g.network_row),
            ("data_row", g.data_row),
            ("head_turn", g.head_turn),
            ("extra_solids", g.extra_solids),
            ("br1_fighters", g.br1_fighters),
            ("br1_stages", g.br1_stages),
            ("br1_hud_gauge", g.br1_hud_gauge),
            ("br1_stage_sound", g.br1_stage_sound),
            ("br1_debris", g.br1_debris),
            ("br1_call_sound", g.br1_call_sound),
            ("br1_select", g.br1_select),
            ("br1_names", g.br1_names),
            ("br1_key_cut", g.br1_key_cut),
            ("br1_ai", g.br1_ai),
            ("br1_cpu_modes", g.br1_cpu_modes),
            ("br1_fixed_movies", g.br1_fixed_movies),
            ("team", g.team),
            ("team_br1", g.team_br1),
            ("team_big_stages", g.team_big_stages),
            ("team_config", g.team_config),
            ("sudden_death_draw", g.sudden_death_draw),
            ("team_walls", g.team_walls),
            ("draw_game", g.draw_game),
            ("br1_launch", g.br1_launch),
            ("br1_wall_radius", g.br1_wall_radius),
            ("thrown_lock", g.thrown_lock),
            ("posed_inside", g.posed_inside),
            ("chamfers", g.chamfers),
            ("team_arcade", g.team_arcade),
            ("spring_rest", g.spring_rest),
        ]
    }

    /// Version `ver`'s row: what it turns on against version `ver − 1` (a flag never turns off again).
    fn row(ver: u32) -> String {
        let (g, prev) = (Gates::of(ver), Gates::of(ver - 1));
        let mut out = format!("v{ver}:");
        for ((name, on), (_, was)) in flags(g).into_iter().zip(flags(prev)) {
            assert!(on || !was, "v{ver}: {name} turned off");
            if on && !was {
                out += &format!(" {name}");
            }
        }
        if ver == 1 || g.q3_scale() != prev.q3_scale() {
            out += &format!(" q3_scale={}", g.q3_scale());
        }
        let stages: Vec<&str> = crate::fight::stage::extra::STAGES.iter().filter(|e| g.reaches(e.sim) && !prev.reaches(e.sim)).map(|e| e.code).collect();
        if !stages.is_empty() {
            out += &format!(" stages={}", stages.join(","));
        }
        out
    }

    /// Every version's rules against `TABLE`.
    #[test]
    fn gates_by_version() {
        let want: Vec<&str> = TABLE.lines().collect();
        let got: Vec<String> = (1..=SIM_VERSION).map(row).collect();
        assert_eq!(want.len(), got.len(), "one row per version 1..=SIM_VERSION: append the new version's row\n{}", got.join("\n"));
        for (w, g) in want.iter().zip(&got) {
            assert_eq!(w, g, "a version's rules changed: an old row never changes, a new rule goes into its version's row");
        }
    }

    /// A table's own versions (`reaches`: the extra stages).
    #[test]
    fn reaches_is_from_its_version_on() {
        assert!(!Gates::of(v::Q3DM6 - 1).reaches(v::Q3DM6) && Gates::of(v::Q3DM6).reaches(v::Q3DM6));
    }
}
