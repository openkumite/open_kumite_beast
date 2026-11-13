//! Fight loading: everything read from disk before the GPU exists (stage, fighters, effects, VRAM, music, sound).
//!
//! State: static data (`Loaded`: not serialised; re-attached by `rebind`)

use crate::assets::Assets;
use crate::assets::roster::CHARACTERS;
use crate::fight::br1::res::Br1Tables;
use crate::fight::version::{Gates, v};
use crate::formats::act_ani::{Act, Ani};
use crate::formats::gltf_model::{self, ModelData};
use crate::formats::tables::Tables;

pub struct FighterRes {
    pub code: String,
    pub models: [ModelData; 2], // human, beast
    pub act: Act,
    pub ani: Ani,
    /// skeletons per form (bone offsets, base rotations, connectors) for the pose records
    pub skels: Vec<crate::fight::anim::act_clips::Skel>,
    /// extra skeleton nodes 19.. per form (spring chains etc.)
    pub extras: [Vec<crate::fight::anim::spring::ExtraNode>; 2],
}

/// An imported fighter: its models (ok_br1_converter's glTF: the mesh in the bind pose, joints `bone_NN`, no clips; in
/// a match they move to the side's `Loaded::fighters` entry) and its logic data (`fight::br1::res`).
pub struct Br1FighterRes {
    pub models: [ModelData; 2], // human, beast
    pub res: crate::fight::br1::res::Br1Res,
}

/// Imported fighter `code` from its install `br1` (an `Assets` on `game_data/br1`, its tables `shared`) in costume `costume` (F.4 & 3):
/// the models' costume variant `costume` modulo their count (engine addition: the imported game has three costumes XX10..XX12,
/// Uriko one, where the main game has four slots).
pub async fn load_br1_fighter(br1: &Assets, shared: &Br1Tables, code: &str, costume: u8) -> Result<Br1FighterRes, String> {
    let res = crate::fight::br1::res::Br1Res::load(br1, code, shared).await?;
    let model = |b: Vec<u8>| {
        let n = ModelData::variant_count(&b) as u32;
        ModelData::from_glb_variant(&b, (n > 0).then(|| costume as u32 % n))
    };
    let human = model(br1.load(&format!("models/{code}_human.glb")).await?)?;
    let beast = model(br1.load(&format!("models/{code}_beast.glb")).await?)?;
    Ok(Br1FighterRes { models: [human, beast], res })
}

/// The imported game's sound slots for a match: (name, VAB bytes, volume per tone). The banks are the
/// install's raw `.SND` VABs (the fight's loader keys a tone's VAG with its ADSR, the pitch from its centre
/// and minimum notes, as the main game's driver does: `audio::Audio::play_tone`); the volumes are the 6-byte records
/// {priority, pan, channel group, volume} of each slot in `sound.volumes` (slot 0 at +0, 1 at +0xC0, 2
/// at +0x2D4, 3 / 4 the side's character's table, by a jump table). `codes`: the sides'
/// imported fighters (a main-game side: an empty slot; the team battle's P3 / P4, engine addition T5: slots 5 / 6);
/// `stage`: the imported stage's bank (`XX/XX10`), None on a main-game stage (an empty slot). A missing or
/// unreadable file leaves its slot empty (it plays nothing).
pub async fn br1_sound_banks(br1: &Assets, tables: &Tables, codes: &[Option<&str>], stage: Option<&str>) -> Vec<(String, Vec<u8>, Vec<u16>)> {
    let defs = tables.table("sound.volumes");
    // the jump table's targets by character (ZIN GRE EVE VAK MIT GAD FOX RON URI)
    const CHR: [usize; 9] = [0x364, 0x5E0, 0x41C, 0x590, 0x530, 0x3C0, 0x4D4, 0x478, 0x64C];
    let table = |code: &str| crate::formats::br1_anm::CHARACTERS.iter().position(|c| *c == code).map(|k| CHR[k]);
    let mut slots: Vec<(Option<String>, Option<usize>)> =
        vec![(Some("CMN/CMN00".into()), Some(0)), (Some("CMN/GAME00".into()), Some(0xC0)), (stage.map(str::to_string), Some(0x2D4))];
    slots.extend(codes.iter().map(|c| (c.map(|c| format!("{c}/{c}00")), c.and_then(table))));
    let mut out = Vec::new();
    for (name, recs) in slots {
        let Some(name) = name else {
            out.push((String::new(), Vec::new(), Vec::new()));
            continue;
        };
        let bytes = br1.load(&format!("audio/sfx/{name}.SND")).await.unwrap_or_default();
        let vols = recs.map(|a| (0..64).map(|k| defs.u16(a + 6 * k + 4)).collect()).unwrap_or_default();
        out.push((name, bytes, vols));
    }
    out
}

/// An imported stage's props (engine addition, `present::br1_props`): the sine table (0x401 halfwords)
/// and type 5's leg angles of its static tables, and the objects of its prop TMD (entry 0x19: ok_br1_converter's
/// `tmd_025_prop<j>`, in their own space).
#[derive(Clone)]
pub struct Br1Props {
    pub sin: Vec<i16>,
    pub flap5: [i16; 4],
    pub objects: Vec<ModelData>,
}

/// The HUD's pages of CMN/INIT.DAT (entry, VRAM place) a fight's VRAM ends with (`load_br2_match`).
const HUD_PAGES: [(usize, (usize, usize)); 6] = [(0, (960, 0)), (1, (896, 0)), (2, (496, 480)), (3, (480, 480)), (4, (464, 480)), (25, (512, 256))];

/// Fighter `i`'s `<CHR>EFT.DAT` entries 1..20 in EffTab: P1's from 40, P2's from 60; the team battle's fighters 2 / 3 (engine
/// addition) from 100 / 120.
pub fn eft_base(i: usize) -> usize {
    let f = crate::fight::side::Fid::of(i);
    f.bank().eft_base(f.slot())
}

/// Whether `code` names an imported fighter (`assets::manifest::BR1_FIGHTERS`; no code of the main game is one).
pub fn is_br1(code: &str) -> bool {
    crate::assets::manifest::BR1_FIGHTERS.contains(&code)
}

/// The main game's code of side `code`: itself, or an imported fighter's proxy (`br1::bridge::br2_proxy`)
/// for what the loader picks by character (the sound banks, the effect sheets, the music).
fn br2_code(code: &str) -> &str {
    if !is_br1(code) {
        return code;
    }
    let chr = crate::formats::br1_anm::CHARACTERS.iter().position(|c| *c == code).unwrap_or(0) as u8;
    CHARACTERS[crate::fight::br1::bridge::br2_proxy(chr) as usize]
}

