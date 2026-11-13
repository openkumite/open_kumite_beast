# Extra stage Q3DM17: Quake III Arena's "The Longest Yard"

An engine addition: a stage the console game does not have, drawn from Quake III Arena's map q3dm17, converted from the
player's own copy of Quake III (its `.pk3` files) by the setup screen's QUAKE III ARENA MAPS, through
ok_q3_converter (the Quake III converter, a separate crate, a git dependency). The files are
the player's local data: never in git, never in the converted game data, never in a build. Without them nothing changes:
the stage is not offered.

Code: `fight/stage/extra.rs` (number, code, stand-in, ring, units), `fight/stage/mod.rs` (`arena_bounds`),
`assets/extras.rs` (the files: the Quake III install's check, `source`, `present`), `formats/q3_map.rs` (the map and
its shaders), `fight/fight_data.rs` (`load_arena`, `Loaded::arena`), `render/lightmapped.rs` +
`render/shaders/lightmapped.wgsl` (the pipelines), `present/fight_scene/arena.rs` (the map in the fight's space, the
sky), `app/setup/q3.rs` + `web.rs` / `worker.rs` (the setup's conversion). The second Quake III stage, Q3DM6, shares
all of it: [`extra_stage_q3dm6.md`](extra_stage_q3dm6.md).

## The files

The Quake III install is the folder `q3/` of the game data folder (or `--assets`; the browser: its own OPFS folder
`q3-<n>`), in ok_q3_converter's layout: every map of the paks as `baseq3/maps/<map>.glb` (glTF, Y up, 1 Quake unit =
0.0254 m; the world in the node `worldspawn`, the lightmap atlas on `TEXCOORD_1`, one material per Quake shader, its
images by relative name such as `../../baseq3/textures/…png`) with `<map>_lightmap.png` and `<map>.entities.json`, every
image the maps use (`baseq3/…png`), every `env/*` sky box and `levelshots/*`, the shader table `shaders.json`, and a
`manifest.json` (`game` "Quake III Arena", `format` 1, `paks`, `maps`, `files` with size and CRC-32). From the real
`baseq3` paks: 36 maps, 1406 files, 273 MB, 2.4 s to convert on desktop. Q3DM17 reads of it:

| file | what for |
|---|---|
| `q3/baseq3/maps/q3dm17.glb` | the map (its world; the items placed in it are not drawn) |
| `q3/shaders.json` | how each surface draws (`formats::q3_map::look`) |
| `q3/baseq3/maps/q3dm17_lightmap.png` | the lightmap atlas |
| `q3/baseq3/textures/…`, `…/models/…` | the map's 74 images and 8 textures its shaders name (light glows, the pewter trim, …) |
| `q3/baseq3/env/space1_{rt,bk,lf,ft,up,dn}.png` | the sky box (below) |

The full list of the files each stage reads is `src/assets/fixtures/q3_stage_files.txt` (an ignored test converts the
real paks and checks them). The stage is offered when the install's quick check passes and its manifest lists the map
and `shaders.json` (`assets::extras::present`, checked when a session starts; the browser checks only the stages'
maps, lightmaps and skies at its start, every file right after an install; debug `OKB_NO_EXTRAS` hides them, as the
golden baseline does); a texture that is missing or does not decode is drawn grey (logged), a missing sky face leaves
the sky black.

Install:

```sh
OKB_SETUP_Q3="<Quake 3>/baseq3" okbeast # the app's own game data, through the setup screen
```

The setup screen offers ADD (REPLACE, REMOVE) QUAKE III ARENA MAPS beside the imported game's disc, after an install at
start or in OPTIONS → CHANGE GAME DATA: CONVERT ITS .PK3 FILES (the `baseq3` folder, the game's folder holding it, or a
`.pk3`; on Android a folder chosen through the system's picker), or USE ITS CONVERTED DATA (a folder this game
converted; okconv-q3's own command line output is refused: convert the paks instead). The install survives a new
main-game install (`platform::game_data::CARRIED`). An install made while a session runs is read from its next fight's
load on; its stages are offered from the next session's start.

Older game data has each map's files under `extras/q3dm17/` (and `extras/q3dm6/`) in the same layout (a copy script of
earlier versions made them): they are still read for a map the Quake III install does not have (or when it is
damaged), and carried over a new install too; `q3/` wins when both have the map.

