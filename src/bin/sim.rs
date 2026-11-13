//! Headless fight simulator: input → move decision → move scripts, with env traces.
//!
//! Headless check of input → move decision → move scripts.
//! `cargo run --bin sim -- ../okb_export YUG ALI "60:,20:R,30:,10:D,20:,1:J,40:,1:K,40:,1:L,60:"`
//! Script = comma-separated `frames:keys` (keys: U D L R J=Square K=Cross L=Circle I=Triangle G=R1).
//! The fight runs through `session::step` (the app's path) with both pads scripted; the match setup env vars, the
//! P2DMG hook (after the step, where the traces read it) and the traces are the sim's own.
//! ROLLBACK=N: the rollback test (`session::rollback`; its report to stderr or ROLLBACK_LOG, exit 3 on a mismatch).
//! STEPLOG=file: the run's step log (`session::determinism_trace`), for `replay --trace` on another target.
//! SIM_VERSION=n: run simulation version n (an older one's logic, as its replays; default the newest).
//! P1 / P2 may be an imported fighter (MIT FOX GRE URI, simulation v15 on): from BR1=dir, else the data folder's
//! `br1` install.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use open_kumite_beast::fight::input::pad::*;
    use open_kumite_beast::session::determinism_trace::{Data, Op, StepLog};
    use open_kumite_beast::session::rollback::{Held, Rollback};
    use open_kumite_beast::session::{self, Cheat, FrameInput};
    use std::sync::Arc;
    let args: Vec<String> = std::env::args().collect();
    let root = args.get(1).cloned().unwrap_or("../okb_export".into());
    if let Ok(mode) = std::env::var("TEAM") {
        team_main(&root, &args, &mode);
        return;
    }
    let p1 = args.get(2).cloned().unwrap_or("YUG".into());
    let p2 = args.get(3).cloned().unwrap_or("ALI".into());
    let script = args.get(4).cloned().unwrap_or("60:,20:R,30:,10:D,20:,1:J,40:,1:K,40:".into());
    let assets = open_kumite_beast::assets::Assets::new(root);
    let stage = std::env::var("STAGE").unwrap_or("ST0".into());
    let sim_version = session::sim_version_env().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
    let sim_version = sim_version.unwrap_or(session::SIM_VERSION);
    let costume = open_kumite_beast::fight::fight_data::direct_costume();
    // BR1=dir: the imported game's install (else the data folder's `br1`), for the codes MIT FOX GRE URI
    let br1 = std::env::var("BR1").ok().map(open_kumite_beast::assets::Assets::new).or_else(|| assets.br1());
    let data = Arc::new(
        pollster::block_on(open_kumite_beast::fight::fight_data::load_match_with(&assets, br1.as_ref(), [&p1, &p2], costume, &stage, false, sim_version))
            .unwrap_or_else(|e| {
                eprintln!("load: {e}");
                std::process::exit(2)
            }),
    );
    if std::env::var("MUSICDBG").is_ok() {
        for (t, mu) in &data.music {
            println!("music track {t:#x}: {} frames at {} Hz ({:.1} s)", mu.pcm.len() / 2, mu.rate, mu.pcm.len() as f32 / 2.0 / mu.rate as f32);
        }
    }
    // the match as the app starts it, with player 2 human (CPU=1 makes it the CPU)
    let debug = session::FightDebug { p2_human: true, ..Default::default() };
    let mut s = session::GameState { fight: Some(open_kumite_beast::session::fight::Fight::new(&data, &debug)), sim_version, ..Default::default() };
    let files = session::FileCache::default();
    let assets = session::Assets { front: None, fight: Some(&*data), backdrop: None, files: &files, prefs: Default::default() };
    // a frame's log for the rollback test: the step's input and cheats, the P2DMG set after it
    let held = Held { front: None, fight: Some(data.clone()), backdrop: None, files: files.clone(), prefs: Default::default() };
    let mut rollback = Rollback::<(FrameInput, Vec<Cheat>, Option<i16>)>::from_env("ROLLBACK");
    let resim = |s: &mut session::GameState, a: &session::Assets, (i, cheats, p2dmg): &(FrameInput, Vec<Cheat>, Option<i16>)| {
        session::step(s, a, *i, cheats, &mut session::Out::default());
        if let (Some(d), Some(g)) = (*p2dmg, &mut s.fight) {
            let g = &mut g.m.fighters[1];
            (g.o_ac, g.o_ae, g.o_aa) = (d, d, d);
        }
    };
    // the step log written, then the summary; a mismatch exits 3
    let finish = |rollback: &mut Option<Rollback<_>>, steplog: &mut Option<(String, StepLog)>| {
        if let Some((path, log)) = steplog.take()
            && let Err(e) = std::fs::write(&path, log.encode())
        {
            eprintln!("STEPLOG: {path}: {e}");
        }
        if rollback.as_mut().is_some_and(|r| r.finish() > 0) {
            std::process::exit(3);
        }
    };
    let mut shots = Shots::from_env(&data);
    let mut wav = Wav::from_env(&data);
    let mut hashes = session::HashTrace::from_env("STATEHASH");
    let m = &mut s.fight.as_mut().expect("the fight").m;
    if let Ok(g) = std::env::var("GAUGE") {
        m.fighters[0].gauge = g.parse().unwrap_or(0x3BFF);
    }
    if let Some(n) = std::env::var("NEED").ok().and_then(|v| v.parse().ok()) {
        m.round.need = n;
    }
    if let Some(model) = std::env::var("MODEL").ok().and_then(|v| v.parse::<u8>().ok()) {
        let custom = open_kumite_beast::frontend::options::Custom { model, ..open_kumite_beast::frontend::options::Custom::normal() };
        m.configure(
            &data,
            open_kumite_beast::fight::sim::Rules {
                need: 2,
                time_limit: 0xE10,
                hp_pct: 100,
                cpu_level: 0x80,
                custom,
                practice: std::env::var("PRACTICE").is_ok(),
                demo: false,
            },
        );
    }
    if let Some(pct) = std::env::var("HP").ok().and_then(|v| v.parse::<i32>().ok()) {
        // ATTACK LEVEL's HP percentage (the front end's default 3 = 75 %)
        let custom = open_kumite_beast::frontend::options::Custom::normal();
        m.configure(
            &data,
            open_kumite_beast::fight::sim::Rules { need: 2, time_limit: 0xE10, hp_pct: pct, cpu_level: 0x80, custom, practice: false, demo: false },
        );
    }
    // P2DMG=d@frame,…: P2's damage set to d at those frames (with ROUNDDBG / DMGDBG: KO tests)
    let p2dmg: Vec<(i16, u64)> = std::env::var("P2DMG")
        .unwrap_or_default()
        .split(',')
        .filter_map(|v| v.split_once('@').and_then(|(d, f)| Some((d.parse().ok()?, f.parse().ok()?))))
        .collect();
    // POSAT=frame:player:x:z,…: the base position (state +0xD4 / +0xD8) of player 1 / 2 set after that frame, as the
    // recorded runs' position writes (head-turn checks; not recorded: no ROLLBACK / STEPLOG with it)
    let posat: Vec<(u64, usize, i16, i16)> = std::env::var("POSAT")
        .unwrap_or_default()
        .split(',')
        .filter_map(|v| {
            let w: Vec<&str> = v.split(':').collect();
            Some((
                w.first()?.parse().ok()?,
                w.get(1)?.parse::<usize>().ok()?.checked_sub(1).filter(|&p| p < 2)?,
                w.get(2)?.parse().ok()?,
                w.get(3)?.parse().ok()?,
            ))
        })
        .collect();
    if std::env::var("CONT").is_ok() {
        // front-end match: no auto rematch, the continue countdown after a lost 1P match
        m.auto_rematch = false;
        m.round.continue_ok = true;
    }
    if let Some(d4) = std::env::var("PRACTICE_D4").ok().and_then(|v| u8::from_str_radix(v.trim_start_matches("0x"), 16).ok()) {
        m.practice_d4 = d4;
    }
    if std::env::var("CPU0").is_ok() {
        m.set_cpu(0, true);
    }
    if std::env::var("CPU").is_ok() {
        m.set_cpu(1, true);
    }
    if std::env::var("DEMO").is_ok() {
        // the attract demo's rules: no round flow, level M.A0 = 0xFF
        let custom = open_kumite_beast::frontend::options::Custom::normal();
        m.configure(
            &data,
            open_kumite_beast::fight::sim::Rules { need: 2, time_limit: 0xE10, hp_pct: 100, cpu_level: 0xFF, custom, practice: false, demo: true },
        );
    }
    // an imported side's AI block for the CPU / PRACTICE settings above (as the round start makes it)
    if std::env::var("CPU").is_ok() || std::env::var("CPU0").is_ok() || std::env::var("PRACTICE_D4").is_ok() {
        for i in 0..2 {
            m.br1_ai_init(&data, i);
        }
    }
    if let Ok(l) = std::env::var("LEVEL") {
        m.cpu_level = u16::from_str_radix(l.trim_start_matches("0x"), 16).unwrap_or(0x80);
        // a main-game side's AI made again; an imported side's block takes the level (it has no AI of the main game)
        if !m.is_br1(1) {
            m.ai[1] = Some(open_kumite_beast::fight::ai::Ai::new(&data.fighters[1].ani, &m.ai_rnd, 1, m.cpu_level, 0));
        }
        let lv = m.cpu_level;
        if let Some(b) = m.br1.as_deref_mut() {
            b.ai[1].level = lv;
        }
    }
    // RINGSCALE=pct (debug, the team battle's ring survey): the first round's ring box
    // scaled by pct / 100 (1..=1000; a box ring only, kind 0), its walls open and not drawn (not recorded: no ROLLBACK /
    // STEPLOG with it)
    if let Some(pct) = std::env::var("RINGSCALE").ok().and_then(|v| v.parse::<i32>().ok()).map(|p| p.clamp(1, 1000))
        && m.world.stage.ring.kind == 0
    {
        let st = &mut m.world.stage;
        let (hx, hz) = ((st.ring.hx as i32 * pct / 100) as i16, (st.ring.hz as i32 * pct / 100) as i16);
        st.ring = open_kumite_beast::fight::stage::AreaRec { hx, hz, w4: (hz as u16 as i32) << 16 | hx as u16 as i32, ..st.ring };
        st.walls.hp = [0; 4];
        st.walls.model = [-1; 4];
        m.fx.arena.half = (hx as i32, hz as i32);
    }
    let mut last = (-2i16, -2i32);
    let mut last2 = (-2i16, -2i32);
    let verbose = std::env::var("V").is_ok();
    let p2script = open_kumite_beast::fight::input_script::parse_script(&std::env::var("P2SCRIPT").unwrap_or_default());
    let mut last_state = (255u8, open_kumite_beast::fight::round::Message::None);
    if let Some(r) = &mut rollback {
        r.start(&s);
    }
    let mut steplog = std::env::var("STEPLOG").ok().filter(|v| !v.is_empty()).map(|path| {
        let data = Data::Fight { p1: p1.clone(), p2: p2.clone(), stage: stage.clone(), costume };
        (path, StepLog::new(data, Default::default(), &s))
    });
    let log = |op: Op, steplog: &mut Option<(String, StepLog)>| {
        if let Some((_, l)) = steplog {
            l.push(op);
        }
    };
    for step in script.split(',') {
        let (n, keys) = step.split_once(':').unwrap_or((step, ""));
        let mut pad = 0u16;
        for c in keys.chars() {
            pad |= match c {
                'U' => UP,
                'D' => DOWN,
                'L' => LEFT,
                'R' => RIGHT,
                'J' => SQUARE,
                'K' => CROSS,
                'C' => CIRCLE,
                'I' => TRIANGLE,
                'G' => R1,
                'S' => 0x800,
                _ => 0,
            };
        }
        for _ in 0..n.parse::<u32>().unwrap_or(1) {
            let m = &mut s.fight.as_mut().expect("the fight").m;
            let mut cheats = Vec::new();
            if let Some((fr, mv)) =
                std::env::var("FORCEMOVE").ok().and_then(|v| v.split_once(':').map(|(a, b)| (a.parse::<u64>().unwrap_or(0), b.parse::<i16>().unwrap_or(0))))
                && m.frame == fr
            {
                cheats.push(Cheat::ForceMove(mv));
            }
            // BREAK=frame:side,…: the debug wall break (the app's cheat) at that frame
            for v in std::env::var("BREAK").unwrap_or_default().split(',') {
                if let Some((fr, side)) = v.split_once(':').and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<usize>().ok()?)))
                    && m.frame == fr
                {
                    cheats.push(Cheat::Break(side));
                }
            }
            if std::env::var("FADEDBG").is_ok() && (m.world.g4a58[2] != 0 || m.world.p4b18.iter().any(|p| p[2] != 0)) && m.frame.is_multiple_of(5) {
                println!(
                    "fade f{} stage {:?} sides {:?} moves {:#x} {:#x}",
                    m.frame, m.world.g4a58, m.world.p4b18, m.fighters[0].move_cur, m.fighters[1].move_cur
                );
            }
            if std::env::var("SNAPDBG").is_ok() {
                for (k, f) in m.fighters.iter().enumerate() {
                    let o = f.draw_off;
                    if o.iter().any(|v| v.abs() > 0.5) {
                        println!("snap f{} P{} off [{:.0} {:.0} {:.0}] move {:#x}", m.frame, k + 1, o[0], o[1], o[2], f.move_cur);
                    }
                }
            }
            if std::env::var("CAMMODE").is_ok() && matches!(m.cam.mode & 0x7F, 1 | 2) {
                println!("cam f{} mode {:#x} jt {:?} pos {:?} ang {:?} zom {:?}", m.frame, m.cam.mode, m.cam.jt, m.cam.pos, m.cam.ang, m.cam.zom);
            }
            if std::env::var("JAWDBG").is_ok() {
                for (k, f) in m.fighters.iter().enumerate() {
                    if let Some((i, sw)) = f.swing.iter().enumerate().find(|(_, s)| s.ang != 0) {
                        println!("jaw f{} P{} node {} ang {} beast {}", m.frame, k + 1, 19 + i, sw.ang, f.beast());
                    }
                }
            }
            if std::env::var("HANDDBG").is_ok() {
                for (k, f) in m.fighters.iter().enumerate() {
                    let h = f.hand_hide[(f.form & 1) as usize];
                    if h != [0, 0, 1, 1] {
                        println!("hand f{} P{} {:?} mirror {}", m.frame, k + 1, h, f.anim.mirror);
                    }
                }
            }
            if std::env::var("NATDBG").is_ok() {
                let l = open_kumite_beast::fight::fx::native_tasks::debug_line(&m.fxn) + &open_kumite_beast::fight::fx::move_effects::debug_line(&m.fxn.mv);
                if !l.is_empty() {
                    println!("f{}{}", m.frame, l);
                }
            }
            if std::env::var("FLAREDBG").is_ok() && m.frame.is_multiple_of(60) {
                println!("  flare {:5} sprites {} cam yaw {}", m.frame, m.fxn.sprites.len(), m.cam.ang[1]);
            }
            if std::env::var("DARKDBG").is_ok() {
                println!("  dark {:4} g4a58 {:?} P1 mv {:#x}", m.frame, m.world.g4a58, m.fighters[0].move_cur);
            }
            if std::env::var("CAMDBG").is_ok() && m.frame % 60 == 23 {
                let c = &m.cam;
                println!(
                    "  cam {:4} ang {:?} pos {:?} zom {:?} mode {} h {} | P1 {:?} P2 {:?}",
                    m.frame, c.ang, c.pos, c.zom, c.mode, c.h, m.fighters[0].m_t, m.fighters[1].m_t
                );
            }
            if std::env::var("CAMDBG2").is_ok() && matches!(m.round.state, 9 | 10) && m.frame.is_multiple_of(10) {
                // where the winner's pelvis lands on screen (−1..1), as a check of the replay camera
                let w = if m.round.state == 9 { 0 } else { 1 };
                let h = m.fighters[w].hips;
                let v = m.cam.view(&data.trig) * glam::Vec4::new(h[0] as f32, h[1] as f32, h[2] as f32, 1.0);
                println!(
                    "  replay {:5} cam mode {} shot {} winner on screen x {:.2} y {:.2} depth {:.0}",
                    m.frame,
                    m.cam.mode,
                    m.cam.timer,
                    v.x / v.z * 360.0 / 160.0,
                    v.y / v.z * 360.0 / 120.0,
                    v.z
                );
            }
            if std::env::var("FXVMDBG").is_ok() && (m.vm.live_models() > 0 || m.vm.live() > 0) && m.frame.is_multiple_of(4) {
                println!("  fx {:5}: scripts {} models {} billboards {}", m.frame, m.vm.live(), m.vm.live_models(), m.vm.draws.len());
            }
            if std::env::var("FXVMDBG").is_ok() && m.frame.is_multiple_of(600) {
                println!(
                    "  fx {:5}: vm live {} draws {} spawns {:?} unsupported {:02X?}",
                    m.frame,
                    m.vm.live(),
                    m.vm.draws.len(),
                    m.vm.stats,
                    m.vm.unsupported
                );
            }
            if let Some(r) = m.result {
                println!("  result at {}: {r:?}", m.frame);
                finish(&mut rollback, &mut steplog);
                return;
            }
            // both pads scripted: they drive the fighters whoever is the CPU
            let p2 = p2script.get(m.frame as usize).copied().unwrap_or(0);
            let i = FrameInput { pad: [pad, p2], forced: [pad, p2], flags: FrameInput::FORCED[0] | FrameInput::FORCED[1], ..Default::default() };
            log(Op::Step(i, cheats.clone()), &mut steplog);
            let mut out = session::Out::default();
            session::step(&mut s, &assets, i, &cheats, &mut out);
            if let Some(w) = &mut wav {
                w.step(&mut out);
            }
            if let (Some(sh), Some(f)) = (&mut shots, &s.fight) {
                sh.take(f, &data);
            }
            // BR1DBG=n: every n frames the imported cores' state, clip, facing, mirror and key timing (S.CA, S.D2, S.D0)
            if let (Some(n), Some(b)) = (std::env::var("BR1DBG").ok().and_then(|v| v.parse::<u64>().ok()), s.fight.as_ref().and_then(|f| f.m.br1.as_deref()))
                && s.fight.as_ref().is_some_and(|f| f.m.frame.is_multiple_of(n.max(1)))
            {
                for k in (0..2).filter(|&k| b.side[k]) {
                    let c = &b.fs[k];
                    println!(
                        "  br1 {:5} P{} state {:#x} clip {} key {} yaw {} f46 {} mirror {} pos {:?} form {} w13 {:?} w16 {:?} m2 {:?} total {} left {} d0 {}",
                        s.fight.as_ref().map_or(0, |f| f.m.frame),
                        k + 1,
                        c.s.state_cur,
                        c.anim.id,
                        c.anim.key,
                        c.anim.yaw_base,
                        c.f.yaw,
                        c.anim.mirror,
                        c.anim.pos,
                        c.f.form,
                        c.world(13),
                        c.world(16),
                        c.form().md[2].wm,
                        c.anim.total,
                        c.anim.left,
                        c.anim.d0
                    );
                }
            }
            // BR1AIDBG: the imported CPU sides' AI blocks after the step (state, pad, timers, action, band, pc, random
            // index, flags, r0)
            if let (true, Some(f)) = (std::env::var("BR1AIDBG").is_ok(), s.fight.as_ref())
                && let Some(b) = f.m.br1.as_deref()
            {
                for k in (0..2).filter(|&k| b.side[k] && b.fs[k].f.cpu != 0) {
                    let a = &b.ai[k];
                    println!(
                        "  br1ai {:5} P{} state {} pad {:#06x} t2a {} t2c {} action {} band {} pc {:#x} idx {} flags {} r0 {} depth {} lv {:#x}",
                        f.m.frame,
                        k + 1,
                        a.state,
                        a.pad,
                        a.t2a,
                        a.t2c,
                        a.action,
                        a.band,
                        a.pc,
                        a.idx,
                        a.flags,
                        a.regs[0],
                        a.depth,
                        a.level
                    );
                }
            }
            // BR1FXDBG: the effect requests of the imported game's code this step logged (`Br1Match::fx_log`, under the frame
            // number the step started from)
            if std::env::var("BR1FXDBG").is_ok()
                && let Some(f) = s.fight.as_ref()
                && let Some(b) = f.m.br1.as_deref()
            {
                for (fr, side, r) in b.fx_log.iter().filter(|(fr, _, _)| *fr + 1 == f.m.frame) {
                    println!("  br1 fx {fr:5} P{} {r:x?}", side + 1);
                }
            }
            hashes.log(&s);
            log(Op::Hash, &mut steplog);
            let m = &mut s.fight.as_mut().expect("the fight").m;
            if std::env::var("STAGEDBG").is_ok() {
                // after the step, as the recorded runs log the game: the stage routine's
                // counters W+0x26 / 28 / 2A, the darkening, the fighters' x and F.0 / F.AC; on an imported stage
                // also its G.4A and both forms
                let (f0, f1) = (&m.fighters[0], &m.fighters[1]);
                let br1 = if data.br1_stage.is_some() {
                    format!(
                        " | br1 g4a {} form {:x} {:x} g50 {:?} debris {}",
                        m.world.stage.br1.g4a,
                        f0.form,
                        f1.form,
                        m.world.stage.br1.g50,
                        m.world.stage.debris.iter().filter(|p| p.alive != 0).count()
                    )
                } else {
                    String::new()
                };
                println!(
                    "  stage {:5} st {:x} cnt {} 9e {} dark {} w {:?} p1 {} f0 {:x} ac {} | p2 {} f0 {:x} ac {}{br1}",
                    m.frame,
                    m.round.state,
                    m.round.cnt,
                    m.m9e(),
                    m.world.g4a58[2],
                    m.world.stage.anim.0,
                    f0.m_t[0],
                    f0.o_00,
                    f0.o_ac,
                    f1.m_t[0],
                    f1.o_00,
                    f1.o_ac
                );
            }
            if std::env::var("ROUNDDBG").is_ok() && (m.round.state, m.round.message) != last_state {
                println!(
                    "  round {:5} state {:#x} idx {} lost {:?} timer {} msg {:?} | P1 mv {:#x} st {:#x} hp {} | P2 mv {:#x} st {:#x} hp {}",
                    m.frame,
                    m.round.state,
                    m.round.index,
                    m.round.lost,
                    m.round.timer,
                    m.round.message.text(),
                    m.fighters[0].move_cur,
                    m.fighters[0].o_00,
                    m.fighters[0].o_b0 - m.fighters[0].o_ac,
                    m.fighters[1].move_cur,
                    m.fighters[1].o_00,
                    m.fighters[1].o_b0 - m.fighters[1].o_ac
                );
                last_state = (m.round.state, m.round.message);
                if m.round.state >= 0xF {
                    println!("         cam mode {} zom {:?} ang {:?}", m.cam.mode, m.cam.zom, m.cam.ang);
                }
            }
            if std::env::var("SNDDBG").is_ok() {
                for id in &m.sounds_started {
                    println!("  snd {:4} id {:#05x} slot {} tone {}", m.frame, id, id >> 6, id & 63);
                }
                for id in m.br1.iter().flat_map(|b| b.sounds_started.iter()) {
                    println!("  br1 snd {:4} id {:#05x} slot {} tone {}", m.frame, id, id >> 6, id & 63);
                }
            }
            if std::env::var("SPECIALDBG").is_ok() && (m.round.special != 0 || m.round.m9e != 0 || matches!(m.round.state, 5 | 6) && m.round.cnt <= 2) {
                let w = &m.fighters[if m.round.state == 6 { 1 } else { 0 }];
                println!(
                    "  special {:5} st {:#x} cnt {} kind {:#x} m9e {} | W mv {:#x} form {:x} F5C {:08x} F6C {:08x} F64 {:08x} chain {} msg {:?}",
                    m.frame,
                    m.round.state,
                    m.round.cnt,
                    m.round.special,
                    m.round.m9e,
                    w.move_cur,
                    w.form,
                    w.flags[2],
                    w.flags[6],
                    w.flags[4],
                    w.chain_sel,
                    m.round.message.text()
                );
            }
            for &(_, p, x, z) in posat.iter().filter(|&&(at, ..)| at == m.frame) {
                m.fighters[p].pos[0] = x;
                m.fighters[p].pos[2] = z;
            }
            let dmg = p2dmg.iter().find(|&&(_, at)| at == m.frame).map(|&(d, _)| d);
            if let Some(d) = dmg {
                let g = &mut m.fighters[1];
                (g.o_ac, g.o_ae, g.o_aa) = (d, d, d);
                log(Op::P2Dmg(d), &mut steplog);
            }
            let f = &m.fighters[0];
            if std::env::var("BEASTDBG").is_ok() && m.frame.is_multiple_of(30) {
                println!("  b{:4} P1 form {:02x} gauge {} d8 {} dmg {} | P2 dmg {}", m.frame, f.form, f.gauge, f.o_d8, f.o_ac, m.fighters[1].o_ac);
            }
            if std::env::var("FDBG").is_ok() {
                let g = &m.fighters[1];
                println!(
                    "  f{:4} held {:04x} F54 {:x} F60 {:08x} P1 mv {:#x} pc {:04X} an {} F5C {:08x} F58 {:08x} F6C {:08x} | P2 mv {:#x} pc {:04X} an {} F58 {:08x} F6C {:08x} pos {:?} {:?}",
                    m.frame,
                    f.held,
                    f.flags[0],
                    f.flags[3],
                    f.move_cur,
                    f.pc,
                    f.anim.id,
                    f.flags[2],
                    f.flags[1],
                    f.flags[6],
                    g.move_cur,
                    g.pc,
                    g.anim.id,
                    g.flags[1],
                    g.flags[6],
                    f.m_t,
                    g.m_t
                );
            }
            if std::env::var("THROWDBG").is_ok() && (f.cand_attack.mv >= 0 || pad & 8 != 0) {
                println!(
                    "  thr {:4} cand {:?} F5C {:08x} F58 {:08x} d76 {} a74 {} test {}",
                    m.frame,
                    f.cand_attack,
                    f.flags[2],
                    f.flags[1],
                    f.o_76,
                    f.o_74,
                    open_kumite_beast::fight::throw::test(f, &m.fighters[1])
                );
            }
            if std::env::var("VELDBG").is_ok() {
                let g = &m.fighters[1];
                println!(
                    "  vel {:4} 00 {:x} 90 {} 92 {} 94 {} 96 {} 98 {} b2 {} b4 {} c4 {} F54 {:x} F58 {:x} F5C {:x} F60 {:x} F6C {:x} 24 {}",
                    m.frame,
                    g.o_00,
                    g.o_90,
                    g.o_92,
                    g.o_94,
                    g.o_96,
                    g.o_98,
                    g.o_b2,
                    g.o_b4,
                    g.o_c4,
                    g.flags[0],
                    g.flags[1],
                    g.flags[2],
                    g.flags[3],
                    g.flags[6],
                    g.o_24
                );
            }
            if std::env::var("KODBG").is_ok() {
                let (a, g) = (&m.fighters[0], &m.fighters[1]);
                println!(
                    "  ko {:4} P1 db {} dd {} c6 {} form {:x} F64 {:x} | P2 F68 {:x} 00 {:x}",
                    m.frame, a.o_db, a.o_dd, a.o_c6, a.form, a.flags[4], g.flags[5], g.o_00
                );
            }
            if std::env::var("HEADDBG").is_ok() {
                // the head turn (state +0x106 / +0x108) and node 2's local matrix (model +0xA4), per fighter
                let rsqrt = data.tables.rsqrt_fn();
                let mut line = format!("HT {:5}", m.frame);
                for (k, g) in m.fighters.iter().enumerate() {
                    let skel = &data.fighters[k].skels[g.beast() as usize];
                    let mat = open_kumite_beast::fight::anim::pose::local_matrix(g, skel, &data.trig, &rsqrt, 2).unwrap_or_default();
                    line += &format!(
                        " | f5c {:08x} yaw {} pitch {} m {:?} t {} {} h {:?} y {} p2 {:?} {}",
                        g.flags[2], g.head_yaw, g.head_pitch, mat, g.m_t[0], g.m_t[2], g.head_w, g.yaw, g.pose[2].dir, g.pose[2].twist
                    );
                }
                println!("{line}");
            }
            if std::env::var("BOTHDBG").is_ok() {
                let mut line = format!("  both {:4}", m.frame);
                for g in &m.fighters {
                    line += &format!(
                        " | ac {} c8 {} b2 {} c4 {} F5C {:x} F58 {:x} F68 {:x} x {}",
                        g.o_ac, g.o_c8, g.o_b2, g.o_c4, g.flags[2], g.flags[1], g.flags[5], g.m_t[0]
                    );
                }
                println!("{line}");
                if std::env::var("NODEDBG").is_ok() {
                    let p = |k: usize, n: usize| m.fighters[k].parts.get(n).copied().unwrap_or_default();
                    println!(
                        "  st {:4} id {} key {} c2 {} c4 {} c8 {} left {} next {} speed {}",
                        m.frame,
                        m.fighters[1].anim.id,
                        m.fighters[1].anim.key,
                        m.fighters[1].anim.c2,
                        m.fighters[1].anim.c4,
                        m.fighters[1].anim.c8,
                        m.fighters[1].anim.left,
                        m.fighters[1].anim.next_dur,
                        m.fighters[1].anim.speed
                    );
                    let nl: Vec<(usize, usize)> = std::env::var("NODES")
                        .ok()
                        .map(|v| v.split(',').filter_map(|t| t.split_once(':').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).collect())
                        .unwrap_or(vec![(0, 4), (1, 15), (1, 14), (1, 13)]);
                    println!("  nodes {:4} {}", m.frame, nl.iter().map(|&(k, n)| format!("| P{}n{} {:?} ", k + 1, n, p(k, n))).collect::<String>());
                }
            }
            if std::env::var("DMGDBG").is_ok() {
                let g = &m.fighters[1];
                println!(
                    "  dmg {:4} P2 ac {} c8 {} ca {} c0 {} cc {} | P1 gauge {} form {:x} | P2 c4 {} yaw {} e4 {} b2 {} base {} d2 {} F68 {:x} F5C {:x} F58 {:x}",
                    m.frame,
                    g.o_ac,
                    g.o_c8,
                    g.o_ca,
                    g.o_c0,
                    g.o_cc,
                    m.fighters[0].gauge,
                    m.fighters[0].form,
                    g.o_c4,
                    g.yaw,
                    g.s_e4,
                    g.o_b2,
                    g.yaw_base,
                    g.s_d2,
                    g.flags[5],
                    g.flags[2],
                    g.flags[1]
                );
                if std::env::var("ANIMDBG").is_ok() {
                    println!(
                        "  anim {:4} P2 id {} src {:?} base {:#x} target {:#x} key {} mirror {}",
                        m.frame, g.anim.id, g.anim.src, g.anim.base, g.anim.target, g.anim.key, g.anim.mirror
                    );
                }
            }
            if std::env::var("P2").is_ok() {
                let g = &m.fighters[1];
                if (g.move_cur, g.anim.id) != last2 {
                    println!("   P2 frame {:4} → move {:#04x} anim {:3} pos {:?} dmg {}", m.frame, g.move_cur, g.anim.id, g.m_t, g.o_ac);
                    last2 = (g.move_cur, g.anim.id);
                }
            }
            if (f.move_cur, f.anim.id) != last {
                println!(
                    "frame {:4} pad {:04x} → move {:#04x} anim {:3} pos {:?} yaw {:5} root {:?}",
                    m.frame,
                    pad,
                    f.move_cur,
                    f.anim.id,
                    f.m_t,
                    f.yaw,
                    (f.s_dc, f.s_de, f.s_e0)
                );
                last = (f.move_cur, f.anim.id);
            } else if verbose && m.frame.is_multiple_of(10) {
                println!(
                    "      {:4}  p2 {:?} F5C0 {:08x} pos {:?} vel {:?} yaw {} root {:?} key {} left {}",
                    m.frame,
                    m.fighters[1].m_t,
                    f.flags[2],
                    f.m_t,
                    f.vel,
                    f.yaw,
                    (f.s_dc, f.s_de, f.s_e0),
                    f.anim.key,
                    f.anim.left
                );
            }
            if let Some(r) = &mut rollback {
                r.check(&s, held.clone(), (i, cheats, dmg), resim);
            }
        }
    }
    let m = &s.fight.as_ref().expect("the fight").m;
    for e in &m.events {
        println!("{e}");
    }
    let p2 = &m.fighters[1];
    println!("P2 damage {} / {} red {} move {:#x} F68 {:08x} gauge P1 {}", p2.o_ac, p2.o_b0, p2.o_ae, p2.move_cur, p2.flags[5], m.fighters[0].gauge);
    let t: Vec<_> = m.fighters[0].trace.iter().collect();
    println!("trace: {t:?}");
    finish(&mut rollback, &mut steplog);
}