/// Everything loaded from disk before the GPU exists.
pub struct Loaded {
    pub stage: ModelData,
    pub stage_name: String,
    /// the fighters by side (two in a match, four in the team battle); `br1` and `br1_ghost` hold one entry per side too
    /// (`Loaded::sides`)
    pub fighters: Vec<FighterRes>,
    pub cmn: Act,
    pub trig: crate::fight::physics::Trig,
    /// Ring half size (x, z) of the stage.
    pub ring: (i32, i32),
    /// stage number 0..12 (ST0..STC): the one whose static tables the fight reads (M.98); the extra stage's stand-in
    /// (`Extra::stand_in`) for it
    pub stage_no: usize,
    /// the stage chosen: `stage_no`, or an extra stage's number (`stage::extra::STAGES`: 13 Q3DM17, 14 Q3DM6)
    pub stage_id: usize,
    /// the extra stage's map, textures and sky (`stage_id` 13..; `stage` is empty then); None also for an extra stage
    /// whose files are missing or broken (drawing only: the fight is the same, drawn on the stand-in's backdrop)
    pub arena: Option<Arena>,
    /// Effect VRAM (EFT.DAT sheets and CLUTs, the fighters' <CHR>EFT.DAT sheets)
    pub vram: crate::formats::vram::Vram,
    /// EFT.DAT entries (the effect table's slots 0..29 and 80..99, entry 88 = hit-spark tables)
    pub eft: Vec<Option<Vec<u8>>>,
    /// EffTab (100 entries): EFT 0..29, stage 0x40.. → 30..39, P1 <CHR>EFT 1..20 → 40..59,
    /// P2 → 60..79, EFT 80..99 (the effect scripts and ED1C4 emitters of all groups)
    pub eff_tab: Vec<Option<Vec<u8>>>,
    /// costumes (F.4) of both fighters
    pub costume: [u8; 2],
    /// the wall debris pieces (stage slot 3, one model each)
    pub debris: Vec<ModelData>,
    /// the stage objects the stage's draw routine moves or shows on alternate frames (`stage::anim::objects`): (object
    /// number, model in world space); empty with an export that has them in `stage`
    pub stage_objs: Vec<(u8, ModelData)>,
    /// the stage's semi-transparent polygons (ST9..STC; texel alpha 1 = drawn opaque, 0.5 = the STP bit: blended
    /// additively), drawn after the rest of the stage; None with an export that has them in `stage`
    pub stage_semi: Option<ModelData>,
    /// `stage` is in ordering-table order (`<ST>_ot.glb`: units `gltf_model::Prim::ot`), drawn as the OT does by
    /// `stage::anim::sorts`; `stage_objs` / `stage_semi` are empty then
    pub stage_ot: bool,
    /// the game's static tables (`engine/tables.okt`)
    pub tables: crate::formats::tables::Tables,
    /// Music: (track number, decoded) for the stage BGM and the match-end jingles (tracks 3 and 4)
    pub music: Vec<(u8, std::sync::Arc<crate::formats::music::Music>)>,
    /// the continue and game-over tracks of the chosen BGM set (round requests 0x2A / 0x27)
    pub music_cont: (u8, u8),
    /// Sound banks of the SFX slots in a fight (COMMON_2, COMMON_1, stage, P1, P2, P1 / P2 game-over voice): (name, bytes, volumes)
    pub sound_banks: Vec<(String, Vec<u8>, Vec<u16>)>,
    /// engine addition (the import S7): an imported fighter's data by side (its `fighters` entry then holds
    /// its models and no main-game scripts); None for a main-game fighter
    pub br1: Vec<Option<Box<Br1FighterRes>>>,
    /// engine addition (the import S7): a main-game side's imported proxy (`br1::bridge::br1_proxy`) in a
    /// mixed pair, the victim an imported throw puppets it with; None else
    pub br1_ghost: Vec<Option<Box<crate::fight::br1::res::Br1Res>>>,
    /// engine addition (the import S8): the imported game's sound slots in a match with an imported fighter
    /// (0 CMN00, 1 GAME00, 2 the stage's XX10, 3 / 4 P1's / P2's fighter XX00; `br1_sound_banks`), as
    /// `sound_banks`; empty without one
    pub br1_banks: Vec<(String, Vec<u8>, Vec<u16>)>,
    /// engine addition (the import S8): an imported stage's records (`fight::stage::br1`; `stage` is its scene,
    /// `stage_no` the stand-in's); None on a main-game or an extra stage
    pub br1_stage: Option<crate::fight::stage::br1::Bounds>,
    /// engine addition (the import S8): an imported stage's background (drawing only, `present::br1_stage_bg`):
    /// its 44-byte record (by G.94) and its panorama (`<code>_sky` of its glb, one turn: 1024 wide)
    pub br1_bg: Option<([u8; 44], gltf_model::Image)>,
    /// engine addition (the import S8): an imported stage's light record (0x44 bytes by G.94, drawing only:
    /// `present::fighter_lighting::fighter_br1`)
    pub br1_light: Option<[u8; 0x44]>,
    /// engine addition (the import S8): an imported stage's props (drawing only, `present::br1_props`); None on a
    /// stage without a drawn prop type or with an export from before ok_br1_converter split them
    pub br1_props: Option<Br1Props>,
    /// engine addition (the import S8): Uriko's stage's wall models (its object list's objects 5..12, `tmd_023_obj<k>`:
    /// the walls by model, then the ones that drop in from G.4A 0x4B), drawn by `present` as Uriko's stage routine draws them;
    /// empty elsewhere or with an older export (they stay in the stage's model)
    pub br1_walls: Vec<ModelData>,
    /// engine addition (the team battle): what its data adds (`load_team`); None in a match
    pub team: Option<TeamData>,
    /// the simulation version the data is for (`session::SIM_VERSION`, a replay's): the clips and the logic's
    /// skeleton math depend on it (`portable`); private, set once with `gates` (`sim_version()`)
    sim_version: u32,
    /// the rules of `sim_version` (engine addition, `fight::version`), derived once from it (`gates()`)
    gates: Gates,
}

impl Loaded {
    /// The simulation version the data is for (`session::SIM_VERSION`, a replay's).
    pub fn sim_version(&self) -> u32 {
        self.sim_version
    }

    /// The rules of its simulation version (`Gates::of(sim_version)`, built with the data).
    pub fn gates(&self) -> Gates {
        self.gates
    }

    /// Simulation version 3 on: the logic's skeleton, spring and clip math through `crate::base::portable` (the same
    /// bits on every target); before it the platform's f32 math, as those versions' replays were recorded.
    pub fn portable(&self) -> bool {
        self.gates.portable_math
    }

    /// The number of sides (fighters): two in a match, four in the team battle; `fighters`, `br1` and `br1_ghost` have
    /// one entry each per side.
    pub fn sides(&self) -> usize {
        assert!(self.br1.len() == self.fighters.len() && self.br1_ghost.len() == self.fighters.len(), "one entry per side");
        self.fighters.len()
    }
}

/// An extra stage's drawing data (engine addition, `extra_stage`, `assets::extras`): the map's world, its textures by
/// path (None: the file is missing or does not decode; drawn grey), the lightmap atlas, the sky box's six faces in
/// the order rt bk lf ft up dn and the sky's cloud layers with their images (`formats::q3_map::Sky`; a layer whose
/// image is missing is left out).
pub struct Arena {
    pub map: crate::formats::q3_map::Map,
    pub textures: Vec<(String, Option<gltf_model::Image>)>,
    pub lightmap: Option<gltf_model::Image>,
    pub sky: Option<[gltf_model::Image; 6]>,
    pub clouds: Vec<(crate::formats::q3_map::CloudLayer, gltf_model::Image)>,
    /// the cloud layer's height (Quake units)
    pub cloud_height: f32,
}

