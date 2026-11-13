# Glossary

Terms of the PlayStation, of the game's data and of the engine, as the code comments use them. Paths are
relative to `src/`.

## PlayStation hardware and formats

| Term | Meaning |
|---|---|
| **`tables.okt`** | the game's static tables as the converter writes them into `engine/tables.okt`, read by name (`formats/tables.rs`). Data format 1 had no such file; such data is converted again. |
| **scratchpad** | the CPU's 1 KB fast RAM. The game keeps the match block there (**M**, below) and the two fighter objects. |
| **GPU primitives: F4 / FT4 / G4** | the PS1 GPU's quads: flat colour, flat textured, Gouraud. The engine's 2D layer (`output/prim.rs`) and the effects (`fight/fx/hit_effects/quads.rs`, `output::psx_quad::PsxQuad`) keep them as PS1 quads. |
| **OT (ordering table)** | the GPU's draw order: an array of linked lists indexed by a depth slot (`otz`, e.g. SZ / 4 from the GTE); a primitive in a higher slot is drawn first (further back). Fixed slots put whole objects behind everything sorted (the stage at 0x80F..0x81E, `render/frame.rs`). The engine reproduces the order, not the lists. |
| **VRAM** | the GPU's 1 MB frame and texture memory, 1024×512 16-bit halfwords. `formats/vram.rs` keeps the part the effects use (a CPU copy filled as the loader does); `frontend/resident.rs` the front end's loads, `present/front_vram.rs` its image. |
| **texture page (tpage)** | a 256×256-texel window of VRAM that textured primitives sample, with its texel depth (4-bit / 8-bit CLUT or 15-bit direct) and blend mode in the tpage word (`output::sprite::Page::tpage`). |
| **CLUT** | colour look-up table: a row of 16 or 256 colours in VRAM that 4 / 8-bit texels index. Palette swaps and palette cycling (HUD gauge, stage water) pick or rewrite CLUT rows (`fight/stage/anim.rs`, `fight/hud.rs`). |
| **STP bit** | bit 15 of a 15-bit colour: the texel is semi-transparent (when the primitive is). Colour 0 with STP clear is fully transparent. |
| **semi-transparency modes 0–3** | the GPU's blend of the back colour B and the front colour F: 0 = B/2 + F/2, 1 = B + F, 2 = B − F, 3 = B + F/4 (`render/shaders/psx.wgsl`, `render/shaders/sprite.wgsl`; `abr` in the effect records). |
| **TIM** | the PS1 image file: id, flags (colour mode, CLUT present), then the CLUT block and the pixel block with their VRAM rectangles (`formats/vram.rs`). |
| **HMD** | Sony's model format the stages are made of; the export converts them to glTF, keeping flags the engine needs (e.g. the semi-transparent polygons, type bit 0x200000, `fight/fight_data.rs`). |
| **GTE** | the geometry coprocessor: **RTPS** (rotate, translate, perspective: a vertex to screen x / y and depth SZ) and **NCCS / NCCT** (normal colour: per-vertex lighting from the light matrices, `present/fighter_lighting.rs`, `render/shaders/model.wgsl`). The engine projects effects with a GTE stand-in (`output::gte::Gte`). |
| **4.12 fixed point** | the GTE's number format: 4096 = 1.0 (rotation matrices, light matrices, sin / cos tables). Angles are 4096 per turn. |
| **MDEC / BS / STR** | the movie hardware decoder (MDEC), its bitstream frames (BS, version 2) and the movie files (`/MOV/*.STR`); `formats/movie.rs` decodes them in software. |
| **XA** | CD-XA ADPCM audio, the game's music (files `MC0n`, 8 channels each). The export keeps each track's ADPCM sectors (`audio/music/BR2_MCnn_chN.xa`, the movies' audio `video/<NAME>.xa`), decoded by `formats::xa`; older exports have MP3 (`….mp3`, decoded with symphonia, `formats::music::decode_mp3`). `Assets::load_music` reads the `.xa` first, `formats::music::decode` tells the two apart by the magic. |
| **SPU / ADPCM / VAB** | the sound processor, its 4-bit ADPCM sample format and Sony's sound bank format (`*.SND` files, magic `pBAV`): programs, tones, ADSR envelopes; `formats/vab.rs` decodes them, `audio/mixer.rs` mixes them. |
| **pad word** | the PS1 controller state, byte-swapped: directions in the high nibble (Up 0x1000, Right 0x2000, Down 0x4000, Left 0x8000), Select 0x100, Start 0x800, buttons in the low byte (L2 01, R2 02, L1 04, R1 08, △ 10, ○ 20, × 40, □ 80). `fight::input::pad` names them. |
| **DualShock** | the controller with two rumble motors; the game's motor state goes to `session::Out::vib` and the app's gamepad rumble (`app/gamepads.rs`). |
| **512×480 HUD space** | the game's 2D screen: 512 wide, 480 lines interlaced. The engine draws all 2D (`output/`) in it and scales to the window (`output/layout.rs`). |

## The game's files (in the converted data, `okb_export/` in development)

| Term | Meaning |
|---|---|
| **`.DAT` containers** | the game's archives with LZ compression (`formats/container.rs`). |
| **ANI** | a character's move file: move scripts, the command table (header word 3), chain programs (word 4) and the CPU's action scripts (word 5) (`formats/act_ani.rs`, `fight/input.rs`, `fight/ai.rs`). |
| **ACT** | an animation key list file: pose records per key (`formats/act_ani.rs`, `fight/anim/playback.rs`, `fight/anim/act_clips.rs`). `CMN.ACT` holds the shared animations. |
| **CMN** | `/CMN/*.DAT`, the front end's common files (fonts, HUD, menu graphics), loaded at boot (`frontend/data.rs`). |
| **EFT.DAT / `<CHR>EFT.DAT`** | the effect files: texture pages, effect scripts (entries 80..99 shared, 11..20 per character) and 3-D effect models (`fight/fx/script_vm/`, `fight/fx/effect_models.rs`, `fight/fight_data.rs`). |
| **ED1C4 records** | the 3-D effect model records (magic 0xED1C4), drawn by the effect VM and the native tasks (`fight/fx/effect_models.rs`). |
| **stage codes ST0..STC** | the 13 fight stages (`fight::fight_data::STAGE_CODES`); ST9..STC are the four the console game never offers (EXTRA1..EXTRA4 here). |

## Data blocks and field names

| Term | Meaning |
|---|---|
| **fighter object, `o_XX`** | the fighter's main block; `Fighter::o_XX` is the engine's serialised id of the field at object +0xXX, used where the field has no better name. Comments also write **F.xx** / **F+0xXX** for it. |
| **state block, `s_XX`** | the fighter's script / state block (object +0x38); `Fighter::s_XX` is the serialised id of the field at state +0xXX. Comments write **S** for it. |
| **F54 / F58 / F5C / F60 / F64 / F68 / F6C** | the fighter object's flag words +0x54 .. +0x6C (`fight::fighter::F58` …, indices into `Fighter::flags`): states, cancel windows, input locks (F60), this frame's hit events (F68), requests (F6C). |
| **M, `M.xx`** | the match block (in the scratchpad): `M.xx` = M +0xXX. Examples: M.92 round state, M.98 stage number, M.9E the afterimage / frame-persistence mode, M.9F the stage darkening (255 = Beast Drive), M.A0 CPU level, M.B0 HUD hidden, M.BC the camera, M.C0 the option struct `opt`. |
| **W, `W.xx` / W+0xXX** | the wall block (`fight::stage::Walls`); the stage routine's counters live at W+0x26 / +0x28 / +0x2A (`fight::stage::anim::Counters`). |
| **`opt`** | the option struct (`frontend/options.rs`). |
| **move script, ops** | the per-move bytecode the move-script VM runs (`fight/move_script/`, the op dispatch in `step.rs`); "op 0x18" etc. are its opcodes. Not the effect VM. |
| **effect VM** | the effect bytecode interpreter (`fight/fx/script_vm/interp.rs`, the VM state in `script_vm/mod.rs`), started by move-script ops 0x16 / 0x17 / 0x18 (`fight/fx/script_link.rs`). |
| **native tasks** | effect tasks run as engine code rather than effect bytecode (`fight/fx/native_tasks.rs`, `fight/fx/move_effects.rs`). |
| **KO replay** | the game's replay of the final KO, re-simulated from a snapshot with the logged pads (`fight/sim/ko_replay.rs`). Not the engine's `.okbr` replays. |
| **Beast Drive** | a beast's super move; darkens the stage to 255 (M.9F) and draws the wireframe ring (`present/drive_grid.rs`). |

## The engine's own terms

| Term | Meaning |
|---|---|
| **the main game, the imported game** | the main game: the game the main disc holds, whose data every install needs (`Kind::Br2`, the `br2_*` names). The imported game: the optional second disc's game, whose fighters, stages and endings the engine imports into the main game's matches and menus (the `br1` modules, `Br1*` types, `BR1_*` names). "Main-game" and "imported" before a noun mean the same (a main-game stage, an imported fighter). |
| **engine addition** | anything the console game does not have (netplay, replays, debug tools, WIDE aspect, the atlas); module headers say so. Everything else reproduces the game's behaviour. |
| **`GameState`** | the whole logic state of a session (`session/state.rs`): the front end (`Front`), the fight (`session::fight::Fight`, holding `fight::sim::Match`), the backdrop fight, the music, the debug setup. Serialisable, no static data, no GPU objects. |
| **`session::step`** | the one function that advances a `GameState` by a 60 Hz frame, from a `FrameInput` and the `Cheat`s, reading static data through `Assets` and returning what the app must do in `Out`. |
| **`FrameInput`** | one frame's input: both pads, the scripted pads (`forced`) and flags: `FORCED`, `LOADED` (the match asked for is loaded), `FETCHED` (the files asked for are there), `CARD_STORED` / `CARD_FAILED` (the memory card store finished). |
| **`Request`** | what a step asks the app for: `Load` a match, `Fetch` files, `Music`, `Prefs`, `Exit`, `Replays`, `Fight` / `FightOver` (build / drop the view). |
| **`Cheat`** | a debug mutation of the state applied by `step` (F2 / F3 1 HP, F4 timer, F6 gauges, `[` / `]` moves, B / N beast, C CPU, F11 unlock, OKB_P2DMG, OKB_FORCEMOVE, OKB_BREAK). Recorded in replays, refused in netplay. |
| **static data** | what the state refers to but does not own: `fight::fight_data::Loaded` (a match's data), `frontend::FrontData` (the front end's), `session::FileCache` (fetched files); lent to `step` as `session::Assets`, held by the app in `session::AssetStore`. |
| **`rebind`** | after a state is deserialised (a snapshot, a save-state file, a peer's): re-attaching its static data from `Assets` (`session::rebind`, `Match::rebind`, `Front::rebind`). |
| **presentation** | what the app builds from the state to show it and never feeds back: the GPU objects (`present::fight_scene::FightView`), the VRAM image (`present::front_vram::VramView`), the movie decoder, the mixer. |
| **`SIM_VERSION`** | the simulation's version (`fight::version::SIM_VERSION`, re-exported as `session::SIM_VERSION`; its doc comment lists every bump): bumped by every change that makes the same start and inputs give other states. A session (and a replay) carries its `sim_version`; old behaviour stays behind a named rule of `fight::version` (`v::*`, asked as a flag of `Gates`). |
| **`STATE_VERSION`** | the runtime snapshot's version (`session::STATE_VERSION`; its doc comment lists every bump): bumped by any change to what `GameState` serialises (postcard snapshots: rewind, rollback test, desync dumps; same build only). |
| **save-state file `.okbs`** | (earlier builds: `.br2s`, still read) `BR2S` + `FILE_FORMAT` (`session::snapshot::FILE_FORMAT`) + CBOR of the state: field-named, so later builds read older files. |
| **replay `.okbr`** | (earlier builds: `.br2r`, still read) `BR2R` + `FORMAT` (`session::replay::FORMAT`, the newest layout this build reads) + the simulation version: a session's start, its inputs, cheats and checkpoints (`session/replay/`); played back by the app, checked headless by the `replay` binary. FORMAT 1: pads 1 / 2; **FORMAT 2** (`FORMAT_TEAM`, engine addition): the input stream also holds pads 3 / 4, written only when the header holds a `TeamSpec` or the session used pads 3 / 4 (every other file stays FORMAT 1), refused before the team battle's simulation version. |
| **step log** | `BR2T` (`session/determinism_trace.rs`): a `sim` / `fetest` run's steps, for `replay --trace` on another target. |
| **`PROTO`** | the netplay packet layout's version (`net::proto::PROTO`; packets start with `BR2N`). |
| **`BR2R`, `BR2S`, `BR2T`, `BR2N`, `BR2W`, `BR2XA`, `BR2PCM`** | the files' and packets' first bytes: historical format ids from before the engine's name, kept so that earlier files (and the converted data's audio) stay readable. |
| **lockstep** | netplay's delay-based lockstep: both sides step the same session with both inputs, delayed by N frames (`net/`). Also **`OKB_LOCKSTEP`**: the app runs exactly one 60 Hz step per presented frame (reproducible screenshots). |
| **rollback test** | a debug check that restores the snapshot of N frames ago and re-steps to the same hash (`session/rollback.rs`; `ROLLBACK=N`, `OKB_ROLLBACK=N`). |
| **determinism matrix** | the cross-target check (native vs wasm32): the state hash after every step must match (`session/determinism_trace.rs`, `base/portable.rs`; scripts outside the repository). |
| **`portable`** | the fight logic's float math from simulation version 3 (`base/portable.rs`): f64 glam and libm, the same bits on every target. |
| **golden baseline** | the reference outputs of `run_baseline.sh` (sim / fetest traces, the team battle's headless shots, app screenshots, saves), kept outside the repository; a refactor must reproduce them byte for byte (`compare.sh`, which prints the count: 241 files, 136 screenshots on 2026-11-07). |
| **sync id** | `frontend::menus::Kind::sync_id`: a fixed number per menu in a replay's sync hash (`Front::sync_key`); a new menu takes a new number, none is reused or renumbered. |

## The team battle (engine addition)

Four fighters on one stage, 2 VS 2 or FREE FOR ALL.

| Term | Meaning |
|---|---|
| **`TeamFight`** | the team battle's world (`fight/team/mod.rs`, `GameState::team`): four fighters, their targets, a `World`, the round flow `TeamRound`, the effects, the camera, the HUD. It stands beside `Match` rather than widening it, so a match's serialised bytes (which the replay traces hash) stay as they were. |
| **slot** | fighter `k`'s player number for every per-fighter routine, `k & 1` (`Fighter::player`): P1 and P3 play as player 0, P2 and P4 as player 1. In 2 VS 2 the slot is the team. |
| **bank** | which copy of the game's per-player resources fighter `k` uses, `k >> 1`: 0 for P1 / P2 (the game's own), 1 for P3 / P4 (the engine's copy: effect groups 5 / 6, EffTab 100..139, VRAM bank 1, sound slots 7 / 8, the AI's random index + 64; `fight/side.rs` `Bank`). |
| **`PlayerExt`** | the per-player `World` state that is per fighter in the team battle (`fight::team::PlayerExt`: the palette fade record `p4b18`, `hud_flash`), kept in `TeamFight::ext` and swapped into the `World`'s slot around each fighter's steps. |
| **`SideExt`** | fighters 2 / 3's part of the effect tasks' per-side state (`fight::fx::native_tasks::SideExt`: Stun's `aura`, `prev_gauge`, `f6c`), kept in `TeamFight::fxn_ext` and lent to the effect code as `FxHost::ext`. |
| **`FxHost`** | the parts of a fight the effect code runs on (`fight/fx/host.rs`), lent by a `Match` or a `TeamFight`: the fighters by index, each one's opponent, and fighters 2 / 3's `SideExt`. |
| **`SceneRef`** | a read-only view of a fight's world for `present` (`fight/scene.rs`), built by `Match::scene` or `TeamFight::scene`. Per-fighter state is drawn through it (`SceneRef::fade(i)`), never from the `World`'s slot `player & 1`, which in a team battle holds whichever of its two fighters stepped last. |
| **target** | the opponent a team-battle fighter's per-fighter steps see (`TeamFight::target`, `fight/team/target.rs`): the nearest enemy still in the round (with hysteresis), one chosen with L2, or its throw partner (`target::Locks`). |
| **FORMAT 2** | the replay layout with pads 3 / 4 (see **replay `.okbr`** above). |
| **TEAM ARCADE** | the team battle's third mode (simulation v26): two players on one team (pad 1's P1 + P3 or pad 2's P2 + P4; one player alone gets a CPU partner) through a **ladder** of 2 VS 2 battles against CPU pairs, with ARCADE's continues, the humans' epilogues and a clear card at the end. `Mode::TeamArcade`; a "run" is the time from the mode choice to the end of the ladder (or GAME OVER, or leaving its screens). |
| **ladder** (TEAM ARCADE) | the run's battles, drawn once when it starts (`frontend::team_arcade::TeamLadder`, `Run::team_ladder`): four pairs of regular fighters on four distinct stages, the imported game's pair third on BIG STAGE with its data, GADO + SHENLONG last on BUILDING1; 5 or 6 stages (`TEAM_ARCADE_MIN_STAGES` / `TEAM_ARCADE_MAX_STAGES`). Not ARCADE's 1-player ladder (`frontend/progress.rs`). |
| **partner pad** | TEAM ARCADE's second human: the pad that presses START first on the partner's plate (the select or the loading screen), pad 2 or pad 3 in a pad-1 run (`TeamLadder::partner_ports`); it plays the partner slot (P3, or P4 in a pad-2 run) for the run (`partner_port`, `slot_port`). None: a CPU partner at the boss's level. |
