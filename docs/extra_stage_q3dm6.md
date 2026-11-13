# Extra stage Q3DM6: Quake III Arena's "The Camping Grounds"

The engine's second Quake III stage, after [Q3DM17](extra_stage_q3dm17.md), and built the same way: the player's own
files converted from Quake III Arena's `.pk3` files into the Quake III install (`q3/`; the setup screen's QUAKE III
ARENA MAPS), never in git, in the converted game data or in a build; without them the
stage is not offered. Everything below that is not said to differ is as for Q3DM17.

Code: the table of extra stages `fight/stage/extra.rs` (`STAGES`, `Q3DM6`: number, code, stand-in, music, centre,
turn, ring, walls), `fight/stage/mod.rs` (`arena_bounds`), `assets/extras.rs` (`STAGES`: the files; `present`: which
stages' files are there), `formats/q3_map.rs` (the map; `sky`: its sky shader), `fight/fight_data.rs` (`load_arena`),
`render/lightmapped.rs` + `render/shaders/lightmapped.wgsl` (`upload_clouds`, `fs_clouds`),
`present/fight_scene/arena.rs` (the map in the fight's space, the sky, the ground), `app/setup/q3.rs` (the setup's
conversion).

## The files

The same install as Q3DM17's (`q3/`, one conversion holds both): `baseq3/maps/q3dm6.glb`,
`baseq3/maps/q3dm6_lightmap.png`, `shaders.json`, and the images the map and its shaders name (134 files, 15 MB with
the shader table; `src/assets/fixtures/q3_stage_files.txt`). Older game data's `extras/q3dm6/` (same layout) is still
read when the install lacks the map.

## In the game

- **Where.** The stage select's row after Q3DM17 (or in Q3DM17's row, 13, when the session has only Q3DM6's files),
  before RANDOM, labelled `Q3DM6`, in the modes that have a stage select (VS, PRACTICE, WATCH, netplay), not
  `--locked`. Its stage number is 14 whatever its row (`Front::row_stage` / `stage_row`: the select's cursor is a
  row, M.98 a stage number). RANDOM still draws from the main game's 13. A direct fight: `--fight --stage Q3DM6`
  (`sim`: `STAGE=Q3DM6`).
- **Simulation v12.** The new row is a simulation change (`SIM_VERSION` 12): `SessionStart::extra_q3dm6` /
  `GameState::extra_q3dm6` (`STATE_VERSION` 10). A file without the field reads false, and before v12 the row is
  never shown, so v10 / v11 replays play as recorded (Q3DM17 keeps its row, number, ring and scale). A direct fight on
  Q3DM6 is accepted from v12 on (`replay::check_header`, `stage::extra::by_code`).
- **Netplay.** The joiner's Hello carries one bit per extra stage (`proto::EXTRA_Q3DM17` 1, `EXTRA_Q3DM6` 2; the layout
  is unchanged, `PROTO` 3); the host offers each stage only when both have its files, refuses a joiner without the
  files of its direct fight's stage, and a joiner rejects such a start.
- **The fight.** Run as STB (EXTRA3) for the game's tables, as Q3DM17; only its ring (below) and the drawing are its
  own.