/// A PNG of the extra stage's files (`path` under `root`, `assets::extras::source`), decoded; None (logged) when
/// missing or broken.
async fn arena_png(root: &Assets, path: &str) -> Option<gltf_model::Image> {
    let decoded = match root.load(path).await {
        Ok(b) => image::load_from_memory_with_format(&b, image::ImageFormat::Png).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    match decoded {
        Ok(im) => {
            let im = im.to_rgba8();
            Some(gltf_model::Image { name: path.to_string(), width: im.width(), height: im.height(), rgba: im.into_raw() })
        }
        Err(e) => {
            log::warn!("extra stage: {path}: {e}");
            None
        }
    }
}

/// An extra stage's files (`assets::extras::Files`, under `root`: `assets::extras::source`): the map with its shader
/// table, every texture its surfaces draw, the lightmap and the sky: the far box the files name (Q3DM17's env/space1:
/// its own sky shader is plain black), else the map's own sky shader's far box and cloud layers (Q3DM6's). Err when
/// the map or the table cannot be read.
async fn load_arena(root: &Assets, files: &crate::assets::extras::Files) -> Result<Arena, String> {
    let glb = root.load(files.map).await?;
    let shaders = root.load_json(files.shaders).await?;
    let map = crate::formats::q3_map::parse(&glb, files.map, &shaders).map_err(|e| format!("{}: {e}", files.map))?;
    for (name, why) in &map.skipped {
        log::info!("extra stage: shader {name} not drawn: {why}");
    }
    let mut textures = Vec::new();
    for t in map.textures() {
        textures.push((t.to_string(), arena_png(root, t).await));
    }
    let lightmap = match &map.lightmap {
        Some(l) => arena_png(root, l).await,
        None => None,
    };
    let own = map.sky.clone().unwrap_or_default();
    let (farbox, layers) = match files.sky {
        Some(f) => (Some(f.to_string()), Vec::new()),
        None => (own.farbox, own.clouds),
    };
    let mut sky = None;
    if let Some(farbox) = farbox {
        let mut faces = Vec::new();
        for f in ["rt", "bk", "lf", "ft", "up", "dn"] {
            faces.push(arena_png(root, &format!("{farbox}_{f}.png")).await);
        }
        sky = faces.into_iter().collect::<Option<Vec<_>>>().and_then(|v| <[gltf_model::Image; 6]>::try_from(v).ok());
    }
    let mut clouds = Vec::new();
    for l in layers {
        if let Some(im) = arena_png(root, &l.texture).await {
            clouds.push((l, im));
        }
    }
    Ok(Arena { map, textures, lightmap, sky, clouds, cloud_height: own.cloud_height })
}

/// The costumes of a direct fight: [0, 1], debug OKB_COSTUME=a,b (0..3; a replay records them).
pub fn direct_costume() -> [u8; 2] {
    #[cfg(not(target_arch = "wasm32"))]
    return crate::base::env::var("COSTUME").ok().and_then(|v| v.split_once(',').and_then(|(a, b)| Some([a.parse().ok()?, b.parse().ok()?]))).unwrap_or([0, 1]);
    #[cfg(target_arch = "wasm32")]
    return [0, 1];
}

/// The fight stages' files by stage number (M.98): `stages/<code>.glb`.
pub const STAGE_CODES: [&str; 13] = ["ST0", "ST1", "ST2", "ST3", "ST4", "ST5", "ST6", "ST7", "ST8", "ST9", "STA", "STB", "STC"];

/// Stage number `n`'s code (a checked `MatchSpec`'s stage; ST0 out of range): 13.. are the extra stages
/// (`stage::extra::STAGES`), 15..25 the imported game's (`stage::br1::STAGES`).
pub fn stage_code(n: u8) -> &'static str {
    if let Some(e) = crate::fight::stage::extra::by_stage(n as usize) {
        return e.code;
    }
    if let Some(s) = crate::fight::stage::br1::by_stage(n as usize) {
        return s.code;
    }
    STAGE_CODES.get(n as usize).copied().unwrap_or("ST0")
}

/// Music tracks (the stage music by the opponent's character) for BGM SELECT = CONSOLE / ARCADE:
/// (stage BGM, continue, game over). Console: by the opponent's character; arcade: by the stage.
pub fn music_tracks(arcade: bool, opp: usize, stage_no: usize) -> (u8, u8, u8) {
    const BGM: [u8; 11] = [0, 1, 2, 5, 3, 6, 4, 7, 8, 0xA, 9];
    const STAGE: [u8; 13] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 0, 1, 2, 3];
    if arcade { (0xA + STAGE[stage_no.min(12)], 5, 6) } else { (0x14 + BGM[opp.min(10)], 0x2A, 0x27) }
}

/// Fight loader (per fighter): `costume` k = F.4 & 3 selects `<CHR>{k+1}P.ANI` / the k+1 P models;
/// `sim_version` = the session's (`Loaded::sim_version`). An imported fighter (`is_br1`, engine addition) comes
/// from the imported game's install beside `assets` (`Assets::br1`), from simulation version `v::BR1_FIGHTERS` on.
pub async fn load_match(assets: &Assets, chr: [&str; 2], costume: [u8; 2], stage: &str, arcade_bgm: bool, sim_version: u32) -> Result<Loaded, String> {
    load_match_with(assets, assets.br1().as_ref(), chr, costume, stage, arcade_bgm, sim_version).await
}

/// `load_match` with the imported game's install `br1` (None: none).
pub async fn load_match_with(
    assets: &Assets,
    br1_dir: Option<&Assets>,
    chr: [&str; 2],
    costume: [u8; 2],
    stage: &str,
    arcade_bgm: bool,
    sim_version: u32,
) -> Result<Loaded, String> {
    let g = Gates::of(sim_version);
    let mut shared = None;
    let mut br1 = [None, None];
    for (k, code) in chr.iter().enumerate() {
        if is_br1(code) {
            if !g.br1_fighters {
                return Err(format!("{code}: an imported fighter needs simulation version {}", v::BR1_FIGHTERS));
            }
            let dir = br1_dir.ok_or_else(|| format!("{code}: no install of the imported game"))?;
            br1[k] = Some(Box::new(load_br1_fighter(dir, br1_shared(dir, &mut shared).await?, code, costume[k]).await.map_err(|e| format!("{code}: {e}"))?));
        }
    }
    if br1.iter().any(Option::is_some) && crate::fight::stage::extra::by_code(stage, g).is_some() {
        return Err(format!("{stage}: no imported fighters on an extra stage yet"));
    }
    // a main-game side facing an imported fighter: its imported proxy's data, which only the proxy
    // victim of an imported throw plays (the cross-engine throws)
    let mut ghost = [None, None];
    if br1.iter().any(Option::is_some) {
        for (k, code) in chr.iter().enumerate() {
            if br1[k].is_none() {
                let c = CHARACTERS.iter().position(|c| c == code).unwrap_or(0) as u8;
                let proxy = crate::formats::br1_anm::CHARACTERS[crate::fight::br1::bridge::br1_proxy(c) as usize];
                let dir = br1_dir.ok_or_else(|| format!("{code}: no install of the imported game"))?;
                ghost[k] = Some(Box::new(
                    crate::fight::br1::res::Br1Res::load(dir, proxy, br1_shared(dir, &mut shared).await?).await.map_err(|e| format!("{proxy}: {e}"))?,
                ));
            }
        }
    }
    // an imported stage (engine addition, the import S8): from simulation v16 with the imported game's install
    let b1stage = crate::fight::stage::br1::by_code(stage, g);
    let banks = match br1_dir {
        Some(dir) if br1.iter().any(Option::is_some) || b1stage.is_some() => {
            let bank = b1stage.map(|s| format!("{}/{}", s.dir, s.bank));
            br1_sound_banks(dir, &br1_shared(dir, &mut shared).await?.tables, &[0, 1].map(|k| br1[k].as_ref().map(|b| b.res.code.as_str())), bank.as_deref())
                .await
        }
        _ => Vec::new(),
    };
    let chr = chr.map(br2_code);
    let br2_stage = if b1stage.is_some() { STAGE_CODES[crate::fight::stage::br1::STAND_IN] } else { stage };
    let mut loaded = load_br2_match(assets, chr, costume, br2_stage, arcade_bgm, sim_version, &mut br1).await?;
    loaded.br1 = br1.into();
    loaded.br1_ghost = ghost.into();
    loaded.br1_banks = banks;
    if let (Some(dir), true) = (br1_dir, loaded.br1.iter().any(Option::is_some)) {
        load_br1_effect_pages(dir, &mut loaded.vram).await;
    }
    if let Some(s) = b1stage {
        let dir = br1_dir.ok_or_else(|| format!("{stage}: no install of the imported game"))?;
        load_br1_stage(dir, &br1_shared(dir, &mut shared).await?.tables, s, &mut loaded, arcade_bgm, chr[1]).await?;
    }
    Ok(loaded)
}

/// The tables of the imported game's install `dir` (`Br1Tables`): loaded the first time a match needs them, then shared
/// by its fighters, proxies, sound banks and stage.
async fn br1_shared<'a>(dir: &Assets, shared: &'a mut Option<Br1Tables>) -> Result<&'a Br1Tables, String> {
    if shared.is_none() {
        *shared = Some(Br1Tables::load(dir).await?);
    }
    Ok(shared.as_ref().expect("loaded above"))
}