/// SHOTS=n,m,… (SHOTDIR=dir, default the working directory; SHOTSIZE=WxH, default 640x480): the fight drawn after
/// those frames' steps through the windowless renderer (as `replay --video` draws, the stored preferences' aspect
/// aside: 4:3), saved as `shot_<frame>.png` — headless screenshots.
#[cfg(not(target_arch = "wasm32"))]
struct Shots {
    r: open_kumite_beast::render::Renderer,
    view: open_kumite_beast::present::fight_scene::FightView,
    frames: Vec<u64>,
    dir: std::path::PathBuf,
    /// SHOTTIME: the frames drawn and the milliseconds spent drawing (draw lists, upload, submit), in the renderer's
    /// `frame_capture` (its CPU part), on the GPU (its timer, the frames it read back) and saving (the GPU's wait, the
    /// readback, the PNG), reported to stderr at the end
    time: Option<[f64; 6]>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Shots {
    fn from_env(data: &open_kumite_beast::fight::fight_data::Loaded) -> Option<Self> {
        let frames: Vec<u64> = std::env::var("SHOTS").ok()?.split(',').filter_map(|v| v.trim().parse().ok()).collect();
        let (w, h) = std::env::var("SHOTSIZE")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
            .unwrap_or((640u32, 480u32));
        let mut r = pollster::block_on(open_kumite_beast::render::Renderer::offscreen(w, h, 2)).unwrap_or_else(|e| {
            eprintln!("SHOTS: {e}");
            std::process::exit(2)
        });
        r.set_fit(open_kumite_beast::output::layout::Fit::Classic);
        r.perf = std::env::var("SHOTTIME").is_ok();
        let mut audio = open_kumite_beast::audio::Audio::offline(48000.0);
        let view = open_kumite_beast::present::fight_scene::FightView::new(&mut r, data, &mut audio);
        let dir = std::env::var("SHOTDIR").map(Into::into).unwrap_or_else(|_| ".".into());
        Some(Self { r, view, frames, dir, time: std::env::var("SHOTTIME").is_ok().then_some([0.0; 6]) })
    }

    fn take(&mut self, f: &open_kumite_beast::session::fight::Fight, data: &open_kumite_beast::fight::fight_data::Loaded) {
        if !self.frames.contains(&f.m.frame) {
            return;
        }
        let Some(g) = open_kumite_beast::present::fight_scene::Game::of(Some(data), Some(f), Some(&self.view)) else { return };
        let t0 = std::time::Instant::now();
        g.draw(&mut self.r);
        self.save(f.m.frame, t0);
    }

    /// The team battle's frame (TEAM=…), as `take`.
    fn take_team(&mut self, s: &open_kumite_beast::session::GameState, data: &open_kumite_beast::fight::fight_data::Loaded) {
        let Some(t) = s.team.as_deref() else { return };
        if !self.frames.contains(&t.frame) {
            return;
        }
        let t0 = std::time::Instant::now();
        self.view.draw_team(t, data, s.team_config.as_ref().map(|c| &c.menu), &mut self.r, false);
        self.save(t.frame, t0);
    }

    fn save(&mut self, frame: u64, t0: std::time::Instant) {
        let t1 = std::time::Instant::now();
        let [w, h] = self.r.viewport().win;
        let path = self.dir.join(format!("shot_{frame}.png"));
        let mut sink = |px: &[u8]| {
            let rgba: Vec<u8> = px.as_chunks::<4>().0.iter().flat_map(|c| [c[2], c[1], c[0], 255]).collect();
            image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8).map_err(|e| e.to_string())
        };
        if let Err(e) = self.r.take_frames(0, &mut sink) {
            eprintln!("SHOTS: {e}");
        }
        if let Some(t) = &mut self.time {
            let gpu = self.r.stats.gpu_ms;
            let add = [1.0, (t1 - t0).as_secs_f64() * 1e3, self.r.stats.cpu_ms, gpu.unwrap_or(0.0), gpu.map_or(0.0, |_| 1.0), t1.elapsed().as_secs_f64() * 1e3];
            t.iter_mut().zip(add).for_each(|(a, b)| *a += b);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Shots {
    fn drop(&mut self) {
        if let Some([n, draw, cpu, gpu, gn, save]) = self.time {
            let n = n.max(1.0);
            eprintln!(
                "SHOTTIME: {n} frames: draw {:.3} ms (frame_capture {:.3} ms), GPU {:.3} ms ({gn} read back), save {:.3} ms a frame",
                draw / n,
                cpu / n,
                gpu / gn.max(1.0),
                save / n
            );
        }
    }
}

/// SOUNDWAV=file.wav: the fight's sound (its SE and music commands) mixed offline at 48 kHz, 800 samples a step
/// (as `replay --video` mixes), into a 16-bit stereo WAV written when the run ends — a headless sound test.
#[cfg(not(target_arch = "wasm32"))]
struct Wav {
    audio: open_kumite_beast::audio::Audio,
    pcm: Vec<f32>,
    path: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl Wav {
    fn from_env(data: &open_kumite_beast::fight::fight_data::Loaded) -> Option<Self> {
        let path = std::env::var("SOUNDWAV").ok().filter(|p| !p.is_empty())?;
        let mut audio = open_kumite_beast::audio::Audio::offline(48000.0);
        // the banks as `FightView::new` loads them
        for (i, (name, bytes, _)) in data.sound_banks.iter().enumerate() {
            if !bytes.is_empty() {
                audio.load_bank(&format!("slot{i}:{name}"), bytes);
            }
        }
        for (i, (_, bytes, _)) in data.br1_banks.iter().enumerate() {
            if !bytes.is_empty() {
                audio.load_bank(&format!("br1slot{i}"), bytes);
            }
        }
        Some(Self { audio, pcm: Vec::new(), path })
    }

    fn step(&mut self, out: &mut open_kumite_beast::session::Out) {
        self.audio.apply(&mut out.audio);
        self.pcm.extend(self.audio.render_offline(800));
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Wav {
    fn drop(&mut self) {
        let data: Vec<u8> = self.pcm.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&48000u32.to_le_bytes());
        b.extend_from_slice(&(48000u32 * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        if let Err(e) = std::fs::write(&self.path, b) {
            eprintln!("SOUNDWAV: {}: {e}", self.path);
            return;
        }
        // the length in steps (800 stereo samples each: frame n's shot ends at sample 800·n) and the RMS of each second
        let rms: Vec<String> =
            self.pcm.chunks(96000).map(|s| format!("{:.4}", (s.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt())).collect();
        eprintln!("SOUNDWAV: {}: {} steps, {} samples at 48 kHz; RMS per second [{}]", self.path, self.pcm.len() / 1600, self.pcm.len() / 2, rms.join(" "));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

/// TEAM=2v2|ffa (the team battle, engine addition): `sim <data> P1 P2 P3 P4` runs a four-CPU match on STAGE (default
/// ST0, one of `fight::stage::TEAM_STAGES`; from simulation v23 also Q3DM6 and BIG00, `TEAM_BIG_STAGES`) to its end
/// or MAXFRAMES (default 30000), through `session::step`. From v23 a fighter may be MIT FOX GRE URI (the imported game's
/// install from BR1=dir, else the data folder's). A
/// fighter whose character is already in the match takes the next costume. Traces: ROUNDDBG (the round states, the
/// fighters out, the wins), TEAMDBG=n (every n frames: position, damage, target, move of each fighter), HITS (the
/// hit log), TEAMCHECK (T7: a fighter stuck in the air or through a wall, `fight::team::watch`), FADEDBG (T8: each
/// fighter's palette fade and the World's two slots); STATEHASH and ROLLBACK=N as
/// for a match. ARCADE=enemy[,partner] (simulation v26): a TEAM ARCADE battle (P1 + P3 the humans' team) with these
/// CPU levels: YOU WIN / YOU LOSE, a loss the CONTINUE countdown (START in SCRIPT continues) and GAME OVER. The result line: `result <winner> after <frames> frames`.
#[cfg(not(target_arch = "wasm32"))]
fn team_main(root: &str, args: &[String], mode: &str) {
    use open_kumite_beast::fight::team::{TeamFight, TeamMode};
    use open_kumite_beast::session::rollback::{Held, Rollback};
    use open_kumite_beast::session::{self, FrameInput};
    use std::sync::Arc;
    let mode = match mode {
        "2v2" => TeamMode::Vs2,
        "ffa" => TeamMode::Ffa,
        m => {
            eprintln!("TEAM={m}: 2v2 or ffa");
            std::process::exit(2)
        }
    };
    let codes: Vec<String> = (2..6).map(|k| args.get(k).cloned().unwrap_or(["YUG", "ALI", "BAK", "ULI"][k - 2].into())).collect();
    let mut costume = [0u8; 4];
    for k in 0..4 {
        costume[k] = (codes[..k].iter().filter(|c| **c == codes[k]).count() % 4) as u8;
    }
    let stage = std::env::var("STAGE").unwrap_or("ST0".into());
    let sim_version = session::sim_version_env().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2)
    });
    let sim_version = sim_version.unwrap_or(session::SIM_VERSION);
    let assets = open_kumite_beast::assets::Assets::new(root);
    let chr = [0, 1, 2, 3].map(|k| codes[k].as_str());
    let t0 = std::time::Instant::now();
    // BR1=dir (simulation v23): the imported game's install for MIT FOX GRE URI and STAGE=BIG00 (else the data folder's)
    let br1 = std::env::var("BR1").ok().map(open_kumite_beast::assets::Assets::new).or_else(|| assets.br1());
    let load = open_kumite_beast::fight::fight_data::load_team_with(&assets, br1.as_ref(), chr, costume, &stage, sim_version);
    let data = Arc::new(pollster::block_on(load).unwrap_or_else(|e| {
        eprintln!("load: {e}");
        std::process::exit(2)
    }));
    let load = t0.elapsed();
    // SCRIPT=… (SCRIPT2 / SCRIPT3 / SCRIPT4): P1 (P2 / P3 / P4) a human playing that pad script (`frames:keys,…` by
    // team frame; pads 3 / 4 `FrameInput::ext`), the others CPUs
    let scripts = ["SCRIPT", "SCRIPT2", "SCRIPT3", "SCRIPT4"].map(|k| std::env::var(k).ok().map(|v| open_kumite_beast::fight::input_script::parse_script(&v)));
    // ARCADE=enemy[,partner] (TEAM ARCADE, simulation v26, 2v2): a TEAM ARCADE battle of the humans' team P1 + P3 with
    // these CPU levels (0..=0xFF, decimal or 0x hex; the partner's default the enemies'): YOU WIN / YOU LOSE, a loss the CONTINUE
    // countdown (START from SCRIPT / SCRIPT3 continues), its end GAME OVER
    let arcade = std::env::var("ARCADE").ok().map(|v| {
        let level = |s: &str| s.strip_prefix("0x").map_or_else(|| s.parse().ok(), |h| u16::from_str_radix(h, 16).ok()).filter(|&l: &u16| l <= 0xFF);
        let l: Vec<u16> = v.split(',').map(|s| level(s.trim())).collect::<Option<_>>().filter(|l: &Vec<u16>| (1..=2).contains(&l.len())).unwrap_or_else(|| {
            eprintln!("ARCADE={v}: enemy[,partner] CPU levels 0..=0xFF");
            std::process::exit(2)
        });
        if mode != TeamMode::Vs2 || !open_kumite_beast::fight::version::Gates::of(sim_version).team_arcade {
            eprintln!("ARCADE: a 2v2 battle of simulation v{}", open_kumite_beast::fight::version::v::TEAM_ARCADE);
            std::process::exit(2)
        }
        let (enemy, partner) = (l[0], *l.get(1).unwrap_or(&l[0]));
        open_kumite_beast::fight::team::ArcadeRules::new(0, std::array::from_fn(|k| if k & 1 == 0 { partner } else { enemy }))
    });
    let cpu = std::array::from_fn(|k| scripts[k].is_none());
    let mut team = TeamFight::with_setup(&data, mode, cpu, 0x80, [open_kumite_beast::assets::game_tables::default_buttons(); 4], arcade);
    // TEAMTIME=frames: every round's time limit (time over, DRAW and SUDDEN DEATH in a short run)
    if let Some(v) = std::env::var("TEAMTIME").ok().and_then(|v| v.parse::<i16>().ok()) {
        team.round.time_limit = v;
        team.round.timer = v;
    }
    let mut s = session::GameState { team: Some(Box::new(team)), sim_version, ..Default::default() };
    let mut shots = Shots::from_env(&data);
    // SOUNDWAV (T6): the four fighters' banks (slots 3 / 4 and 7 / 8, the imported game's 3..6) as a match's
    let mut wav = Wav::from_env(&data);
    if std::env::var("RINGDBG").is_ok() {
        let st = &s.team.as_ref().expect("the team battle").world.stage;
        println!("ring {:?} outer {:?} walls {:?}", st.ring, st.outer, st.walls.hp);
    }
    let files = session::FileCache::default();
    let assets = session::Assets { front: None, fight: Some(&*data), backdrop: None, files: &files, prefs: Default::default() };
    let held = Held { front: None, fight: Some(data.clone()), backdrop: None, files: files.clone(), prefs: Default::default() };
    let mut rollback = Rollback::<FrameInput>::from_env("ROLLBACK");
    let resim = |s: &mut session::GameState, a: &session::Assets, i: &FrameInput| session::step(s, a, *i, &[], &mut session::Out::default());
    if let Some(r) = &mut rollback {
        r.start(&s);
    }
    let mut hashes = session::HashTrace::from_env("STATEHASH");
    let max: u64 = std::env::var("MAXFRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(30000);
    let every: u64 = std::env::var("TEAMDBG").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let rounddbg = std::env::var("ROUNDDBG").is_ok();
    let fadedbg = std::env::var("FADEDBG").is_ok();
    let mut last = (255u8, [false; 4]);
    let mut last_msg = open_kumite_beast::fight::round::Message::None;
    let t1 = std::time::Instant::now();
    let mut frames = 0u64;
    let snddbg = std::env::var("SNDDBG").is_ok();
    let mut sfx = SfxStat::default();
    // TEAMCHECK (T7): after every frame, a fighter stuck in the air or outside a walled ring (`fight::team::watch`)
    let mut watch = std::env::var("TEAMCHECK").is_ok().then(open_kumite_beast::fight::team::watch::Watch::default);
    let mut findings = 0usize;
    loop {
        let t = s.team.as_ref().expect("the team battle");
        if t.result.is_some() || t.frame >= max {
            break;
        }
        let before = t.sound_queue.len();
        let mut i = FrameInput::default();
        for (k, sc) in scripts.iter().enumerate() {
            let p = sc.as_ref().and_then(|sc| sc.get(t.frame as usize).copied()).unwrap_or(0);
            if k < 2 {
                i.pad[k] = p;
            } else {
                i.ext[k - 2] = p;
            }
        }
        let mut out = session::Out::default();
        session::step(&mut s, &assets, i, &[], &mut out);
        // the step's sound mixed before its frame is drawn: frame n's shot ends at sample 800·n, as a match's
        if let Some(w) = &mut wav {
            w.step(&mut out);
        }
        frames += 1;
        hashes.log(&s);
        let t = s.team.as_ref().expect("the team battle");
        if let Some(sh) = &mut shots {
            sh.take_team(&s, &data);
        }
        if let Some(w) = &mut watch {
            for l in w.step(t, &data) {
                findings += 1;
                println!("check: {l}");
            }
        }
        sfx.frame(before, t.sound_queue.len(), t.sounds_started.len());
        if snddbg {
            for id in &t.sounds_started {
                println!("  snd {:5} id {:#05x} slot {} tone {} queue {}", t.frame, id, id >> 6, id & 63, t.sound_queue.len());
            }
            // (T6) the imported fighters' (or BIG STAGE's own) sounds, as the session plays them (`br1slot<n>`)
            let br1 = t.br1.as_ref().map_or(&t.world.stage.br1.sounds, |b| &b.sounds_started);
            for id in br1 {
                println!("  snd {:5} id {:#05x} br1slot {} tone {}", t.frame, id, id >> 6, id & 63);
            }
        }
        // FADEDBG (T8): each fighter's palette fade (`PlayerExt::p4b18`) and the World's two slots, while one is set
        if fadedbg && (t.ext.iter().any(|e| e.p4b18 != [0; 4]) || t.world.p4b18.iter().any(|p| *p != [0; 4])) {
            println!("fade {:5} ext {:?} world {:?}", t.frame, t.ext.map(|e| e.p4b18), t.world.p4b18);
        }
        if rounddbg && (t.round.state, t.round.out) != last {
            println!(
                "frame {:5} round {} state {:#x} out {:?} wins {:?} timer {}",
                t.frame, t.round.index, t.round.state, t.round.out, t.round.wins, t.round.timer
            );
            last = (t.round.state, t.round.out);
        }
        // (TEAM ARCADE) ROUNDDBG: each new message of the arcade states (YOU WIN / YOU LOSE, CONTINUE? n, GAME OVER)
        if rounddbg && t.arcade.is_some() && t.round.state >= 0xD && t.round.message != last_msg {
            println!("frame {:5} message {:?}", t.frame, t.round.message.text());
            last_msg = t.round.message;
        }
        if every > 0 && t.frame.is_multiple_of(every) {
            for (k, f) in t.fighters.iter().enumerate() {
                println!(
                    "  {:5} P{} {} pos ({:5},{:5}) head y {:5} dmg {:3}/{:3} tgt P{} move {:#05x} F58 {:08x} out {}",
                    t.frame,
                    k + 1,
                    data.fighters[k].code,
                    f.m_t[0],
                    f.m_t[2],
                    f.head[1],
                    f.o_ac,
                    f.o_b0,
                    t.target[k] + 1,
                    f.move_cur,
                    f.flags[1],
                    t.round.out[k]
                );
            }
            // (T7) the imported cores (state, place, F.50 / F.64) and the throws across the engines
            if let Some(b) = t.br1.as_deref() {
                for k in (0..4).filter(|&k| b.side[k]) {
                    let c = &b.fs[k];
                    println!(
                        "  {:5} P{} core state {:#05x} pos {:?} y0 {} f50 {:08x} f64 {:08x} root y {} vy {} g {} floor {} step {} key {} stun {} vel {:?}",
                        t.frame,
                        k + 1,
                        c.s.state_cur,
                        c.anim.pos,
                        c.world(0)[1],
                        c.f.f50,
                        c.f.f64,
                        t.fighters[k].m_t[1],
                        c.f.vy,
                        c.f.gravity,
                        c.f.floor,
                        c.f.step,
                        c.anim.key,
                        c.f.hit_stun,
                        c.f.vel
                    );
                }
                for (k, c) in b.throws.iter() {
                    let g = match &c.ghost {
                        open_kumite_beast::fight::br1::bridge::Ghost::Br1(g) => format!("br1 proxy state {:#05x} pos {:?}", g.s.state_cur, g.anim.pos),
                        open_kumite_beast::fight::br1::bridge::Ghost::Br2(g) => format!("br2 proxy move {:#05x} m_t {:?}", g.move_cur, g.m_t),
                    };
                    println!("  {:5} throw P{} on P{} frames {} {}", t.frame, k + 1, c.victim + 1, c.frames, g);
                }
            }
        }
        if let Some(r) = &mut rollback {
            r.check(&s, held.clone(), i, resim);
        }
    }
    let run = t1.elapsed();
    let t = s.team.as_ref().expect("the team battle");
    if std::env::var("HITS").is_ok() {
        for e in &t.events {
            println!("{e}");
        }
    }
    println!(
        "result {:?} after {} frames (wins {:?}; {} hits; load {:.2} s, {:.1} µs / frame)",
        t.result,
        t.frame,
        t.round.wins,
        t.events.iter().filter(|e| e.contains(" hits ")).count(),
        load.as_secs_f64(),
        run.as_secs_f64() * 1e6 / frames.max(1) as f64
    );
    if std::env::var("SFXSTAT").is_ok() {
        sfx.report();
    }
    if watch.is_some() {
        println!("check: {findings} findings");
    }
    if rollback.as_mut().is_some_and(|r| r.finish() > 0) {
        std::process::exit(3);
    }
}

/// SFXSTAT (the team battle): how loaded the SFX queue (one sound started per frame) runs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct SfxStat {
    frames: u64,
    queued: u64,
    started: u64,
    /// the sum of the queue lengths after each frame (Little's law: the mean wait = sum / started)
    waiting: u64,
    max_len: usize,
    /// frames that ended with the queue full (32 waiting: a move script's next sound is dropped)
    full: u64,
    /// frames that ended with sounds still waiting
    backlog: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl SfxStat {
    fn frame(&mut self, before: usize, after: usize, started: usize) {
        self.frames += 1;
        self.queued += (after + started).saturating_sub(before) as u64;
        self.started += started as u64;
        self.waiting += after as u64;
        self.max_len = self.max_len.max(after);
        self.full += (after >= 32) as u64;
        self.backlog += (after > 0) as u64;
    }

    fn report(&self) {
        println!(
            "sfx: {} frames, {} queued, {} started, mean wait {:.2} frames, longest queue {}, {} frames with a backlog, {} full",
            self.frames,
            self.queued,
            self.started,
            self.waiting as f64 / self.started.max(1) as f64,
            self.max_len,
            self.backlog,
            self.full
        );
    }
}
