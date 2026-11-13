//! A loaded match on screen: `FightView` (the GPU objects) and `Game` (the fight as the app draws it).
//!
//! State: presentation (reads the state, never writes it)
//!
//! A loaded match on screen: the GPU objects that draw the fight's logic state (`FightView`, the app's) and the
//! three together as the app draws them (`Game`: `session::fight::Fight`, its data, its view).
//!
//! The parts: `scene.rs` draws a fight's frame (`FightView::draw_capture`), `arena.rs` an extra stage's map and
//! sky (engine addition), `stage_draw.rs` the stage's part of it
//! and the stage alone (the backdrop, the shadow floor), `stage_cull.rs` the stage
//! polygons the PS1 drops (the stage sorter's drivers, the GPU's size limit), `debug_hud.rs` the OKB_OLDHUD stand-in
//! (engine addition). `team.rs` draws the team battle (engine addition: `TeamGame`, `FightView::draw_team`).

mod arena;
#[cfg(not(target_arch = "wasm32"))]
mod debug_hud;
mod scene;
mod stage_cull;
mod stage_draw;
mod team;

use crate::fight::fight_data::Loaded;
use crate::fight::scene::SceneRef;
use crate::render::{GpuModel, Instance, Renderer};
use crate::session::fight::Fight;
use stage_draw::ot_units;

/// A loaded match: its data, the fight's logic state and the GPU objects that show it.
/// A fight as the app draws it: the state's `Fight`, its data from the app's store and its view.
#[derive(Clone, Copy)]
pub struct Game<'a> {
    pub data: &'a Loaded,
    pub fight: &'a Fight,
    pub view: &'a FightView,
}

/// The team battle as the app draws it (engine addition): the state's `TeamFight`, its data, its view.
#[derive(Clone, Copy)]
pub struct TeamGame<'a> {
    pub data: &'a Loaded,
    pub team: &'a crate::fight::team::TeamFight,
    pub view: &'a FightView,
    /// the pause's CONTROLLER CONFIG. screen when open (T6, `session::TeamConfig`)
    pub config: Option<&'a crate::frontend::menus::Menu>,
}

impl<'a> TeamGame<'a> {
    /// The three parts, when all are there, and the open button-mapping screen.
    pub fn of(
        data: Option<&'a Loaded>,
        team: Option<&'a crate::fight::team::TeamFight>,
        view: Option<&'a FightView>,
        config: Option<&'a crate::frontend::menus::Menu>,
    ) -> Option<Self> {
        Some(Self { data: data?, team: team?, view: view?, config })
    }

    pub fn draw_capture(&self, r: &mut Renderer, capture: bool) -> Option<(u32, u32, Vec<u8>)> {
        self.view.draw_team(self.team, self.data, self.config, r, capture)
    }

    /// The window title's status line (T6, as `Game::status`): the mode, each fighter's health (`-` once out of the
    /// round), the stage, the round's message, the wins by team.
    pub fn status(&self) -> String {
        let t = self.team;
        let mode = match t.mode {
            crate::fight::team::TeamMode::Vs2 => "2 VS 2",
            crate::fight::team::TeamMode::Ffa => "FREE FOR ALL",
        };
        let fighters: Vec<String> = (0..crate::fight::team::N)
            .map(|k| {
                let f = &t.fighters[k];
                let hp = if t.round.out[k] { "-".to_string() } else { format!("{}/{}", (f.o_b0 as i32 - f.o_ac as i32).max(0), f.o_b0) };
                format!("P{} {} {hp}", k + 1, self.data.fighters[k].code)
            })
            .collect();
        format!(
            "OpenKumiteBeast · TEAM {mode} · {} · {} · {} · wins {:?}",
            fighters.join(" "),
            self.data.stage_name,
            t.round.message,
            &t.round.wins[..t.mode.teams()]
        )
    }
}

