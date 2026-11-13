# Contributing

How to build, run and check the engine, how a change is reviewed, and step-by-step recipes for the usual kinds of
work. Read [`ARCHITECTURE.md`](ARCHITECTURE.md) first for the layout. Paths are relative to the repository root;
`src/…` files are named as in `ARCHITECTURE.md`.

## Setup

- Rust stable (edition 2024) with `rustfmt` and `clippy`; for the web build the `wasm32-unknown-unknown` target and
  [trunk](https://trunkrs.dev); for fuzzing a nightly toolchain and `cargo-fuzz`; for the dependency checks
  `cargo-deny` and `cargo-audit`.
- The converted game export beside the repo: `../okb_export` (made from the disc image by `okconv-br2 convert IMAGE
  ../okb_export` of the converter `ok_br2_converter`, or by the setup screen; not in git). Every tool takes its path; the unit
  tests that need it look there (or at `OKB_EXPORT=dir`) and are skipped when it is missing.
- Optional, the extra stages Q3DM17's and Q3DM6's files (Quake III's maps, from the developer's own copy; never in
  git): the setup screen's QUAKE III ARENA MAPS (`OKB_SETUP_Q3="<Quake 3>/baseq3"`) converts its `.pk3` files into
  the game data's `q3/`, which can be copied to `../okb_export/q3/` (an older `../okb_export/extras/<map>/` is still
  read); without a stage's files it is not offered and its tests are skipped
  ([`docs/extra_stage_q3dm17.md`](docs/extra_stage_q3dm17.md), [`docs/extra_stage_q3dm6.md`](docs/extra_stage_q3dm6.md)).
  The tests that convert the paks themselves (`--ignored`) read `OKB_Q3_PAKS=dir` or `../../19_games/PC/Quake 3/baseq3`.
- Access to the three converter crates `ok_br2_converter`, `ok_br1_converter` and `ok_q3_converter`: private
  repositories in the GitHub organisation `openkumite`, git dependencies on a release tag (`tag = "vX.Y.Z"` in
  `Cargo.toml`, the commit pinned in `Cargo.lock`). Cargo fetches them over SSH with the git command line
  (`.cargo/config.toml`: `net.git-fetch-with-cli`), so an SSH key with access to the organisation must be loaded
  (`ssh -T git@github.com` answers with your login); a CI needs a deploy key or a token. Cargo reads their manifests
  for every build, also `--no-default-features`, and the fuzz crate needs them too. A converter change reaches the
  engine only through GitHub: push and tag the converter (`git tag -s vX.Y.Z`), raise the `tag` and `version` of all
  its entries in `Cargo.toml` (desktop and wasm32), then `cargo update -p <crate>`.
- The tooling tree beside the repo, outside git: `../.research/` (the golden baseline and the determinism scripts).
- `cargo test` installs the git hooks (cargo-husky, scripts in `.cargo-husky/hooks/`): fmt + clippy before a commit;
  tests, cargo-deny and cargo-audit before a push.

**Memory rule.** A full build peaks at about 1.6 GB per rustc process; run one heavy job (build, test, trunk,
baseline) at a time with `CARGO_BUILD_JOBS=4`. Fuzz builds compile the whole GPU / window / audio stack with
sanitizers (`fuzz/target` is about 4 GB): build them alone with `CARGO_BUILD_JOBS=2`.

**Your save.** The desktop app keeps the memory card, the preferences and the replays in the user's config
directory. For any test run set `OKB_SAVE_DIR` to a scratch directory (`OKB_SAVE_DIR=$(mktemp -d)`); add
`OKB_MUTE=1` for silence.

## Build and run

```sh
cargo build --release --bins                     # the app (target/release/okbeast) and the tools
OKB_SAVE_DIR=$(mktemp -d) cargo run --release -- --assets ../okb_export                       # from the boot logos
OKB_SAVE_DIR=$(mktemp -d) cargo run --release -- --assets ../okb_export --fight --p1 YUG --p2 ALI --stage ST0   # a direct fight
cargo run --release -- --help                    # every option and the main OKB_* variables
```