/// The imported game's effect pages and CLUTs (CMN/EFFECT.DAT entries 0..4) into the effects' VRAM image at the places
/// `present::br1_fx` draws them from (engine addition: the PS1 frame buffer's area of the image, which nothing else uses;
/// at their own places they would cover the main game's stage effect pages). A missing file leaves them out.
async fn load_br1_effect_pages(dir: &Assets, vram: &mut crate::formats::vram::Vram) {
    use crate::formats::vram::{BR1_CLUT_FROM, BR1_CLUT_TO, BR1_PAGES};
    let c = crate::formats::container::read(&dir.load("engine/CMN/EFFECT.DAT").await.unwrap_or_default());
    for t in c.iter().take(5).flatten() {
        let [_, Some([x, y, _, _])] = crate::formats::vram::Vram::tim_rects(t, None, None) else { continue };
        let at = if let Some((_, to)) = BR1_PAGES.iter().find(|(from, _)| *from == (x, y)) {
            *to
        } else if (BR1_CLUT_FROM.0..BR1_CLUT_FROM.0 + 32).contains(&x) && (BR1_CLUT_FROM.1..BR1_CLUT_FROM.1 + 16).contains(&y) {
            (x - BR1_CLUT_FROM.0 + BR1_CLUT_TO.0, y - BR1_CLUT_FROM.1 + BR1_CLUT_TO.1)
        } else {
            continue;
        };
        vram.load_tim(t, None, Some(at));
    }
}

/// An imported stage `s` in place of the stand-in's (engine addition, the import S8): its records from the static tables
/// (`fight::stage::br1::Bounds`), its scene (ok_br1_converter's `stages/<code>.glb`: the stage's models and its
/// background, in world space), no stand-in objects, debris or ordering-table stage; its stage theme (`music_track`) as
/// the fight's BGM in the stand-in's BGM entry (`music_tracks`; `opp` the opponent's main-game code).
async fn load_br1_stage(
    dir: &Assets,
    tables: &Tables,
    s: &crate::fight::stage::br1::Br1Stage,
    loaded: &mut Loaded,
    arcade_bgm: bool,
    opp: &str,
) -> Result<(), String> {
    use crate::fight::stage::br1::{Bounds, STAND_IN, music_file, music_track};

    let b = Bounds::of(tables, s);
    loaded.br1_stage = Some(b);
    // (per stage: the light, `stage.light`; the sky band, `stage.colours`)
    let at = |name: &str, size: u32| tables.table(name).origin() + size * s.g94 as u32;
    loaded.br1_light = Some(std::array::from_fn(|k| tables.abs_u8(at("stage.light", 0x44) + k as u32)));
    // the scene: the stage file's first model (entry 0x17) and the background; entry 0x18 is the piece a broken wall
    // falls into (`pieces` of them, as the main game's debris), 0x19 the models the stage's code places (`br1_props`)
    let glb = dir.load(&format!("stages/{}.glb", s.code)).await?;
    let err = |e: String| format!("{}: {e}", s.code);
    // (ok_br1_converter's sky band, cap and floor disc stand in for the background, drawn as the game does instead:
    // `br1_bg`)
    let bg_node = format!("{}_background", s.code);
    // one decode of the scene, split by node: the stage, the props' objects, the piece (and Uriko's walls, below)
    let all = ModelData::from_glb_with(&glb, None, &|n| n != bg_node).map_err(err)?;
    let part = |name: &str| -> Result<ModelData, String> {
        let mut m = all.extract(&|n| n == name).map_err(err)?;
        only_used_images(&mut m);
        Ok(m)
    };
    let nodes: std::collections::HashSet<&str> = all.prims.iter().map(|p| all.prim_node(p)).collect();
    // (all eight walls' objects, else the merged model as an older export's)
    let uriko_walls = s.g94 == 8 && (5..=12).all(|k| nodes.contains(format!("tmd_023_obj{k}").as_str()));
    let uriko_wall = |n: &str| uriko_walls && n.strip_prefix("tmd_023_obj").and_then(|k| k.parse::<u8>().ok()).is_some_and(|k| (5..=12).contains(&k));
    loaded.stage = all.extract(&|n| n != "tmd_024" && !n.starts_with("tmd_025") && !uriko_wall(n)).map_err(err)?;
    // the props of the types the engine draws (`stage::br1::PROP_STAGES`: ZIN's, VAK's), each object with its images
    let mut props = Vec::new();
    if crate::fight::stage::br1::PROP_STAGES.contains(&s.g94) {
        loop {
            let name = format!("tmd_025_prop{}", props.len());
            if !nodes.contains(name.as_str()) {
                break;
            }
            props.push(part(&name)?);
        }
    }
    loaded.br1_props = (!props.is_empty()).then(|| Br1Props {
        sin: (0..=0x400).map(|k| tables.table("math.sin").i16_at(k)).collect(),
        flap5: [0, 1, 2, 3].map(|k| tables.table("stage.flap").i16_at(k)),
        objects: props,
    });
    // Uriko's stage's walls (its stage routine draws them itself: object 5 + the wall's model, from G.4A 0x4B object 9 + it;
    // ok_br1_converter ≥ br1-s8b's `tmd_023_obj<k>`), out of the stage's model and drawn on their own
    loaded.br1_walls = if uriko_walls { (5..=12).map(|k| part(&format!("tmd_023_obj{k}"))).collect::<Result<_, _>>()? } else { Vec::new() };
    let sky = format!("{}_sky", s.code);
    loaded.br1_bg = loaded.stage.images.iter().find(|i| i.name == sky).map(|i| {
        // one turn of the strip: 64 cells (the game builds no more)
        let w = i.width.min(1024);
        let rgba = (0..i.height as usize).flat_map(|y| i.rgba[y * i.width as usize * 4..(y * i.width as usize + w as usize) * 4].iter().copied()).collect();
        let mut cfg = [0u8; 44];
        for (k, c) in cfg.iter_mut().enumerate() {
            *c = tables.abs_u8(at("stage.colours", 44) + k as u32);
        }
        (cfg, gltf_model::Image { name: sky.clone(), width: w, height: i.height, rgba })
    });
    // (the sky only feeds `br1_bg`: the stage keeps the images its polygons use)
    only_used_images(&mut loaded.stage);
    // the piece, with its own images only
    let piece = part("tmd_024")?;
    loaded.debris = vec![piece; b.pieces.clamp(0, 16) as usize];
    loaded.stage_name = s.name().to_string();
    loaded.stage_id = s.stage as usize;
    loaded.stage_ot = false;
    loaded.stage_objs.clear();
    loaded.stage_semi = None;
    match dir.load(&music_file(music_track(s.g94))).await.ok().and_then(crate::formats::music::decode) {
        Some(m) => {
            let opp = CHARACTERS.iter().position(|c| *c == opp).unwrap_or(0);
            let (bgm, _, _) = music_tracks(arcade_bgm, opp, STAND_IN);
            let m = std::sync::Arc::new(m);
            match loaded.music.iter_mut().find(|(t, _)| *t == bgm) {
                Some(e) => e.1 = m,
                None => loaded.music.insert(0, (bgm, m)),
            }
        }
        None => log::warn!("{}: its music {} is missing or broken", s.code, music_file(music_track(s.g94))),
    }
    Ok(())
}

/// Drops the images of `m` no polygon draws with, renumbering the polygons' textures.
fn only_used_images(m: &mut ModelData) {
    let mut to = vec![None; m.images.len()];
    let mut n = 0;
    for p in &m.prims {
        if let Some(t) = p.texture
            && t < to.len()
            && to[t].is_none()
        {
            to[t] = Some(n);
            n += 1;
        }
    }
    let mut kept: Vec<Option<gltf_model::Image>> = vec![None; n];
    for (k, img) in std::mem::take(&mut m.images).into_iter().enumerate() {
        if let Some(j) = to[k] {
            kept[j] = Some(img);
        }
    }
    m.images = kept.into_iter().flatten().collect();
    for p in &mut m.prims {
        p.texture = p.texture.and_then(|t| to.get(t).copied().flatten());
    }
}