## In the game

- **Where.** On the stage select of every mode that has one (VS, PRACTICE, WATCH, netplay; the ladders and the attract
  demo never pick it) as `Q3DM17`, the row after EXTRA4 and before RANDOM, when the session has the files
  (`SessionStart::extra_stage`) and is not `--locked`. RANDOM still draws from the main game's 13 stages (its draw is
  the same with or without the files). A direct fight: `--fight --stage Q3DM17` (`sim`: `STAGE=Q3DM17`).
- **Simulation v10.** The new select row is a simulation change (`SIM_VERSION` 10; RANDOM's row moves down by one
  when the files are there); `GameState::extra_stage` is new state (`STATE_VERSION` 9). Older replays have no
  `extra_stage` in their header (it reads false) and older versions never show the row, so they play as recorded.
  A replay of a fight on Q3DM17 plays back without the files too: the map is drawing only (`fight_data::Loaded::arena`
  is None when the files are missing or broken, logged), so the fight is the same, drawn on STB's backdrop without the
  map. A direct fight on Q3DM17 is accepted from simulation v10 on (`replay::check_header`).
- **Simulation v11.** The map's scale went from 8 to 7 game units per Quake unit (below), and with it the ring and
  the drop (`stage::extra::scale`, `Extra::ring_half`, `Extra::ring_drop`); a v10 replay keeps v10's ring and is drawn at v10's
  scale (`arena.rs` takes the fight's simulation version), so it plays and looks as recorded.
- **Simulation v14.** The teleporter stands solid (below, "The platform and the ring"); a v10..v13 replay keeps the
  fighters walking through it.
- **Netplay.** The joiner's Hello says whether it has the files (`proto::EXTRA_Q3DM17`, `PROTO` 3); the host puts
  the stage into the agreed start only when both have them (`net::handshake`), so a joiner without the files never
  sees the row and the two selects always agree. A host's direct fight on Q3DM17 (`--fight --stage Q3DM17`) refuses a
  joiner without the files, and a joiner rejects such a start without them. Nothing is sent of the files themselves.
- **The fight.** It runs as the main-game stage `Extra::stand_in` (STB, EXTRA3) for everything the game's tables decide:
  M.98 (the effect scripts' stage number), the fighters' light set, the sound bank (none), the stage routine, the
  sparks' arena. Only its ring is its own (`stage::Stage::new` with id 13, `arena_bounds`; from v14 the teleporter in
  it, `Extra::solids`), and the drawing.
- **Music.** As every stage: BGM SELECT = CONSOLE plays the opponent's track; ARCADE plays BUILDING2's
  (`Extra::music_stage`), the night rooftop's track, chosen as the closest in mood to an open platform high in
  the sky.

## The platform and the ring

The map's "big central platform" as the stage: the floating platform above the middle of the map (its floor
x 32..352, y −96..224 at z 1280, the Quad Damage's in Quake III, with the pentagram on its floor and a teleporter on
its far edge), open to the void on all four sides. The other candidates were measured in the map and rejected: the
large deck at z 0 has the jump-pad block (100 units high) in its middle and walls under the side wings on two sides,
so no box on it has open edges; its open front strip is 96 units deep at the centre; the side wings carry the
teleporters and lamps.