/// The fight on screen: the GPU objects of the stage, the fighters and the debris, the debug camera. Draws a `Fight`.
pub struct FightView {
    stage_gpu: GpuModel,
    stage_inst: Instance,
    /// by fighter (the team battle's four: engine addition)
    fighter_gpu: Vec<[GpuModel; 2]>,
    fighter_inst: Vec<Instance>,
    shadow_inst: Vec<Instance>,
    debris_gpu: Vec<GpuModel>,
    debris_inst: Vec<Instance>,
    /// the stage objects the stage routine moves or shows on alternate frames (`stage::anim::objects`)
    obj_gpu: Vec<(u8, GpuModel, Instance)>,
    /// the stage's semi-transparent polygons (`Loaded::stage_semi`)
    semi_gpu: Option<(GpuModel, Instance)>,
    /// a stage in ordering-table order (`Loaded::stage_ot`): per unit (object or `gltf_model::FLOOR`) its draws in
    /// `stage_gpu` and an instance
    ot_units: Vec<(u8, (u32, u32), Instance)>,
    /// the polygons of those units the PS1 does not draw, kept out of `stage_gpu`'s indices per frame
    cull: Option<stage_cull::StageCull>,
    /// the textures drawn through the stage palette (`stage::anim::paletted`): model (0 = stage, 1 + k = debris
    /// piece k), image, CLUT
    pal_tex: Vec<(usize, usize, u16)>,
    /// an extra stage's map and sky (`Loaded::arena`; engine addition)
    arena: Option<arena::ArenaView>,
    /// the Beast Drive line grids (built at stage load; none from stage 9 on)
    drive_grid: Option<[crate::present::drive_grid::Grid; 5]>,
    pub cam_yaw: f32,
    pub cam_dist: f32,
    /// +1 per drawn frame (paused too), the labels' blink clock
    dc: std::cell::Cell<u32>,
    /// debug overlay primitives drawn on top (512×480 space)
    pub overlay: Vec<crate::output::prim::Prim>,
    /// a replay's HUD switched off (the playback controls; engine addition): no HUD, round messages or labels
    pub hide_hud: bool,
    /// engine addition (the import S8): an imported side's models' joints (human, beast) and its spring chains
    /// by form (`present::br1_fighter`); None for a main-game side
    br1: Vec<Option<Br1Side>>,
    /// engine addition (the import S8): an imported stage's background (`present::br1_stage_bg`)
    br1_bg: Option<(crate::present::br1_stage_bg::Background, GpuModel, Instance)>,
    /// engine addition (the import S8): a match with an imported fighter: its effects' tables and their motion
    /// (`present::br1_fx`)
    br1_fx: Option<(crate::present::br1_fx::Tables, std::cell::RefCell<crate::present::br1_fx::Br1Fx>)>,
    /// engine addition (the import S8): an imported stage's props (`present::br1_props`), their objects' GPU models
    /// and the instances of the parts drawn
    br1_props: Option<(std::cell::RefCell<crate::present::br1_props::Props>, Vec<GpuModel>, Vec<Instance>)>,
    /// engine addition (the import S8): Uriko's stage's wall models (`Loaded::br1_walls`), drawn as Uriko's stage routine draws
    /// them
    br1_walls: Vec<(GpuModel, Instance)>,
}

/// The stage model's transform: an imported stage's (ok_br1_converter's scene in 1/256 units, the renderer's 1/300;
/// engine addition, the import S8), the team battle's enlarged stage (`team_scale`), else none.
fn stage_xf(data: &Loaded) -> glam::Mat4 {
    if data.br1_stage.is_some() { glam::Mat4::from_scale(glam::Vec3::splat(256.0 / 300.0)) } else { team_scale(data) }
}

/// `data`'s effect VRAM to the GPU: bank 0, and the team battle's bank 1 (engine addition, `TeamData::vram1`).
pub fn upload_vram(r: &mut Renderer, data: &Loaded) {
    r.upload_vram(&data.vram.words);
    if let Some(t) = &data.team {
        r.upload_vram_bank1(&t.vram1.words);
    }
}

/// The team battle's stage (engine addition): the stage's geometry, its objects and
/// ordering-table units scaled in x / z with the ring (`TeamData::stage_scale`), the fighters and effects not; else none.
fn team_scale(data: &Loaded) -> glam::Mat4 {
    match &data.team {
        Some(t) => {
            let s = t.stage_scale as f32 / 100.0;
            glam::Mat4::from_scale(glam::Vec3::new(s, 1.0, s))
        }
        None => glam::Mat4::IDENTITY,
    }
}