/// `load_match` with the sides' imported-game data `br1` loaded (their `chr` the proxies' codes).
async fn load_br2_match(
    assets: &Assets,
    chr: [&str; 2],
    costume: [u8; 2],
    stage: &str,
    arcade_bgm: bool,
    sim_version: u32,
    br1: &mut [Option<Box<Br1FighterRes>>; 2],
) -> Result<Loaded, String> {
    let [p1, p2] = chr;
    // an extra stage (engine addition): every main-game stage file of its stand-in; its map from the Quake III install (or the
    // older extras folder), drawing only (the fight needs nothing of it): missing or broken, the fight runs the same,
    // drawn without the map
    let extra_stage = crate::fight::stage::extra::by_code(stage, Gates::of(sim_version));
    let extra = extra_stage.is_some();
    let arena = match extra_stage {
        Some(e) => match crate::assets::extras::source(assets, e.index()) {
            Some(root) => {
                load_arena(&root, &crate::assets::extras::STAGES[e.index()]).await.inspect_err(|e| log::warn!("extra stage: {e}; drawn without the map")).ok()
            }
            None => {
                log::warn!("extra stage {}: no files; drawn without the map", e.code);
                None
            }
        },
        None => None,
    };
    let stage = extra_stage.map_or(stage, |e| STAGE_CODES[e.stand_in]);
    let manifest = assets.load_json("manifest.json").await?;
    let stage_entry = manifest["stages"].as_array().and_then(|a| a.iter().find(|s| s["code"] == stage)).ok_or(format!("stage {stage} not in manifest"))?;
    let stage_no = STAGE_CODES.iter().position(|s| *s == stage).unwrap_or(0);
    // the stage as the ordering table draws it: one unit per object and the floor, polygons in drawing order, the
    // textures as the fight's VRAM has them (`<ST>_ot.glb`, ok_br2_converter::stages); without it the older split
    // exports below, depth-tested
    // `<ST>_ot_v2.glb` (ok_br2_converter::stages, where it differs): the textured polygons' colours above 128 kept
    // (texel × colour / 128 brightens up to 2×), else the v1 file with them clamped at 128 (also for a v2 file that
    // does not parse, engine addition)
    let mut ot = None;
    if extra {
        // the extra stage: no main-game stage model (an empty one stands in, not drawn)
        ot = Some(gltf_model::ModelData::empty());
    } else if let Ok(b) = assets.load(&format!("stages/{stage}_ot_v2.glb")).await {
        match ModelData::from_glb(&b) {
            Ok(m) => ot = Some(m),
            Err(e) => log::warn!("stages/{stage}_ot_v2.glb: {e}; the v1 file instead"),
        }
    }
    let stage_v2 = ot.is_some() && !extra;
    if ot.is_none()
        && let Ok(b) = assets.load(&format!("stages/{stage}_ot.glb")).await
    {
        ot = Some(ModelData::from_glb(&b)?);
    }
    let stage_ot = ot.is_some() && !extra;
    // ST9..STC: the semi-transparent polygons (HMD type bit 0x200000) in their own model and the stage without them
    // (`<ST>_semi.glb` / `<ST>_opaque.glb`, ok_br2_converter::stages); an older export has them opaque
    let semi = if stage_no >= 9 && !stage_ot && !extra {
        match (assets.load(&format!("stages/{stage}_opaque.glb")).await, assets.load(&format!("stages/{stage}_semi.glb")).await) {
            (Ok(o), Ok(s)) => Some((o, s)),
            _ => None,
        }
    } else {
        None
    };
    // the stage without its moved and never-drawn objects, and those objects alone (`stage_anim`; written by
    // ok_br2_converter::stages); an older export has the whole stage in one static model
    let split = if !stage_ot && !extra && semi.is_none() && crate::fight::stage::anim::split(stage_no) {
        assets.load(&format!("stages/{stage}_base.glb")).await.ok()
    } else {
        None
    };
    // a split export with a part missing or broken: the whole stage file, as without one (engine addition)
    let parts = match (&semi, &split) {
        (Some((o, s)), _) => Some((o, Some(s))),
        (None, Some(b)) => Some((b, None)),
        (None, None) => None,
    };
    let parts = match parts {
        Some((base, semi)) => split_parts(assets, stage, stage_no, base, semi.map(Vec::as_slice))
            .await
            .inspect_err(|e| log::warn!("stage {stage}: {e}; the whole stage file instead"))
            .ok(),
        None => None,
    };
    let (mut stage_model, stage_semi, stage_objs) = match (ot, parts) {
        (Some(m), _) => (m, None, Vec::new()),
        (None, Some(p)) => p,
        (None, None) => (ModelData::from_glb(&assets.load(stage_entry["file"].as_str().unwrap_or("")).await?)?, None, Vec::new()),
    };
    // the floor tiles' CLUT rows by depth (`FightView`)
    stage_model.floor_tiles().map_err(|e| format!("stage {stage}: {e}"))?;
    let cmn = Act::parse(&assets.load("engine/CMN.ACT").await?);
    let tables = assets.br2_tables().await?;
    let trig = crate::fight::physics::Trig::from_tables(&tables);
    let index = assets.load_json("engine/index.json").await?;
    let ring =
        index["rings"][stage_no].as_array().map(|r| (r[2].as_i64().unwrap_or(1536) as i32, r[3].as_i64().unwrap_or(1536) as i32)).unwrap_or((1536, 1536));
    // sound banks (by slot): stage banks 8, 9, A, C use STAGE_7; STB has none
    let st_bank = match stage_no {
        0..=7 => Some(format!("STAGE_{stage_no}.SND")),
        11 => None,
        _ => Some("STAGE_7.SND".to_string()),
    };
    let chara = |c: &str| format!("CHARA_{:02}.SND", CHARACTERS.iter().position(|x| *x == c).unwrap_or(0));
    let mut sound_banks = Vec::new();
    // slots 5 / 6: the game-over voices of P1 / P2 (GOVER_cc, loaded into slot 2 at GAME OVER)
    let gover = |c: &str| format!("GOVER_{:02}.SND", CHARACTERS.iter().position(|x| *x == c).unwrap_or(0));
    for name in
        [Some("COMMON_2.SND".to_string()), Some("COMMON_1.SND".to_string()), st_bank, Some(chara(p1)), Some(chara(p2)), Some(gover(p1)), Some(gover(p2))]
    {
        let Some(name) = name else {
            sound_banks.push((String::new(), Vec::new(), Vec::new()));
            continue;
        };
        let bytes = assets.load(&format!("engine/snd/{name}")).await.unwrap_or_default();
        let vols = index["sound_defs"][&name].as_array().map(|a| a.iter().map(|d| d[3].as_u64().unwrap_or(0x1000) as u16).collect()).unwrap_or_default();
        sound_banks.push((name, bytes, vols));
    }
    // the boot pages of CMN/INIT.DAT stay resident: HUD and menu fonts, cursors (the fight data loaded
    // below goes over them where it overlaps, as in the game)
    let mut vram = crate::formats::vram::Vram::default();
    let init = crate::formats::container::read(&assets.load("engine/INIT.DAT").await.unwrap_or_default());
    let boot: [(usize, (usize, usize)); 22] = [
        (0, (960, 0)),
        (1, (896, 0)),
        (2, (496, 480)),
        (3, (480, 480)),
        (4, (464, 480)),
        (5, (368, 480)),
        (6, (352, 496)),
        (16, (464, 496)),
        (17, (496, 496)),
        (18, (480, 496)),
        (19, (448, 496)),
        (20, (384, 496)),
        (21, (400, 496)),
        (22, (416, 496)),
        (23, (432, 496)),
        (24, (832, 256)),
        (25, (512, 256)),
        (26, (448, 480)),
        (27, (432, 480)),
        (8, (896, 448)),
        (9, (928, 384)),
        (7, (832, 448)),
    ];
    for (k, at) in boot {
        if let Some(Some(t)) = init.get(k) {
            vram.load_tim(t, if k == 7 { Some((352, 480)) } else { None }, Some(at));
        }
    }
    // effect data and VRAM
    let eft = crate::formats::container::read(&assets.load("engine/eft/EFT.DAT").await.unwrap_or_default());
    for k in 0x100..0x104 {
        if let Some(Some(t)) = eft.get(k) {
            vram.load_tim(t, None, None);
        }
    }
    let mut eff_tab: Vec<Option<Vec<u8>>> = vec![None; 100];
    for k in (0..30).chain(80..100) {
        eff_tab[k] = eft.get(k).cloned().flatten();
    }
    for (side, code) in [p1, p2].iter().enumerate() {
        let c = crate::formats::container::read(&assets.load(&format!("engine/eft/{code}EFT.DAT")).await.unwrap_or_default());
        if let Some(Some(t)) = c.first() {
            let (clut, pix) = if side == 0 { ((96, 480), (960, 256)) } else { ((112, 480), (960, 320)) };
            vram.load_tim(t, Some(clut), Some(pix));
        }
        for k in 0..20 {
            eff_tab[eft_base(side) + k] = c.get(1 + k).cloned().flatten();
        }
    }
    // stage effects (loop): entries 0x40+k → EffTab[30+k]; textures 18 / 19 → (768,256) / (704,256),
    // CLUTs 32+k → (256+16k, 480)
    {
        let st = crate::formats::container::read(&assets.load(&format!("engine/stage/{stage}.DAT")).await.unwrap_or_default());
        for k in 0..10 {
            eff_tab[30 + k] = st.get(0x40 + k).cloned().flatten();
        }
        for (e, at) in [(18usize, (768, 256)), (19, (704, 256))] {
            if let Some(Some(t)) = st.get(e) {
                vram.load_tim(t, None, Some(at));
            }
        }
        for k in 0..16usize {
            if let Some(Some(t)) = st.get(32 + k) {
                vram.load_tim(t, None, Some((256 + 16 * k, 480)));
            }
        }
    }
    // every stage: the darkening builds the CLUT restore / TILE packets with no stage test
    crate::fight::stage::anim::clear_stp(&mut vram);
    // the HUD pages again on top (they must win over any fight data, as the HUD reloads them per match)
    for (k, at) in HUD_PAGES {
        if let Some(Some(t)) = init.get(k) {
            vram.load_tim(t, None, Some(at));
        }
    }
    // music: BGM by the opponent's character (BGM set 0): track 0x14 + table[chr]; XA file
    // MC0{track/8+1}, channel track%8
    let mut debris = Vec::new();
    // the pieces' `_v2` files (ok_br2_converter::stages) come as a set with the stage's (ST3): looked for only with
    // the stage's v2 file in use, until one is missing; a v2 file that does not parse: the piece's v1 file (engine
    // addition)
    let mut try_v2 = stage_v2;
    for k in 0..8 {
        let mut piece = None;
        if try_v2 {
            let name = format!("stages/{stage}_debris{k}_v2.glb");
            match assets.load(&name).await {
                Ok(b) => piece = ModelData::from_glb(&b).inspect_err(|e| log::warn!("{name}: {e}; the v1 file instead")).ok(),
                Err(_) => try_v2 = false,
            }
        }
        if piece.is_none() {
            piece = match assets.load(&format!("stages/{stage}_debris{k}.glb")).await {
                Ok(b) => ModelData::from_glb(&b).ok(),
                Err(_) => None,
            };
        }
        let Some(m) = piece else { break };
        debris.push(m);
    }
    let opp = CHARACTERS.iter().position(|c| *c == p2).unwrap_or(0);
    // an extra stage: BGM SELECT = ARCADE plays its `Extra::music_stage`'s track (Q3DM17 BUILDING2's, Q3DM6 ROOM's)
    let music_stage = extra_stage.map_or(stage_no, |e| e.music_stage);
    let (bgm, cont, gover) = music_tracks(arcade_bgm, opp, music_stage);
    let mut music = Vec::new();
    for track in [bgm, 3, 4, cont, gover, 2] {
        if let Ok(bytes) = assets.load_music(&crate::formats::music::track_file(track)).await
            && let Some(m) = crate::formats::music::decode(bytes)
        {
            music.push((track, std::sync::Arc::new(m)));
        }
    }
    let mut fighters = Vec::new();
    for (k, (code, costume)) in [(p1, costume[0] as u32 + 1), (p2, costume[1] as u32 + 1)].into_iter().enumerate() {
        if let Some(b) = &mut br1[k] {
            // an imported side: its models (moved here; drawn in the bind pose until S8) and its main-game
            // proxy's (`code`) scripts and poses, which only the proxy victim of a main-game throw plays
            // (`fight::sim::br1`: the cross-engine throws); no clips
            let models = std::mem::replace(&mut b.models, [ModelData::empty(), ModelData::empty()]);
            let act = Act::parse(&assets.load(&format!("engine/{code}.ACT")).await?);
            let ani = Ani::parse(assets.load(&format!("engine/{code}{costume}P.ANI")).await?);
            fighters.push(FighterRes { code: b.res.code.clone(), models, act, ani, extras: Default::default(), skels: Vec::new() });
            continue;
        }
        fighters.push(load_br2_fighter(assets, code, costume).await?);
    }
    // every fighter clip is built from the ACTs (own, CMN.ACT, and the opponent's for throw victims)
    for i in 0..2 {
        if br1[i].is_some() {
            continue;
        }
        let opp_act = fighters[1 - i].act.clone();
        fighter_clips(&mut fighters[i], &cmn, Some(&opp_act), Gates::of(sim_version));
    }
    if fighters.len() != 2 {
        return Err("fighters".into());
    }
    Ok(Loaded {
        stage: stage_model,
        stage_name: extra_stage.map_or_else(|| stage_entry["name"].as_str().unwrap_or(stage).to_string(), |e| e.name.into()),
        fighters,
        cmn,
        trig,
        ring,
        stage_no,
        stage_id: extra_stage.map_or(stage_no, |e| e.stage as usize),
        arena,
        sound_banks,
        vram,
        eft,
        eff_tab,
        costume,
        debris,
        stage_objs,
        stage_semi,
        stage_ot,
        tables,
        music,
        music_cont: (cont, gover),
        br1: vec![None, None],
        br1_ghost: vec![None, None],
        br1_banks: Vec::new(),
        br1_stage: None,
        br1_bg: None,
        br1_light: None,
        br1_props: None,
        br1_walls: Vec::new(),
        team: None,
        sim_version,
        gates: Gates::of(sim_version),
    })
}