- **Scale.** 1 Quake unit = 7 game units (`stage::extra::scale`, from simulation v11; v10: 8), measured so that a
  Quake III player model stands as tall on the map as a main-game fighter:
  - Quake III: the 23 player models of `baseq3` (ok_q3_converter's `players/<name>_default.glb`: lower, upper and head
    joined by their tags, the first frame of LEGS_IDLE + TORSO_STAND) stand 56 (visor) to 74 (slash) Quake units
    from the feet (always about 24 below the origin, the 56-unit box's floor) to the top: median 61.2, mean 62.6
    (60.7 to the head's top: wings, shoulder pads and orbb's body lifted out); e.g. sarge 57.8, doom 60.5, anarki
    62.2, biker 61.2, keel 64.4. The box (56) is lower than the model: the head goes above its top at 32.
  - The main game: the 11 fighters in human form, the skinned mesh as the renderer poses it in the fight's idle stance
    (over a second after FIGHT), from the ground to the top: ALI 427, BAK 402, BUS 453, GAD 504, JEN 443, LON 396, MAR
    417, RON 398, STU 421, ULI 373, YUG 436; mean 424 game units.
  - 424 / 61.2 = 6.9: 7 makes the median Quake III player 428 game units. The map agrees: the teleporter on the
    platform is 124 Quake units high (its opening 118), about twice a player model, as in Quake III. (At v10's 8
    the models were 490, the fighters 87 % of them.)

  The platform is then 2240 × 2240 game units (the main game's rings: 3072).
- **Orientation.** The fighters line up along the map's y axis; the camera starts on the map's −x side, looking at
  the teleporter behind them, the map's main structure below and the sky round it (`to_fight` in `arena.rs`: game
  x = −(Quake y − 64) · 7, z = (Quake x − 192) · 7, y = −(Quake z − 1280) · 7).
- **Ring** (`arena_bounds` in `fight/stage/mod.rs`): a box (kind 0) of half sizes 1120 / 1120 (`Extra::ring_half`: the
  platform's edges, 160 Quake units from its centre; v10: 1280), no walls (hp 0 on all four sides: open, as STB's), no
  outer area. The main game's open-stage rules then apply unchanged: walking does not leave the ring (the ring walls'
  push blocks every side in normal play), a fighter in a hit reaction passes the edge and is rung out (OUT OF RING when
  the edge parts are over it), and falls to the ground outside, `ring_drop` = 8960 below the floor (v10: 10240): the
  map's deck 1280 Quake units down, where the fighter is seen lying from the platform. The shadows follow the ring's
  ground (0 on the platform, the drop outside).