/// Fighter `i`'s light set: the main game's, or on an imported stage the imported game's
/// (`fighter_lighting::fighter_br1`, engine addition; a main-game side keeps its colour flash's back colour and its
/// hit flash, aimed as the light set-up aims it); `br1_flash`: an imported hit's light #3 flash (`br1_fx::Br1Fx::light3`);
/// `br1_back`: the back colour the imported game's back colour scripts set per fighter (`br1_fx::Br1Fx::back`).
fn lights(
    data: &Loaded,
    s: &SceneRef,
    i: usize,
    br1_flash: Option<[i32; 3]>,
    br1_back: [[Option<i32>; 3]; crate::present::br1_fx::SIDES],
) -> crate::present::fighter_lighting::Lights {
    use crate::present::fighter_lighting::{fighter_br1, flash_dir};
    let fi = &s.fighters[i];
    // (the stage's lighting hook watches P2 when P2 is the imported game's Uriko, else P1; the team battle: `Br1Scene::watched`)
    let glow = s.fighters[s.br1.map_or(0, |c| c.watched())].beast();
    // (the light record's back colours are P1's and P2's: a team battle's fighter takes its slot's, `i & 1`)
    let mut l = match (&data.br1_light, &data.br1_stage) {
        (Some(rec), Some(b)) if data.br1[i].is_none() => {
            let mut l = fighter_br1(&data.tables, rec, &data.trig, b.g94, i & 1, s.cam.ang[1], s.frame, s.fx.flash.or(br1_flash), glow);
            if s.fx.flash.is_some() {
                l.dir[2] = flash_dir(&data.trig, fi);
            }
            if let Some(c) = s.back_colour[i] {
                l.back = c;
            }
            l
        }
        (Some(rec), Some(b)) => fighter_br1(&data.tables, rec, &data.trig, b.g94, i & 1, s.cam.ang[1], s.frame, br1_flash, glow),
        _ => crate::present::fighter_lighting::fighter(&data.tables, &data.trig, data.stage_no, i & 1, fi, s.back_colour[i], s.fx.flash.or(br1_flash)),
    };
    for (c, v) in br1_back[i].iter().enumerate() {
        if let Some(v) = v {
            l.back[c] = *v;
        }
    }
    l
}

/// An imported fighter's view: its rigs and its chains' motion by form.
struct Br1Side {
    rig: [crate::present::br1_fighter::Rig; 2],
    chains: std::cell::RefCell<[crate::present::br1_fighter::Chains; 2]>,
    /// its character in the game's order (`formats::br1_anm::CHARACTERS`)
    chr: u8,
}

impl<'a> Game<'a> {
    /// The three parts, when all are there.
    pub fn of(data: Option<&'a Loaded>, fight: Option<&'a Fight>, view: Option<&'a FightView>) -> Option<Self> {
        Some(Self { data: data?, fight: fight?, view: view? })
    }

    pub fn draw(&self, r: &mut Renderer) {
        self.view.draw_capture(self.fight, self.data, r, false);
    }

    pub fn draw_capture(&self, r: &mut Renderer, capture: bool) -> Option<(u32, u32, Vec<u8>)> {
        self.view.draw_capture(self.fight, self.data, r, capture)
    }

    /// `n`: the drawn frame for the stage animation (the front end's frame counter).
    pub fn draw_stage(
        &self,
        r: &mut Renderer,
        cam: &crate::fight::camera::Camera,
        n: u64,
        quads: &[crate::render::PsxQuad],
        capture: bool,
    ) -> Option<(u32, u32, Vec<u8>)> {
        self.view.draw_stage(&self.fight.m.scene(self.data), self.data, r, cam, n, quads, capture)
    }

    /// State lines for the debug overlay and the support report.
    pub fn debug_lines(&self) -> Vec<String> {
        let r = &self.fight.m.round;
        let mut v = vec![format!(
            "FIGHT {} frame {} round {} state {:#x} cnt {} timer {} lost {:?}/{} cam {} result {:?}",
            self.data.stage_name, self.fight.m.frame, r.index, r.state, r.cnt, r.timer, r.lost, r.need, self.fight.m.cam.mode, self.fight.m.result
        )];
        for i in 0..2 {
            let f = &self.fight.m.fighters[i];
            let p = self.fight.m.world_pos(i);
            v.push(format!(
                "P{} {} {} move {:#x} anim {} hp {}/{} gauge {:#x} form {:02x} pos {:.2} {:.2} {:.2} pad {:04x}",
                i + 1,
                self.data.fighters[i].code,
                if self.fight.m.cpu[i] { "CPU" } else { "HUMAN" },
                f.move_cur,
                f.anim.id,
                (f.o_b0 - f.o_ac).max(0),
                f.o_b0,
                f.gauge,
                f.form,
                p.x,
                p.y,
                p.z,
                self.fight.m.pads[i]
            ));
        }
        if let Some(t) = self.fight.m.fighters.iter().flat_map(|f| f.trace.last()).last() {
            v.push(format!("trace {t}"));
        }
        v
    }

