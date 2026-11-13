# Determinism

The simulation (`session/`, `frontend/`, `fight/`: layer L2, see [`../ARCHITECTURE.md`](../ARCHITECTURE.md)) must be a
pure function of its start and its inputs: the same `SessionStart` and the same sequence of `FrameInput`s and
`Cheat`s give the same `GameState` after every step, on every target (macOS / Linux / Windows, aarch64 / x86_64,
wasm32 in the browser) and in every later build that runs the same simulation version. Replays, the rewind, the
rollback test, netplay and the golden baseline all rest on it. Source paths are relative to `src/`; the commands run
from the repository root.

## The rules

1. **Integer first.** The state is the game's integers and fixed point (4.12 matrices, 4096-unit angles, 1/16 and
   1/128 sub-units), computed in the game's order with its rounding: `>>` floors, C division truncates
   (`i32 /` does too), wraparound is explicit (`wrapping_add`, `as i16`).
2. **No float in what is simulated, except bit-exact float.** Where the engine needs float to reach a value the logic
   uses (the posed glTF skeleton for hit spheres, the push-apart and the effect VM's bone matrices; the spring
   chains; the clips built from ACT poses), it goes through `base/portable.rs` and `formats/gltf_pose.rs` from
   simulation version 3: glam's f64 types (one scalar implementation on every target; Rust never fuses `a * b + c`)
   and the trig of the pure-Rust `libm` crate instead of the platform's. IEEE `+ − × ÷`, `sqrt` and the conversions
   are exact everywhere and may be used directly (e.g. `isqrt` in `fight/camera.rs`: `(v as f64).sqrt() as i32`).
   Not allowed in a value that feeds the state: std's `sin` / `cos` / `atan2` / `powf` …, glam's f32 SIMD types,
   `mul_add`, and any sum whose order depends on the target. f32 helpers that only build presentation matrices
   (`Camera::view`, `Camera::projection`, `Match::transform`) are fine: nothing reads them back.
3. **Pinned math crates.** `glam` (`=0.30.10`) and `libm` (`=0.2.16`) are pinned with `=` in `Cargo.toml`: an
   update that changes one result bit is a simulation change (`session::tests::v3_skeleton_bits` fails first).
4. **Random numbers are state.** The game's libc `rand` (`base/libc_rand.rs`) is called with a seed the
   caller owns, and each seed is a field of the state (the front end's, the fight's, the effects', the CPU's
   shuffled random table in `fight/ai.rs`). The session's first seed is `SessionStart::seed` (the clock, or
   `OKB_SEED=n`), recorded in a replay's header and sent to the netplay peer.
5. **Nothing from outside inside a step.** No clock, no environment, no disk, no network, no thread, no `HashMap` /
   `HashSet` iteration order in `session::step`. Everything that comes from outside is an input: the pads and the
   completions (`FrameInput`: `LOADED`, `FETCHED`, `CARD_STORED` / `CARD_FAILED`), the debug mutations (`Cheat`),
   the preferences (`Assets::prefs`), the static data (`Assets`). `tests/layering.rs` forbids the platform crates
   and `std::fs` in L2 and lists every env read and hash map there in its report. The env reads that remain are
   debug traces that only print (`base/trace.rs` mutes them in re-simulations), `OKB_COSTUME` (a direct fight's
   costumes, read at load and recorded in the replay header), `OKB_ASPECT` (a preference, not state) and the debug
   switch `OKB_FLARETEST`. The hash maps there (`session::FileCache`, the clip builders' in `fight/fight_data.rs` and
   `fight/anim/act_clips.rs`) may only be looked up by key, never iterated in an order that reaches the state.
6. **Static data is not state.** `Loaded`, `FrontData` and `FileCache` are `#[serde(skip)]` in the state and put back
   by `session::rebind`; the state may hold only ids and indices into them, which `rebind` checks after a decode.
7. **Output is not state.** Sound commands, requests, rumble (`Out`) go to the app; a re-simulation (rollback test,
   rewind, replay seek) throws them away.

## Versions: which one to bump