/// A main-game fighter's models, ACT, ANI and extra nodes (`costume` 1..4: the k P files), without clips.
async fn load_br2_fighter(assets: &Assets, code: &str, costume: u32) -> Result<FighterRes, String> {
    let human = ModelData::from_glb(&assets.load(&format!("characters/{code}_{costume}P_human.glb")).await?)?;
    let beast = ModelData::from_glb(&assets.load(&format!("characters/{code}_{costume}P_beast.glb")).await?)?;
    let act = Act::parse(&assets.load(&format!("engine/{code}.ACT")).await?);
    let ani = Ani::parse(assets.load(&format!("engine/{code}{costume}P.ANI")).await?);
    let extras = [crate::fight::anim::spring::extra_nodes(&ani, 0), crate::fight::anim::spring::extra_nodes(&ani, 1)];
    Ok(FighterRes { code: code.to_string(), models: [human, beast], act, ani, extras, skels: Vec::new() })
}

/// Fighter `fr`'s clips and skeletons per form: its own ACT's (`anim_`), CMN.ACT's (`cmn_`) and the throw victims'
/// moves of the opponent's ACT `opp_act` (`opp_`; None: none, the team battle's are `team_opp_clips`).
fn fighter_clips(fr: &mut FighterRes, cmn: &Act, opp_act: Option<&Act>, g: Gates) {
    let ids = crate::fight::anim::act_clips::opponent_anim_ids(&fr.ani);
    for form in 0..2 {
        let skel = crate::fight::anim::act_clips::Skel::new(&fr.ani, form, fr.models[form].joints.len(), g.portable_math);
        let m = &fr.models[form];
        let bc = fr.ani.u16(0x30) as i32;
        let mut clips = std::collections::HashMap::new();
        for id in 0..fr.act.offsets.len() {
            if let Some(c) = crate::fight::anim::act_clips::build(m, &skel, &fr.act, id, false, 0x80) {
                clips.insert(format!("anim_{id:03}"), c);
            }
        }
        for id in 0..cmn.offsets.len() {
            if let Some(c) = crate::fight::anim::act_clips::build(m, &skel, cmn, id, true, bc) {
                clips.insert(format!("cmn_{id:03}"), c);
            }
        }
        if let Some(opp_act) = opp_act {
            for &id in &ids {
                if let Some(c) = crate::fight::anim::act_clips::build(m, &skel, opp_act, id, false, bc) {
                    clips.insert(format!("opp_{id:03}"), c);
                }
            }
        }
        fr.models[form].clips = clips;
        fr.skels.push(skel);
    }
}