- **The teleporter** (simulation v14) stands solid in the ring: a fighter walks up to it and slides round it, as at a
  wall; before v14 (v10..v13 replays) the fighters walk through it. Its footprint is a constant table
  (`stage::extra::Q3DM17_SOLIDS`, Quake units; `Extra::solids` turns and scales it as the ring), measured once in
  `q3dm17.glb`'s world (the surfaces of `models/mapobjects/teleporter/*` above the platform), never read from the map
  at run time, so a fight runs the same without the files:

  | part | measured (Quake x, y) | table | game (x, z), 7 per unit |
  |---|---|---|---|
  | the pad: a regular octagon, its corners 41.3..42.4 from (287.6, 64.4), 12 high (17 at its centre) | x 246.3..329.0, y 22.0..106.8 | disc (288, 64), r 42 | centre (0, 672), r 294 |
  | the pad's step towards the platform's middle, 12 high | x 229.6..263.3, y 53.4..74.6 | box (246, 64) ± (17, 11) | x ±77, z 259..497 |
  | the ring's feet on the pad (the frame at z 1294..1322) | x 264.2..311.7, y 2.6..126.0 | box (288, 64) ± (24, 62) | x ±434, z 504..840 |
  | the ring's narrower sides and their knobs, 35..62 over the floor | x 277.6..299.0, y −23.8..152.4 | box (288, 64) ± (11, 88) | x ±616, z 595..749 |

  All of it is below a fighter's head (61 Quake units over the floor), so it is solid from the floor up, without a
  height, as the main game's walls are. Its far side is 154 game units inside the platform's far edge (z 1120), on the
  game's z axis behind the fighters' start line.

  The main game has nothing inside its rings; the closest mechanism is the ring walls' push (`physics::ring_walls`),
  used turned round (`physics::solids`, after the ring walls in `push_apart_step`): each body sphere but the first is
  tested against each part, a box's overlap pushed out along the axis it went in least, the disc's along the line
  from its centre (with `physics::hypot`), the deepest push per direction of all spheres taken, added to the ring
  walls' push and applied the same way (a thrower and its victim together). It blocks whom an
  indestructible wall (hp −1, as ST4's) blocks: everyone but a fighter past the ring's edge or rung out. It sets none
  of the walls' contact bits (F6C's sides, M_KABE, the walls' impact power): those name the ring's four sides, which
  the move scripts' wall reactions and the hit code read; a fighter knocked into the teleporter stops there without a
  wall hit. The push-apart (sphere against sphere) was the other candidate: it moves both fighters along
  their line and weighs them, which an immovable part does not fit. The drawn teleporter is a model of Quake III's;
  the table above is the only place the logic knows it from.
- The camera is the main game's (`fight/camera.rs`, no stage data); round the platform it stands in the open sky, so no
  wall of the map comes between it and the fighters. As on every stage the fighters are drawn over the stage (the depth
  reset after the stage, `render/frame.rs`), so the teleporter never covers them.

## Drawing

`render::lightmapped`: the world's triangles as one vertex / index buffer, drawn in the stage's place (before the
depth reset, depth-tested and written), each surface as `formats::q3_map::look` reduces its Quake shader:

- the base texture (the shader's first still texture stage; a surface without a shader: its glTF texture) times the
  lightmap (ok_q3_converter's atlas, already overbright-scaled) or, for surfaces without one, the vertex colours
  (`rgbGen vertex`, the implicit shader) or nothing (`rgbGen identity`); colours multiplied in gamma space as Quake III's
  frame buffer does, then the stage darkening (Beast Drive) subtracted in linear light as on the main game's stages;
- plus one additive glow texture (a light's `.blend` / `.glow` stage, the jump and launch pads' glow, the pentagram's
  red glow), drawn steady;
- opaque, alpha-tested (`alphaFunc`), alpha-blended or added (`blendFunc`), the blended ones after the rest without
  depth writes; two-sided where `cull none` / `disable`;
- a `tcMod scroll` on the base stage (the light beams, the teleporter's energy);
- textures with mipmaps (box-filtered), linear filtering, repeat; the lightmap clamped.

The sky: q3dm17's own sky shader `textures/skies/blacksky` is plain black (`map gfx/colors/black.tga`, no
`skyParms`); the space the user asked for is Quake III's farbox `env/space1` (the shader `textures/skies/skybox`'s
far box: stars, asteroids, a red nebula and a planet), drawn as a sky box at the far plane with the far box's face
layout (its faces rt bk lf ft up dn = +X +Y −X −Y +Z −Z). Its cloud layer (`killsky_2.tga`, added, scrolling) is
left out.

Not reproduced (the surfaces still draw with their base texture and lightmap):

| what | where in q3dm17 |
|---|---|
| environment mapping (`tcGen environment`) | the pewter trim's chrome layer, `kmlamp_white` (a lamp's glass: not drawn, it has no other stage) |
| animated colours (`rgbGen wave`): drawn steady at full strength | the light glows' pulse, the jump pads', launch pads' and pentagram's flashing glow, the banner's blink |
| `animMap` | the launch pad's arrows (its first frame would be a glow stage; left out) |
| `tcMod` other than `scroll` (`scale`, `turb`, `stretch`, `rotate`), and any `tcMod` on a non-base stage | the hologram floor's scrolling girl, the electric floor under `diamond2c_ow`, the jump pads' rings |
| `deformVertexes` | the lamp bots' bobbing; `autosprite` flares (`flare03`, `bot_flare`) are not drawn |
| stages beyond base + lightmap + one glow; `alphaGen` | the banner's scrolling text, the jump pads' stretching rings, `steed1gf`'s specular alpha |
| the sky's cloud layer; fog | — |
| the items placed in the map (weapons, armour, health, the Quad) | not drawn (only `worldspawn`) |