The current numbers are the constants themselves (and each one's doc comment lists every bump); this page does not
repeat them.

| Constant | Where | Bump when | Effect of a mismatch |
|---|---|---|---|
| `SIM_VERSION` | `fight/version.rs` | the same start and inputs give other states: any logic change, a behaviour fix, a new menu row, a pinned crate update | a replay of a newer version is refused; an older one runs the old path (`fight::version::Gates`) |
| `STATE_VERSION` | `session/snapshot.rs` | the serialised `GameState` changes: a field added, removed, retyped or reordered, an enum variant added (postcard is field-ordered) | runtime snapshots (rewind, rollback test, desync dumps) of another version or build are refused; a replay compares its full-state checkpoint hashes only when built by the same build and `STATE_VERSION` |
| replay `FORMAT` | `session/replay/mod.rs` | the `.okbr` envelope or a change CBOR's field defaults cannot carry | a newer format is refused; the layouts this build reads are listed in the module doc: FORMAT 1 (pads 1 / 2) and FORMAT 2 (`FORMAT_TEAM`, engine addition: the input stream also holds pads 3 / 4, written only when the header holds a `TeamSpec` or the session used pads 3 / 4, refused before the team battle's simulation version `fight::version::v::TEAM`); every other file is still written as FORMAT 1 |
| save-state `FILE_FORMAT` | `session/snapshot.rs` | a change of meaning in `.okbs` (CBOR is field-named: added fields take their `#[serde(default)]`) | `load_file` migrates (none so far) |
| `PROTO` | `net/proto.rs` | any change of a netplay packet's layout | the peer is refused; the handshake also requires the same `SIM_VERSION`, `STATE_VERSION` and build (`platform::support::VERSION`) |

**the imported game's data** (engine addition, the import): a
match with imported fighters reads the optional second disc's raw files (its static tables, CMN.ACT, the nine `XX31.DAT`), never
anything a converter computed. Their content id (`assets::manifest::br1_content_id`: CRC-32 over those files'
manifest CRCs) is part of the start (`SessionStart::br1`, the replay header; the netplay Hello, since `PROTO` 4): a replay
with one plays only over the same data (`session::replay::br1_fits`), a netplay session keeps it only when both
peers have the same. Imported fighters draw from their own random stream (`Br1Match::rand`), never from
`Match::rand`, so a match without them runs as before.