    pub fn status(&self) -> String {
        let f = &self.fight.m.fighters[0];
        format!(
            "OpenKumiteBeast · {} {}/{} vs {} {}/{} · {} · {} · P1 move {:#x} anim {} key {} pad {:04x}{}",
            self.data.fighters[0].code,
            (self.fight.m.fighters[0].o_b0 - self.fight.m.fighters[0].o_ac).max(0),
            self.fight.m.fighters[0].o_b0,
            self.data.fighters[1].code,
            (self.fight.m.fighters[1].o_b0 - self.fight.m.fighters[1].o_ac).max(0),
            self.fight.m.fighters[1].o_b0,
            self.data.stage_name,
            self.fight.m.round.message,
            f.move_cur,
            f.anim.id,
            f.anim.key,
            f.held,
            f.trace.last().map(|t| format!(" · {t}")).unwrap_or_default()
        )
    }
}

impl FightView {
    /// GPU bytes (textures, buffers) of the fight's models, for the performance overlay.
    pub fn gpu_bytes(&self) -> [u64; 2] {
        std::iter::once(&self.stage_gpu)
            .chain(self.fighter_gpu.iter().flatten())
            .chain(&self.debris_gpu)
            .chain(self.br1_props.iter().flat_map(|(_, g, _)| g))
            .chain(self.br1_walls.iter().map(|(g, _)| g))
            .chain(self.obj_gpu.iter().map(|(_, g, _)| g))
            .chain(self.semi_gpu.iter().map(|(g, _)| g))
            .map(|m| m.bytes)
            .chain(self.arena.iter().map(arena::ArenaView::bytes))
            .fold([0, 0], |a, b| [a[0] + b[0], a[1] + b[1]])
    }