/// The team battle's opponent clips of fighter `fr` (engine addition): the throw victims' moves of fighter `k`'s ACT
/// `opp_act` as `opp<k>_` (`clip_for_vs`), the skeletons `fighter_clips` built.
fn team_opp_clips(fr: &mut FighterRes, k: usize, opp_act: &Act) {
    let ids = crate::fight::anim::act_clips::opponent_anim_ids(&fr.ani);
    let bc = fr.ani.u16(0x30) as i32;
    for form in 0..2 {
        let (m, skel) = (&fr.models[form], &fr.skels[form]);
        let clips: Vec<_> = ids
            .iter()
            .filter_map(|&id| crate::fight::anim::act_clips::build(m, skel, opp_act, id, false, bc).map(|c| (format!("opp{k}_{id:03}"), c)))
            .collect();
        fr.models[form].clips.extend(clips);
    }
}

/// What a team battle's data adds to a match's (engine addition).
pub struct TeamData {
    /// costumes (F.4) of all four fighters (`Loaded::costume` holds the first two)
    pub costume: [u8; 4],
    /// the stage model's scale in x / z, in percent (the ring's, `stage::TEAM_SCALE_PCT`; drawing only)
    pub stage_scale: u16,
    /// VRAM bank 1 (drawing only): `Loaded::vram` with fighters 2 / 3's `<CHR>EFT.DAT` sheets and CLUTs at P1's / P2's
    /// places, which the effects of fighters 2 / 3 sample (their scripts address their pages as P1's / P2's)
    pub vram1: crate::formats::vram::Vram,
}

/// Engine addition (the team battle): a four-fighter match on stage `stage` (one of
/// `stage::TEAM_STAGES`; from simulation v23 also `TEAM_BIG_STAGES`: Q3DM6 with its local files, BIG STAGE with the
/// install of the imported game), with the imported game's install beside the game data (`load_team_with`).
pub async fn load_team(assets: &Assets, chr: [&str; 4], costume: [u8; 4], stage: &str, sim_version: u32) -> Result<Loaded, String> {
    load_team_with(assets, assets.br1().as_ref(), chr, costume, stage, sim_version).await
}

/// `load_team` with the imported game's install `br1_dir` (None: none). Fighters 0 / 1 load as a match's P1 / P2;
/// fighters 2 / 3 (bank 1: P3 / P4) add their models and scripts, their `<CHR>EFT.DAT` entries as EffTab 100..119 /
/// 120..139 and their character sound banks as slots 7 / 8. Every main-game fighter gets the throw victims' clips
/// of each other main-game fighter (`team_opp_clips`). Their effect VRAM pages go into bank 1
/// (`TeamData::vram1`). From simulation v23 (T5) a fighter may be an imported one (its data, models and proxy as
/// in a match: `load_match_with`; a main-game fighter's imported proxy when any is on stage; the imported game's
/// sound slots 5 / 6 for P3's / P4's `XX00`).
pub async fn load_team_with(
    assets: &Assets,
    br1_dir: Option<&Assets>,
    chr: [&str; 4],
    costume: [u8; 4],
    stage: &str,
    sim_version: u32,
) -> Result<Loaded, String> {
    let g = Gates::of(sim_version);
    if !g.team {
        return Err(format!("the team battle needs simulation version {}", v::TEAM));
    }
    if let Some(c) = chr.iter().find(|c| is_br1(c))
        && !g.team_br1
    {
        return Err(format!("{c}: imported fighters in the team battle need simulation version {}", v::TEAM_BR1));
    }
    let b1stage = crate::fight::stage::br1::by_code(stage, g);
    let id = STAGE_CODES
        .iter()
        .position(|s| *s == stage)
        .or_else(|| crate::fight::stage::extra::by_code(stage, g).map(|e| e.stage as usize))
        .or_else(|| b1stage.map(|s| s.stage as usize));
    if !id.is_some_and(|n| crate::fight::stage::team_stage(n, g)) {
        return Err(format!("{stage}: not a team battle stage"));
    }
    let mut shared = None;
    let mut br1: [Option<Box<Br1FighterRes>>; 4] = Default::default();
    for (k, code) in chr.iter().enumerate() {
        if is_br1(code) {
            let dir = br1_dir.ok_or_else(|| format!("{code}: no install of the imported game"))?;
            br1[k] = Some(Box::new(load_br1_fighter(dir, br1_shared(dir, &mut shared).await?, code, costume[k]).await.map_err(|e| format!("{code}: {e}"))?));
        }
    }
    // a main-game fighter's imported proxy (the victim of an imported throw puppets it)
    let mut ghost: [Option<Box<crate::fight::br1::res::Br1Res>>; 4] = Default::default();
    if br1.iter().any(Option::is_some) {
        for (k, code) in chr.iter().enumerate() {
            if br1[k].is_none() {
                let c = CHARACTERS.iter().position(|c| c == code).unwrap_or(0) as u8;
                let proxy = crate::formats::br1_anm::CHARACTERS[crate::fight::br1::bridge::br1_proxy(c) as usize];
                let dir = br1_dir.ok_or_else(|| format!("{code}: no install of the imported game"))?;
                ghost[k] = Some(Box::new(
                    crate::fight::br1::res::Br1Res::load(dir, proxy, br1_shared(dir, &mut shared).await?).await.map_err(|e| format!("{proxy}: {e}"))?,
                ));
            }
        }
    }
    let banks = match br1_dir {
        Some(dir) if br1.iter().any(Option::is_some) || b1stage.is_some() => {
            let bank = b1stage.map(|s| format!("{}/{}", s.dir, s.bank));
            let codes: Vec<Option<&str>> = br1.iter().map(|b| b.as_ref().map(|b| b.res.code.as_str())).collect();
            br1_sound_banks(dir, &br1_shared(dir, &mut shared).await?.tables, &codes, bank.as_deref()).await
        }
        _ => Vec::new(),
    };
    let codes = chr.map(br2_code);
    let br2_stage = if b1stage.is_some() { STAGE_CODES[crate::fight::stage::br1::STAND_IN] } else { stage };
    let [b0, b1, b2, b3] = br1;
    let mut br1_01 = [b0, b1];
    let mut d = load_br2_match(assets, [codes[0], codes[1]], [costume[0], costume[1]], br2_stage, false, sim_version, &mut br1_01).await?;
    let mut br1 = [None, None, b2, b3];
    for k in 2..4 {
        if let Some(b) = &mut br1[k] {
            // an imported fighter: its models and its main-game proxy's scripts (as `load_br2_match`'s)
            let models = std::mem::replace(&mut b.models, [ModelData::empty(), ModelData::empty()]);
            let act = Act::parse(&assets.load(&format!("engine/{}.ACT", codes[k])).await?);
            let ani = Ani::parse(assets.load(&format!("engine/{}{}P.ANI", codes[k], costume[k] as u32 + 1)).await?);
            d.fighters.push(FighterRes { code: b.res.code.clone(), models, act, ani, extras: Default::default(), skels: Vec::new() });
            continue;
        }
        let mut f = load_br2_fighter(assets, codes[k], costume[k] as u32 + 1).await?;
        // the match's `opp_` clips are never played here (`clip_opp` names the thrower): fighters 2 / 3 get none
        fighter_clips(&mut f, &d.cmn, None, g);
        d.fighters.push(f);
    }
    [br1[0], br1[1]] = br1_01;
    for f in &mut d.fighters[..2] {
        for m in &mut f.models {
            m.clips.retain(|name, _| !name.starts_with("opp_"));
        }
    }
    for i in (0..4).filter(|&i| br1[i].is_none()) {
        for k in (0..4).filter(|&k| k != i && br1[k].is_none()) {
            let opp_act = d.fighters[k].act.clone();
            team_opp_clips(&mut d.fighters[i], k, &opp_act);
        }
    }
    let chr = codes;
    d.eff_tab.resize(140, None);
    // The imported game's effect pages in both banks (before bank 1 is copied)
    if let (Some(dir), true) = (br1_dir, br1.iter().any(Option::is_some)) {
        load_br1_effect_pages(dir, &mut d.vram).await;
    }
    let index = assets.load_json("engine/index.json").await?;
    // bank 1: fighter 2's sheet where P1's goes (the fighter loader's places), fighter 3's where P2's; then the HUD pages on
    // top again, as `load_br2_match` ends (the stage's pages and CLUTs and the STP clear lie outside these places)
    let mut vram1 = crate::formats::vram::Vram { words: d.vram.words.clone() };
    for k in 2..4 {
        let code = chr[k];
        let c = crate::formats::container::read(&assets.load(&format!("engine/eft/{code}EFT.DAT")).await.unwrap_or_default());
        if let Some(Some(t)) = c.first() {
            let (clut, pix) = if k == 2 { ((96, 480), (960, 256)) } else { ((112, 480), (960, 320)) };
            vram1.load_tim(t, Some(clut), Some(pix));
        }
        for e in 0..20 {
            d.eff_tab[eft_base(k) + e] = c.get(1 + e).cloned().flatten();
        }
    }
    // sound slots 7 / 8: P3's / P4's CHARA_xx.SND (slots 5 / 6 stay the game-over voices)
    for code in [chr[2], chr[3]] {
        let name = format!("CHARA_{:02}.SND", CHARACTERS.iter().position(|x| *x == code).unwrap_or(0));
        let bytes = assets.load(&format!("engine/snd/{name}")).await.unwrap_or_default();
        let vols = index["sound_defs"][&name].as_array().map(|a| a.iter().map(|d| d[3].as_u64().unwrap_or(0x1000) as u16).collect()).unwrap_or_default();
        d.sound_banks.push((name, bytes, vols));
    }
    d.br1 = br1.into();
    d.br1_ghost = ghost.into();
    d.br1_banks = banks;
    let init = crate::formats::container::read(&assets.load("engine/INIT.DAT").await.unwrap_or_default());
    for (k, at) in HUD_PAGES {
        if let Some(Some(t)) = init.get(k) {
            vram1.load_tim(t, None, Some(at));
        }
    }
    if let Some(s) = b1stage {
        let dir = br1_dir.ok_or_else(|| format!("{stage}: no install of the imported game"))?;
        load_br1_stage(dir, &br1_shared(dir, &mut shared).await?.tables, s, &mut d, false, chr[1]).await?;
    }
    // the big stages (T5) at their own size
    let big = id.is_some_and(|n| crate::fight::stage::TEAM_BIG_STAGES.contains(&n));
    let stage_scale = if big { 100 } else { crate::fight::stage::TEAM_SCALE_PCT };
    d.team = Some(TeamData { costume, stage_scale, vram1 });
    assert!(d.fighters.len() == 4 && d.br1.len() == 4 && d.br1_ghost.len() == 4 && d.eff_tab.len() == 140 && d.sound_banks.len() == 9);
    Ok(d)
}