**The game data's content id** (engine addition): the converter writes `content_id` into `manifest.json`, a CRC-32 over
the CRCs of the files the simulation reads (ok_br2_converter's `docs/content_id.md`; data without it is refused by
`assets::manifest::quick_check` and the app's start: convert it again). The app puts it into every session's start
(`SessionStart::content`, so the replay header; not written when None, so the tools' and the fixtures' starts keep
their bytes): a netplay peer with another id is refused (the Hello's `content`, since `PROTO` 5), a replay recorded
over other data plays with a logged warning (`session::replay::content_warning`), one without the id plays as before.

`frontend/serial.rs` fixes `Front`'s serialised field order: a change to its `flat!` list is a `STATE_VERSION`
change, and a renamed or retyped field also breaks older `.okbs` files. Never rename a serde field or variant, a
`section_hashes` name or a `frontend::menus::Kind::sync_id` number.

## The version gates (`fight::version`)

`SIM_VERSION` is the newest version; a session runs the version of its start (`GameState::sim_version`, a replay's
recorded one, or `OKB_SIM_VERSION=n` / `SIM_VERSION=n` for the headless tools). A change to the simulation keeps
the old behaviour behind the version that introduced the new one, asked as a named rule (R2): `fight/version.rs` holds
`SIM_VERSION` with its changelog, one constant per rule (`v::WIN_RESET`, `v::TEAM_WALLS` …) and `Gates`, the rules of
one version as flags:

```rust
// fight/sim/setup.rs (v6: the match-end reset of round state 0xD / 0xE)
self.round.win_reset = data.gates.win_reset;
// fight/move_script/sound.rs (v4: the beast revert's sound)
if !g.revert_sound { (0x400, 7) } else { (7, 0) }
```

The fight reads the rules from its data (`Loaded::gates`, built with `Loaded::sim_version`), the session and the
front end build them from the state (`Gates::of(s.sim_version)`, `Front::gates`). The comparisons with a version
are in `fight/version.rs` only (`tests/version_gates.rs` refuses one elsewhere), and `version::tests::gates_by_version`
holds every version's rules against a checked-in table. Each bump gets a line in `SIM_VERSION`'s doc comment;
the recipe is in [`../CONTRIBUTING.md`](../CONTRIBUTING.md#7-a-simulation-change-sim_version-bump).

## What depends on it

- **Replays** (`session/replay/`): the start and the inputs only, re-simulated. Checkpoints every `CHECK_EVERY` (60)
  steps hold `sync_hash` (`session/replay/sync.rs`: values every later build computes the same way) and the full
  state hash (`session::hash`, FNV-1a of the postcard bytes; compared only by the build that wrote the file). Old
  replays are kept playing by the `sim_version` gates. `cargo test` checks the fixtures in `session/fixtures/`
  (`ls src/session/fixtures` for the current set; as of 2026-11-07):

  | Version | Replays (`.okbr`) and traces |
  |---|---|
  | v1 | `menu_v1.br2r` (kept under the earlier extension); the save-state file `state_v1.okbs` |
  | v4 | `select_v4`, `versus_v4`, `movies_v4_unlock` |
  | v5 / v6 / v7 | `fight_st3_v5`, `fight_st5_v5`, `revert_v5`; `fight_st9_v6`; `fight_stb_v7` |
  | v10 / v11 (Q3DM17) | `select_q3dm17_v10` / `_v11`, `fight_q3dm17_v10` / `_v11` |
  | v15 (imported fighters, over `../okb_export/br1`) | `fight_br1_v15`, `fight_br1_mixed_v15`; trace `br1_v15_hashes.txt` |
  | v20 (the imported game's AI, ARCADE's boss) | `fight_br1_cpu_v20`, `fight_br1_uri00_v20`, `arcade_br1_boss_v20`; trace `br1_v20_hashes.txt` |
  | v22 (the team battle) | `team_ffa_v22.okbr` (FORMAT 2) with `team_ffa_v22_hashes.txt`; the state-hash trace `team_v22_hashes.txt` |
  | v23 (the team battle's imported fighters) | the state-hash trace `team_br1_v23_hashes.txt` |
  | v24 (the team battle's menus with the imported game) | `team_br1_menus_v24.okbr` with `team_br1_menus_v24_hashes.txt` |
  | v26 (TEAM ARCADE) | `team_arcade_v26.okbr` (the menus, a continue, a retry won) with `team_arcade_v26_br2r_hashes.txt`; the state-hash trace `team_arcade_v26_hashes.txt` |

  The traces (`*_hashes.txt`) hash the fight's or the team battle's postcard bytes at fixed steps; their record
  tests are named in [`../CONTRIBUTING.md`](../CONTRIBUTING.md#8-touching-the-fight-frame-or-a-per-player-resource).
- **Rollback test** (`session/rollback.rs`; `ROLLBACK=N` in `sim` / `fetest`, `OKB_ROLLBACK=N` in the app): after
  every step the state of N steps ago is restored from its snapshot bytes and re-stepped; the hash must match. It
  proves the state is complete (nothing outside it influences a step) and `rebind` correct.
- **Rewind and save slots** (`session/debug_history.rs`): snapshots plus re-simulation.
- **Netplay** (`net/`): no state is sent, only inputs; both sides compare their state hash every 60 frames and stop
  with "desync at frame F" and the section hashes (`session::section_hashes`) when they differ
  ([`netplay.md`](netplay.md)).
- **The golden baseline**: screenshots and traces of fixed scripts, compared byte for byte after every refactor.

## How to check

```sh
cargo test --release                                   # fixtures, v3_skeleton_bits, round trips, rollback tests
ROLLBACK=8 target/release/sim ../okb_export YUG ALI "60:,20:R,1:J,40:"      # exit 3 on a mismatch
ROLLBACK=8 target/release/fetest ../okb_export
target/release/replay ../okb_export src/session/fixtures/*.okbr             # every file in sync: exit 0
STATEHASH=a.hash target/release/sim ../okb_export YUG ALI "60:,1:J,40:"     # the hash after every step
```

The cross-target matrix (scripts in the research tree beside the repo, outside git: `../.research/determinism/`) builds
`replay` for each target one at a time and diffs the per-step hashes of 26 `sim` / `fetest` routes against the
native run:

```sh
../.research/determinism/build_targets.sh native x86_64 wasm     # x86_64 runs under Rosetta 2, wasm under deno
../.research/determinism/run_matrix.sh <empty out dir>           # PASS: every target's every trace identical
```

`run_matrix.sh` takes `REPLAYS=<dir>` to trace stored `.okbr` files too (version 3 and later: older versions ran the
platform's float math and replay only on the platform that recorded them) and `SIM_VERSION=n`. A step log of
any `sim` / `fetest` run (`STEPLOG=file`) can be re-hashed on another target with `replay --trace EXPORT FILE`
(`session/determinism_trace.rs`; on wasm32 the exported `trace_file`).