Without `--assets` the app reads the game data folder beside the save (`<config dir>/openkumitebeast/game_data`, so
`OKB_SAVE_DIR` moves it too), checked at every start (`assets::manifest`: the format, every listed file's size, the
engine's required set); `--assets DIR` is the developer's override (checked the same way, a failure ends the program).
Missing or unusable data (or a start that cannot read it) opens the setup screen (`src/app/setup/`), which copies a
folder of converted data in, checks every file's CRC-32 and starts the game. Scripted runs of it:
`OKB_SAVE_DIR=$(mktemp -d) OKB_SETUP_FROM=../okb_export OKB_SETUP_SCAN= OKB_UISHOTS=10,45 okbeast` (the source dropped at
UI frame 30, installed at 60; `OKB_SETUP_SLOW=ms` slows the copy for shots of its progress). With the `converter` feature (default; the
crate `ok_br2_converter`, a git dependency, see Setup) the screen also converts a disc image (`.cue` / `.bin` / `.chd` /
`.iso`; `OKB_SETUP_FROM` may name one); `cargo test --release -- --ignored convert_the_disc` converts `../Bloody Roar II
(USA).cue` end to end, and `--no-default-features` leaves it out of the program (only converted folders; Cargo still
fetches the crate). `OKB_SETUP_SLOW` slows a conversion too (a pause per step and per file checked).
The optional second disc (`game_data/br1/`: its fighters and stages on the select from simulation v19): an install at start ends on a page
offering ADD BLOODY ROAR 1 DISC, and OPTIONS → CHANGE GAME DATA offers ADD / REPLACE / REMOVE BLOODY ROAR 1 DISC
(its BIN/CUE with both tracks, a CHD or ISO, or a folder this setup converted before); `OKB_SETUP_BR1=PATH` adds
PATH wherever that row is offered, e.g. `OKB_SETUP_FROM="../Bloody Roar II (USA).cue" OKB_SETUP_BR1=".../Bloody Roar
(USA).cue"` installs both at start; `cargo test --release -- --ignored convert_the_br1_disc` converts
`~/Downloads/Games/PS1/Bloody Roar (USA)/Bloody Roar (USA).cue` (or `BR1_DISC`).

Web (the browser build has netplay over WebRTC on, `index.html`):

```sh
trunk build --release --public-url ./ && cp -r dist ../okb_export/play
# serve ../okb_export over HTTP, then open http://localhost:8000/play/?assets=../&fight=1&p1=YUG&p2=ALI&stage=ST0
```

Without `?assets=URL` the page uses the game data installed in the browser's own storage (OPFS, a folder
`data-<n>` named by localStorage `okb_data_dir`), checked at start; missing data opens the setup screen, whose
CHOOSE FOLDER... copies a folder of converted data in (`src/app/setup/web.rs`); CONVERT DISC IMAGE's CHOOSE
FILE... (a `.cue` with its `.bin`, or a `.bin`, `.chd`, `.iso`) converts it in a Web Worker (the `convert_worker`
bin, `src/app/setup/worker.rs`, linked by `index.html`; about two minutes in Chrome). A private window without OPFS is
told to use a normal one.

Android (a debug-signed APK; the game data is not in it: the setup screen copies the player's converted folder on
first start; design and lifecycle in [`docs/android.md`](docs/android.md)):

```sh
android/build.sh                                 # arm64-v8a → android/build/okbeast.apk (ABIS="arm64-v8a x86_64" for both)
adb install -r android/build/okbeast.apk             # then start "OpenKumiteBeast" and choose the copied okb_export folder
adb logcat -s okb                                # the engine's log
```

It needs the Android SDK (`ANDROID_HOME`, build-tools, `platforms/android-35`), an NDK, `cargo-ndk`, a JDK and the
`aarch64-linux-android` Rust target. Clippy for the target: `cargo ndk -t arm64-v8a -P 26 clippy --release --lib --
-D warnings`.