/// A split stage export's parts (`load_match`): the stage without its objects (`base`: `<ST>_base.glb`, or
/// `<ST>_opaque.glb` with `semi`), the semi-transparent polygons, and the objects `stage::anim::objects` moves; Err
/// when one is missing or does not parse.
async fn split_parts(
    assets: &Assets,
    stage: &str,
    stage_no: usize,
    base: &[u8],
    semi: Option<&[u8]>,
) -> Result<(ModelData, Option<ModelData>, Vec<(u8, ModelData)>), String> {
    let base = ModelData::from_glb(base)?;
    let semi = semi.map(ModelData::from_glb).transpose()?;
    let mut objs = Vec::new();
    for &k in crate::fight::stage::anim::objects(stage_no) {
        let name = format!("stages/{stage}_obj{k}.glb");
        objs.push((k, ModelData::from_glb(&assets.load(&name).await?).map_err(|e| format!("{name}: {e}"))?));
    }
    Ok((base, semi, objs))
}

impl Loaded {
    /// Side `i`'s imported-game data (None: a main-game fighter).
    pub fn br1_res(&self, i: usize) -> Option<&crate::fight::br1::res::Br1Res> {
        self.br1[i].as_deref().map(|b| &b.res)
    }

    /// ACTs for fighter `me` against `opp`: its own, CMN.ACT and the opponent's (a throw victim plays its thrower's).
    pub fn acts(&self, me: usize, opp: usize) -> crate::fight::anim::playback::Acts<'_> {
        crate::fight::anim::playback::Acts {
            own: &self.fighters[me].act,
            cmn: &self.cmn,
            opp: &self.fighters[opp].act,
            root_scale: self.fighters[me].ani.u16(0x30) as i32,
            ani: &self.fighters[me].ani,
        }
    }
}

/// A match's clip of animation `a` (a team battle's fighters have no `opp_` clips: `clip_for_vs` with `clip_opp`).
pub fn clip_for<'a>(m: &'a ModelData, a: &crate::fight::anim::playback::Anim) -> Option<&'a gltf_model::Clip> {
    clip_for_vs(m, a, None)
}

/// `clip_for` of a fighter whose opponent is fighter `opp` of a team battle (its `opp<k>_` clips, `load_team`); None:
/// a match's (`opp_`).
pub fn clip_for_vs<'a>(m: &'a ModelData, a: &crate::fight::anim::playback::Anim, opp: Option<usize>) -> Option<&'a gltf_model::Clip> {
    let id = a.id;
    if id < 0 {
        return None;
    }
    match (a.src, opp) {
        (crate::fight::anim::playback::Src::Opp, None) => m.clips.get(&format!("opp_{id:03}")),
        (crate::fight::anim::playback::Src::Opp, Some(k)) => m.clips.get(&format!("opp{k}_{id:03}")),
        _ => m.clips.get(&format!("anim_{id:03}")).or_else(|| m.clips.get(&format!("cmn_{id:03}"))),
    }
}

impl Loaded {
    /// The opponent whose `opp` clips fighter `me` plays against fighter `opp`: a team battle's (`clip_for_vs`), None
    /// in a match.
    pub fn clip_opp(&self, opp: usize) -> Option<usize> {
        self.team.as_ref().map(|_| opp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_used_images_keeps_the_used_ones() {
        let img = |k: u32| gltf_model::Image { name: format!("i{k}"), width: k, height: 1, rgba: vec![k as u8; 4 * k as usize] };
        let prim = |t: Option<usize>| gltf_model::Prim {
            first_index: 0,
            count: 0,
            texture: t,
            hidden: false,
            lit: false,
            wall: None,
            shadow: false,
            over: false,
            ot: None,
            semi: false,
            node: 0,
        };
        let mut m = ModelData {
            vertices: Vec::new(),
            indices: Vec::new(),
            prims: [Some(2), None, Some(2), Some(5)].map(prim).to_vec(),
            images: (0..3).map(img).collect(),
            nodes: Vec::new(),
            joints: Vec::new(),
            inverse_bind: Vec::new(),
            clips: Default::default(),
        };
        only_used_images(&mut m);
        assert_eq!(m.images.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["i2"]);
        assert_eq!(m.prims.iter().map(|p| p.texture).collect::<Vec<_>>(), [Some(0), None, Some(0), None]);
    }
}
