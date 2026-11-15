# OpenKumiteBeast (`open_kumite_beast`) — Rust + wgpu

A Rust + wgpu engine that runs a PS1-era 3D fighting game from data the player converts from their own original disc
(the converters are separate tools), for macOS, Windows, Linux and the web: the game logic uses integer math and a fixed
frame order, so it plays frame for frame like the console game; the engine around it is modern code. Engine additions
(netplay, replays, debug tools, WIDE aspect, the GRAPHICS menu) are marked as such. Assets come from the converted
export (made from the disc image by the setup screen's CONVERT DISC IMAGE or by `okconv-br2 convert` of the converter
crate `ok_br2_converter`, e.g. `../okb_export`), including the raw `engine/` pack (`*.ANI`, `*.ACT`, `CMN.ACT`).

| Read | For |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | the layers, the module map, the frame step by step, where to find what |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | build, run, checks, the baseline, fuzzing, the review process, recipes for new work |
| [`docs/determinism.md`](docs/determinism.md) | the simulation's rules, `SIM_VERSION` / `STATE_VERSION`, replays and rollback |
| [`docs/netplay.md`](docs/netplay.md) | playing over the network, how the lockstep works |
| [`docs/debug_hooks.md`](docs/debug_hooks.md) | every `OKB_*` variable, URL parameter and debug key |
| [`docs/glossary.md`](docs/glossary.md) | PS1 and engine terms |
| [`SUPPORT.md`](SUPPORT.md) | reporting a problem, the debug keys, the options |

```sh
cargo run --release -- --assets ../okb_export --p1 YUG --p2 ALI --stage ST0 --fight   # desktop (Metal / Vulkan / DX12)
trunk build --release --public-url ./ && cp -r dist ../okb_export/play          # web: http://localhost:8000/play/?assets=../&fight=1&p1=YUG&p2=ALI&stage=ST0 (without ?assets= the setup screen installs the data in the browser)
cargo run --release --bin sim -- ../okb_export YUG ALI "60:,20:R,30:,1:J,40:"   # headless: every move change for a scripted pad
```

Set `OKB_SAVE_DIR` to a scratch directory for test runs (the memory card, preferences and replays live there).

Controls (P1): `WASD` directions, `J` Square (punch), `K` Cross (kick), `J`+`K` throw, `L` Circle (beast), `I` Triangle, `U` R1, `O` L1,
`Enter` START, or the first gamepad; P2: arrows + `Numpad1`–`6`, or the second gamepad (P2 is the CPU in a direct fight unless
`OKB_P2=human`; `C` toggles it). During a fight the window title shows both fighters' health, the stage, the
round message and P1's move, animation, key, pad and the last unimplemented op it met. Debug keys: [`SUPPORT.md`](SUPPORT.md),
[`docs/debug_hooks.md`](docs/debug_hooks.md).

Unlocks: every character (GADO and SHENLONG too) and every fourth costume (START at the select) is selectable from
the start, and nothing is written to the save for it. The game's own progression (the hidden characters after an
arcade clear, a character's fourth costume after its arcade clear) is behind `--locked` (or `OKB_LOCKED` set to any
value, as the other `OKB_` switches: `OKB_LOCKED=0` locks too) on desktop and `?locked=1` in the browser; it shows the
save's real progress. The stage select (VS, WATCH, CUSTOM VS HUMAN / VS COMPUTER / PRACTICE; L1 / R1) also offers the
four stages the console game never lets you pick (ST9..STC, shown as EXTRA1..EXTRA4 after BUILDING2; RANDOM picks among
all 13); with `--locked` it is the game's nine and RANDOM. Movies, pictures and the CUSTOM items keep the save's
progression (F11 in the debug tools opens them for the session). `--unlock` / `?unlock=1` are accepted and do
nothing. Netplay uses the host's choice; a replay plays with the one it was recorded with.

Netplay (UDP on the desktop, WebRTC rooms in the browser): `okbeast --host 7000` / `okbeast --join HOST:7000`; details in
[`docs/netplay.md`](docs/netplay.md).

## Nix (Linux)

`flake.nix` builds `okbeast` for x86_64 and aarch64 Linux, with a desktop entry and icon (macOS, Windows, the web and
Android use the release packages). Flakes must be enabled (`experimental-features = nix-command flakes`). Like every
build it contains no game data: on first start the setup screen converts your own disc.

```sh
nix run github:openkumite/open_kumite_beast                 # play without a checkout
nix profile add github:openkumite/open_kumite_beast         # install okbeast (older Nix: nix profile install)
nix build                                                   # in a checkout: result/bin/okbeast
nix develop                                                 # the toolchain and libraries for the checks and the web build
```

On NixOS, add the flake as an input and put
`inputs.open_kumite_beast.packages.${pkgs.stdenv.hostPlatform.system}.default` in `environment.systemPackages` (or
`home.packages`). On other distributions a Nix-built program may not find the system's GPU driver; if the game reports
no GPU adapter, start it through [nixGL](https://github.com/nix-community/nixGL) (`nixGL okbeast`, or
`nixVulkanIntel okbeast` / `nixVulkanNvidia okbeast`).

## License

MIT (see `LICENSE`). Third-party code and its licenses: `THIRD_PARTY_NOTICES.md`.

## Trademarks

All game names and trademarks mentioned here belong to their respective owners. This project is not affiliated with or
endorsed by them, and it contains no game data: the player converts the data from their own disc.
