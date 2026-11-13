| `SHOTDIR=dir` | the ignored `app/replay_menu.rs` `team_arcade_list_shot` (TEAM ARCADE A4): the replay list with TEAM ARCADE entries drawn headless to `replay_list_team_arcade.png` | the working directory |
# Debug hooks: environment variables, URL parameters and keys

Every switch the code reads, grepped from `src/` (2026-11-02, restructure stage 1; paths and module paths as of stage 4;
updated 2026-11-07 for the team battle up to simulation v25 and the test-only variables). All are engine
additions. Paths are relative to `src/`. "Recorded" says whether a replay (`.okbr`) carries the hook's effect: hooks
that set up the session go into its `SessionStart` (the replay's header), scripted pads into the `FrameInput`s, debug
mutations into `Cheat`s; the others change only what this run shows or writes.
Every `OKB_` variable is also read under the name earlier builds used, `BR2_…` (`base::env`: the `OKB_` one first).

The browser build reads no environment (`DebugEnv::read` is all off on wasm32); its switches are the URL parameters
below. Set `OKB_SAVE_DIR` to a scratch directory for any test run, so the player's save is never touched.

## Script syntax

`OKB_SCRIPT` .. `OKB_SCRIPT4`, `OKB_FESCRIPT` .. `OKB_FESCRIPT4`, `OKB_UISCRIPT`, `OKB_NETSCRIPT`, and the `sim` /
`fetest` scripts (`P2SCRIPT`, `SCRIPT` .. `SCRIPT4`, `FESCRIPT` .. `FESCRIPT4`, the `sim` script argument) are
`frames:keys,…` (`fight/input_script.rs`, `parse_script`): each step holds the keys for that many frames (at most an
hour). Keys: `U D L R` directions, `J` □, `K` ×, `C` ○, `I` △, `G` R1, `H` L1, `S` START, `E` SELECT (no key for L2
or R2). Example: `60:,20:R,1:J,40:` (60 idle frames, walk right 20, □ once, 40 idle).

## The app (`okbeast`, desktop)

Read once at start by `app/debug_env.rs` (`DebugEnv::read`) unless another owner is given.

### Session setup (recorded in `SessionStart`)

| Variable | Owner | Effect |
|---|---|---|
| `OKB_LOCKED` (any value) | `main.rs`, `bin/fetest.rs` | the game's own progression (`--locked`): the save's characters and fourth costumes, the nine stages |
| `OKB_UNLOCK` | `main.rs` (doc only) | accepted, does nothing (everything is open by default since simulation v5) |
| `OKB_SEED=n` | `debug_env.rs` | the front end's random seed instead of the clock |
| `OKB_SIM_VERSION=n` | `debug_env.rs`, `session::parse_sim_version` | run simulation version n (1..=`SIM_VERSION`; anything else is warned about and ignored) |
| `OKB_BR1=dir` | `assets/source.rs` (`Assets::br1`) | desktop: the imported game's install read instead of the game data's `br1` folder (e.g. `../okb_export/br1` beside `--assets ../okb_export`); its content id goes into the session's start (engine addition, the import S8) |
| `OKB_NOINTRO` | `debug_env.rs` | start at the title instead of the boot logos (also implied by `OKB_FESCRIPT` / `OKB_SCRIPT`); the browser: `?nointro=1` |
| `OKB_NAMETEST=mode:value` | `debug_env.rs` | NAME ENTRY test for YUGO: mode 0 = arcade (value = clear time in frames, default 7323), 3 = survival (wins) |
| `OKB_EPILOGUE=c` | `debug_env.rs` | character c's arcade epilogue, then the ending movie |
| `BR2_MCFAULT=name` | `debug_env.rs`, `frontend::memcard::Fault::parse` | one forced memory card result: `nocard`, `unformatted` / `unfmt`, `newcard`, `changed`, `full`, `other`, `nodata`, `dataerr` / `vererr` |
| `OKB_P2=human` | `debug_env.rs`, `session/fight.rs` | player 2 is human (default: the CPU; C toggles it in a fight) |
| `OKB_WATCH` | `debug_env.rs` | CPU vs CPU |
| `OKB_NO_EXTRAS` | `assets/extras.rs` | the extra stages Q3DM17 and Q3DM6 are not offered even when their files are in the game data (`q3/` or `extras/`) (the golden baseline sets it; also `fetest`). A session's start records whether it was offered, so replays are not affected |
| `OKB_Q3=dir` | `assets/source.rs` (`Assets::q3`) | desktop: a Quake III install (a `q3/` folder the setup converted) read instead of the game data's `q3` folder, for the extra stages Q3DM17 / Q3DM6 (`assets/extras.rs`); a folder without a usable one leaves only the older `extras/<map>/` (engine addition) |
| `OKB_MODEL=n` | `debug_env.rs` | CUSTOM rules with MODEL TYPE n (0..3), with `OKB_CFLAGS=hex` (the custom flags) and `OKB_PRACTICE` (PRACTICE) |
| `OKB_CONT` | `debug_env.rs` | the continue screen after a lost direct fight |
| `OKB_GAUGE=n` | `debug_env.rs` | P1 starts with this beast gauge (0..0x3BFF) |
| `OKB_P2DMG=d` | `debug_env.rs` | P2 starts the first round with damage d (a quick KO); see also the `d@frame` form below |
| `OKB_COSTUME=a,b` | `fight/fight_data.rs` (`direct_costume`) | a direct fight's costumes (0..3; default `0,1`); recorded in the replay header; also read by `sim` |
| `OKB_TEAM=2v2\|ffa` | `debug_env.rs` (`TeamDebug`), `app/events.rs` (`start_up`), `app/mod.rs` (`begin_session`), `session::new_team_state` | engine addition, debug (the menus' TEAM BATTLE is the way in since stage T4): the app starts a team battle instead of the title, drawn by `FightView::draw_team`; with `OKB_TEAM_CHARS=P1,P2,P3,P4` (codes, default `YUG,ALI,BAK,ULI`; a duplicate takes the next costume; from simulation v23 also `MIT FOX GRE URI` with the imported game's install, `OKB_BR1`), `OKB_TEAM_STAGE=STn` (default ST0; ST0, ST2, ST5, ST7, ST8; from v23 also `Q3DM6`, `BIG00`) and `OKB_TEAM_HUMANS=n` (pads 1..n human, at most 4; default 1, 0 = four CPUs). `OKB_SCRIPT` .. `OKB_SCRIPT4` drive pads 1 .. 4 by the battle's frame, `OKB_SHOTS` saves its frames. Not in netplay or a playback, and **not recorded** (a replay holds a match or the front end; a team battle of the menus is, in replay FORMAT 2) |
| `OKB_TEAM_ARCADE=n` | `debug_env.rs`, `session::Entry::TeamArcade`, `frontend::team_arcade` (`debug_team_arcade`) | engine addition, debug (TEAM ARCADE A4, simulation v26): the front end starts in a TEAM ARCADE run of pad 1 at stage n's loading screen (1 .. 6, the last at most; the ladder drawn as the mode choice draws it), the humans' fighters P1 / P3 of `OKB_TEAM_CHARS` (the main roster's codes; default `YUG` and `BAK`), the partner pad 2 with `OKB_TEAM_HUMANS=2` (else a CPU). From there the run goes on as the menus' (the ending after the last stage). Before v26 the title. Unlike `OKB_TEAM` it is **recorded**: the start is the session's (`SessionStart::entry`), so its replays play back |

### Scripted input and timed mutations (recorded)

| Variable | Owner | Effect |
|---|---|---|
| `OKB_SCRIPT` | `debug_env.rs` | scripted pad 1 of the fight; without `OKB_FESCRIPT` it also starts a direct fight (`main.rs`). The app quits 5 frames after the last `OKB_SHOTS` frame |
| `OKB_SCRIPT2` | `debug_env.rs` | scripted pad 2 of the fight (with `OKB_P2=human`) |
| `OKB_SCRIPT3` / `OKB_SCRIPT4` | `debug_env.rs`, `app/stepping.rs` | engine addition (the team battle, T4): scripted pads 3 / 4 of the team battle by its frame (`FrameInput::forced_ext`, from simulation v22; a human slot: joined on the select, or `OKB_TEAM_HUMANS`) |
| `OKB_FESCRIPT` .. `OKB_FESCRIPT4` | `debug_env.rs` | scripted pads 1 .. 4 of the front end, from the title; with `OKB_SCRIPT` the fight is reached through the menus; pads 3 / 4 (engine addition, T4, from simulation v22) only on the team battle's screens (join with START) |
| `OKB_P2DMG=d@frame[,d@frame…]` | `debug_env.rs`, `app/stepping.rs` (`env_cheats`) | P2's damage set to d at those fight frames (`Cheat::P2Dmg`) |
| `OKB_FORCEMOVE=frame:move` | `debug_env.rs`, `app/stepping.rs` | P1's move request at that fight frame (`Cheat::ForceMove`; the effect debug line is logged from then on) |
| `OKB_BREAK=side` | `debug_env.rs`, `app/stepping.rs` | that wall (0 = +X, 1 = −X, 2 = +Z, 3 = −Z) broken at fight frame 330 (`Cheat::Break`) |
| `OKB_NETSCRIPT` | `debug_env.rs`, `app/netplay.rs` | netplay: this side's scripted pad by its local frame, ORed into this side's input (which the lockstep sends); with `OKB_SHOTS` the app quits 5 frames after the last |
| `OKB_NETSHOTS=n,m,…` | `debug_env.rs`, `app/draw.rs` | netplay: those session frames (`GameState::frame`) saved as `okb_net_<n>.png` (the fight or the front end); the app does not quit for them |
| `OKB_NETEXIT=n` | `debug_env.rs`, `app/netplay.rs` | n UI frames after a netplay session of the NETWORK menu ended and the menu is back: that frame saved as `okb_net_end.png`, then the app quits |
| `OKB_NETCODE=CODE` | `debug_env.rs`, `app/netplay.rs` | the NETWORK menu's HOST takes this room code instead of a random one (a scripted joiner types it) |
| `OKB_NETBIND=ADDR` | `debug_env.rs`, `app/netplay.rs` | a direct-IP HOST of the NETWORK menu listens on this address (e.g. `127.0.0.1`) instead of every interface, shows it as its address and looks up no network address |

### Capture and run control (not recorded)

| Variable | Owner | Effect |
|---|---|---|
| `OKB_SHOTS=n,m,…` | `debug_env.rs`, `app/capture.rs` | fight frames saved as `okb_frame_<n>.png` in the working directory; with a script the app quits 5 frames after the last |
| `OKB_FESHOTS=n,m,…` | `debug_env.rs`, `app/capture.rs` | front-end frames (counted from the start) saved as `okb_fe_<n>.png` (the first frame drawn at or after each) |
| `OKB_FEEXIT` | `debug_env.rs` | quit 5 frames after the last `OKB_FESHOTS` frame |
| `OKB_LOCKSTEP` | `debug_env.rs`, `app/events.rs` | exactly one 60 Hz step per frame that reached the screen (reproducible screenshots; an occluded window waits) |
| `OKB_FAST=n` | `debug_env.rs`, `app/events.rs` | n fight steps per drawn frame until close to a `OKB_SHOTS` frame (16 when n does not parse) |
| `OKB_FEFAST=n` | `debug_env.rs`, `app/events.rs`, `app/stepping.rs` (`fe_fast`), `app/capture.rs` | n front-end steps per drawn frame (movies, menus, the attract title) while the next `OKB_FESHOTS` frame still to save is 64 or more front-end frames ahead; never during a fight, while a match load waits for its drawn frames, after a step that made a request (a fetch, load or music arrives after the redraw), or once all shots are saved (16 when n does not parse, at least 1). The steps are the same, only the frames between are not drawn, so with `OKB_LOCKSTEP` and scripted input the shots and the save come out the same (overlay toasts count drawn frames, so a toast can last longer) (the front end's VRAM image is rebuilt from the state at the drawn frame, a movie's frame decoded from its counter) |
| `OKB_UISCRIPT` | `debug_env.rs`, `app/playback.rs` | scripted pad 1 of the replay list, the playback controls and the NETWORK menu, by their own 60 Hz frames |
| `OKB_UISHOTS=n,m,…` / `OKB_UIEXIT` | `debug_env.rs`, `app/draw.rs`, `app/events.rs` | those UI frames saved as `okb_ui_<n>.png`; quit 5 UI frames after the last |
| `OKB_SETUP_FROM=PATH` / `OKB_SETUP_BR1=PATH` / `OKB_SETUP_Q3=PATH` / `OKB_SETUP_SCAN=dirs` / `OKB_SETUP_SLOW=ms` | `debug_env.rs`, `app/setup/mod.rs` | the setup screen: PATH (a folder or a disc image) dropped at UI frame 30 and installed once checked, the game started a second after the done page; OKB_SETUP_BR1: PATH added as the imported game's disc wherever ADD BLOODY ROAR 1 DISC is offered (after that install, or over the game from OPTIONS → CHANGE GAME DATA, after which the app restarts without the OKB_SETUP_* hooks; its check or install failing ends the hook with an error in the log, and a OKB_SETUP_FROM run quits); OKB_SETUP_Q3 (engine addition): the same for QUAKE III ARENA MAPS (Quake III Arena's baseq3 folder, the game's folder, a .pk3 or a converted folder; after OKB_SETUP_BR1's when both are set); the scan's folders; a pause per copied file or conversion step |
| `OKB_NOPLAYUI` | `debug_env.rs`, `app/playback.rs` | no playback overlay (a playback's frames as the recording's) |
| `OKB_DEBUGKEYS=frame:key,…` | `debug_env.rs`, `app/tools.rs` (`parse_keys`) | the debug tools' keys pressed at those drawn frames (with `OKB_DEBUG`): `P`, `.`, `F12`, `SF12`, `S1`..`S4`, `BS` / `SBS` / `/BS` (Backspace / Shift+Backspace down, up), `F7` (screenshot `okb_dbg_<frame>.png`), `Q` (quit) |
| `OKB_VRAMDUMP=path` | `debug_env.rs`, `app/capture.rs` | the front end's VRAM written to path on every upload |
| `OKB_RECORD=path` | `debug_env.rs`, `app/replays.rs` | the whole session written there as a replay file at exit |
| `OKB_STATEHASH=1\|path` | `app/mod.rs`, `app/stepping.rs`, `session::HashTrace` | `frame hash` after every step, to stderr (`1`) or to that file |
| `OKB_ROLLBACK=N` | `app/mod.rs`, `app/stepping.rs`, `session/rollback.rs` | the rollback test after every step (N frames back, 1..10000); report to stderr or `OKB_ROLLBACK_LOG=path`, mismatch dumps to `OKB_ROLLBACK_DUMP=dir` (default the temp dir) |

### Display, sound, logging (not recorded)

| Variable | Owner | Effect |
|---|---|---|
| `OKB_DEBUG` (any value) | `main.rs` | the debug overlay at start and the debug tools (`--debug`; keys below) |
| `OKB_PERF=1` | `main.rs`, `app/perf.rs` (`Perf`) | the performance overlay at start (frame times, CPU / GPU time, draw counts, memory) |
| `OKB_LOG=level` | `main.rs`, `platform::support::init` | log level (`info` default; `debug`, `warn` …) |
| `OKB_MUTE` | `debug_env.rs`, `present/fight_scene/mod.rs` | no audio output |
| `OKB_ASPECT=4:3\|wide` | `frontend/options.rs` (`Prefs::from_stored`) | overrides the saved SCREEN MODE (also `43`, `4/3`, `WIDE`) |
| `OKB_WINDOW=WxH` / `WxHpt` | `debug_env.rs`, `app/events.rs`, `app/draw.rs` | the surface size in physical pixels (kept even when the OS clamps the window) / the window size in logical points |
| `OKB_WINDOWPOS=x,y` | `debug_env.rs` | the window position in logical points (two netplay windows side by side) |
| `OKB_CAMYAW=radians` | `present/fight_scene/mod.rs` (reads it), `present/fight_scene/scene.rs` | the debug camera orbit turned at start (as Q / E) |
| `OKB_OLDHUD` | `present/fight_scene/scene.rs`, `present/fight_scene/debug_hud.rs` | the old stand-in HUD drawn over the fight |
| `OKB_VRAMDBG` | `present/fight_scene/scene.rs` | the hit-effect texture pages drawn on screen |
| `OKB_FLARETEST` | `fight/fx/native_tasks.rs` | the sun flare and its models lowered into view (changes the drawing; read inside the fight logic's module) |
| `OKB_SPRITES=old\|ab` | `render/pipelines.rs`, `render/frame.rs` | atlas sprites drawn from the VRAM copy as before / captured frames drawn both ways with the differences logged |
| `OKB_ATLASDUMP=prefix` | `render/atlas_cache.rs` | the sprite atlas layers saved as `prefix_<layer>.png` whenever something was baked |
| `OKB_SOUNDLOG=path` | `audio/mod.rs` (`Audio::new`) | every sound call (and the steps' frame marks) written there, one line each; also in `fetest` |
| `OKB_SWAPDBG` | `fight/sim/frame.rs` | trace: the input swap every 30 frames, to stderr (also in `sim`) |
| `OKB_NETSIM=lat_ms,loss_pct[,jitter_ms]` | `net/link.rs` | netplay over a simulated network (both directions; latency and jitter up to 5000 ms); also `nettest` |
| `OKB_SAVE_DIR=dir` | `platform/storage.rs` (`dir`) | the config directory (save, preferences, replays) instead of `APPDATA` / `XDG_CONFIG_HOME` / `~/.config` + `openkumitebeast`, i.e. `dir/openkumitebeast` (an earlier build's `br2_engine` beside it renamed to it once, under `OKB_SAVE_DIR` / `BR2_SAVE_DIR` too; decided at the first use and kept for the process); `fetest` and `nettest` set it to their own temp dirs |

`OKB_GIT` is not a runtime variable: `build.rs` sets it at compile time (the short commit, `+` when the tree has
changes; `base::build::VERSION`, re-exported by `platform::support`), redone when HEAD, the index, the branch's ref
or a tracked file of the build changes (`src/` with its shaders and fixtures, `Cargo.toml`, `Cargo.lock`, `build.rs`;
in a git worktree too: git names the paths). An edit only outside those (the docs) rebuilds nothing, so the `+` may be
missing until the next rebuild or commit.

## URL parameters (browser build)

Read by `app/mod.rs` (`web_main`) and `app/netplay.rs` (`web_setup`): `?log=level`, `?assets=URL` (an export served over HTTP; without it the game data installed in the browser's storage, OPFS, and
the setup screen when there is none),
`?fight=1&p1=YUG&p2=ALI&stage=ST0` (a direct fight), `?locked=1`, `?debug=1`, `?perf=1`, `?unlock=1` (accepted, does
nothing); netplay `?signal=ws://…&room=CODE` with `?stun=`, `?turn=&turnuser=&turnpass=` and `?delay=N`.

## Headless binaries

### `sim` (`bin/sim.rs`): `sim EXPORT P1 P2 SCRIPT`

P1 / P2 may be an imported fighter (MIT FOX GRE URI; engine addition, the import S7, simulation v15): from
`BR1=dir` (the imported game's install), else the export's `br1` folder; `FORCEMOVE`
then forces a state of its table. `STAGE` may name an imported stage from simulation v16 (EVE00 GRE00 ZIN00 MIT00 GAD00
VAK00 FOX00 RON00 URI00 SMA00 BIG00; the import S8, `fight::stage::br1`), from the same install.

Setup: `STAGE=ST0..STC`, `CPU=1` (P2 is the CPU), `CPU0=1` (P1 too), `LEVEL=hex` (CPU level 00..FF), `DEMO=1` (the
attract demo's rules), `NEED=n` (rounds to win), `HP=pct` (ATTACK LEVEL's HP percentage), `GAUGE=n`, `MODEL=n` with
`PRACTICE=1`, `PRACTICE_D4=hex`, `CONT=1` (continue countdown), `P2DMG=d@frame,…`, `FORCEMOVE=frame:move`,
`POSAT=frame:player:x:z,…` (a base position set after that frame; not recorded: no
`ROLLBACK` / `STEPLOG` with it), `RINGSCALE=pct` (the first round's ring box scaled by pct / 100, its walls open and
not drawn: the team battle's ring survey; not recorded either), `P2SCRIPT=…` (pad 2), `SIM_VERSION=n`, `OKB_COSTUME=a,b`.

Traces (stdout, per frame or on change): `V` (every move change), `P2` (P2's moves), `ROUNDDBG` (round state and
messages), `DMGDBG` (damage, gauge), `HITDBG` (hit tests, stderr, from `fight/duel/hits.rs`), `SNAPDBG` (draw offsets),
`FXVMDBG` (effect VM scripts and models), `CAMDBG` / `CAMDBG2` / `CAMMODE` (camera; the KO replay's framing),
`DARKDBG` (stage darkening), `FADEDBG` (palette fades), `HEADDBG` (the head turn: accumulators, node 2's local
matrix and world position, per frame), `FDBG` / `BOTHDBG` (both fighters), `NODEDBG` with
`NODES=k:n,…` (animation and skeleton nodes), `ANIMDBG`, `VELDBG`, `KODBG`, `THROWDBG`, `BEASTDBG`, `SPECIALDBG`
(special KO), `BREAK=frame:side,…` (the debug wall break at that frame), `STAGEDBG` (stage routine counters; on an imported stage also its G.4A, both forms, the wall counters G.50.. and the debris alive), `SNDDBG` (sounds started), `MUSICDBG` (the music tracks),
`JAWDBG`, `HANDDBG`, `NATDBG` (native effect tasks), `FLAREDBG`, `BR1DBG=n` (every n frames the imported cores' state, clip,
facing, mirror, hands, form and key timing S.CA / S.D2 / S.D0; `SNDDBG` also lists their sounds), `BR1FXDBG` (their effect requests, `Br1Match::fx_log`), `BR1AIDBG` (every frame the imported CPU sides' AI blocks: state, pad, timers, action, band, VM position, random index, flags, r0, depth, level; `LEVEL=hex` sets their level too). From the logic itself (stderr, quiet while a
re-simulation runs, `base::trace::is_muted`): `TRAILDBG` (op 0D trails, `fight/move_script/step.rs`), `SHAPEDBG`
(op 1D node scale, `fight/move_script/step.rs`), `PUSHDBG` (push-apart, `fight/physics.rs`), `SOLIDDBG` (the Q3DM17 teleporter's solid parts, `fight/physics.rs`), `GUARDDBG` (P1's guard
decision, `fight/sim/frame.rs`), `OKB_SWAPDBG`.

Checks: `ROLLBACK=N` (with `ROLLBACK_LOG`, `ROLLBACK_DUMP`; exit 3 on a mismatch), `STATEHASH=1|path`,
`STEPLOG=path` (the run's step log for `replay --trace` on another target).

The team battle (engine addition): `TEAM=2v2|ffa` runs `sim <data> P1 P2 P3 P4` as a
four-CPU match on `STAGE` (ST0, ST2, ST5, ST7, ST8; from simulation v23, T5, also `Q3DM6` with its local files and
`BIG00` with the imported game's install, at their own size) to its end or `MAXFRAMES` (default 30000); a character already
in the match takes the next costume. From v23 a fighter may be an imported one (MIT FOX GRE URI; the install from
`BR1=dir`, else the data folder's; `HITS` logs its hits with the engine that ran them, the throws across the engines as "grab on
… (across the engines: …)" and their ends). `SCRIPT=…` makes P1 a human playing that pad script (by the battle's frame; its
target gets the target ring in `SHOTS`; `SCRIPT2` .. `SCRIPT4` P2 .. P4, pads 3 / 4 as `FrameInput::ext`).
`TEAMTIME=frames` sets every round's time limit, for a time over in a short run: from v25 a tie in the deciding round
ends the match in DRAW GAME; before v25 a SUDDEN DEATH round followed. `ARCADE=enemy[,partner]` (TEAM ARCADE A3, simulation
v26, `TEAM=2v2` only): a TEAM ARCADE battle of the humans' team P1 + P3 with these CPU
levels (0..=0xFF, decimal or 0x hex; the partner's default the enemies'): YOU WIN / YOU LOSE, a loss or DRAW GAME the
CONTINUE? countdown (START from `SCRIPT` / `SCRIPT3` continues: `Continue`), its end GAME OVER (`GameOver`); `ROUNDDBG`
prints the messages. Traces: `ROUNDDBG` (round state, the fighters
out, the wins), `TEAMDBG=n` (every n frames each
fighter's position, head height, damage, target, move, F58; from T7 also each imported core's state, place, root
height, F.50 / F.64, speeds, frame step, key and stun, and each throw across the engines with its proxy), `HITS` (the hit log), `RINGDBG` (the ring), `SNDDBG` (each
sound started, with the SFX queue's length), `SFXSTAT` (at the end: sounds queued and started, the mean wait, the
longest queue, the frames with a backlog or a full queue); checks `ROLLBACK=N`, `STATEHASH`. `TEAMCHECK=1` (T7,
`fight::team::watch`): after every frame a `check:` line for a fighter in the air for more than 240 frames outside a
throw, or with a body sphere's centre more than 0x80 past a wall or Q3DM6's chamfers, and `check: n findings` at the end. `SHOTS` (below) draws the
battle (`FightView::draw_team`). The last line: `result <winner> after <frames> frames (…, µs / frame)`.

Output (engine addition, the import S8; not recorded): `SHOTS=n,m,…` with `SHOTDIR=dir` (default the working
directory) and `SHOTSIZE=WxH` (default 640x480): the fight drawn after those frames' steps through the windowless
renderer (`render::offscreen`, 4:3) into `shot_<frame>.png` — headless screenshots (`SHOTTIME`: at the end, to
stderr, the milliseconds a shot frame took to draw, in `frame_capture`, on the GPU and to save); `SOUNDWAV=file.wav`:
the fight's sound mixed offline (48 kHz, 800 samples a step, as `replay --video`) into a 16-bit stereo WAV — a headless
sound test; at the end it prints to stderr the length in steps and the RMS of each second. From T6 also a team battle
(`TEAM=`: the four fighters' banks, slots 3 / 4 and 7 / 8, the imported game's 3..6, the stage's BGM, the announcer). The
step's sound is mixed before its frame is drawn, so `shot_<n>.png` ends at sample 800·n: with `SHOTS` at every frame
from 1 (e.g. `SHOTS=$(seq -s, 1 N)`) the PNGs at 60 fps and the WAV mux in sync, e.g. `ffmpeg -framerate 60 -start_number
1 -i shot_%d.png -i team.wav -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest out.mp4`.

### `fetest` (`bin/fetest.rs`): `fetest EXPORT`

Keeps its save in `$TMPDIR/okb_fetest` (it sets `OKB_SAVE_DIR`). `FRAMES=n` (default 4000), `FESCRIPT=…` (pad 1),
`P1_MANUAL=1` (P1 does not confirm the select by itself), `WINS=W0,W1,C,G,Xn…` (the fights' results in order:
winner P1 / P2, continue, game over, challenger), `ALL=1` (log every change of the front end's description, not only of the screen), `FE_UNLOCK=1` (everything open, as
F11), `OKB_LOCKED`, `SIM_VERSION=n`, `ROLLBACK=N`, `STATEHASH`, `STEPLOG`, `OKB_SOUNDLOG`. Engine additions (the
the import S9; not recorded): `FESCRIPT2=…` (pad 2, in place of its □ on the select every 40 frames); `FESCRIPT3` /
`FESCRIPT4` (pads 3 / 4: the team battle's screens, T4; a team battle's result from `WINS` too: `Wn` team / player n
won, `D` a DRAW GAME, `C` / `G` a TEAM ARCADE battle's continue / GAME OVER, anything else QUIT; a TEAM ARCADE battle not
run adds 2400 frames to the run's play time); `FESHOTS=n,m,…`
with `SHOTDIR` / `SHOTSIZE` as `sim`'s `SHOTS`: the front end drawn after those frames' steps through the windowless
renderer into `fe_<frame>.png` (headless menu screenshots; a playing movie's frame too, S10); `FELOAD=1`: every match the front end asks for loaded as
the app loads it (`fight_data::load_match`; a team battle `load_team`) and the result printed; `OKB_BR1=dir`: the imported game's install (the
select's imported row and stages from simulation v19, e.g. `OKB_BR1=../okb_export/br1`). It starts the session
through `session_start`, so the app's session-setup hooks above (`OKB_SEED`, `OKB_NAMETEST`, `BR2_MCFAULT`, …) apply.

### Others

- `replay` (`bin/replay.rs`): `replay EXPORT FILE.okbr…`, or `replay --trace EXPORT FILE` (a replay or a step log:
  the state hash after every step).
- `nettest` (`bin/nettest.rs`): `nettest EXPORT [OUT_DIR]`; `NETTEST_PORT`, `NETTEST_SIGNAL` (a matchbox server: a
  WebRTC room instead of UDP), `NETTEST_ROOM`, `NETTEST_DELAY`, `NETTEST_LAG=h,j`, `NETTEST_DESYNC=frame`,
  `NETTEST_LOCKED=h,j`, `NETTEST_STAGE=NAME`, `NETTEST_BR1=1` (both offer the imported game's install of `OKB_BR1=dir`
  or the data folder, and both pick from the select's imported row), `NETTEST_WATCH=1` (the host picks WATCH: both sides
  CPU; with `NETTEST_BR1` imported fighters' AI, the import S10), `OKB_NETSIM`, `OKB_LOG` (default `warn`); sets
  `OKB_SAVE_DIR` to `OUT_DIR/save`.
- `cliptest` (`bin/cliptest.rs`): `V=1` lists every differing key (the main game), every mismatch against the dumps
  (`br1 … dumps`).
- `br1test` (`bin/br1test.rs`): `V=1` prints every step that differs from the recorded run's, with each differing field
  (`br1test BR1_DIR init RUN_DIR…`: every field of a round start's reset that differs, not only the first 40);
  in `frames` mode `STAGE=name` limits the report to one stage (`Inputs`, `Decide`, `Push` …), `COVER=1` adds the
  coverage (the decision's sections run, the recorded events, the states taken per character); in `ai` mode
  `URI=1` and `SYNTH=1` (the engine's AI block after synthetic patches of every dump).
- `sndtest` (`bin/sndtest.rs`): `sndtest BANK.SND [program] [note]`; `TONE=n` renders a tone offline, `OUT=path`
  writes it as WAV.
- `fxvmtest`, `mdectest`: arguments only.

### Tests (`cargo test`)

The tests that need the game's data or a disc image read these; each falls back to the default beside the repo and is
skipped when its data is missing (the `#[ignore]`d ones run by hand with `-- --ignored`).

| Variable | Read by | Default |
|---|---|---|
| `OKB_EXPORT=dir` | the tests on the main game data (`session/tests.rs`, `session/replay/tests.rs`, `fight/team/tests.rs`, `fight/sim/br1_tests.rs`, …) | `../okb_export` |
| `BR1_EXPORT=dir` | the tests on the imported game's install (`fight/team/br1_tests.rs`, `fight/sim/br1_tests.rs`, `session/replay/tests.rs` (the imported fixtures), `session/tests.rs`, `formats/tables.rs`, `formats/br1_anm.rs`) | `../okb_export/br1` |
| `BR1_RUNS=dir` | `fight/br1/frame_tests.rs` (`frame_runs`: the fight frame against recorded reference runs, with the imported game's install) | none: skipped |
| `OKB_REFERENCE=dir` | `formats/xa.rs` (`disc_streams_match_the_exported_mp3`) | `../br2_export`: the old Python export, read only for its MP3 files (a decoding reference) |
| `OKB_DISC=file` | the disc image: `formats/xa.rs` (the XA streams; a raw 2352-byte image, not an ISO) and the ignored `app/setup/convert.rs` `convert_the_disc` (the `.cue`) | `../Bloody Roar II (USA).bin` / `.cue` |
| `OKB_XA_OUT=dir` | the ignored `formats/xa.rs` `write_disc_xa`: every music channel and movie audio of the disc as `.xa` files, laid out as the export's (required; it panics without it) | none |
| `BR1_DISC=file` | the ignored `app/setup/convert.rs` `convert_the_br1_disc` (the second disc's `.cue`) | `~/Downloads/Games/PS1/Bloody Roar (USA)/Bloody Roar (USA).cue` |

## Keys

### Always on (desktop and browser; `app/keys.rs` `debug_key`, `platform::support::HELP`)

| Key | Does | State? |
|---|---|---|
| F1 | overlay: build, fps, screen, round state, both fighters | no |
| F2 / F3 | P2 / P1 at 1 HP (in a fight) | `Cheat::OneHp` |
| F4 | round timer to 1 s | `Cheat::Timer` |
| F5 | SCREEN MODE WIDE / 4:3 (saved preference) | no |
| F6 | both beast gauges full | `Cheat::Gauges` |
| F7 | screenshot `okb_shot_<time>.png` (desktop) | no |
| F8 | support report `okb_report_<time>.txt` (the browser: the console) | no |
| F9 | pause | no |
| F10 | fast forward ×4 | no |
| F11 | everything unlocked for the session (in the front end) | `Cheat::Unlock` |
| `` ` `` | performance overlay | no |
| Alt+Enter | WINDOW / FULLSCREEN | no |
| Esc | quit (desktop) | no |

In netplay F2, F3, F4, F6, F9, F10 and F11 are refused (a toast says so).

### In a fight, outside netplay (`app/events.rs` `window_event`)

`P` pause, `.` one step while paused, `]` / `[` P1's next / previous move (`Cheat::NextMove` / `PrevMove`), `C`
player 2 human / CPU (`Cheat::Cpu`), `B` / `N` beast form of P1 / P2 (`Cheat::Beast`), `Q` / `E` debug camera orbit,
`Z` / `X` zoom (view only).

### Debug tools (`OKB_DEBUG=1`, `--debug`, `?debug=1`; `app/tools.rs`, `platform::support::TOOLS_HELP`)

`P` pause on every screen, `.` step, `F12` save slot, `Shift+F12` load it, `Shift+F1`..`F4` pick slot 1..4 (the
same again: slot 0), hold `Backspace` to rewind (`Shift+Backspace` twice as fast). A restored state stops the memory
card from being written to disk. These keys do not reach the pads while the tools are on (Backspace is P1's SELECT
otherwise).

### Replay playback (the pads; `app/playback.rs`)

START pause, □ one frame, LEFT / RIGHT speed (×0.25 … ×4), L1 / R1 five seconds back / on, L2 the match's start,
UP / DOWN the round's start / the next round, SELECT the HUD, ○ the input display, × the key help, △ back to the
replay list (`--replay`: quit).

### Pads

P1: `WASD`, `J` □, `K` ×, `L` ○, `I` △, `U` R1, `O` L1, `Enter` START, `Backspace` SELECT. P2: arrows, numpad 1..6
(□ × ○ △ R1 L1), numpad Enter START, numpad `.` SELECT. Gamepads map to the same bits ([gamepads.md](gamepads.md)).
