//! Clip checks: the main game's runtime ACT clips against the converter's; the imported game's pose port against the original.
//!
//! `cliptest OKB_EXPORT CODE`: clips built at runtime from an ACT (`act_clips`) against the converter's exported
//! ones (rotations must match; the body translation differs: the game's vs the converter's feet-on-floor height).
//!
//! `cliptest br1 BR1_DIR …` (port addition, the import; BR1_DIR = the imported game's install, `game_data/br1`):
//!
//! - `dumps DUMP_DIR…`: every PCSX-Redux RAM dump (`fN.ram` + `fN.spad`, `.research/br1/harness/out`) of an
//!   imported-game fight, both fighters, against `fight::br1::pose`: the base matrices of the form setup, the local matrices
//!   and translations from the dump's records (with the head turn), the world walk, and for consecutive dumps
//!   (fN, fN+1) the animation step, the facing and the head turn of frame N+1 from N's state. `V=1` prints every
//!   mismatch.
//! - `fixture DUMP_DIR OUT`: the derived inputs and a CRC-32 of the game's matrices per fighter of each dump, for the
//!   test `fight::br1::pose::tests::harness_dumps` (`src/fight/br1/fixtures/pose1.jsonl`).
//! - `pose CODE FORM CLIP FRAMES [MIRROR]`: plays clip CLIP (own table, then CMN.ACT) of fighter CODE's form 0 / 1
//!   from the bind pose for FRAMES frames at the normal step and prints node 0..18's world positions each frame.
//! - `shot CODE FORM CLIP FRAME OUT.png [MIRROR]`: that clip's pose after FRAME frames drawn with the fighter's model
//!   (`models/XX_*.glb`) by the offscreen renderer; `dumpshot DUMP OUT.png` (DUMP without `.ram`): both fighters of a
//!   dump posed by the port from the dump's records, to set beside the harness's screenshot of that frame.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("br1") {
        if let Err(e) = br1::main(&args[1..]) {
            eprintln!("cliptest br1: {e}");
            std::process::exit(1);
        }
        return;
    }
    use open_kumite_beast::fight::anim::act_clips;
    use open_kumite_beast::formats::gltf_model::Track;
    let root = args.first().cloned().unwrap_or("../okb_export".into());
    let code = args.get(1).cloned().unwrap_or("YUG".into());
    let rd = |p: &str| std::fs::read(format!("{root}/{p}")).unwrap();
    let ani = open_kumite_beast::formats::act_ani::Ani::parse(rd(&format!("engine/{code}1P.ANI")));
    let act = open_kumite_beast::formats::act_ani::Act::parse(&rd(&format!("engine/{code}.ACT")));
    for (form, f) in ["human", "beast"].iter().enumerate() {
        let m = open_kumite_beast::formats::gltf_model::ModelData::from_glb(&rd(&format!("characters/{code}_1P_{f}.glb"))).unwrap();
        let skel = act_clips::Skel::new(&ani, form, m.joints.len(), true);
        for id in [1usize, 0x90, 320] {
            let Some(g) = m.clips.get(&format!("anim_{id:03}")) else { continue };
            let Some(c) = act_clips::build(&m, &skel, &act, id, false, 0x80) else {
                println!("{f} {id}: none");
                continue;
            };
            let (mut dr, mut dt) = (0f32, 0f32);
            for ch in &g.channels {
                let Some(o) = c.channels.iter().find(|x| x.node == ch.node && std::mem::discriminant(&x.track) == std::mem::discriminant(&ch.track)) else {
                    continue;
                };
                match (&ch.track, &o.track) {
                    (Track::R(a), Track::R(b)) => {
                        for (x, y) in a.iter().zip(b) {
                            dr = dr.max(1.0 - x.dot(*y).abs())
                        }
                    }
                    (Track::T(a), Track::T(b)) => {
                        for (k, (x, y)) in a.iter().zip(b).enumerate() {
                            let e = (*x - *y).length();
                            if e > 0.01 && std::env::var("V").is_ok() {
                                println!("   node {} key {k}: exp {x} got {y}", m.nodes[ch.node].name);
                            }
                            dt = dt.max(e)
                        }
                    }
                    _ => {}
                }
            }
            println!("{f} anim {id}: max rot err {dr:.6} (1-|dot|), max translation err {dt:.4} m, dur {} vs {}", g.duration, c.duration);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod br1 {
    use glam::{Mat4, Vec3, Vec4};
    use open_kumite_beast::fight::br1::pose::{self, Anim, Banks, HeadInputs, M, Parts, Rec, Src};
    use open_kumite_beast::fight::br1::res::Br1Res;
    use open_kumite_beast::formats::br1_anm::CHARACTERS;
    use open_kumite_beast::formats::gltf_model::ModelData;
    use open_kumite_beast::render::Renderer;
    use std::path::Path;

    /// The fighter blocks F of P1 / P2 in the scratchpad.
    const FIGHTERS: [u32; 2] = [0x1F80_00F4, 0x1F80_01E0];

    /// A RAM dump: 2 MB of RAM and the 1 KB scratchpad.
    struct Dump {
        name: String,
        frame: u32,
        ram: Vec<u8>,
        spad: Vec<u8>,
    }

    impl Dump {
        fn read(path: &Path) -> Option<Self> {
            let stem = path.file_stem()?.to_str()?;
            let frame = stem.trim_start_matches(|c: char| c.is_ascii_alphabetic()).parse().ok()?;
            let ram = std::fs::read(path).ok()?;
            let spad = std::fs::read(path.with_extension("spad")).ok()?;
            (ram.len() == 0x20_0000).then(|| Self { name: path.display().to_string(), frame, ram, spad })
        }
        fn bytes(&self, a: u32, n: usize) -> &[u8] {
            let (mem, o) = if a >> 20 == 0x1F8 { (&self.spad, (a & 0x3FF) as usize) } else { (&self.ram, (a & 0x1F_FFFF) as usize) };
            mem.get(o..o + n).unwrap_or(&[0; 64][..n.min(64)])
        }
        fn u8(&self, a: u32) -> u8 {
            self.bytes(a, 1)[0]
        }
        fn i16(&self, a: u32) -> i16 {
            let b = self.bytes(a, 2);
            i16::from_le_bytes([b[0], b[1]])
        }
        fn u32(&self, a: u32) -> u32 {
            let b = self.bytes(a, 4);
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        }
        fn v3(&self, a: u32) -> [i16; 3] {
            [self.i16(a), self.i16(a + 2), self.i16(a + 4)]
        }
        fn m(&self, a: u32) -> M {
            std::array::from_fn(|i| std::array::from_fn(|j| self.i16(a + 6 * i as u32 + 2 * j as u32) as i32))
        }
        fn wt(&self, a: u32) -> [i32; 3] {
            std::array::from_fn(|i| self.u32(a + 4 * i as u32) as i32)
        }
    }

    /// One fighter of a dump: its fields as the pose reads them.
    struct Side {
        code: usize,
        f: u32,
        s: u32,
        md: u32,
        anim: Anim,
        rec: Vec<Rec>,
        form: usize,
        step: i16,
        angles: [i16; 3],
        f4c: u32,
        f50: u32,
        head: bool,
        nodes: usize,
    }

    /// The fighter whose ANM the dump holds at `base` (by its 0x40 header bytes).
    fn which(d: &Dump, base: u32, res: &[Option<Br1Res>]) -> Option<usize> {
        res.iter().position(|r| r.as_ref().is_some_and(|r| d.bytes(base, 0x40) == &r.anm.bytes[..0x40]))
    }

    fn side(d: &Dump, k: usize, res: &[Option<Br1Res>]) -> Option<Side> {
        let f = FIGHTERS[k];
        let base = d.u32(f + 0x2C);
        let code = which(d, base, res)?;
        let r = res[code].as_ref()?;
        let opp = d.u32(f + 0x28);
        let obase = d.u32(opp + 0x2C);
        let s = d.u32(f + 0x30);
        let cmn = d.u32(s + 0xB0).wrapping_sub(8);
        // a pointer into the own ANM, CMN.ACT or the opponent's ANM → (source, file offset)
        let olen = which(d, obase, res).and_then(|c| res[c].as_ref()).map_or(0, |r| r.anm.bytes.len());
        let loc = |a: u32| -> Option<(Src, usize)> {
            let inr = |b: u32, n: usize| a.wrapping_sub(b) < n as u32;
            if inr(base, r.anm.bytes.len()) {
                Some((Src::Own, (a - base) as usize))
            } else if inr(cmn, r.cmn.len()) {
                Some((Src::Cmn, (a - cmn) as usize))
            } else if inr(obase, olen) {
                Some((Src::Opp, (a - obase) as usize))
            } else {
                None
            }
        };
        let b8 = d.u32(s + 0xB8);
        let (src, clip) = if b8 == 0 { (Src::Own, None) } else { loc(b8).map(|(s, o)| (s, Some(o)))? };
        let target = loc(d.u32(s + 0xC0)).map_or(0, |(_, o)| o);
        let anim = Anim {
            flip_next: d.u8(s + 0xA4) != 0,
            id: d.i16(s + 0xAA),
            src,
            clip,
            target,
            root_scale: d.i16(s + 0xC4),
            scale: d.i16(s + 0xC6),
            keep_root: d.i16(s + 0xC8),
            total: d.i16(s + 0xCA),
            elapsed: d.i16(s + 0xCC),
            key: d.i16(s + 0xCE),
            d0: d.i16(s + 0xD0),
            left: d.i16(s + 0xD2),
            next_dur: d.i16(s + 0xD4),
            speed: d.i16(s + 0xD6),
            yaw_base: d.i16(s + 0xD8),
            yaw: d.i16(s + 0xDA),
            pos: d.v3(s + 0xDC),
            root: d.v3(s + 0xE4),
            bind1: d.v3(s + 0xEC),
            t1: d.v3(s + 0xF4),
            bind4: d.v3(s + 0xFC),
            t4: d.v3(s + 0x104),
            turn: d.i16(s + 0x10C),
            mirror: d.u8(s + 0x128) != 0,
            head_yaw: d.i16(s + 0x12E),
            head_pitch: d.i16(s + 0x130),
        };
        let p = d.u32(f + 0x38);
        let rec = (0..19).map(|n| Rec { dir: d.v3(p + 0x18 * n), twist: d.i16(p + 0x18 * n + 6) }).collect();
        Some(Side {
            code,
            f,
            s,
            md: d.u32(f + 0x1C),
            anim,
            rec,
            form: (d.u8(f + 5) & 1) as usize,
            step: d.i16(f + 0x80),
            angles: d.v3(f + 0x44),
            f4c: d.u32(f + 0x4C),
            f50: d.u32(f + 0x50),
            head: d.u32(f + 0x54) & 0x1_0000 != 0 && d.u8(f) < 0x80,
            nodes: d.u8(f + 7) as usize,
        })
    }

    #[derive(Default)]
    struct Tally {
        checks: Vec<(&'static str, usize, usize)>,
    }

    impl Tally {
        fn add(&mut self, what: &'static str, ok: bool, detail: impl FnOnce() -> String) {
            let e = match self.checks.iter().position(|c| c.0 == what) {
                Some(i) => &mut self.checks[i],
                None => {
                    self.checks.push((what, 0, 0));
                    self.checks.last_mut().unwrap()
                }
            };
            e.1 += 1;
            if !ok {
                e.2 += 1;
                if e.2 <= 3 || std::env::var("V").is_ok() {
                    println!("MISMATCH {what}: {}", detail());
                }
            }
        }
    }

    fn load_all(root: &str) -> Vec<Option<Br1Res>> {
        let rd = |p: &str| std::fs::read(format!("{root}/{p}"));
        let (Ok(cmn), Ok(tables)) = (
            rd("engine/CMN/CMN.ACT"),
            pollster::block_on(open_kumite_beast::fight::br1::res::Br1Tables::load(&open_kumite_beast::assets::Assets::new(root.to_string()))),
        ) else {
            return vec![];
        };
        CHARACTERS.iter().map(|c| rd(&format!("engine/{c}/{c}31.DAT")).ok().and_then(|a| Br1Res::new(c, a, cmn.clone(), &tables).ok())).collect()
    }

    const USAGE: &str = "usage: cliptest br1 BR1_DIR dumps DIR… | fixture DIR OUT | pose CODE FORM CLIP FRAMES [MIRROR] | shot CODE FORM CLIP FRAME OUT.png [MIRROR] | dumpshot DUMP OUT.png";

    pub fn main(args: &[String]) -> Result<(), String> {
        let root = args.first().ok_or(USAGE)?;
        let res = load_all(root);
        if res.iter().all(Option::is_none) {
            return Err(format!("{root}: no data of the imported game"));
        }
        match args.get(1).map(String::as_str) {
            Some("dumps") => dumps(&res, &args[2..]),
            Some("pose") => play(&res, &args[2..]),
            Some("shot") => shot(root, &res, &args[2..]),
            Some("dumpshot") => dumpshot(root, &res, &args[2..]),
            Some("fixture") => fixture(&res, args.get(2).ok_or("fixture: no DUMP_DIR")?, args.get(3).ok_or("fixture: no OUT")?),
            _ => Err(USAGE.into()),
        }
    }

    fn dumps(res: &[Option<Br1Res>], dirs: &[String]) -> Result<(), String> {
        let mut t = Tally::default();
        let mut pairs = 0;
        let mut fighters = std::collections::BTreeSet::new();
        for dir in dirs {
            let mut ds: Vec<Dump> = std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?.filter_map(|e| Dump::read(&e.ok()?.path())).collect();
            ds.sort_by_key(|d| d.frame);
            for (i, d) in ds.iter().enumerate() {
                let sides: Vec<Option<Side>> = (0..2).map(|k| side(d, k, res)).collect();
                for (k, sd) in sides.iter().enumerate() {
                    let Some(sd) = sd else { continue };
                    fighters.insert(CHARACTERS[sd.code]);
                    check_dump(&mut t, d, sd, res, k);
                }
                // the step of frame N+1 from dump N
                let Some(n) = ds.get(i + 1).filter(|n| n.frame == d.frame + 1) else { continue };
                pairs += 1;
                for k in 0..2 {
                    let (Some(a), Some(b)) = (side(d, k, res), side(n, k, res)) else { continue };
                    let ob = side(d, 1 - k, res);
                    check_step(&mut t, d, n, &a, &b, ob.as_ref(), res, k);
                }
            }
        }
        println!("fighters seen: {fighters:?}; consecutive dump pairs: {pairs}");
        let mut bad = 0;
        for (what, n, b) in &t.checks {
            println!("{what:40} {n:6} compared, {b} differ");
            bad += b;
        }
        if bad > 0 { Err(format!("{bad} mismatches")) } else { Ok(()) }
    }

    /// The dump's own consistency with the port: base matrices, local matrices / translations, the world walk.
    fn check_dump(t: &mut Tally, d: &Dump, sd: &Side, res: &[Option<Br1Res>], k: usize) {
        let r = res[sd.code].as_ref().unwrap();
        let skel = &r.skels[sd.form];
        let name = || format!("{} P{} {}", d.name, k + 1, CHARACTERS[sd.code]);
        let rs = r.tables.rsqrt_fn();
        // the form setup's base matrices (F+0x40) and node count
        let bm = d.u32(sd.f + 0x40);
        t.add("form: node count (F.07)", sd.nodes == skel.nodes(), || format!("{}: {} vs {}", name(), sd.nodes, skel.nodes()));
        for n in 0..skel.nodes() {
            let g = d.m(bm + 0x20 * n as u32);
            t.add("form: base matrices (F+0x40)", g == skel.bm[n], || format!("{} node {n}: game {g:?} port {:?}", name(), skel.bm[n]));
        }
        // the bone maps the sampler reads (S+0x120 / +0x124)
        for m in 0..2 {
            let p = d.u32(sd.s + 0x120 + 4 * m);
            t.add("form: bone maps (S+0x120)", d.bytes(p, 20) == &r.anm.bone_map(sd.form, m == 1)[..], name);
        }
        // local matrices of nodes 1..18 (with the head turn) and translations
        let opp = d.u32(sd.f + 0x28);
        let half = pose::node1_halved(sd.f4c, sd.f50, d.u32(opp + 0x4C), d.u32(opp + 0x50));
        let pos = d.v3(sd.md + 0x18).map(|v| v as i32);
        let (lm, lt) = pose::locals(&sd.anim, &sd.rec, skel, sd.angles, pos, half, sd.head, &r.trig, &rs);
        for n in 0..19 {
            let g = d.m(sd.md + 0x50 * n as u32 + 4);
            t.add(if n == 0 { "locals: node 0 matrix" } else { "locals: matrices nodes 1..18" }, g == lm[n], || {
                format!("{} node {n}: game {g:?} port {:?} (head {})", name(), lm[n], sd.head)
            });
        }
        for n in 1..skel.nodes() {
            let g = d.v3(sd.md + 0x50 * n as u32 + 0x18).map(|v| v as i32);
            t.add("locals: translations", g == lt[n], || format!("{} node {n}: game {g:?} port {:?}", name(), lt[n]));
        }
        // the world walk from the stored node 0 world: the matrices exactly, the translations relative to the parent
        // (the push-apart and the walls shift whole subtrees afterwards, 0x8018F27C). Chain nodes 19..: the chain
        // physics (0x80158458, not ported here) rewrites their matrices, so only the walk's translation step is
        // checked there, from the stored parent
        let mut m = lm.clone();
        m[0] = d.m(sd.md + 0x20);
        let mut t16 = lt.clone();
        t16[0] = d.wt(sd.md + 0x34);
        let w = pose::world(skel, &m, &t16);
        let node = |n: usize| sd.md + 0x50 * n as u32;
        for n in 1..skel.nodes() {
            let p = skel.parents[n].max(0) as usize;
            let (gm, gt, gp) = (d.m(node(n) + 0x20), d.wt(node(n) + 0x34), d.wt(node(p) + 0x34));
            let rel = |a: [i32; 3], b: [i32; 3]| -> [i32; 3] { std::array::from_fn(|i| a[i].wrapping_sub(b[i])) };
            let (what, ok, port) = if n < 19 {
                ("world: nodes 1..18 (m, t − parent t)", gm == w[n].0 && rel(gt, gp) == rel(w[n].1, w[p].1), rel(w[n].1, w[p].1))
            } else {
                let step = pose::world_step(&(d.m(node(p) + 0x20), gp), &pose::IDENTITY, t16[n]).1;
                ("world: chain nodes 19.. (t − parent t)", rel(gt, gp) == rel(step, gp), rel(step, gp))
            };
            t.add(what, ok, || format!("{} node {n}: game {gm:?} {:?} port {:?} {port:?}", name(), rel(gt, gp), w[n].0));
        }
    }

    /// Frame N+1's animation step from dump N's state (`a`), against dump N+1 (`b`).
    #[allow(clippy::too_many_arguments)]
    fn check_step(t: &mut Tally, d: &Dump, n: &Dump, a: &Side, b: &Side, opp: Option<&Side>, res: &[Option<Br1Res>], k: usize) {
        let r = res[a.code].as_ref().unwrap();
        let name = || format!("{} → f{} P{} {}", d.name, n.frame, k + 1, CHARACTERS[a.code]);
        let opp_anm = opp.and_then(|o| res[o.code].as_ref()).map(|o| &o.anm);
        let banks: Banks = r.banks(opp_anm);
        let mut anim = a.anim.clone();
        let mut rec = a.rec.clone();
        let started = anim.clip != b.anim.clip || anim.id != b.anim.id;
        if started {
            // the script started clip N+1's (op 0x05 → 0x8018262C) before the step: the start from N's state, the
            // root bake committing N's yaw and root offset (the auto-turn's facing is then not separable: S.D8 left out)
            if !pose::start_clip(&mut anim, &mut rec, &banks, &r.trig, b.step, b.anim.id as i32) {
                t.add("clip start: found (op 0x05)", false, || format!("{}: clip {} not found", name(), b.anim.id));
                return;
            }
        } else if anim.turn == 0 {
            // the per-fighter frame turns the fighter (S.D8) before its script (0x8017B608's auto-turn): the step
            // sees that
            anim.yaw_base = b.anim.yaw_base;
        }
        pose::step(&mut anim, &mut rec, &banks, a.form, b.step);
        pose::turn(&mut anim, d.wt(a.md + 0x34), &r.trig);
        if started {
            let mut want = b.anim.clone();
            (want.pos, want.yaw_base, want.head_yaw, want.head_pitch) = (anim.pos, anim.yaw_base, anim.head_yaw, anim.head_pitch);
            t.add("clip start + step: records nodes 1..18", rec[1..] == b.rec[1..], || {
                let i = (1..19).find(|&i| rec[i] != b.rec[i]).unwrap();
                format!("{} node {i}: game {:?} port {:?}", name(), b.rec[i], rec[i])
            });
            t.add("clip start + step: state (but S.D8, S.DC..E0)", anim == want, || {
                let (g, p) = (format!("{want:?}"), format!("{anim:?}"));
                let diff: Vec<String> = g.split(", ").zip(p.split(", ")).filter(|(x, y)| x != y).map(|(x, y)| format!("game {x} port {y}")).collect();
                format!("{}: {}", name(), diff.join("; "))
            });
            return;
        }
        t.add("step: records nodes 1..18", rec[1..] == b.rec[1..], || {
            let i = (1..19).find(|&i| rec[i] != b.rec[i]).unwrap();
            format!("{} node {i}: game {:?} port {:?}", name(), b.rec[i], rec[i])
        });
        // S+0xDC..+0xE0 are moved by the physics later in the frame; the head turn is checked below
        let mut want = b.anim.clone();
        want.pos = anim.pos;
        (want.head_yaw, want.head_pitch) = (anim.head_yaw, anim.head_pitch);
        t.add("step: state S+0xA4..+0x130", anim == want, || {
            let (g, p) = (format!("{want:?}"), format!("{anim:?}"));
            let diff: Vec<String> = g.split(", ").zip(p.split(", ")).filter(|(x, y)| x != y).map(|(x, y)| format!("game {x} port {y}")).collect();
            format!("{}: {}", name(), diff.join("; "))
        });
        let f46 = pose::facing(&anim);
        t.add("step: facing F.46", f46 == b.angles[1], || format!("{}: game {} port {f46}", name(), b.angles[1]));
        // the head turn of frame N+1 (after the step): node 0 / 2 world positions of both fighters from dump N
        let Some(o) = opp else { return };
        let w = HeadInputs { own0: d.wt(a.md + 0x34), own2: d.wt(a.md + 0xD4), opp0: d.wt(o.md + 0x34), opp2: d.wt(o.md + 0xD4) };
        let sq = r.tables.sqrt_fn();
        pose::head_turn(&mut anim, &rec, b.head, f46, &w, &r.trig, &sq);
        t.add("head turn: S+0x12E / +0x130", (anim.head_yaw, anim.head_pitch) == (b.anim.head_yaw, b.anim.head_pitch), || {
            format!("{} (on {}): game {:?} port {:?}", name(), b.head, (b.anim.head_yaw, b.anim.head_pitch), (anim.head_yaw, anim.head_pitch))
        });
    }

    /// `fixture DIR OUT`: per fighter of every dump in DIR one JSON line with the pose's inputs as the dump holds them
    /// and a CRC-32 (`pose::digest`) of the game's matrices; the test `fight::br1::pose::tests::harness_dumps` runs
    /// the port on them (derived numbers only: no game file's bytes).
    fn fixture(res: &[Option<Br1Res>], dir: &str, out: &str) -> Result<(), String> {
        let mut ds: Vec<Dump> = std::fs::read_dir(dir).map_err(|e| format!("{dir}: {e}"))?.filter_map(|e| Dump::read(&e.ok()?.path())).collect();
        ds.sort_by_key(|d| d.frame);
        let mut lines = vec![format!(
            "# cliptest br1 BR1_DIR fixture {}: one fighter of a RAM dump per line",
            Path::new(dir).file_name().and_then(|s| s.to_str()).unwrap_or("")
        )];
        for d in &ds {
            for k in 0..2 {
                let Some(sd) = side(d, k, res) else { continue };
                let r = res[sd.code].as_ref().unwrap();
                let nn = r.skels[sd.form].nodes();
                let node = |n: usize| sd.md + 0x50 * n as u32;
                let m: Vec<M> = (0..nn).map(|n| d.m(node(n) + 4)).collect();
                let t16: Vec<[i32; 3]> = (0..nn).map(|n| d.v3(node(n) + 0x18).map(|v| v as i32)).collect();
                let w: Vec<pose::World> = (0..nn).map(|n| (d.m(node(n) + 0x20), d.wt(node(n) + 0x34))).collect();
                let opp = d.u32(sd.f + 0x28);
                let half = pose::node1_halved(sd.f4c, sd.f50, d.u32(opp + 0x4C), d.u32(opp + 0x50));
                let v = serde_json::json!({
                    "frame": d.frame, "side": k, "code": CHARACTERS[sd.code], "form": sd.form, "step": sd.step,
                    "head": sd.head, "half": half, "angles": sd.angles, "pos": t16[0], "w0": d.wt(node(0) + 0x34),
                    "w2": d.wt(node(2) + 0x34), "digest": pose::digest(&r.skels[sd.form].parents, &m, &t16, &w),
                    "anim": sd.anim, "rec": sd.rec.iter().map(|q| [q.dir[0], q.dir[1], q.dir[2], q.twist]).collect::<Vec<_>>(),
                });
                lines.push(v.to_string());
            }
        }
        std::fs::write(out, lines.join("\n") + "\n").map_err(|e| format!("{out}: {e}"))?;
        println!("{out}: {} fighters of {} dumps", lines.len() - 1, ds.len());
        Ok(())
    }

    /// One fighter to draw: its model, its nodes' integer world matrices and translations, its parts.
    struct Posed<'a> {
        model: &'a ModelData,
        world: Vec<pose::World>,
        parts: Parts,
        form: usize,
    }

    /// The glTF joint matrices of `p`: the game nodes (`bone_NN`) at the port's integer world (PS1 axes → glTF:
    /// (x, −y, −z) / 256, ok_br1_converter's scale), every other node at its rest transform under its parent; the part
    /// joints (`vis_objNN`) scaled to 0 when the object is hidden.
    fn joint_matrices(p: &Posed) -> Vec<Mat4> {
        let nodes = &p.model.nodes;
        let bone = |n: &str| n.strip_prefix("bone_").and_then(|k| k.parse::<usize>().ok());
        let mut g: Vec<Option<Mat4>> = vec![None; nodes.len()];
        fn resolve(i: usize, p: &Posed, bone: &dyn Fn(&str) -> Option<usize>, g: &mut Vec<Option<Mat4>>) -> Mat4 {
            if let Some(m) = g[i] {
                return m;
            }
            let n = &p.model.nodes[i];
            let m = if let Some((wm, wt)) = bone(&n.name).and_then(|k| p.world.get(k)) {
                let s = [1.0, -1.0, -1.0];
                let c = |j: usize| Vec3::new(wm[0][j] as f32 * s[0] * s[j], wm[1][j] as f32 * s[1] * s[j], wm[2][j] as f32 * s[2] * s[j]) / 4096.0;
                Mat4::from_cols(
                    c(0).extend(0.0),
                    c(1).extend(0.0),
                    c(2).extend(0.0),
                    (Vec3::new(wt[0] as f32, -wt[1] as f32, -wt[2] as f32) / 256.0).extend(1.0),
                )
            } else {
                let scale = match n.name.strip_prefix("vis_obj").and_then(|k| k.parse::<usize>().ok()) {
                    Some(k) if p.parts.is_hidden(p.form, k) => Vec3::ZERO,
                    Some(_) => Vec3::ONE,
                    None => n.s,
                };
                let local = Mat4::from_scale_rotation_translation(scale, n.r, n.t);
                n.parent.map_or(local, |q| resolve(q, p, bone, g) * local)
            };
            g[i] = Some(m);
            m
        }
        let glob: Vec<Mat4> = (0..nodes.len()).map(|i| resolve(i, p, &bone, &mut g)).collect();
        p.model.joints.iter().zip(&p.model.inverse_bind).map(|(&j, ib)| glob[j] * *ib).collect()
    }

    /// Draws `fighters` (640×480) seen from game position `eye` (y down, z away) looking along +z, into `out`.
    fn render(fighters: &[Posed], eye: [f32; 3], out: &str) -> Result<(), String> {
        let (w, h) = (640u32, 480u32);
        let mut r = pollster::block_on(Renderer::offscreen(w, h, 2))?;
        // the PS1 projection (H = 300 on a 320×240 screen) of view space (x right, y down, z depth)
        let (n, f, hh) = (16.0f32, 60000.0f32, 300.0f32);
        let proj = Mat4::from_cols(
            Vec4::new(hh / 160.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, -hh / 120.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, f / (f - n), 1.0),
            Vec4::new(0.0, 0.0, -n * f / (f - n), 0.0),
        );
        let vp = proj * Mat4::from_translation(-Vec3::from(eye)) * Mat4::from_scale(Vec3::new(256.0, -256.0, -256.0));
        let gpu: Vec<_> = fighters.iter().map(|p| r.upload(p.model)).collect();
        let inst: Vec<_> = fighters.iter().map(|_| r.instance()).collect();
        for (p, i) in fighters.iter().zip(&inst) {
            r.set_instance(i, vp, Mat4::IDENTITY, &joint_matrices(p));
        }
        let items: Vec<_> = gpu.iter().zip(&inst).collect();
        r.frame(&items, [0.25, 0.3, 0.35]);
        let mut png = None;
        r.take_frames(0, &mut |px: &[u8]| {
            let rgba: Vec<u8> = px.as_chunks::<4>().0.iter().flat_map(|c| [c[2], c[1], c[0], 255]).collect();
            png = Some(rgba);
            Ok(())
        })?;
        let rgba = png.ok_or("no frame read back")?;
        image::RgbaImage::from_raw(w, h, rgba).ok_or("frame size")?.save(out).map_err(|e| format!("{out}: {e}"))?;
        println!("{out}");
        Ok(())
    }

    fn model(root: &str, code: &str, form: usize) -> Result<ModelData, String> {
        let p = format!("{root}/models/{code}_{}.glb", ["human", "beast"][form & 1]);
        ModelData::from_glb(&std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?)
    }

    /// `shot CODE FORM CLIP FRAME OUT [MIRROR]`: the clip played from the bind pose for FRAME frames, drawn from the
    /// front-left.
    fn shot(root: &str, res: &[Option<Br1Res>], args: &[String]) -> Result<(), String> {
        let code = args.first().ok_or("shot: no CODE")?;
        let r = CHARACTERS.iter().position(|c| c == code).and_then(|i| res[i].as_ref()).ok_or(format!("{code}: not found"))?;
        let num = |i: usize, d: i32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
        let (form, clip, frames) = (num(1, 0) as usize & 1, num(2, 1), num(3, 0));
        let out = args.get(4).ok_or("shot: no OUT")?;
        let skel = &r.skels[form];
        let banks = r.banks(None);
        // facing +x (the left side of the screen), as P1 at a round's start
        let mut a = Anim { root_scale: r.anm.params.root_scale as i16, bind1: skel.bind1(), yaw_base: 0x400, mirror: num(5, 0) != 0, ..Default::default() };
        let mut rec = vec![Rec::default(); 19];
        // the bind pose as the records' start: every direction along +y (the skeleton's rest), no twist
        for q in rec.iter_mut() {
            q.dir = [0, 0x7FFF, 0];
        }
        if !pose::start_clip(&mut a, &mut rec, &banks, &r.trig, 16, clip) {
            return Err(format!("{code}: no clip {clip}"));
        }
        for _ in 0..frames.max(1) {
            pose::step(&mut a, &mut rec, &banks, form, 16);
        }
        let rs = r.tables.rsqrt_fn();
        let (m, t16) = pose::locals(&a, &rec, skel, [0, pose::facing(&a), 0], [0; 3], false, false, &r.trig, &rs);
        let mut parts = Parts::default();
        parts.reset_hands(form);
        let model = model(root, code, form)?;
        render(&[Posed { model: &model, world: pose::world(skel, &m, &t16), parts, form }], [0.0, -250.0, -1300.0], out)
    }

    /// `dumpshot DUMP OUT`: both fighters of a RAM dump (`DUMP` without `.ram`) posed by the port from the dump's
    /// records and state, at their places (node 0's world), for a look beside the harness's screenshot of that frame.
    fn dumpshot(root: &str, res: &[Option<Br1Res>], args: &[String]) -> Result<(), String> {
        let path = args.first().ok_or("dumpshot: no DUMP")?;
        let out = args.get(1).ok_or("dumpshot: no OUT")?;
        let d = Dump::read(Path::new(&format!("{path}.ram"))).ok_or(format!("{path}.ram: not a dump"))?;
        let mut models = Vec::new();
        let mut sides = Vec::new();
        for k in 0..2 {
            let Some(sd) = side(&d, k, res) else { continue };
            let r = res[sd.code].as_ref().unwrap();
            let skel = &r.skels[sd.form];
            let rs = r.tables.rsqrt_fn();
            let opp = d.u32(sd.f + 0x28);
            let half = pose::node1_halved(sd.f4c, sd.f50, d.u32(opp + 0x4C), d.u32(opp + 0x50));
            let (mut m, mut t16) = pose::locals(&sd.anim, &sd.rec, skel, sd.angles, [0; 3], half, sd.head, &r.trig, &rs);
            // chain nodes: the game's own local matrices (the chain physics is not ported here)
            for (n, mn) in m.iter_mut().enumerate().skip(19) {
                *mn = d.m(sd.md + 0x50 * n as u32 + 4);
            }
            t16[0] = d.wt(sd.md + 0x34);
            models.push(model(root, CHARACTERS[sd.code], sd.form)?);
            // the hidden objects as the dump's object list has them (+0x0C of each 0x7C record)
            let list = d.u32(sd.f + 0x18);
            let mut parts = Parts { hidden: [0; 2] };
            for o in 0..26u32 {
                parts.set(sd.form, o as u8, d.i16(list + 0x7C * o + 0xC) as u16, false);
            }
            sides.push((pose::world(skel, &m, &t16), parts, sd.form));
        }
        let posed: Vec<Posed> = sides.into_iter().zip(&models).map(|((world, parts, form), model)| Posed { model, world, parts, form }).collect();
        // the camera between the two (node 0's world), 1500 units back
        let mid = |i: usize| posed.iter().map(|p| p.world[0].1[i] as f32).sum::<f32>() / posed.len().max(1) as f32;
        render(&posed, [mid(0), -250.0, mid(2) - 1500.0], out)
    }

    /// `pose CODE FORM CLIP FRAMES [MIRROR]`: a clip played from the bind pose, node 0..18's world positions per frame.
    fn play(res: &[Option<Br1Res>], args: &[String]) -> Result<(), String> {
        let code = args.first().ok_or("pose: no CODE")?;
        let r = CHARACTERS.iter().position(|c| c == code).and_then(|i| res[i].as_ref()).ok_or(format!("{code}: not found"))?;
        let num = |i: usize, d: i32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
        let (form, clip, frames, mirror) = (num(1, 0) as usize & 1, num(2, 1), num(3, 30), num(4, 0) != 0);
        let skel = &r.skels[form];
        let banks = r.banks(None);
        let mut a = Anim { root_scale: r.anm.params.root_scale as i16, bind1: skel.bind1(), mirror, ..Default::default() };
        let mut rec = vec![Rec::default(); 19];
        if !pose::start_clip(&mut a, &mut rec, &banks, &r.trig, 16, clip) {
            return Err(format!("{code}: no clip {clip}"));
        }
        let rs = r.tables.rsqrt_fn();
        for f in 0..frames {
            pose::step(&mut a, &mut rec, &banks, form, 16);
            let (m, t16) = pose::locals(&a, &rec, skel, [0, pose::facing(&a), 0], a.pos.map(|v| v as i32), false, false, &r.trig, &rs);
            let w = pose::world(skel, &m, &t16);
            let pts: Vec<String> = w.iter().take(19).map(|(_, t)| format!("{},{},{}", t[0], t[1], t[2])).collect();
            println!("frame {f} key {} left {}: {}", a.key, a.left, pts.join(" "));
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
