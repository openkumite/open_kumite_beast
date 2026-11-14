# Architecture

How `open_kumite_beast` is laid out, how a frame flows through it and where to find things. Read this first; then
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the build, the checks and the recipes for new work. Terms (OT, CLUT, GTE,
`o_XX`, `M.9F`, `rebind` …) are in [`docs/glossary.md`](docs/glossary.md). Paths below are relative to `src/`
unless they start with `docs/`, `tests/`, `fuzz/` or a file at the repository root.

## What this is

A Rust + wgpu engine that runs a PS1-era 3D fighting game from data the player converts from their own original disc
(the converters are separate tools), for macOS, Windows, Linux, the web (wasm32) and Android
([`docs/android.md`](docs/android.md)). It is not an emulator: the game logic is written in Rust with integer arithmetic
and a fixed update order, so a match plays the same frame for frame as on the console; the engine around it (rendering,
audio, input, loading, platform) is ordinary modern code.

- **Fidelity rule.** At the native settings (4:3, RENDER QUALITY 1×) the engine must look and play exactly like the
  game on the console. Anything the console game does not have (netplay, replays, the debug tools, WIDE aspect, the
  sprite atlas, the GRAPHICS menu) is an **engine addition** and says so in its module header or comment.
- **Comments describe behaviour.** Comments say what the code does, the data it reads and the invariants it keeps;
  the checks that pin the behaviour down are the unit tests, the replay fixtures and the screenshot baseline
  ([`CONTRIBUTING.md`](CONTRIBUTING.md)).
- **Assets** come from the converted export (the game data folder beside the save, `platform/game_data.rs`, checked
  at start by `assets/manifest.rs`; or `--assets DIR`, e.g. `../okb_export`, made by the converter `ok_br2_converter`:
  the setup screen, or its CLI `okconv-br2 convert IMAGE OUT`): glTF models, PNG / TIM textures, CD-XA music (`.xa`;
  older exports MP3), and the game's own files in `engine/` (`*.ANI`, `*.ACT`, `CMN.ACT`; the game's static tables in
  `tables.okt`). Nothing in
  this repo contains game data. The engine's extra stages Q3DM17 and Q3DM6 read Quake III's maps from the player's own
  copy, converted from its `.pk3` files (the setup screen's QUAKE III ARENA MAPS) into
  `game_data/q3/` (the browser: an OPFS folder `q3-<n>`), with its own `manifest.json` and quick check; older game
  data's `extras/q3dm17/` and `extras/q3dm6/` are still read when the install lacks a map (`assets/extras.rs`,
  [`docs/extra_stage_q3dm17.md`](docs/extra_stage_q3dm17.md), [`docs/extra_stage_q3dm6.md`](docs/extra_stage_q3dm6.md)).