    /// The GPU objects of `data`'s stage, fighters and debris; the stage VRAM and sound banks go up, the sound starts.
    pub fn new(r: &mut Renderer, data: &Loaded, audio: &mut crate::audio::Audio) -> Self {
        let mut v = Self {
            stage_gpu: r.upload(&data.stage),
            stage_inst: r.instance(),
            fighter_gpu: data.fighters.iter().map(|f| [r.upload(&f.models[0]), r.upload(&f.models[1])]).collect(),
            fighter_inst: data.fighters.iter().map(|_| r.instance()).collect(),
            shadow_inst: data.fighters.iter().map(|_| r.instance()).collect(),
            debris_gpu: data.debris.iter().map(|m| r.upload(m)).collect(),
            debris_inst: data.debris.iter().map(|_| r.instance()).collect(),
            obj_gpu: data.stage_objs.iter().map(|(k, m)| (*k, r.upload(m), r.instance())).collect(),
            semi_gpu: data.stage_semi.as_ref().map(|m| (r.upload(m), r.instance())),
            ot_units: if data.stage_ot { ot_units(&data.stage).into_iter().map(|(u, range)| (u, range, r.instance())).collect() } else { Vec::new() },
            cull: data.stage_ot.then(|| stage_cull::StageCull::new(&data.stage)),
            pal_tex: Vec::new(),
            arena: data.arena.as_ref().zip(crate::fight::stage::extra::by_stage(data.stage_id)).map(|(a, e)| arena::ArenaView::new(r, e, a, data.gates())),
            // (the team battle: over its enlarged ring)
            drive_grid: crate::present::drive_grid::build_scaled(&data.tables, data.stage_no, data.team.as_ref().map_or(100, |t| t.stage_scale)),
            cam_yaw: 0.0,
            cam_dist: 0.0,
            dc: std::cell::Cell::new(0),
            overlay: Vec::new(),
            hide_hud: false,
            br1: (0..data.sides())
                .map(|i| {
                    let res = data.br1_res(i)?;
                    let chr = crate::formats::br1_anm::CHARACTERS.iter().position(|c| *c == res.code).unwrap_or(0) as u8;
                    let m = &data.fighters[i].models;
                    Some(Br1Side {
                        rig: [0, 1].map(|k| crate::present::br1_fighter::Rig::new(&m[k])),
                        chains: std::cell::RefCell::new([0, 1].map(|k| crate::present::br1_fighter::Chains::new(res, k, chr))),
                        chr,
                    })
                })
                .collect(),
            br1_fx: (0..data.sides())
                .find_map(|i| data.br1_res(i))
                .map(|res| (crate::present::br1_fx::Tables::of(&res.tables), std::cell::RefCell::new(crate::present::br1_fx::Br1Fx::default()))),
            br1_props: data.br1_props.as_ref().zip(data.br1_stage.as_ref()).map(|(t, b)| {
                let props = crate::present::br1_props::Props::new(t, b);
                // (at most 4 slots of 3 parts and one of 1 among the drawn types: 24 is room enough)
                (std::cell::RefCell::new(props), t.objects.iter().map(|m| r.upload(m)).collect(), (0..24).map(|_| r.instance()).collect())
            }),
            br1_walls: data.br1_walls.iter().map(|m| (r.upload(m), r.instance())).collect(),
            br1_bg: data.br1_bg.as_ref().map(|(cfg, image)| {
                let image = crate::formats::gltf_model::Image { name: image.name.clone(), width: image.width, height: image.height, rgba: image.rgba.clone() };
                let bg = crate::present::br1_stage_bg::Background { cfg: *cfg, image };
                let gpu = r.upload(&bg.model());
                (bg, gpu, r.instance())
            }),
        };
        // debug: OKB_CAMYAW=radians starts the debug orbit turned (as Q / E), e.g. 3.14 to see behind the ring
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(a) = crate::base::env::var("CAMYAW").ok().and_then(|v| v.parse().ok()) {
            v.cam_yaw = a;
        }
        // the CLUT rows the stage routine copies (ST3 sea, ST5 tube lights): those textures drawn from their texel
        // indices and the stage's CLUT block, the row moved per frame (`stage_anim`); the floor's textures likewise,
        // each tile showing the row of its depth (the CLUT's y with its low 4 bits replaced)
        let floor_tex: Vec<usize> = data.stage.prims.iter().filter(|p| p.ot == Some(crate::formats::gltf_model::FLOOR)).filter_map(|p| p.texture).collect();
        if !crate::fight::stage::anim::cluts(data.stage_no).is_empty() || !floor_tex.is_empty() {
            use crate::fight::stage::anim::{PAL_H, PAL_W, PAL_X, PAL_Y};
            let pal = r.palette(PAL_W, PAL_H, &crate::fight::stage::anim::palette(&data.vram));
            let in_block = |c: u16| {
                let (x, y) = crate::fight::stage::anim::clut_xy(c);
                (PAL_X..PAL_X + PAL_W).contains(&x) && (PAL_Y..PAL_Y + PAL_H).contains(&y)
            };
            let models = std::iter::once((&data.stage, &mut v.stage_gpu)).chain(data.debris.iter().zip(v.debris_gpu.iter_mut()));
            for (mi, (m, gm)) in models.enumerate() {
                for (ii, img) in m.images.iter().enumerate() {
                    if let Some((clut, index)) = crate::fight::stage::anim::paletted(data.stage_no, img, &data.vram) {
                        let (x, y) = crate::fight::stage::anim::clut_xy(clut);
                        r.set_paletted(gm, ii, img.width, img.height, &index, &pal, [x - PAL_X, y - PAL_Y]);
                        v.pal_tex.push((mi, ii, clut));
                    } else if mi == 0
                        && floor_tex.contains(&ii)
                        && let Some((clut, index)) = crate::fight::stage::anim::page_indices(img, &data.vram, in_block)
                    {
                        let (x, y) = crate::fight::stage::anim::clut_xy(clut);
                        r.set_paletted(gm, ii, img.width, img.height, &index, &pal, [x - PAL_X, (y & !15) - PAL_Y]);
                    }
                }
            }
        }
        upload_vram(r, data);
        for (i, (name, bytes, _)) in data.sound_banks.iter().enumerate() {
            if !bytes.is_empty() {
                audio.load_bank(&format!("slot{i}:{name}"), bytes);
            }
        }
        // The imported game's slots (engine addition, the import S8)
        for (i, (_, bytes, _)) in data.br1_banks.iter().enumerate() {
            if !bytes.is_empty() {
                audio.load_bank(&format!("br1slot{i}"), bytes);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if crate::base::env::var("MUTE").is_err() {
            audio.start(); // on the web the first key press starts it
        }
        v
    }
}