Scripted runs, screenshots and every debug switch: [`docs/debug_hooks.md`](docs/debug_hooks.md). Netplay:
[`docs/netplay.md`](docs/netplay.md). Gamepads (layouts, players, the user's mapping file, the connect log):
[`docs/gamepads.md`](docs/gamepads.md).

## Checks

`unsafe` is denied crate-wide (`[lints.rust] unsafe_code = "deny"` in `Cargo.toml`). The only allowed sites are the
platform boundary (`platform/android.rs`, the FFI in `platform/support.rs`, the Android entry in `lib.rs`) and the
`fetest`/`nettest` start-up `set_var`; each carries `#[allow(unsafe_code)]` and a SAFETY note. New `unsafe` elsewhere
fails the build.

Run before every commit (the git hooks run `fmt` and the first `clippy` before a commit, `test`, `deny` and
`audit` before a push; the others are yours to run):

```sh
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo clippy --release --all-targets --features netplay-webrtc -- -D warnings
cargo clippy --release --target wasm32-unknown-unknown --features netplay-webrtc --bins -- -D warnings
cargo clippy --release --target wasm32-unknown-unknown --no-default-features --features netplay-webrtc --bins -- -D warnings
cargo ndk -t arm64-v8a -P 26 clippy --release --lib -- -D warnings      # Android (cargo-ndk, an NDK)
cargo test --release                             # unit tests, replay fixtures, rollback tests, tests/layering.rs,
                                                 # tests/android_icon.rs (android/res as it writes it),
                                                 # tests/original_text.rs (no game text in the code)
                                                 # tests/no_original_refs.rs (no addresses of the recreated program,
                                                 # research notes or porting tags; its ALLOWED lists only shrink)
cargo deny check && cargo audit
trunk build --release --public-url ./
```

After any change to the dependencies (`Cargo.toml`, `Cargo.lock`), run `tools/third_party_notices.sh`: it rewrites
`THIRD_PARTY_NOTICES.md` from `cargo tree -e normal,build --target all --features netplay-webrtc` (every target, the
union of the desktop, web and Android builds; path crates left out). A new license must also pass `deny.toml`'s allow
list, and a notable one (copyleft, a data license, extra notices) gets a line in the file's header in the script.

### Tools

| Tool | Command | What it checks |
|---|---|---|
| `sim` | `target/release/sim ../okb_export YUG ALI "60:,20:R,1:J,40:"` | a fight headless through `session::step`; env traces (`V`, `ROUNDDBG`, `HITDBG`, `FDBG` …, list in `docs/debug_hooks.md`) |
| `fetest` | `target/release/fetest ../okb_export` (`FESCRIPT=…`, `WINS=…`) | the front-end flow headless: title, selects, ladder, story, routing |
| `replay` | `target/release/replay ../okb_export FILE.okbr…` | re-simulates replays and compares every checkpoint (exit 0 = in sync) |
| `replay --video` | `target/release/replay --video out.mp4 [--size 1080p] [--aspect 4:3] ../okb_export FILE.okbr` | renders a replay into a 60 fps video with its sound, without a window, through the system's ffmpeg ([`docs/replay_video.md`](docs/replay_video.md)) |
| rollback test | `ROLLBACK=8 target/release/sim …`, `ROLLBACK=8 target/release/fetest …`, `OKB_ROLLBACK=8` in the app | the state is complete and the steps deterministic (exit 3 on a mismatch) |
| `nettest` | `target/release/nettest ../okb_export` (`OKB_NETSIM=80,5`) | a whole netplay session between two headless peers |
| `fxvmtest` | `target/release/fxvmtest ../okb_export` | every effect script spawned and run for 300 frames |
| `cliptest` | `target/release/cliptest ../okb_export YUG`; `cliptest br1 BR1_DIR dumps DUMP_DIR…` | runtime ACT clips against the converter's; the imported game's pose code against recorded RAM dumps (also `shot`, `dumpshot`, `pose`, `fixture`: the file's header) |
| `br1test` | `target/release/br1test BR1_DIR [carried \| null] RUN_DIR…`; `br1test BR1_DIR frames [carried] RUN_DIR…` | the imported game's script step against recorded reference runs at every step; `frames`: every stage of the fight frame and its requests against the recorded runs; `br1test BR1_DIR init RUN_DIR…`: the round-start reset (`fight/br1/init.rs`) at every round start of those runs; `br1test BR1_DIR ai DUMP_DIR…` (`URI=1`, `SYNTH=1` against a synthetic reference): the CPU AI (`fight/br1/ai.rs`) on every frame of recorded RAM-dump runs |
| `sndtest`, `mdectest` | `sndtest BANK.SND [program] [note]`, `mdectest BSF FRAME OUT.ppm` | a sound bank, a movie frame |

### Replays as videos

`replay --video OUT [options] [EXPORT_DIR] FILE.okbr` (desktop) draws every step of a replay with the app's own
`present` / `render` code into an offscreen target (`render/offscreen.rs`), mixes its sound offline (48 kHz, 800
samples a step) and pipes both into the system's ffmpeg (found on `PATH`, or `--ffmpeg PATH`): H.264 through
VideoToolbox when it works, else libx264, with AAC, by default at 3840×2160 (`--size 1440p|1080p|720p|WxH`), in the
stored SCREEN MODE (`--aspect 4:3|wide`). `--codec ffv1` writes a lossless `.mkv`, to compare frames with the app's
screenshots (`OKB_WINDOW=WxH OKB_LOCKSTEP=1 OKB_NOPLAYUI=1 OKB_SHOTS=F okbeast --replay FILE`: video frame `F - 1` of a
direct fight). The video has one frame per step (frame *n*, from 0, = the state after step *n* + 1), the whole file from its
first step, no overlay; a file out of sync stops it with an error. Set `OKB_SAVE_DIR` as for the app (the screen mode
comes from the stored preferences). Design, options and checks: [`docs/replay_video.md`](docs/replay_video.md).

### The golden baseline (every refactor, every visible change)

`../.research/refactor_baseline/run_baseline.sh` runs a fixed set of outputs: `sim` / `fetest` traces, the headless
shots of the team battle and TEAM ARCADE (`sim SHOTS`, `fetest FESHOTS`) and the app's screenshots (fights and menus, each app route with its own `OKB_SAVE_DIR`);
`compare.sh` compares two runs byte for byte and prints the count (`PASS: N files identical (M screenshots)`; on
2026-11-07, golden of SIM_VERSION 25: 241 files, 136 screenshots; with TEAM ARCADE's six routes, A5, SIM_VERSION 26:
283 files, 160 screenshots; `fe_team_arcade_br1` reads the imported game's data from `../okb_export/br1`). The routes are listed in
`../.research/refactor_baseline/README.md`. It builds nothing and runs `open_kumite_beast/target/release/*` of the project root (a worktree: run a copy of the
script with its `ENGINE=` line pointed at the worktree). Keep the window visible while the app routes run.

```sh
cargo build --release --bins
../.research/refactor_baseline/run_baseline.sh /tmp/okb_after          # ~2 min; BASELINE_FAST=0: every frame drawn
../.research/refactor_baseline/compare.sh ../.research/refactor_baseline/golden /tmp/okb_after   # PASS: N files identical (M screenshots)
```

`ROUTES=<regex>` re-runs single routes (a partial run; do not compare it with the whole golden).

### The determinism matrix (every simulation or math change)

```sh
../.research/determinism/build_targets.sh native x86_64 wasm   # one target at a time
../.research/determinism/run_matrix.sh /tmp/okb_matrix          # PASS: 26 traces identical on every target
```

Background and the version rules: [`docs/determinism.md`](docs/determinism.md).

### Fuzzing

Sixteen targets in `fuzz/fuzz_targets/`: `container_read`, `act_parse`, `ani_parse`, `vab_parse`, `movie_parse`,
`tim_load`, `settings_json`, `input_script`, `load_replay`, `load_state`, `q3_map_parse` (input: a little-endian u32
length, the glb, the shader table's JSON), and the imported game's (the import S11): `br1_anm_parse` (`XX31.DAT` /
`CMN.ACT`), `br1_tables` (any bytes parsed as a `tables.okt`, its imported check, `Br1Tables` and `Br1Res` over it),
`br1_art` (the select portraits' containers), `br1_ai_state` (a decoded AI block, run when `Br1Ai::valid` accepts it),
and the team battle's (T6): `team_state` (input: a mode byte, then postcard bytes: a `TeamFight` decoded, a `TeamRound`
that `check` accepts ticked for 600 frames on four blank fighters, the targets kept enemies). Seeds for the imported
targets: the imported install's `engine/*/XX31.DAT`, `engine/CMN/CMN.ACT`, `engine/tables.okt`, `engine/CMN/SELE00.DAT`.

```sh
CARGO_BUILD_JOBS=2 cargo +nightly fuzz build          # alone: nothing else building (memory rule)
cargo +nightly fuzz run load_replay
```

Seed `fuzz/corpus/<target>/` with real files for faster coverage; corpora and crash artifacts are not committed.
A `q3_map_parse` seed from the extra stage's files (`f=../okb_export/q3`, `g=$f/baseq3/maps/q3dm17.glb`):
`{ perl -e 'print pack("V", -s $ARGV[0])' $g; cat $g $f/shaders.json; } > fuzz/corpus/q3_map_parse/q3dm17`.

## Review process

Every change to the code goes through the review loop before it is committed or merged (project `CLAUDE.md`,
workflow `review-fix-loop`): a reviewer reads `git diff <base>` and the code it touches, runs fmt / clippy / tests
and returns findings by severity; blocking and major findings are fixed (or declined with a reason, e.g. fidelity to
the game's behaviour) and the change is reviewed again, until approved. Moves are done with `git mv` and reviewed with
`git diff -M` (`--color-moved=dimmed-zebra` shows what changed inside moved code). State in the review context
which baseline, matrix and tool runs you did and their results.

Style: match the surrounding code (names, comment density, the 160-column `rustfmt.toml`); comments describe
behaviour, data and invariants; mark anything the console game does not have as an engine addition; no
speculative abstractions; keep every file's `//!` header current (`ARCHITECTURE.md`, "Module headers").

## Recipes

Each recipe: the files to touch, an existing example to copy, how to verify. Every recipe ends with the checks
above and the review loop; anything that changes what the simulation computes is also recipe 7.

### 1. A new piece of game logic

1. Write down the behaviour first: what the step reads and writes, its place in the frame order, the tables it uses
   and the invariants it keeps (ranges, wraparound, which fields it may touch).
2. Choose the module by domain (`ARCHITECTURE.md`, "Where to find what"): fight logic under `fight/`, front-end
   screens under `frontend/`. New file: add it to `tests/layering.rs`'s `FILES` and its parent's `mod` list, and
   give it the `//!` header (a one-line purpose and `State:`).
3. Keep the arithmetic exact: integer types, shifts, truncating divisions, wraparound and update order as the
   behaviour needs them; fields keep their serialised ids when the meaning is unknown (`fight/fighter.rs`'s `o_XX` /
   `s_XX`); the game's static tables are read through `Loaded::tables` (`formats::tables`, `engine/tables.okt`) by
   name and offset, not copied (a table the converter does not provide yet: add it to ok_br2_converter's
   `gamedata::TABLES` and `tables::BR2`). Example to copy: `src/fight/throw.rs` (one function per step, each
   with a doc comment), called from its place in the frame order in `src/fight/sim/frame.rs`.
4. Add a trace: a `sim` env flag in `src/bin/sim.rs` that prints the state you changed (as `THROWDBG`), or, inside
   the logic, `if std::env::var("X").is_ok() && !crate::base::trace::is_muted()` (as `PUSHDBG` in
   `src/fight/physics.rs`); list it in `docs/debug_hooks.md`.
5. Pin it down: a unit test of the pure parts (integer math, state machines) and, for a change a match can see, a
   replay fixture or a state-hash trace (recipe 7).
6. Verify: `cargo test`, the `sim` routes you touched, the baseline (expected identical unless the change fixes what
   was wrong before: then recipe 7).

### 2. A new effect

Choose the kind by how the game runs it:

| Kind | Files | Example to copy |
|---|---|---|
| hit-effect task (the hit code's lists 6 / 8 / 10 / 11 / 14) | `src/fight/fx/hit_effects/mod.rs` (a `Kind` variant and its task type, the spawner), `update.rs` (its arm of `update`), `quads.rs` (what it draws) | the ring bursts (`Kind::RingParent` / `RingChild`, `ring_spawn` in `mod.rs`, their arms in `update.rs`) |
| effect-script instruction (EFT.DAT bytecode) | `src/fight/fx/script_vm/ops.rs` (or inline in `interp.rs`'s `exec`), `interp.rs` (`op_size`) | `op00` / `op02` in `ops.rs` |
| native task with 3-D effect models | `src/fight/fx/native_tasks.rs` (a `Task` variant, its spawn function, `tick`, `prims`) | `wall_mark` |
| move-script effect (ops 0x16 / 0x17 / 0x18 sub-types) | `src/fight/fx/move_effects.rs` (a task struct and its `Task` variant, its spawn function, its arm in `tick`, its part of `prims`), dispatched from `src/fight/fx/script_link.rs` (`effect_op`) | `comet` (op18 type C: `Comet`, `Task::Comet`) |

The task state is part of the fight's state (`State: serialised`): new fields change `STATE_VERSION`, and an
effect that was missing before changes the simulation (`SIM_VERSION`, recipe 7) when its task touches anything
else (a random seed, a list slot, a fighter). Verify: `target/release/fxvmtest ../okb_export`, the `sim` traces
`FXVMDBG` / `NATDBG`, and app screenshots of the move (`OKB_FORCEMOVE=frame:move` with `OKB_SHOTS`), compared with
the baseline.

### 3. A new stage feature

- Ring, walls, floor records (logic): `src/fight/stage/mod.rs` (`Stage`, `Walls`; read from the `stage.*` tables).
- A stage's per-frame routine (moving objects, palette cycling, flicker): `src/fight/stage/anim.rs` (the
  `ROUTINES` / `OBJECTS` tables, `step`, `Counters`, `Rows`).
- Drawing it: `src/present/fight_scene/stage_draw.rs` (`stage_anim`, `stage_ot`, `semi`, `backdrop`); new GPU
  inputs in `src/render/` only through the existing `Renderer` API.
- Loading what it needs: `src/fight/fight_data.rs` (`load_match`, fields of `Loaded`).

Example to copy: the ST5 wall lights (`src/fight/stage/anim.rs` routine table entries and the tests `st5_far_wall_lights`,
`st3_sea_phases` in the same file). Verify: `STAGEDBG=1 target/release/sim ../okb_export YUG ALI "…" ` with
`STAGE=ST5`, app screenshots `--fight --stage ST5` with `OKB_SCRIPT` / `OKB_SHOTS`, the baseline.

### 4. A new menu or screen

- A list menu: `src/frontend/menus/mod.rs` (a `Kind` variant and **a new `Kind::sync_id` number: never reuse or
  renumber one**, its rows, `Action`s if it asks the front end for something), `actions.rs` (what confirm and ←/→
  do: copy `sound_action`), `draw.rs` (its rows), `flow.rs` (opening and leaving it from `Front`). Example: the
  GRAPHICS menu (`Kind::Graphics`, `graphics_action`), an engine addition.
- A screen: a `Screen` variant in `src/frontend/screen.rs`, its arm in `tick.rs` (`tick_screen`) and in `draw.rs`,
  the decoded-state checks in `check.rs` (`check`: every index in range), its sync values (`sync_key`) and its
  text (`describe`); a large screen gets its own file (as `vs.rs`). New `Front` fields go into
  `src/frontend/serial.rs`'s `flat!` list.
- Both change the serialised state (`STATE_VERSION`); a change to an existing flow (a new row in the mode menu) is a
  `SIM_VERSION` change with the old layout behind the version (example: v2's REPLAY row, `frontend::screen::mode_rows`).
- Verify: a `fetest` route (`FESCRIPT=…`, the log of the front end's descriptions), app screenshots (`OKB_NOINTRO=1
  OKB_FESCRIPT=… OKB_FESHOTS=… OKB_FEEXIT=1`), the replay fixtures still play, and the baseline (new screenshots:
  ask for a new golden, recipe 7).

### 5. A new debug tool or `OKB_*` hook

- A key: an F-key in `src/app/keys.rs` (`debug_key`), a fight's letter key (`[` `]` C B N P `.` Q E Z X) in the
  fight-key match of `App::window_event` (`src/app/events.rs`, all off in netplay); help text in
  `platform::support::HELP` (or `TOOLS_HELP` for the debug tools of `src/app/tools.rs`); F-keys that change one side
  only are refused in netplay in `debug_key`.
- Anything that **changes the state** must be a `Cheat` (`src/session/state.rs`): applied by `session::step`
  (`src/session/step.rs`, `cheat` at the step's start or in `frame`), so that replays record it and the rollback
  test re-applies it; validate a replay's value in `cheat_ok` (`src/session/replay/mod.rs`). Netplay turns cheats
  off. Example: `OKB_BREAK` → `Cheat::Break` (`src/app/debug_env.rs`, `src/app/stepping.rs` `env_cheats`,
  `src/session/step.rs` `frame`).
- An env hook: a field of `DebugEnv` read in `DebugEnv::read` (`src/app/debug_env.rs`; all off on wasm32). Session
  setup goes into `SessionStart` (`src/session/start.rs`, a `#[serde(default)]` field: it is part of a replay's
  header) through `DebugEnv::start`. A trace that only prints may read the env inside the logic (recipe 1, step 4).
- Document it in `docs/debug_hooks.md` (owner, effect, whether it is recorded).
- Verify: the tool with and without netplay; a replay recorded with it (`OKB_RECORD=path`) plays in sync
  (`target/release/replay`); `OKB_ROLLBACK=8` stays silent.

### 6. A new netplay message

1. `src/net/proto.rs`: a `Msg` variant at the end, its kind byte (the next number) in `Msg::kind` and its arm in
   `decode`'s `match kind`, its fields in `encode` and `decode` (every length capped, no trailing bytes), a row in the
   header's table, and **`PROTO` + 1**.
2. Handle it: during the connect phase in `src/net/handshake.rs`, during the session in `src/net/mod.rs`
   (`Netplay`); the app's part in `src/app/netplay.rs`.
3. Tests: add it to `proto.rs`'s `samples` (so `round_trip` and `garbage_rejected` cover it), and a handshake / lockstep
   test with the in-memory link (`src/net/link.rs`) for its handling, malformed values included.
4. Verify: `target/release/nettest ../okb_export`, again with `OKB_NETSIM=80,10` and with `NETTEST_DESYNC=600`;
   the app as host and joiner on 127.0.0.1 (`--host 127.0.0.1:7000` / `--join 127.0.0.1:7000`, each with its own
   `OKB_SAVE_DIR`). The packet layout is documented in [`docs/netplay.md`](docs/netplay.md) and `proto.rs`.

### 7. A simulation change (`SIM_VERSION` bump)

Any change that makes the same start and inputs give other states: a behaviour fix, a missing effect, a menu row.

1. **Before the change**, record fixtures with the current build: a replay covering what changes (the app records
   every finished match in its replay store; `OKB_RECORD=path` writes the whole session at exit), saved as
   `src/session/fixtures/<what>_v<current>.okbr`; check it with `target/release/replay ../okb_export FILE`.
2. Add a "still plays" test in `src/session/replay/tests.rs` that decodes it with `include_bytes!` and runs
   `simulate` in sync (copy `v5_fights_still_play`), and, if useful, a test of the new behaviour at the new version
   (copy `v6_round_and_match_end_reset`).
3. Bump `SIM_VERSION` in `src/fight/version.rs` and add its line to the doc comment (what changed).
   Name the rule: a constant in `version::v` (named for the rule, not the stage: `TEAM_WALLS`, not `T7_SIM`), its
   flag in `Gates` and `Gates::of`, and append the version's row to `version::tests::TABLE` (an old row never
   changes).
4. Keep the old path behind the flag: `if data.gates.rule { new } else { old }` in the fight (`Loaded::gates`),
   `Gates::of(s.sim_version)` in the session, `Front::gates()` in the front end
   ([`docs/determinism.md`](docs/determinism.md)). `tests/version_gates.rs` refuses a raw `sim_version` comparison
   outside `fight/version.rs`.
5. If what is serialised changed (a field, a variant, `Front`'s `flat!` list): bump `STATE_VERSION` in
   `src/session/snapshot.rs` with its doc line, give new fields `#[serde(default)]` (older `.okbs` files load) and
   extend `rebind` / `check` for new ids and indices.
6. Verify: `cargo test` (the new and the old fixtures), `target/release/replay` over `src/session/fixtures/*.okbr`,
   the rollback test, the determinism matrix, and the baseline. If pixels or traces change, list every changed file
   from `compare.sh` with the reason; the user approves a new golden, which is then re-recorded (two runs that
   compare PASS) and the old one kept as `golden_pre_<change>` in `.research/refactor_baseline/`.

### 8. Touching the fight frame or a per-player resource

The team battle (engine addition) runs the match's routines
over four fighters in a frame of its own. A change to a match is not automatically a change to the team battle.

- **Mirror the frame order; share the step.** `fight/sim/frame.rs` (`Match::tick`) and `fight/team/frame.rs`
  (`TeamFight::tick`) list the same steps in the same order: both orchestrators stay (they are the documented frame
  orders). A step that runs alike in both is one kernel both call: `fight/duel/frame.rs` (the sound queues, the flash
  tasks, the fades, the skeleton's parts and tail, the stage update), `fight/duel/br1.rs` (the imported requests
  and stage sounds, walls, context, the cross-engine proxies and hit kernels), `fight/duel/` `mod.rs` / `hits.rs`
  (a fighter's input and step, the hit pairs), `fx::effect_lists`, `camera::Subject::of`. Change a step there and
  both frames have it; a step added or moved in one orchestrator needs its call in the other, with the opponent as
  the fighter's target (`self.target[k]`) and per-fighter state indexed by fighter, not by player. Where the two
  differ (the M.9E check, the pause's freeze, the CONTROLLER CONFIG.'s open timing) the caller passes it in; the
  v25 wall lock is the caller's to set before `duel::frame::stage_step` (the team sets `walls.lock = 1` first; set
  it after the call and the team's walls can break). The team frame leaves out the KO replay, the challenger, the attract demo, the scripted cameras and
  the vibration (its header says so); say in the change when a step stays match-only.
- **Per-player resources follow the bank rule** (`fight/side.rs`). Fighter `k` plays as player `k & 1` (its slot)
  with copy `k >> 1` (its bank) of every per-player resource. A new `[_; 2]` by player in `World`,
  `native_tasks::State` or the effect VM needs one of these:
  - a per-fighter copy swapped into the slot around fighter `k`'s step (`team::PlayerExt`: `p4b18`, `hud_flash`);
  - a bank-1 copy (`native_tasks::SideExt` through `FxHost::ext`; the VM's groups 5 / 6, `Vm::groups_ext`);
  - an array by fighter on `TeamFight` (`flash_task`, `back_colour`).
  Two fighters of one slot are on stage at once; an index by `player` alone silently shares the resource between
  them.
- **Draw per-fighter state from the scene.** `present` reads a fighter's state through `fight::scene::SceneRef`
  (`SceneRef::fade(i)` for the palette fade), never through `World`'s slot `player & 1`. In a team battle that slot
  holds whichever of its two fighters stepped last (T8: Busuzima's cloak hid his teammate).
- **Fixtures.** The team fixtures are state-hash traces, each with its own record test (run by hand,
  `cargo test --release <name> -- --ignored`, after an intended change):

  | Trace | Checked by | Recorded by |
  |---|---|---|
  | `session/fixtures/team_v22_hashes.txt` | `fight::team::tests::team_fixtures` | `team_fixture_hashes_record` |
  | `session/fixtures/team_br1_v23_hashes.txt` | `fight::team::br1_tests::br1_team_fixtures` | `team_br1_fixture_hashes_record` |
  | `session/fixtures/team_ffa_v22.okbr` + `team_ffa_v22_hashes.txt` | `session::replay::tests::team_ffa_fixture` | `team_ffa_fixture_record` |
  | `session/fixtures/team_br1_menus_v24.okbr` + `team_br1_menus_v24_hashes.txt` | `session::replay::tests::team_br1_fixture` | `team_br1_fixture_record` |
  | `session/fixtures/team_arcade_v26_hashes.txt` | `fight::team::tests::team_arcade_fixtures` | `team_arcade_fixture_hashes_record` |
  | `session/fixtures/team_arcade_v26.okbr` + `team_arcade_v26_br2r_hashes.txt` | `session::replay::tests::team_arcade_fixture` | `team_arcade_fixture_record` |

  They hash `TeamFight`'s postcard bytes, so a change of its layout re-records them. A change of behaviour keeps the
  old path behind the version (recipe 7) and keeps the old traces unchanged.
- **The check matrix.** Run the team battle headless with the wall / air check and the rollback test over several
  line-ups and stages (with the imported game's install for MIT FOX GRE URI and BIG00):

  ```sh
  TEAM=2v2 STAGE=Q3DM6 TEAMCHECK=1 ROLLBACK=8 MAXFRAMES=12000 target/release/sim ../okb_export FOX MIT GRE URI
  ```

  Pass: `check: 0 findings` and exit 0 (exit 3 is a rollback mismatch). T7's matrix: 8 line-ups × ST0 / ST2 / ST5 /
  ST7 / ST8 / BIG00 / Q3DM6 × 2v2 / ffa.
- **The baseline** has four team routes (`team_2v2_*`, `team_ffa_*`) and six TEAM ARCADE routes (`team_arcade_*` by
  `sim ARCADE=`, `fe_team_arcade_*` by `fetest` with `FESHOTS`); a team change that moves pixels or round traces lists them as
  recipe 7 does.