- **The converter** (the setup screen's CONVERT DISC IMAGE, feature `converter`) is the crate `ok_br2_converter`, its
  own git repository, a git dependency on a tagged release (`github.com/openkumite/ok_br2_converter`, public; see
  `CONTRIBUTING.md`, Setup). The same feature brings the crates `ok_br1_converter` (the imported game's disc) and
  `ok_q3_converter` (Quake III Arena's maps for the extra stages), each its own repository, used the same way; a
  converter change reaches the engine as a new tag and a new `tag = ` in `Cargo.toml`. `app/setup/imports.rs` opens their inputs
  through `ok_br2_converter`'s disc image and file readers.
- **The imported game's disc** (optional, engine addition): its converted data lives in `game_data/br1/` (the browser: an
  OPFS folder `br1-<n>`), with its own `manifest.json`, quick check (`assets/manifest.rs` `quick_check_br1`) and
  setup pages (ADD / REPLACE / REMOVE BLOODY ROAR 1 DISC); the main game data's record and check never look at it. The
  imported fighters' file reader (`formats/br1_anm.rs`; their tables `formats/tables.rs`), static data, pose code, state-script interpreter and
  fight frame (input, throws, move decision, physics, hits: `fight/br1/`) are checked against recorded reference runs
  by `cliptest br1`, `br1test` and the tests; from simulation v15 a match can hold them (MIT FOX GRE URI; `fight/sim/br1.rs`, the
  bridge `fight/br1/bridge.rs`: their moves inside the game's rules),
  reached through the `sim` tool, the direct fight and (simulation v19) the menus: the select's imported row in VS
  and PRACTICE, the stage select's imported stages (`frontend/select.rs`, their pictures `assets/br1_art.rs`),
  drawn in their own pose with their costumes, chains and sounds (`present/br1_fighter.rs`); from simulation v16 the
  imported game's stages too (`fight/stage/br1.rs`, their background `present/br1_stage_bg.rs`, props `present/br1_props.rs`),
  their effects (`present/br1_fx.rs`); from simulation v20 their CPU AI (`fight/br1/ai.rs`) in the CPU modes, ARCADE's
  extra boss the imported game's Uriko and their imported endings (`frontend/progress.rs`).

## Layers

The source is one crate with one top-level folder per layer or domain. A module may use its own layer and the layers
below it, never one above; `tests/layering.rs` checks every `crate::…` path in `src/**` against these rules on every
`cargo test` (and so on every push), with an empty exception list.

```
L5  app/                      the winit loop, input devices, debug tools, replay UI, netplay UI, capture
L4  present/                  the state → draw lists and GPU objects (reads the state, never writes it)
L3  render/  audio/  net/     engine services: wgpu, the mixer + cpal, netplay transports + lockstep
L2  session/ → frontend/ → fight/        DETERMINISTIC LOGIC (60 Hz, integer, serialisable)
L1  assets/  output/  formats/           export access + roster; draw / sound vocabulary; file formats
L0  base/  platform/                     portable math, rand, hashes, serde helpers; clock, storage, support
```

The rules (`tests/layering.rs`):

1. Only own or lower layers. Inside L2 the order is session → frontend → fight; `frontend` may also use
   `session::start` (`SessionStart`, `FileCache`, `SIM_VERSION` …), which names no frontend item but `memcard::Fault`.
2. L2 never names `render`, `present`, `audio`, `net`, `platform`, `app`, nor wgpu, winit, cpal, gilrs, web-sys or
   `std::fs`. The listed exception: the debug-only file output of `session/hash.rs` (`OKB_STATEHASH`) and
   `session/rollback.rs`. Env reads and hash maps in L2 are listed in the test's report.
3. `render` and `audio` use only L0–L1: the renderer takes draw lists and GPU uploads and knows no game type.
4. `net` uses the session's API (`FrameInput`, the state hashes, `SessionStart`).
5. Float math that feeds L2 state goes through `base::portable` ([`docs/determinism.md`](docs/determinism.md)).

`cargo test --release --test layering -- --nocapture` prints the edge report. A new file must be added to the test's
`FILES` table (the test fails on a file it does not know); one exception: `frontend/data.rs` (`FrontData`) is counted
as `assets` there, where it belongs by layer.

## Module map

Each top folder, its purpose and its key files. Every file's own `//!` header says more.

| Folder | Layer | Purpose | Key files |
|---|---|---|---|
| `base/` | L0 | what every layer may use | `portable.rs` (the logic's bit-exact f64 math), `libc_rand.rs` (the PS1 libc `rand`), `hash.rs` (FNV-1a, CRC-32), `serde_util.rs`, `decode_limit.rs` (bounded decoding of untrusted bytes), `trace.rs` (the logic's debug-trace mute; `flag`, the debug-trace switches read once), `build.rs` (the build stamp) |
| `platform/` | L0 | services of the machine | `clock.rs` (`now`, `perf_now`, `clock_seed`), `storage.rs` (save / prefs / replay files, localStorage), `game_data.rs` (the game data folder, the install's swap and recovery; the optional installs' folders `Dirs::sub` (`br1`, `q3`) carried over a new install (`CARRIED`), the browser's slots `web::BR2` / `web::BR1` / `web::Q3`), `support.rs` (logger, crash and support reports, `HELP`), `android.rs` (the activity, app storage, forwarded input) |
| `formats/` | L1 | readers of the game's and the export's files | `container.rs` (`*.DAT` + LZ), `act_ani.rs`, `br1_anm.rs` (the imported game's fighter file), `vram.rs` (TIM, the effects' VRAM; where the imported game's effect pages go: `BR1_PAGES`, `BR1_CLUT_FROM` / `BR1_CLUT_TO`), `gltf_model.rs`, `gltf_pose.rs`, `movie.rs` (BS / MDEC), `vab.rs` (sound banks), `music.rs` (`Music`, MP3), `xa.rs` (CD-XA ADPCM), `pcm.rs` (the imported game's CD-DA track, BR2PCM v1), `q3_map.rs` (a Quake III map from ok_q3_converter and its sky shader: the extra stages Q3DM17, Q3DM6), `tables.rs` (the converted static tables `engine/tables.okt`, by name: the main game data's format 3, the imported game's disc data's 4) |
| `assets/` | L1 | access to the export, shared tables | `source.rs` (`Assets`: disk or HTTP), `manifest.rs` (the install record, `Verdict`, the quick check, the required set; the imported game's install's `Br1Verdict` and `quick_check_br1`), `roster.rs` (character codes and names; the select's imported cells), `text.rs` (the games' text by key from their converted text tables, `text::get`; placeholders without them), `br1_art.rs` (the imported game's select portraits as the front end's pictures), `extras.rs` (the extra stages' local files, Q3DM17 and Q3DM6: the Quake III install's `Q3Verdict` and quick check, `source` (the install or the older `extras/`), `present`) |
| `output/` | L1 | what logic and presentation hand the renderer and the mixer | `prim.rs` (512×480 HUD-space quads), `font.rs` (the text renderer), `layout.rs` (window layout, 4:3 / WIDE), `sprite.rs`, `psx_quad.rs`, `gte.rs` (the effects' projection), `backdrop.rs`, `sound.rs` (`Cmd`, `Queue`), `tiny_font.rs` (the engine's bitmap font of the setup screen) |
| `fight/` | L2 | one match: fighters, move scripts, hits, rounds, camera, effects, stage | `sim/` (`Match`; `frame.rs` = the frame order), `duel/` (what a match and the team battle share, by fighter index: `mod.rs` a fighter's input and step against its opponent, `hits.rs` the hit pair functions, `setup.rs` a fresh fighter, the effect VM, the decoded state's checks, `frame.rs` the frame steps both frames run the same way, `br1.rs` the imported parts and the bridge helpers both share), `version.rs` (`SIM_VERSION` and its changelog, one constant per rule `v::*`, `Gates`: a version's rules as flags), `side.rs` (`Fid`, `Slot`, `Bank`: a fighter's slot and bank, bank 1's resources; `PerSlot`), `scene.rs` (`SceneRef`: the world as `present` draws it), `fighter.rs`, `move_script/`, `move_decision/`, `input.rs`, `physics.rs`, `hit.rs`, `throw.rs`, `round.rs`, `camera.rs`, `ai.rs`, `hud.rs`, `pause.rs`, `stage/` (`mod.rs` the ring, outer area and walls; `anim.rs` the stage routine; `extra.rs` the engine's extra stages Q3DM17, Q3DM6: their table; `br1.rs` the imported game's stages: their table and records), `anim/`, `br1/` (the imported game's fighters: static data, pose, `object.rs` the fighter block, `script/` the state scripts, `frame.rs` the fight frame with `input.rs`, `throw.rs`, `decision.rs`, `physics.rs`, `hit.rs`, `reference.rs` / `reference_frame.rs` the reference runners' cores, `init.rs` the round-start reset, `ai.rs` the CPU AI, `bridge.rs` the face, the hit exchange with the game's rules and the throws across the engines), `sim/br1.rs` (a match's imported fighters: their frame, the mixed pair's, the throws across the engines), `team/` (the team battle, engine addition: `TeamFight`, four fighters beside `Match`; `frame.rs` its frame, `target.rs` each fighter's target, `round.rs` `TeamRound`, `arcade.rs` TEAM ARCADE's `ArcadeRules`, `hud.rs` `TeamHud`, `br1.rs` its imported fighters: `TeamBr1`, each core on a match's mixed path against its target's view, the throws across the engines keyed by thrower), `fx/` (`host.rs` `FxHost`: the parts of a match or a team battle the effect code runs on), `fight_data.rs` (`Loaded`), `rules.rs`, `input_script.rs` |
| `fight/fx/` | L2 | the fight's effects | `hit_effects/` (the hit code's tasks), `script_vm/` (the effect bytecode VM), `script_link.rs` (VM ↔ fight), `native_tasks.rs`, `move_effects.rs`, `effect_models.rs` (ED1C4 3-D models) |
| `frontend/` | L2 | boot, title, menus, selects, VS, story, ranking, memory card | `mod.rs` (`Front`), `screen.rs` (`Screen`, `Mode`, `Event`), `tick.rs` (a frame), `draw.rs`, `check.rs`, `title.rs`, `select.rs`, `vs.rs`, `team.rs` (the team battle's screens: mode choice, 4-player select, stage, loading, result), `team_arcade.rs` (TEAM ARCADE's ladder, ending and clear card), `progress.rs`, `menus/`, `memcard.rs`, `options.rs` (`Settings`, `Prefs`, `Card`), `story_flow.rs`, `story_screens.rs`, `ranking.rs`, `movie.rs`, `resident.rs`, `serial.rs`, `data.rs` (`FrontData`) |
| `session/` | L2 | the whole logic state and its one step; snapshots, replays, determinism checks | `state.rs` (`GameState`, `FrameInput`, `Cheat`, `Out`, `Request`, `Assets`), `step.rs` (`step`), `start.rs` (`SessionStart`; `SIM_VERSION` re-exported from `fight::version`), `fight.rs` (`Fight`), `snapshot.rs` (`STATE_VERSION`, save states, `rebind`), `hash.rs`, `replay/` (`.okbr`), `rollback.rs`, `debug_history.rs`, `determinism_trace.rs`, `fixtures/` |
| `render/` | L3 | wgpu | `mod.rs` (`Renderer`), `pipelines.rs` (`Renderer::new`), `frame.rs` (a frame's passes), `models.rs`, `instances.rs`, `targets.rs`, `atlas_cache.rs`, `gpu_timer.rs`, `offscreen.rs` (the windowless renderer and its frame readback, `replay --video`), `lightmapped.rs` (lightmapped meshes, sky boxes and cloud layers: the extra stages' maps and skies), `shaders/*.wgsl` |
| `audio/` | L3 | sound output | `mod.rs` (`Audio`: banks, cpal, `apply`), `mixer.rs` (the SPU stand-in) |
| `net/` | L3 | netplay ([`docs/netplay.md`](docs/netplay.md)) | `mod.rs` (`Netplay`: lockstep), `proto.rs` (packets, `PROTO`), `handshake.rs`, `connect.rs`, `link.rs`, `udp.rs`, `webrtc.rs` |
| `present/` | L4 | the state on screen | `fight_scene/` (`mod.rs`: `Game`, the team battle's `TeamGame`, `FightView`; `scene.rs` (a frame; `draw_world` its world from a `SceneRef`, shared with the team battle), `team.rs` (the team battle: `draw_team`, the target rings, the markers), `stage_draw.rs`, `stage_cull.rs`, `arena.rs` (the extra stages' maps), `debug_hud.rs`), `front_vram.rs` (`VramView`), `fighter_lighting.rs`, `backdrop.rs`, `drive_grid.rs`, `br1_fighter.rs` (an imported fighter: its pose from its core, toggles, chains), `br1_stage_bg.rs` (an imported stage's background), `br1_props.rs` (its props), `br1_fx.rs` (the imported game's effects) |
| `app/` | L5 | the program: window, loop, devices, tools | `mod.rs` (`App`, `run`, `web_main`), `events.rs` (winit), `stepping.rs` (a step), `requests.rs`, `draw.rs`, `keys.rs`, `debug_env.rs`, `devices.rs`, `gamepads.rs` + `pad_layout.rs` (gamepads, [`docs/gamepads.md`](docs/gamepads.md)), `tools.rs`, `replays.rs`, `replay_store.rs`, `replay_menu.rs`, `playback.rs`, `netplay.rs`, `net_menu.rs`, `capture.rs`, `overlay.rs`, `perf.rs`, `web_files.rs`, `android.rs` + `touch.rs` (Android: input, lifecycle, touch controls), `video.rs` + `video_ffmpeg.rs` (`replay --video`: a replay drawn and mixed without a window into a video), `setup/` (the setup screen when the game data is missing or unusable: `screen.rs` the pages, `draw.rs` the picture, `pickers.rs` the scan and BROWSE, `job.rs` the check and the copy, `convert.rs` CONVERT DISC IMAGE through `ok_br2_converter`, `q3.rs` QUAKE III ARENA MAPS (the `.pk3` files converted through `imports::q3`, `ok_q3_converter`), `imports.rs` (opens the imported game's disc image and the Quake III paks for `ok_br1_converter` / `ok_q3_converter`), `plan.rs` what a copy takes, `web.rs` the browser's picker and copy, `worker.rs` the browser's conversion worker) |
| `bin/` | — | headless tools ([`CONTRIBUTING.md`](CONTRIBUTING.md#tools)) | `sim.rs`, `fetest.rs`, `replay.rs`, `nettest.rs`, `fxvmtest.rs`, `cliptest.rs`, `br1test.rs`, `mdectest.rs`, `sndtest.rs` |

`lib.rs` declares the folders (all `pub` but `app`) and re-exports the app's entry points and the clock; `main.rs`
is the desktop binary `okbeast` (its `--help`).

### The imported game's files

The import (engine addition, stages S0..S11) in every layer; each file's header says what it does.

| File | Layer | What |
|---|---|---|
| `formats/br1_anm.rs` | L1 | the fighter file `XX.ANM` / `XX31.DAT` and the clip blocks of `CMN.ACT` |
| `assets/br1_art.rs` | L1 | the select portraits (`CMN/SELE00.DAT`, `SELE01.DAT`) as the front end's pictures |
| `fight/br1/mod.rs` | L2 | the folder's map: a fighter's static data, pose, scripts and frame |
| `fight/br1/res.rs` | L2 | a fighter's static data: its ANM, CMN.ACT, the static tables (`tables.okt`, the shared clips), both forms' skeletons |
| `fight/br1/address_map.rs` | L2 | the address map of the imported game's data: its clip pointers into the program image (`image_offset`) |
| `fight/br1/object.rs` | L2 | the fighter block F, the script state S and the RAM they point at (serialised) |
| `fight/br1/pose.rs` | L2 | clip playback, the pose sampler, node matrices, the world walk |
| `fight/br1/script/mod.rs`, `script/ops.rs` | L2 | the state-script interpreter and its 42 ops |
| `fight/br1/input.rs` | L2 | the pad word, button timing, chain programs, the command matcher |
| `fight/br1/decision.rs` | L2 | the move decision |
| `fight/br1/physics.rs` | L2 | the ring and ground, velocities, gravity and landing, the automatic turn, the push-apart |
| `fight/br1/hit.rs` | L2 | hits, guard and their reactions |
| `fight/br1/throw.rs` | L2 | throw resolution and the throw test |
| `fight/br1/frame.rs` | L2 | the fight frame: both inputs, the throws, each fighter's frame, the pair's movement |
| `fight/br1/init.rs` | L2 | the round-start reset |
| `fight/br1/ai.rs` | L2 | the CPU AI: the block's init, the driver, its VM |
| `fight/br1/bridge.rs` | L2 | `Br1Match`, an imported fighter's face in the match, the hit and health exchange with the game's rules, the throws across the engines, the HUD gauge |
| `fight/br1/reference.rs`, `reference_frame.rs` | L2 | the reference runners' cores (debug only: `br1test`, `cliptest br1`) |
| `fight/br1/frame_tests.rs`, `script/tests.rs` | L2 | tests of the frame and the interpreter (synthetic and against recorded reference runs) |
| `fight/stage/br1.rs` | L2 | the imported stages: their table, rings, walls, records, stage counters (G.4A), Uriko's walls, music |
| `fight/sim/br1.rs` | L2 | a match's imported fighters: their frame, the mixed pair's, the round starts, the AI's place, the throws across the engines |
| `fight/sim/br1_tests.rs` | L2 | the move sheets inside the game's rules, the round trips, the stages' and the AI's checks |
| `present/br1_fighter.rs` | L4 | a fighter on screen: the glTF posed from the core, part toggles, spring chains |
| `present/br1_fx.rs` | L4 | the effects: particles, emitters, the effect tasks, the effect scripts, the shake (presentation only) |
| `present/br1_props.rs` | L4 | an imported stage's props (ZIN's slider, VAK's wanderers) |
| `present/br1_stage_bg.rs` | L4 | an imported stage's background strip and colour bands |
| `bin/br1test.rs` | — | the reference runner against recorded reference runs (`br1test BR1_DIR init|ai …`) |

The front end's imported parts live in the shared files (`frontend/select.rs`, `vs.rs`, `progress.rs`, `movie.rs`,
`story_screens.rs`; `assets/roster.rs`, `assets/manifest.rs`); the fixtures are `session/fixtures/*br1*`,
`fight/br1/fixtures/`, `present/fixtures/br1_drop.txt`.

### The team battle's files

The team battle (engine addition, stages T0..T8; four fighters, 2 VS 2 / FREE FOR ALL). How it fits beside the match:

- **`TeamFight` beside `Match`.** The team battle is a world of its own (`GameState::team: Option<Box<TeamFight>>`);
  `Match` is not widened. Why: the replay fixtures hash `Match`'s postcard bytes after every step, so any change of
  its layout would break every recorded trace. The per-fighter routines are shared instead: they take the fighter and
  its opponent by index (`fight/duel/mod.rs`, `move_script::run(fs, me, opp)`, the hit pair functions), and the few
  leaf routines that still take a `[Fighter; 2]` run on a pair view (the fighter and its target swapped into a scratch
  pair and back).
- **Slot and bank.** Fighter `k` plays as player `k & 1` (its slot: what every per-fighter routine sees in
  `Fighter::player`) with copy `k >> 1` (its bank) of the game's per-player resources. Bank 1 is the engine's copy
  for P3 / P4: the effect VM's groups 5 / 6 over EffTab 100..139 (`script_link::fx_group`, `Vm::groups_ext`), the pad
  words after the four fighter blocks (`script_link::pad_addr`), VRAM bank 1 (CLUT bit 15), sound slots 7 / 8, the AI's
  random index + 64. `fight/side.rs` names them: `Fid::slot` / `bank`, and `Bank` holds bank 1's table (`clut_bit`,
  `br2_sound`, `br1_sound`, `ai_offset`, `vm_group`, `eft_base`).
- **The `PlayerExt` swap.** The per-player `World` state that two fighters of one slot would share (`World::p4b18`,
  the palette fades; `World::hud_flash`) is kept per fighter in `TeamFight::ext: [PlayerExt; 4]` and copied into the
  `World`'s slot `k & 1` before fighter `k`'s steps and back after them (`TeamFight::in_slot`). Those `World` arrays
  are `PerSlot`s, indexed by a `Slot` only. The effect tasks' per-side state of fighters
  2 / 3 (`native_tasks::SideExt`: `aura`, `prev_gauge`, `f6c`) is `TeamFight::fxn_ext`, lent as `FxHost::ext`; the
  colour flash and back colour are arrays by fighter on `TeamFight` (`flash_task`, `back_colour`).
- **Draw from the scene.** `present` draws a fight's world through `fight::scene::SceneRef` (`Match::scene`,
  `TeamFight::scene`), which lends the stage, the darkening and the fades but not the `World`. Per-fighter state is
  read from it (`SceneRef::fade(i)` / `body(i)`: a team battle's fighter its own `PlayerExt` record, T8), never from
  the `World`'s slot, which holds whichever of its two fighters stepped last.

| File | Layer | What |
|---|---|---|
| `fight/team/mod.rs` | L2 | `TeamFight` (serialised in `GameState::team`), `TeamMode`, `TeamOutcome`, `PlayerExt`, the decoded state's `check`, `scene` and `fx_host` |
| `fight/team/frame.rs` | L2 | the frame: the pause, targets, input, throws, the fighter steps against their targets, physics, hits of every enemy pair, push-apart, stage, round flow, effects, the camera over all four, sound |
| `fight/team/target.rs` | L2 | each fighter's target: the throw lock (v25: kept after the release, `thrown`), L2's cycle and lock, the nearest enemy with hysteresis |
| `fight/team/round.rs` | L2 | `TeamRound`: the round states, KO'd fighters out, time over by health %, DRAW / D.K.O., SUDDEN DEATH (before v25) / DRAW GAME (v25); TEAM ARCADE (v26): YOU WIN / YOU LOSE, 0xF CONTINUE? (`fight::round::countdown`), 0x10 GAME OVER |
| `fight/team/arcade.rs` | L2 | `ArcadeRules` (TEAM ARCADE, v26, `TeamFight::arcade`): the humans' team, the CPU level of each fighter, the countdown's and GAME OVER's counters |
| `fight/team/hud.rs` | L2 | `TeamHud`: four of the match's bars at their size (P1 / P2 at the top, P3 / P4 at the bottom), names in team colours, round lamps |
| `fight/team/br1.rs` | L2 | `TeamBr1`: the imported fighters' cores on a match's mixed path against their targets' views, the throws across the engines keyed by thrower (`bridge::CrossThrows`) |
| `fight/team/tests.rs`, `br1_tests.rs` | L2 | targets, rounds, throws, the state-hash fixtures `session/fixtures/team_*_hashes.txt` |
| `fight/team/watch.rs` | L2 | `Watch` (T7): the check after each frame, nobody stuck in the air or through a wall (`sim TEAMCHECK`, the tests) |
| `fight/fx/host.rs` | L2 | `FxHost`: the effect code on a match or a team battle (the fighters by index, each one's opponent, fighters 2 / 3's `SideExt` as `ext`) |
| `fight/fx/script_link.rs` | L2 | `fx_group` (fighters 2 / 3's script and emitter groups 5 / 6 over EffTab 100..139), `pad_addr` (their pad words at M+0x508, after the four fighter blocks) |
| `fight/fx/script_vm/mod.rs` | L2 | `Vm::groups_ext` (the script groups 5 / 6, static, put back by `rebind`), `Vm::bank_of` (a script of groups 5 / 6 draws from VRAM bank 1) |
| `fight/fx/native_tasks.rs` | L2 | `SideExt` (fighters 2 / 3's `aura`, `prev_gauge`, `f6c`; `TeamFight::fxn_ext`) |
| `fight/scene.rs` | L2 | `SceneRef` (the world as `present` draws it: the stage, the darkening, `fade(i)` / `body(i)`, a team battle's fighter's own fade, T8; no `World`), `Br1Scene` |
| `fight/side.rs` | L2 | `Fid` (`slot`, `bank`, `ai_index`), `Slot`, `Bank` (bank 1's resources), `bank_bit`, `PerSlot` (`World::p4b18` / `hud_flash`, `native_tasks::State`'s per-side arrays) |
| `fight/version.rs` | L2 | the team battle's versions among the others: `v::TEAM` (22), `TEAM_BR1` / `TEAM_BIG_STAGES` (23), `TEAM_CONFIG` / `SUDDEN_DEATH_DRAW` (24), `TEAM_WALLS`, `DRAW_GAME`, `BR1_LAUNCH`, `BR1_WALL_RADIUS`, `THROWN_LOCK`, `POSED_INSIDE`, `CHAMFERS` (25); `Gates` |
| `fight/duel/` | L2 | the routines the match and the team battle share by fighter index: `mod.rs` (`input`, `fighter`, `skeleton`, `Parts`, `FLASH`, `timers`), `hits.rs` (`probe`, `land`, `combo_drop` / `combo_reset`, `hit_fx`), `setup.rs` (`fresh_fighter`, `effect_vm`, `rings`, `check_world`, `fighter_fits`), `frame.rs` (the frame kernels both frames call at their place in their own order: `sound_tick`, `colour_flash`, `stage_fade_step` / `fade_step`, `skeleton_parts` / `skeleton_tail`, `stage_step`, `br1_stage_debris`), `br1.rs` (`think`, `proxy_damage`, `G98`; `Br1Host` with `requests` / `stage_sounds`; `rings`, `walls_in` / `walls_out`, `ctx`; the cross-engine proxies `pose_br1_ghost`, `br2_ghost`, `proxy_sees`, `br2_proxy_damage`, `puppet_from`, `release`; the hit kernels `probe_on_core`, `probe_core`, `land_on_face`) |
| `fight/stage/mod.rs` | L2 | `TEAM_STAGES`, `TEAM_BIG_STAGES`, `team_stage`, `TEAM_SCALE_PCT`, `Stage::team` (the ring ×1.5; from v25 the walls kept) |
| `fight/stage/extra.rs`, `fight/physics.rs` | L2 | (T7, v25) Q3DM6's chamfered corners as walls: `Extra::chamfer_q` / `chamfers`, `physics::chamfers`, `physics::inside` |
| `fight/br1/bridge.rs` | L2 | `CrossThrows` (the throws across the engines by thrower), `launch` (v25, `version::v::BR1_LAUNCH`: a main-game knock-down launches an imported core, in a match too) |
| `fight/fight_data.rs` | L2 | `load_team` / `load_team_with` (four fighters' data, VRAM bank 1, sound slots 7 / 8), `TeamData` (`Loaded::team`: costumes, `stage_scale`, `vram1`) |
| `frontend/screen.rs` | L2 | `TeamSpec` (the mode, four fighters, costumes, stage, CPUs: the loader's input and the replay header's), `Screen::Team*` (with TEAM ARCADE's clear card `Screen::TeamArcadeClear`), `Mode::TeamArcade`, `Event::LoadTeam` |
| `frontend/team.rs` | L2 | the screens: mode choice, the 4-player select (with the imported row), the stage panel, loading, result |
| `frontend/team_arcade.rs` | L2 | TEAM ARCADE (v26): `TeamLadder` (the run's CPU pairs and stages, its draw, `check`, the rank column, the partner's pad and the pads by slot, the rounds won / lost, the epilogues shown), `TEAM_ARCADE_MIN_STAGES` / `TEAM_ARCADE_MAX_STAGES` (5 / 6), `Front::begin_team_arcade`, the arcade select's parts (`arcade_sel`, `arcade_join`, `team_arcade_spec`), `team_arcade_levels` (the battle's `ArcadeRules::levels`), `team_arcade_over` (a battle's end: the next stage, a continue back to the arcade select, GAME OVER to the mode menu, the ending after the last stage), the ending (`team_arcade_next_ending`: the humans' epilogues, then the clear card `Screen::TeamArcadeClear`, then CONGRATULATIONS), `team_arcade_info` (the replay header's), `debug_team_arcade` (`OKB_TEAM_ARCADE`) |
| `session/step.rs` | L2 | `start_team`, `team_frame` (four pads, the pause's `TeamConfig`; in a TEAM ARCADE run the pads by slot and the play time), `end_team`; replay FORMAT 2 in `session/replay/inputs.rs` |
| `session/replay/mod.rs` | L2 | (TEAM ARCADE) `Header::team_arcade` (`TeamArcadeInfo`: the stage, the ladder's length, the continues), checked by `check_header`; `Entry::TeamArcade` (`OKB_TEAM_ARCADE`); the ladder's checkpoint values in `session/replay/sync.rs` |
| `session/state.rs` | L2 | `GameState::team` / `team_spec` / `team_config`, `TeamConfig` (the pause's button-mapping screen), `FrameInput::ext` / `forced_ext` (pads 3 / 4), `Request::LoadTeam` / `TeamOver` |
| `present/fight_scene/mod.rs` | L4 | `TeamGame` (the state's `TeamFight`, its data, its view, the open config menu; `status` for the window title) |
| `present/fight_scene/team.rs` | L4 | `draw_team`: the world (`scene.rs` `draw_world`), target rings, P1..P4 markers, the HUD, the pause and its CONTROLLER CONFIG. screen |
| `present/fight_scene/scene.rs` | L4 | `draw_world` from a `SceneRef` (the fighters' fades and BKOUT by `SceneRef::fade`), shared by a match and the team battle |
| `render/shaders/psx.wgsl` | L3 | VRAM bank 1: a quad whose CLUT word has bit 15 set samples rows 512..1023 of the VRAM texture (`Renderer::upload_vram_bank1`) |
| `present/fight_scene/arena.rs` | L4 | (T6) `courtyard_clip`: on Q3DM6 the map between the zoomed-out camera and the ring is not drawn (`render/shaders/lightmapped.wgsl`) |
| `fuzz/fuzz_targets/team_state.rs` | — | a decoded `TeamFight` / `TeamRound` (never built here: see CONTRIBUTING's fuzzing) |

The app's parts: `app/debug_env.rs` (`OKB_TEAM`, `OKB_TEAM_ARCADE`), `app/replay_menu.rs` (the list line of a team battle and of a TEAM ARCADE battle, `team_arcade_label`), `app/draw.rs` (the window title's status `TeamGame::status`, the F7
screenshot), `app/devices.rs` (`PadSlots` of four), `app/video.rs` (`replay --video` of a team replay); headless:
`sim TEAM=` (with `SHOTS`, `SOUNDWAV`; `ARCADE=` a TEAM ARCADE battle), `fetest` (`FESCRIPT3` / `4`; `WINS` `C` / `G` a TEAM ARCADE continue / GAME OVER).

## The frame, step by step

The app runs the session at a fixed 60 Hz, independent of the display rate; drawing reads the state the steps left.

1. **Wake-up.** winit calls `App::about_to_wait` (`app/events.rs`), which asks for a redraw (or waits for FRAME
   CAP's next slot, `app/stepping.rs` `next_slot`). The redraw arrives as `WindowEvent::RedrawRequested` in
   `App::window_event` (`app/events.rs`).
2. **How many steps.** The redraw adds the elapsed time to an accumulator and runs one step per 1/60 s in it (with
   `OKB_LOCKSTEP=1` exactly one per presented frame; netplay may let one slot pass; a replay's playback, a skip and
   the debug fast-forward change the count). The pads are read once per redraw (`App::pads` in `app/stepping.rs`; keyboard mapping in `app/devices.rs`, gamepads in
   `app/gamepads.rs`, [`docs/gamepads.md`](docs/gamepads.md)).
3. **The step's input.** `App::step` (`app/stepping.rs`) builds the `FrameInput` (`frame_input`: both pads, the
   scripted pads of `OKB_SCRIPT` / `OKB_FESCRIPT`, and the flags `LOADED`, `FETCHED`, `CARD_STORED` /
   `CARD_FAILED` for what the app finished since the last step) and the `Cheat`s (debug keys, `env_cheats`). In a
   playback both come from the replay file (`app/replays.rs`), in netplay from the lockstep (`app/netplay.rs`).
4. **The session step.** `session::step(&mut GameState, &Assets, FrameInput, &[Cheat], &mut Out)`
   (`session/step.rs`): the completed loads and the card store (`inputs`), the key cheats (`cheat`), then `frame`:
   - in a fight: `Fight::demo_step`, `config_step`, `script_step`, `challenger_step` (`session/fight.rs`), then
     `Fight::tick` → `Match::tick` (`fight/sim/frame.rs`), which runs `frame_step` in the game's order (its
     header has the table: input, throws, decision and move scripts, physics, hits, stage, round flow, effect lists,
     camera, sound) and from simulation v6 `stage_anim_step`; a finished match goes back to the front end
     (`end_match`);
   - in a team battle (engine addition, `GameState::team`):
     `team_frame` (`session/step.rs`: the four pads, pads 3 / 4 from `FrameInput::ext`; the pause's CONTROLLER CONFIG.
     screen `TeamConfig` from simulation v24) → `TeamFight::tick` (`fight/team/frame.rs`): the pause (`pause_step`),
     then `frame_step` with the match's steps per fighter against its target (`target.rs` first, then input, throws,
     the fighter steps through `fight/duel/mod.rs`, physics, every ordered pair of enemies' hits, the push-apart of every
     pair, stage, `TeamRound` (`round.rs`), effect lists, the camera over all four, sound; an imported fighter's
     parts by `team/br1.rs` in each step), then `TeamFight::hud_step` (`TeamHud`) and the started sounds into `Out`;
     a finished battle goes back to the front end (`end_team`);
   - otherwise the front end: `Front::tick` (`frontend/tick.rs`) runs the screen up (`tick_screen`) and returns an
     `Event`, which `handle` (`session/step.rs`) turns into state changes and `Request`s (`Load`, `Fetch`, `Music`,
     `Fight` …).
5. **The step's output.** `Out` (`session/state.rs`) holds the sound commands (`output::sound::Queue`), the
   `Request`s, `card_dirty` and the rumble motors. Back in `App::step`: the replay recorder, netplay, the debug
   history and the rollback test see the step; then `Audio::apply` (`audio/mod.rs`) plays the sound queue (a
   replay seeking or skipping is silent; the rollback test's and the debug tools' re-simulations throw their `Out`
   away), the gamepad rumble is set, every `Request` is served by `App::request` (`app/requests.rs`: a match load
   starts asynchronously, `start_load`; fetched files; music; the preferences) and the card is stored
   (`store_card`, `platform::storage`). Completions come back as flags of a later step's `FrameInput`.
6. **Present.** After the steps, `App::draw` (`app/draw.rs`) draws the state as it is now:
   - a fight: `present::fight_scene::Game::of(Option<&Loaded>, Option<&Fight>, Option<&FightView>)` (`None` unless
     all three are there) → `FightView::draw_capture` (`present/fight_scene/scene.rs`): camera, stage
     (`stage_draw.rs`), fighters with their lighting (`present/fighter_lighting.rs`), effects as PS1 quads through the
     GTE stand-in, then the HUD and labels as `output::prim::Prim`s;
   - a team battle: `TeamGame::of(…, Option<&TeamFight>, …, the open `TeamConfig` menu)` → `FightView::draw_team`
     (`present/fight_scene/team.rs`): `draw_world` (shared with a match: the four fighters, VRAM bank 1, the scaled
     stage, on Q3DM6 the courtyard clip of `arena.rs`), the target rings, the P1..P4 markers, `TeamHud`, the pause;
   - the front end: `Front::draw` (`frontend/draw.rs`) gives `Prim`s in the 512×480 HUD space, its VRAM image is
     kept up by `present::front_vram::VramView` (`sync`), and `output::layout::to_quads` maps the prims to the
     window.
7. **Render.** `render::Renderer` (`render/frame.rs`: `frame`, `frame_capture` → `frame_once`) draws the models,
   the ordering-table stage, the PS1 quads and the 2D layer, and presents. Sound is mixed on cpal's thread by
   `audio/mixer.rs` from what `Audio::apply` queued.

The headless binaries run the same `session::step` without steps 1, 6 and 7: `sim` drives a fight, `fetest` the
front end, `replay` a recorded file (`session::replay::simulate`), `nettest` two netplay peers. `replay --video` runs
the same playback (`session::replay::Headless`) with steps 6 and 7 drawn by an offscreen renderer and the sound mixed
offline into a video file (`app/video.rs`, [`docs/replay_video.md`](docs/replay_video.md)).

## Three kinds of data

| Kind | What | Where | Rules |
|---|---|---|---|
| **Simulated state** | everything `step` reads and writes: `GameState` (`session/state.rs`) with `Front` (`frontend/mod.rs`, serialised through `frontend/serial.rs`), `Fight` (`session/fight.rs`) holding `fight::sim::Match` (fighters, round, camera, effects, stage …); the team battle's (engine addition) `GameState::team` (`fight::team::TeamFight`, beside `Fight`), `team_spec` (the `TeamSpec` it was loaded from: a rematch, a save state's reload) and `team_config` (its pause's button-mapping screen, `TeamConfig`); TEAM ARCADE's (engine addition) ladder `Run::team_ladder` in `Front` (`frontend::team_arcade::TeamLadder`: the CPU pairs and stages drawn at the run's start, the humans' fighters and pads, the rounds, the epilogues shown) and a battle's `TeamFight::arcade` (`ArcadeRules`) | L2 types with `Serialize` / `Deserialize` | integer or `base::portable` math only; no references to static data (they are `#[serde(skip)]` and put back by `rebind`); hashed after every step (`session::hash`), snapshotted (`save_state`, postcard), saved (`save_file`, CBOR `.okbs`), sent in netplay as hashes only. A change to its fields is a `STATE_VERSION` bump; a change to its behaviour a `SIM_VERSION` bump ([`docs/determinism.md`](docs/determinism.md)) |
| **Static data** | read-only data the state refers to: `fight::fight_data::Loaded` (a match: models, ANI / ACT, clips, effect files, VRAM, sound banks; a team battle's four fighters, with `Loaded::team`: `TeamData`, the four costumes, the stage scale, VRAM bank 1), `frontend::FrontData` (the CMN files and banks), `session::FileCache` (fetched files: movies, story pages) | built by the loaders (`load_match`, `load_team`), held by the app in `session::AssetStore`, lent to `step` as `session::Assets` | never written by `step`; never serialised; re-attached after a decode by `session::rebind` (`Match::rebind`, `TeamFight::rebind`, `Front::rebind`), which also checks every id and index the decoded state uses |
| **Presentation** | what shows the state: `present::fight_scene::FightView` (GPU models and instances), `present::front_vram::VramView` (the front end's VRAM image and movie decoder), the `render::Renderer`, the `audio::Audio` mixer, the window, the overlay | `present/`, `render/`, `audio/`, `app/` | rebuilt from the state at any time (a loaded save state, a rewind, a replay seek); never read back by the logic |

The boundary is what makes save states, the rewind, the rollback test, replays and netplay possible: the logic is a
pure function of its start (`SessionStart`) and its inputs.

## Determinism, replays, netplay

- The simulation must give the same states for the same start and inputs on every target and in every later build
  that runs the same `sim_version`. The rules (no float outside `base::portable`, no clock / env / disk / hash-map
  order in a step, `SIM_VERSION` and `STATE_VERSION`) and how to check them: [`docs/determinism.md`](docs/determinism.md).
- **Replays** (`session/replay/`, `.okbr`): the start plus every step's `FrameInput` and `Cheat`s, with checkpoint
  hashes every 60 steps; the app records every session (`app/replays.rs`) and keeps finished matches in its replay
  store (`app/replay_store.rs`). Old replays keep playing through the `sim_version` gates; the fixtures in
  `session/fixtures/` prove it in `cargo test`.
- **Netplay** (`net/`): two peers run the same session in delay-based lockstep and compare state hashes: see
  [`docs/netplay.md`](docs/netplay.md).
- **Debug tools** (`app/tools.rs`, `session/debug_history.rs`): save slots and rewind on snapshots; every mutation
  of the state is a `Cheat` so that replays reproduce it. All `OKB_*` switches: [`docs/debug_hooks.md`](docs/debug_hooks.md).

## Module headers

Every source file starts with a `//!` header. Its first line says what the file is; the tags follow where they apply:

```
//! <What it is, in one line>.
//!
//! Engine addition: <why>                  when the console game does not have it (or said in the first line)
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! <Longer notes: layouts, the update order, what lives in which part of a split module.>
```

`State:` tells whether a type of the file is serialised as simulated state, so that a change there needs
`STATE_VERSION` / `SIM_VERSION` thought. The forms in use:

- `serialised: …` — the file's types are part of `GameState` (or, as for `Settings`, part of it through the card);
- `none of its own: steps the serialised Match` (or updates / reads another type) — logic over state that
  another file defines;
- `static data (… re-attached by rebind)` — loaded or built data, not serialised;
- `presentation (reads the state, never writes it)`;
- `vocabulary: …` — types serialised only where the state embeds them (`output::prim`, `output::layout`,
  `output::sprite`);
- a file format (`the replay file's format …`, `the step log file …`) — serialised, but not part of `GameState`;
- `none`.

About a third of the files carry no `State:` line: they hold no simulated state (all of `app/`, `net/`, `platform/`
and `bin/`, part of `base/` and `formats/`, the replay player and recorder, and a few others). In many files the old
first sentence still follows the new one-line purpose; when you edit one, merge the two.

## Where to find what

| Task | Start here |
|---|---|
| the frame order of a fight | `fight/sim/frame.rs` (header table), `fight/sim/mod.rs` (`Match`) |
| a fighter's fields (`o_XX`, `s_XX`, flag words) | `fight/fighter.rs` |
| a move-script opcode | `fight/move_script/step.rs` (`step`), sizes in `fight/move_script/mod.rs` (`op_size`) |
| which move a fighter picks (walk, guard, attacks, hit reactions) | `fight/move_decision/mod.rs` (`decide`), `reaction.rs` |
| pad bits, commands, chains | `fight/input.rs` |
| movement, gravity, ring-out, walls | `fight/physics.rs`, `fight/stage/mod.rs` |
| hits, damage, guard | `fight/hit.rs`, `fight/duel/hits.rs`; throws `fight/throw.rs` |
| Imported fighters in a match (the face, the health, hits and throws across the engines) | `fight/sim/br1.rs`, `fight/br1/bridge.rs` |
| rounds, KO, time over, continue | `fight/round.rs`; the KO replay `fight/sim/ko_replay.rs` |
| the team battle's logic (four fighters, 2 VS 2 / FREE FOR ALL: targets, hits, throws, rounds, camera, HUD) | `fight/team/` (`frame.rs` lists the match's steps of `fight/sim/frame.rs` in the same order; the steps that run alike are one kernel in `fight/duel/frame.rs` / `duel/br1.rs` that both call); its imported fighters and the big stages BIG STAGE / Q3DM6 (simulation v23) `fight/team/br1.rs` |
| a per-player resource with four fighters (slot and bank, `PlayerExt`, `SideExt`, effect groups 5 / 6, sound slots 7 / 8) | `fight/side.rs`, `fight/team/mod.rs`, `fight/fx/host.rs`, `fight/fx/script_link.rs`; recipe 8 in [`CONTRIBUTING.md`](CONTRIBUTING.md#8-touching-the-fight-frame-or-a-per-player-resource) |
| drawing the team battle | `present/fight_scene/team.rs` (`draw_team`), `scene.rs` (`draw_world` from `SceneRef`), VRAM bank 1 (CLUT bit 15, `render/shaders/psx.wgsl`) |
| the team battle's menus, session and replays | `frontend/team.rs`, `session/step.rs` (`start_team`, `team_frame`, `end_team`), pads 3 / 4 `FrameInput::ext`, replay FORMAT 2 (`session/replay/inputs.rs`); headless `sim` with `TEAM=` (`SHOTS`, `TEAMCHECK`), `fetest` with `FESCRIPT3` / `4`, in the app TEAM BATTLE on the mode menu (debug `OKB_TEAM`) |
| TEAM ARCADE (the co-op ladder of 2 VS 2 battles: the ladder's draw, the arcade select and the partner pad, CPU levels, YOU WIN / YOU LOSE, CONTINUE?, GAME OVER, the ending and clear card) | `frontend/team_arcade.rs` (the ladder, the run's screens, the ending), `frontend/team.rs` (the screens it shares with the team battle), `fight/team/arcade.rs` + `fight/team/round.rs` (states 0xF / 0x10), `session/step.rs` (the pads by slot, the play time), the replay header's `team_arcade`; headless `sim TEAM=2v2 ARCADE=`, `fetest WINS` `C` / `G`; in the app TEAM BATTLE → TEAM ARCADE (debug `OKB_TEAM_ARCADE`) |
| the CPU opponent | `fight/ai.rs`; an imported fighter's `fight/br1/ai.rs` |
| the fight camera | `fight/camera.rs` |
| the extra stages Q3DM17 / Q3DM6 (Quake III's maps, local files) | [`docs/extra_stage_q3dm17.md`](docs/extra_stage_q3dm17.md), [`docs/extra_stage_q3dm6.md`](docs/extra_stage_q3dm6.md), `fight/stage/extra.rs`, `present/fight_scene/arena.rs` |
| the fight HUD and labels | `fight/hud.rs` (logic → prims), fonts `output/font.rs` |
| the in-fight pause | `fight/pause.rs` |
| hit sparks, shards, rings, flashes | `fight/fx/hit_effects/` |
| effect scripts (EFT.DAT bytecode) | `fight/fx/script_vm/`; started from move scripts in `fight/fx/script_link.rs` |
| effects run as native tasks (not scripts) | `fight/fx/native_tasks.rs`, `fight/fx/move_effects.rs`; 3-D effect models `fight/fx/effect_models.rs` |
| stage animation, palette cycling | `fight/stage/anim.rs` (logic), `present/fight_scene/stage_draw.rs` (drawing) |
| what a fight loads | `fight/fight_data.rs` (`load_match`, `Loaded`) |
| title, mode menu, attract demo | `frontend/title.rs`, `frontend/tick.rs` |
| character select, VS screen | `frontend/select.rs`, `frontend/vs.rs` |
| arcade ladder, routing after a fight | `frontend/progress.rs` (TEAM ARCADE's ladder: `frontend/team_arcade.rs`) |
| OPTIONS / SOUND / CUSTOM / CONTROLLER CONFIG. / GRAPHICS menus | `frontend/menus/` (`mod.rs` `Kind`, `actions.rs`, `draw.rs`, `flow.rs`, `controller.rs`) |
| the option struct, the save file, the engine's preferences | `frontend/options.rs`; file I/O `platform/storage.rs` |
| MEMORY CARD screens | `frontend/memcard.rs` |
| story mode, endings, movies, pictures | `frontend/story_flow.rs`, `frontend/story_screens.rs`, `frontend/movie.rs` |
| NAME ENTRY, ranking | `frontend/ranking.rs` |
| the state, the step, the session start | `session/state.rs`, `session/step.rs`, `session/start.rs` |
| `SIM_VERSION` (its changelog, a version's rules `Gates`, one constant per rule `v::*`), `STATE_VERSION` | `fight/version.rs`, `session/snapshot.rs` |
| a fighter's slot and bank, bank 1's per-player resources | `fight/side.rs` (`Fid`, `Slot`, `Bank`, `PerSlot`) |
| replays (file, recorder, player, headless check) | `session/replay/`; the app's side `app/replays.rs`, `app/replay_store.rs`, `app/replay_menu.rs`, `app/playback.rs`; to video `app/video.rs` |
| save states, rewind | `session/snapshot.rs`, `session/debug_history.rs`, `app/tools.rs` |
| netplay | `net/`, `app/netplay.rs`, `bin/nettest.rs`; the NETWORK menu `app/net_menu.rs` ([`docs/network_menu.md`](docs/network_menu.md)) |
| drawing a fight | `present/fight_scene/` |
| GPU pipelines, shaders | `render/pipelines.rs`, `render/frame.rs`, `render/shaders/` |
| PS1 semi-transparency, CLUT sampling | `render/shaders/psx.wgsl`, `render/shaders/sprite.wgsl` |
| window layout, WIDE / 4:3, HUD scaling | `output/layout.rs` |
| sound effects, music | `output/sound.rs` (logic's commands), `audio/` (mixer), `formats/vab.rs`, `formats/music.rs`, `formats/xa.rs` (`Assets::load_music`: `.xa` first, else `.mp3`) |
| keyboard, gamepads, rumble | `app/devices.rs` (keyboard, `PadSlots`), `app/gamepads.rs`, `app/pad_layout.rs`; [`docs/gamepads.md`](docs/gamepads.md) |
| Android: build, activity, touch controls, lifecycle | [`docs/android.md`](docs/android.md), `android/`, `app/android.rs`, `app/touch.rs`, `platform/android.rs` |
| a debug key, a `OKB_*` variable | `app/keys.rs` (F-keys), `app/events.rs` (`window_event`: the fight's letter keys), `app/debug_env.rs`; list in [`docs/debug_hooks.md`](docs/debug_hooks.md) |
| the layer rules | `tests/layering.rs` |
| the source names no address of the recreated program (and the files allowed to) | `tests/no_original_refs.rs` (`ALLOWED`, `ALLOWED_CODE`) |
| the game's text (names, menus, messages) | `assets/text.rs` (`text::get`, `text::item`, `Label`), the converted table `text.strings`; `tests/original_text.rs` keeps it out of the code |

## Verification toolbox

All commands are in [`CONTRIBUTING.md`](CONTRIBUTING.md): the unit tests and the replay fixtures (`cargo test`), the
headless `sim` traces and `fetest` routes, the golden screenshot baseline (`run_baseline.sh` + `compare.sh`,
every file byte for byte: the count `compare.sh` prints, 241 files with 136 screenshots on 2026-11-07), the `replay`
checker, the team battle's check matrix (`sim TEAM=… TEAMCHECK=1 ROLLBACK=8`), the rollback test (`ROLLBACK=N`), the cross-target determinism matrix,
`nettest`, and the fuzz targets in `fuzz/`.