- **Music.** BGM SELECT = CONSOLE plays the opponent's track, as everywhere; ARCADE plays ROOM's (`music_stage` 4): ST4,
  the one main-game stage closed in by walls on every side (invisible and indestructible, as this ring's), chosen for
  the enclosed stone courtyard. (q3dm6's own music, `fla22k_02`, is Quake III's, not on the disc.)

## The courtyard and the ring

The map measured in the glb (its world triangles, Quake units): the floors by height in 16-unit cells, a cell free when
a floor lies at the level and nothing from 6 to 160 units above it; the largest free rectangles per level:

| where | level | free rectangle | why not |
|---|---|---|---|
| **the centre courtyard** (the Quad Damage, "center courtyard") | z 0 | 464 × 432 inside its chamfered corners; walls 640 × 480 | chosen: the largest, open to the sky, walled all round |
| the upper floor ("upper floor") | z 512 | 464 × 400 | smaller, and not the open courtyard asked for |
| the lower floor (east, "lower floor") | z 0 | 368 × 368 | smaller, under the z 256 floor (a 256-unit ceiling) |
| the hall north of the courtyard ("plasma gun") | z 0 | 400 × 320 | smaller, under the z 256 floor, steps inside |
| a floor east of the courtyard | z 256 | 336 × 432 | smaller (336 wide) |
| a floor north of the courtyard | z 256 | 624 × 224 | 224 deep: about a fighter's reach |

The courtyard: floor z 0, open to the sky (the walls rise past 500; the first overhang, the balconies' undersides at
the north corners, at z 256), flat between its walls:

- **west** x −896 and **east** x −256 (inner faces);
- **north** y 0 (the wall with the big riveted doors and a fitting jutting out at z ≈ 110..165 over x −628..−548);
- **south** y −480: the faces of the arcade's pillars; between them three arches open into alcoves to y −512, the
  middle one into the corridor and stairs south (x −640..−512);
- the **corners** chamfered at 45°: the north ones 128 units along each wall (from (−896, −128) to (−768, 0), and
  mirrored), the south ones 65 along each wall at y −480 (the diagonal runs on to y −512 behind the pillars).

The ring (`arena_bounds`): a box (kind 0) round the courtyard's middle (x −576, y −240), half sizes 320 × 240 Quake
units = **2240 × 1680** game units at the scale of 7 (Q3DM17's, from v11), its four sides walls (hp −1: invisible,
indestructible, ST4's kind; no wall models), drop 1, no outer area: the main game's closed-room rules (ST4, and the
engine's ST9 / STA hut). Walking stops at the wall, a hit carries a fighter against it (his hips at most 45 short of it:
`extra_stage_q3dm6_walls`); nobody is rung out.

The fit: the box's sides are the courtyard's walls exactly (the south side the pillars' faces; the arches behind them
and the doorways are closed off). The mismatch is in the corners: the box's corner lies inside the chamfers, at the
north corners 90 Quake units (≈ 630 game units) from the chamfer's face, at the south ones 46 (≈ 320); a fighter pushed
into a north corner can stand with his body partly in the chamfer's stone (drawn over it: the fighters are drawn after
the stage). The main game's ring code has boxes and circles only, a circle could not reach the long walls, and a box
inside the chamfers (464 × 432, x −804..−340) would stop the fighters 92 units short of the long walls; the walls' own
planes were preferred. In a match that stays so. In the team battle (engine addition, from simulation v25) the chamfers
are walls too: four spread fighters reach the corners, so each body sphere is pushed back out of the chamfer's half
plane (`Extra::chamfers`, `physics::chamfers`).

- **Orientation** (`Extra::turn`): the fighters line up along the map's x axis (the courtyard's long side); the game's
  x = Quake x + 576, z = Quake y + 240 (× 7), y = −Quake z × 7. The camera starts on the north side, looking at the
  south arcade and its middle arch with the stairs behind.
- **The camera** is the main game's and knows no stage geometry: with the fighters near a wall it stands outside the
  courtyard, sees through the backs of the walls (Quake's faces are one-sided) and, past the floor's edge, would see
  the void. So the stage draws a **ground** (`Extra::ground`, presentation only): the courtyard's flagstones
  (`textures/gothic_floor/largerblock3b3`, mapped as the map's floor triangles, lit by one of their lightmap texels)
  1 unit under the floor, 16384 units round; the map's own floors cover it. Rooms seen from outside stay as they
  are (one-sided walls, the neighbouring corridors).

## Drawing

As Q3DM17's, plus the sky: q3dm6's sky shader is `textures/skies/xtoxicskytim_q3dm5` (`skyParms - 512 -`: no far
box, clouds at 512; two stages, `bluedimclouds` scaled 3 × 2 scrolling 0.15 / 0.15 and `topclouds` added, scaled
3 × 3 scrolling 0.05 / 0.05), not black, so it is drawn itself (`formats::q3_map::sky`, `render::lightmapped`'s
clouds): the cloud mapping (the view direction meets a sphere of radius 4096 whose top is 512
above the eye, st = the acos of the point's x and y from the sphere's centre, then each stage's tcMods), both layers
in one pass, scrolling with the fight's time. Below the horizon the sky is black (Quake III draws the clouds down the
sides; with the camera outside the walls they would show under the floor's edge). In a fight the sky shows only
when the camera looks up over the walls.

Not reproduced: as Q3DM17's list; here among them the autosprite flares (`slamp3`, `gratelamp_flare`).
