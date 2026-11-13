//! The imported reference runner (port addition, debug only): the port's script step against the original's.
//!
//! `br1test BR1_DIR RUN_DIR…` (BR1_DIR = the imported game's install, `game_data/br1`; RUN_DIR = a run of
//! `.research/br1/specs/s4/runs.sh`: `steps.bin` + `events.txt`, the original's state at every entry and return of the
//! script step 0x8018118C under PCSX-Redux). For every step, both fighters loaded from the entry snapshot, the port's
//! `fight::br1::script::step` run, and both compared field by field with the return snapshot, with the random
//! counter, the camera words and the requests (sounds, effects) against the calls the original made.
//!
//! - `independent` (default): every step from the original's entry state.
//! - `carried`: the port's own state carried from step to step through the whole run; what the rest of the frame
//!   (input, decision, physics, hits: not ported yet) changed between the original's return of one step and the entry
//!   of the next is fed into it field by field (`reference::feed`), so a port error stays visible in later steps.
//!
//! - `null`: the step not run (the entry state against the return): what the step changes, the check's baseline.
//!
//! Prints the steps per character and state with the mismatching ones, and per field path the mismatch count with the
//! first case. `V=1` prints every mismatching step.
//!
//! `br1test BR1_DIR frames [carried] RUN_DIR…` (S5; RUN_DIR = a run of `.research/br1/specs/s5/runs.sh`: `frames.bin`,
//! both fighters at every stage boundary of the original's fight frame): each stage of the port's frame (the inputs,
//! each fighter's decision part, script step and velocity, the pair's move and its push / walls / angles) against the
//! original's, from the original's state before it; `carried`: the port's own state through the whole run, the
//! stages it does not run (the hits, the time between two frames) fed in. `STAGE=` limits the report to one stage;
//! `COVER=1` adds the coverage: the decision's sections the port ran, the original's events, the states taken.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = run() {
        eprintln!("br1test: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run() -> Result<(), String> {
    use open_kumite_beast::fight::br1::object::Br1Fighter;
    use open_kumite_beast::fight::br1::reference::{self, Snap};
    use open_kumite_beast::fight::br1::res::Br1Res;
    use open_kumite_beast::fight::br1::script::{Call, Request};
    use open_kumite_beast::formats::br1_anm::CHARACTERS;
    use std::collections::BTreeMap;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: br1test BR1_DIR [carried | null | frames [carried] | init | ai] RUN_DIR…";
    let root = args.first().ok_or(usage)?;
    if args.get(1).map(String::as_str) == Some("frames") {
        return frames(root, &args[2..]);
    }
    if args.get(1).map(String::as_str) == Some("init") {
        return init(root, &args[2..]);
    }
    if args.get(1).map(String::as_str) == Some("ai") {
        return ai(root, &args[2..]);
    }
    let mode = args.get(1).map(String::as_str).filter(|m| ["carried", "null"].contains(m));
    let (carried, null) = (mode == Some("carried"), mode == Some("null"));
    let dirs = &args[if mode.is_some() { 2 } else { 1 }..];
    if dirs.is_empty() {
        return Err(usage.into());
    }
    let verbose = std::env::var("V").is_ok();
    let rd = |p: &str| std::fs::read(format!("{root}/{p}"));
    let (cmn, tables) = (
        rd("engine/CMN/CMN.ACT").map_err(|e| e.to_string())?,
        pollster::block_on(open_kumite_beast::fight::br1::res::Br1Tables::load(&open_kumite_beast::assets::Assets::new(root.to_string())))?,
    );
    let res: Vec<Option<Br1Res>> =
        CHARACTERS.iter().map(|c| rd(&format!("engine/{c}/{c}31.DAT")).ok().and_then(|a| Br1Res::new(c, a, cmn.clone(), &tables).ok())).collect();

    // per (character, state): steps, mismatching steps; per field path: count, first case
    let mut states: BTreeMap<(usize, i16), (usize, usize)> = BTreeMap::new();
    let mut fields: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let (mut steps, mut bad_steps) = (0, 0);
    // instructions run, by (op, sub byte)
    let mut ops: BTreeMap<(u8, u8), usize> = BTreeMap::new();
    // steps of a fighter whose data the imported install does not have
    let mut no_data = 0usize;
    for dir in dirs {
        let bytes = std::fs::read(format!("{dir}/steps.bin")).map_err(|e| format!("{dir}/steps.bin: {e}"))?;
        let snaps = reference::parse(&bytes)?;
        let events = reference::parse_events(&std::fs::read_to_string(format!("{dir}/events.txt")).unwrap_or_default());
        let mut ev: BTreeMap<(u32, u32), Vec<&reference::Event>> = BTreeMap::new();
        for e in &events {
            ev.entry((e.frame, e.fighter)).or_default().push(e);
        }
        let mut port: Option<[Br1Fighter; 2]> = None;
        let mut prev_post: Option<[Br1Fighter; 2]> = None;
        let mut rand_port = None;
        let mut prev_rand = None;
        let mut i = 0;
        while i + 1 < snaps.len() {
            let (pre, post) = (&snaps[i], &snaps[i + 1]);
            if pre.tag != 1 || post.tag != 2 || pre.side != post.side {
                i += 1;
                continue;
            }
            i += 2;
            let me = pre.side;
            let (Some(r0), Some(r1)) = (res.get(pre.chr(0)).and_then(Option::as_ref), res.get(pre.chr(1)).and_then(Option::as_ref)) else {
                no_data += 1;
                continue;
            };
            let rs = [r0, r1];
            let load = |sn: &Snap| -> [Br1Fighter; 2] { [reference::load(sn, 0, rs), reference::load(sn, 1, rs)] };
            let orig_pre = load(pre);
            let mut fs = orig_pre.clone();
            let mut rand = pre.rand;
            if carried {
                if let (Some(p), Some(pp)) = (&port, &prev_post) {
                    fs = std::array::from_fn(|k| reference::feed(&p[k], &pp[k], &orig_pre[k]));
                }
                if let (Some(rp), Some(pr)) = (rand_port, prev_rand) {
                    rand = if pr == pre.rand { rp } else { pre.rand };
                }
            }
            let g = reference::globals(pre);
            let mut trace = Vec::new();
            let (reqs, rand_after) = if null { (Vec::new(), rand) } else { reference::run_step(&mut fs, me, rs, &g, rand, &mut trace) };
            for pc in trace {
                let b = &rs[me].anm.bytes;
                let (op, sub) = (b.get(pc as usize).copied().unwrap_or(0xFF), b.get(pc as usize + 1).copied().unwrap_or(0));
                *ops.entry((op, sub)).or_default() += 1;
            }
            let want = load(post);
            let mut diffs: Vec<(String, String, String)> = Vec::new();
            for k in 0..2 {
                for (p, a, b) in reference::diff(&fs[k], &want[k]) {
                    diffs.push((format!("P{} {p}", k + 1), a, b));
                }
            }
            if rand_after != post.rand {
                diffs.push(("random counter".into(), rand_after.to_string(), post.rand.to_string()));
            }
            // the camera words of op 0x1B
            let mut cam = pre.cam.to_vec();
            for (_, r) in &reqs {
                if let Request::Cam { word, value } = r {
                    let o = word.offset();
                    cam[o..o + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
            if cam != post.cam {
                diffs.push(("camera words".into(), format!("{cam:02x?}"), format!("{:02x?}", post.cam)));
            }
            // the requests against the calls the original made in this step
            let md = [0, 1].map(|k| u32::from_le_bytes(pre.f(k)[0x1C..0x20].try_into().unwrap()));
            let md_post = [0, 1].map(|k| u32::from_le_bytes(post.f(k)[0x1C..0x20].try_into().unwrap()));
            let theirs: Vec<Request> = ev
                .get(&(pre.frame, reference::FIGHTERS[me]))
                .map(|v| v.iter().filter_map(|e| reference::event_request(e, md).or_else(|| reference::event_request(e, md_post))).collect())
                .unwrap_or_default();
            let ours: Vec<Request> = reqs
                .iter()
                .map(|r| r.1)
                .filter(|r| !matches!(r, Request::Cam { .. } | Request::Call { call: Call::Idle | Call::FormSetup | Call::NodeScale, .. }))
                .collect();
            if ours != theirs {
                diffs.push(("requests".into(), format!("{ours:?}"), format!("{theirs:?}")));
            }
            let chr = pre.chr(me);
            let st = states.entry((chr, orig_pre[me].s.state_cur)).or_default();
            st.0 += 1;
            steps += 1;
            if !diffs.is_empty() {
                st.1 += 1;
                bad_steps += 1;
                if verbose {
                    println!("frame {} P{} {} state {}: {} fields differ", pre.frame, me + 1, CHARACTERS[chr], orig_pre[me].s.state_cur, diffs.len());
                    for (p, a, b) in &diffs {
                        println!("    {p}: port {a} original {b}");
                    }
                }
            }
            for (p, a, b) in diffs {
                let e = fields.entry(p).or_insert((0, format!("frame {} P{}: port {a} original {b}", pre.frame, me + 1)));
                e.0 += 1;
            }
            prev_post = Some(want);
            port = Some(fs);
            rand_port = Some(rand_after);
            prev_rand = Some(post.rand);
        }
    }
    println!("{} mode: {steps} steps, {bad_steps} with a difference", mode.unwrap_or("independent"));
    if no_data > 0 {
        println!("{no_data} steps skipped: a fighter's data is not in the BR1 install");
    }
    println!("states (character state: steps / differing):");
    let mut line = String::new();
    for ((c, s), (n, b)) in &states {
        line += &format!(" {}:{s} {n}/{b}", CHARACTERS[*c]);
        if line.len() > 100 {
            println!("{line}");
            line.clear();
        }
    }
    if !line.is_empty() {
        println!("{line}");
    }
    if !ops.is_empty() {
        println!("ops run (op.sub: count), {} different:", ops.len());
        let mut line = String::new();
        for ((o, s), n) in &ops {
            line += &format!(" {o:02X}.{s:02X}:{n}");
            if line.len() > 110 {
                println!("{line}");
                line.clear();
            }
        }
        if !line.is_empty() {
            println!("{line}");
        }
    }
    if !fields.is_empty() {
        println!("differing fields (steps, first case):");
        for (p, (n, first)) in &fields {
            println!("  {p:40} {n:5}  {first}");
        }
    }
    if bad_steps > 0 { Err(format!("{bad_steps} of {steps} steps differ")) } else { Ok(()) }
}

/// The S5 frame runner (`frames` mode).
/// `br1test BR1_DIR init RUN_DIR…` (S7): every round start of the runs' `frames.bin` (a fighter's distance word
/// reset to 0x200 between two frames: 0x8017947C ran) against the port's `init::reset` of the fighter before it, in
/// the modes 0 / 1 / 2 (the closest one reported), and the fields that differ.
#[cfg(not(target_arch = "wasm32"))]
fn init(root: &str, dirs: &[String]) -> Result<(), String> {
    use open_kumite_beast::fight::br1::init::{self, Setup};
    use open_kumite_beast::fight::br1::reference::diff;
    use open_kumite_beast::fight::br1::reference_frame as rf;
    use open_kumite_beast::fight::br1::res::Br1Res;
    use open_kumite_beast::fight::br1::script::Ctx;
    use open_kumite_beast::formats::br1_anm::CHARACTERS;
    let rd = |p: &str| std::fs::read(format!("{root}/{p}"));
    let (cmn, tables) = (
        rd("engine/CMN/CMN.ACT").map_err(|e| e.to_string())?,
        pollster::block_on(open_kumite_beast::fight::br1::res::Br1Tables::load(&open_kumite_beast::assets::Assets::new(root.to_string())))?,
    );
    let res: Vec<Option<Br1Res>> =
        CHARACTERS.iter().map(|c| rd(&format!("engine/{c}/{c}31.DAT")).ok().and_then(|a| Br1Res::new(c, a, cmn.clone(), &tables).ok())).collect();
    let verbose = std::env::var("V").is_ok();
    let mut total = (0usize, 0usize);
    // records of a fighter whose data the imported install does not have
    let mut no_data = 0usize;
    for dir in dirs {
        let bytes = std::fs::read(format!("{dir}/frames.bin")).map_err(|e| format!("{dir}/frames.bin: {e}"))?;
        let recs = rf::parse(&bytes)?;
        let pick = |r: &rf::Rec| -> Option<[&Br1Res; 2]> { Some([res.get(r.snap.chr(0))?.as_ref()?, res.get(r.snap.chr(1))?.as_ref()?]) };
        for i in 0..recs.len() {
            let b = &recs[i];
            let Some(rs) = pick(b) else {
                no_data += 1;
                continue;
            };
            let ob = rf::load2(b, rs);
            let before = if i == 0 { None } else { Some(rf::load2(&recs[i - 1], rs)) };
            for k in 0..2 {
                let start = match &before {
                    None => b.snap.tag == 1,
                    Some(oa) => b.snap.tag == 1 && ob[k].f.opp_dist == 0x200 && oa[k].f.opp_dist != 0x200,
                };
                if !start {
                    continue;
                }
                let mut best: Option<(u8, Vec<(String, String, String)>)> = None;
                for mode in [0u8, 1, 2] {
                    let mut fs = before.clone().unwrap_or_else(|| ob.clone());
                    init::reset_input(&mut fs[k]);
                    if before.is_none() {
                        fs[k] = init::new(rs[k], k as u8, b.snap.chr(k) as u8, ob[k].f.cpu);
                        fs[k].s.state_req = ob[k].s.state_cur;
                    } else {
                        fs[k].s.state_req = ob[k].s.state_cur;
                    }
                    let (mut r, mut out) = (b.snap.rand, Vec::new());
                    let w = rf::world(b);
                    let mut ctx = Ctx {
                        res: rs,
                        rand: &mut r,
                        stage: w.stage,
                        round: 0,
                        walls: [true; 4],
                        need: w.opts.need,
                        g_ad: 0,
                        g_e8: 0,
                        out: &mut out,
                        trace: None,
                        rings: None,
                    };
                    let setup = Setup { fast_regen: w.opt2c & 0x80 != 0, practice: false, vitality: ob[k].f.vitality };
                    init::reset(&mut fs, k, mode, &setup, &mut ctx);
                    let d = diff(&fs[k], &ob[k]);
                    if best.as_ref().is_none_or(|(_, bd)| d.len() < bd.len()) {
                        best = Some((mode, d));
                    }
                }
                let (mode, d) = best.expect("two modes");
                total.0 += 1;
                total.1 += !d.is_empty() as usize;
                println!("{dir} rec {i} frame {} P{} {}: mode {mode}, {} fields differ", b.snap.frame, k + 1, CHARACTERS[b.snap.chr(k)], d.len());
                for (p, x, y) in d.iter().take(if verbose { usize::MAX } else { 40 }) {
                    println!("    {p}: port {x} original {y}");
                }
            }
        }
    }
    println!("{} round starts, {} differ", total.0, total.1);
    if no_data > 0 {
        println!("{no_data} records skipped: a fighter's data is not in the BR1 install");
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn frames(root: &str, args: &[String]) -> Result<(), String> {
    use open_kumite_beast::fight::br1::object::Br1Fighter;
    use open_kumite_beast::fight::br1::reference::diff;
    use open_kumite_beast::fight::br1::reference_frame::{self as rf, Stage};
    use open_kumite_beast::fight::br1::res::Br1Res;
    use open_kumite_beast::formats::br1_anm::CHARACTERS;
    use std::collections::BTreeMap;

    let carried = args.first().map(String::as_str) == Some("carried");
    let dirs = &args[carried as usize..];
    if dirs.is_empty() {
        return Err("usage: br1test BR1_DIR frames [carried] RUN_DIR…".into());
    }
    let verbose = std::env::var("V").is_ok();
    let only = std::env::var("STAGE").ok();
    let rd = |p: &str| std::fs::read(format!("{root}/{p}"));
    let (cmn, tables) = (
        rd("engine/CMN/CMN.ACT").map_err(|e| e.to_string())?,
        pollster::block_on(open_kumite_beast::fight::br1::res::Br1Tables::load(&open_kumite_beast::assets::Assets::new(root.to_string())))?,
    );
    let res: Vec<Option<Br1Res>> =
        CHARACTERS.iter().map(|c| rd(&format!("engine/{c}/{c}31.DAT")).ok().and_then(|a| Br1Res::new(c, a, cmn.clone(), &tables).ok())).collect();
    // per stage: steps, differing steps; per (stage, field): count, first case
    let mut stages: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut fields: BTreeMap<(String, String), (usize, String)> = BTreeMap::new();
    let (mut steps, mut bad) = (0usize, 0usize);
    // the decision's sections reached, the original's events, the states taken per character
    let mut sections: BTreeMap<open_kumite_beast::fight::br1::decision::Section, usize> = BTreeMap::new();
    let mut census: BTreeMap<String, usize> = BTreeMap::new();
    let mut states: BTreeMap<String, std::collections::BTreeSet<i16>> = BTreeMap::new();
    let mut excluded: BTreeMap<String, usize> = BTreeMap::new();
    // records of a fighter whose data the imported install does not have
    let mut no_data = 0usize;
    for dir in dirs {
        let bytes = std::fs::read(format!("{dir}/frames.bin")).map_err(|e| format!("{dir}/frames.bin: {e}"))?;
        let recs = match rf::parse(&bytes) {
            Ok(r) => r,
            Err(e) => {
                println!("{dir}: skipped ({e})");
                continue;
            }
        };
        // the call log (when the run has one): the original's requests per stage
        let calls = std::fs::read_to_string(format!("{dir}/calls.txt")).ok().map(|t| rf::parse_calls(&t));
        let mut by_rec: BTreeMap<usize, Vec<&rf::Call>> = BTreeMap::new();
        for c in calls.iter().flatten() {
            by_rec.entry(c.rec).or_default().push(c);
        }
        // the fighters' data by the record's characters (a demo changes them between fights)
        let pick = |r: &rf::Rec| -> Option<[&Br1Res; 2]> { Some([res.get(r.snap.chr(0))?.as_ref()?, res.get(r.snap.chr(1))?.as_ref()?]) };
        let run = dir.rsplit('/').next().unwrap_or(dir);
        let mut port: Option<([Br1Fighter; 2], open_kumite_beast::fight::br1::physics::Walls, u8)> = None;
        let mut ko = [false; 2];
        for i in 0..recs.len().saturating_sub(1) {
            let (a, b) = (&recs[i], &recs[i + 1]);
            let (Some(st), Some(rs)) = (rf::stage(a, b), pick(a)) else {
                no_data += (rf::stage(a, b).is_some() && pick(a).is_none()) as usize;
                port = None;
                continue;
            };
            if (a.snap.chr(0), a.snap.chr(1)) != (b.snap.chr(0), b.snap.chr(1)) {
                port = None;
                continue;
            }
            let name = format!("{run} {}-{}", CHARACTERS[a.snap.chr(0)], CHARACTERS[a.snap.chr(1)]);
            let (oa, ob) = (rf::load2(a, rs), rf::load2(b, rs));
            let (wa, wb) = (rf::walls(a), rf::walls(b));
            let (mut fs, mut walls, _) = match (&port, carried) {
                (Some(p), true) => p.clone(),
                _ => (oa.clone(), wa, a.snap.rand),
            };
            // the random counter is the original's (its other consumers, effects and the frame tick, are
            // presentation: the port's draws are compared within each stage)
            let mut rand = a.snap.rand;
            if !st.implemented() {
                if carried {
                    rf::feed(&mut fs, &mut walls, &mut rand, (&oa, &ob), a, b);
                    port = Some((fs, walls, rand));
                }
                continue;
            }
            // the known exclusions: the original's KO replay playback (the round flow's) and its guts save (dropped)
            let known = if st == Stage::Inputs && rf::playback(a) {
                Some("KO replay playback")
            } else if matches!(st, Stage::Decide(k) if rf::guts(&oa[k], &ob[k])) {
                Some("guts save (dropped)")
            } else {
                None
            };
            if let Some(why) = known {
                *excluded.entry(format!("{st:?}: {why}")).or_default() += 1;
                if carried {
                    rf::feed(&mut fs, &mut walls, &mut rand, (&oa, &ob), a, b);
                    port = Some((fs, walls, rand));
                }
                continue;
            }
            let mut trace = Vec::new();
            let before = [fs[0].f.status, fs[1].f.status];
            let ko_k = matches!(st, Stage::Velocity(k) if ko[k]);
            let reqs = rf::run(st, &mut fs, &mut walls, &mut rand, a, (&oa, &ob, b), rs, Some(&mut trace), ko_k);
            if let Stage::Decide(k) = st {
                ko[k] = before[k] == 1 && fs[k].f.status == 0x80;
            }
            // (the script step's trace holds its pcs: only the decision's sections counted)
            for s in trace.into_iter().filter_map(open_kumite_beast::fight::br1::decision::section) {
                *sections.entry(s).or_default() += 1;
            }
            // what the original did in this stage (the runs' coverage)
            for k in 0..2 {
                let (x, y) = (&oa[k], &ob[k]);
                let c = CHARACTERS[y.f.chr as usize % 9];
                let mut tally = |what: &str| *census.entry(format!("{c} {what}")).or_default() += 1;
                match st {
                    Stage::Decide(j) if j == k => {
                        states.entry(c.to_string()).or_default().insert(y.s.state_req);
                        if y.s.state_req != x.s.state_req && y.s.state_req >= 0x100 {
                            tally("command / chain state");
                        }
                        if y.f.f64 & 0x10_0000 != 0 && x.f.f64 & 0x10_0000 == 0 {
                            tally("ring-out");
                        }
                        if y.f.f54 & 0x4_0000 != 0 {
                            tally("airborne frame");
                        }
                        if y.f.status & 0x80 != 0 && x.f.status & 0x80 == 0 {
                            tally("KO");
                        }
                        if y.f.form & 1 != x.f.form & 1 {
                            tally("form change in the decision (revert)");
                        }
                        if y.s.state_req != x.s.state_req && (0x40..0x80).contains(&y.s.state_req) {
                            tally("hit reaction state");
                        }
                    }
                    Stage::Inputs => {
                        if y.f.f64 & 0x40_0000 != 0 && x.f.f64 & 0x40_0000 == 0 && y.f.f64 & 0x20_0000 == 0 {
                            tally("throw grab");
                        }
                        if y.f.f64 & 0x60_0000 == 0x60_0000 && x.f.f64 & 0x60_0000 != 0x60_0000 {
                            tally("throw whiff");
                        }
                        if y.s.cand_mv != x.s.cand_mv && y.s.cand_mv >= 0 || y.s.att_prio != x.s.att_prio && y.s.att_prio >= 0 {
                            tally("command registered");
                        }
                        if y.s.chain_len != x.s.chain_len {
                            tally("chain follow-up queued");
                        }
                    }
                    Stage::Move if x.f.f64 & 0x6_0000 != 0 => tally("moved as a throw victim"),
                    Stage::Hits if y.f.f54 & 0x600_0000 != 0 && (x.f.f54 & 0x600_0000 == 0 || x.f.f60 != y.f.f60) => {
                        let r = y.f.f60;
                        let combo = ob[1 - k].f.combo;
                        tally(if y.f.f54 & 0x400_0000 != 0 { "hit taken" } else { "guarded" });
                        for (bit, name) in [
                            (0x80_0000, "crush / big hit"),
                            (0x800, "launch 0x800"),
                            (0x1_0000, "blow-away 0x10000"),
                            (0x100_0000, "strong stagger 0x1000000"),
                            (0x4_0000, "launch 0x40000"),
                            (0x4000, "knockdown 0x4000"),
                            (0x2000, "stagger 0x2000"),
                            (0x8000, "spin 0x8000"),
                            (0x200_0000, "down hit 0x2000000"),
                            (0x10_0000, "low"),
                            (0x2, "from behind"),
                        ] {
                            if r & bit != 0 {
                                tally(name);
                            }
                        }
                        if combo >= 2 {
                            tally("hit in a combo (attacker D0 >= 2)");
                        }
                        if y.f.f50 & 0x800 != 0 || y.f.f54 & 0x4_0000 != 0 {
                            tally("hit / guarded in the air");
                        }
                        if x.f.f50 & 0x8000_0000 != 0 {
                            tally("hit while attacking (counter)");
                        }
                    }
                    Stage::Script(j) if j == k && y.f.form & 1 != x.f.form & 1 => tally("form change in the script"),
                    Stage::Push => {
                        if y.f.f54 & 0x80_0000 != 0 {
                            tally("push-apart");
                        }
                        if y.f.f64 & 0xF != 0 {
                            tally("wall contact");
                        }
                        if y.f.f64 & 0xF0 != 0 {
                            tally("near a wall");
                        }
                    }
                    _ => {}
                }
            }
            let mut diffs: Vec<(String, String, String)> = Vec::new();
            // the original's MD links against the port's derivation (`Br1Res::linked`)
            for k in 0..2 {
                let l = rf::linked(b, k);
                if l != rs[k].linked {
                    diffs.push((format!("P{} linked nodes", k + 1), format!("{:x?}", rs[k].linked), format!("{l:x?}")));
                }
            }
            for k in (0..2).filter(|&k| fs[k] != ob[k]) {
                for (p, x, y) in diff(&fs[k], &ob[k]) {
                    diffs.push((format!("P{} {p}", k + 1), x, y));
                }
            }
            if walls != wb {
                diffs.push(("walls".into(), format!("{walls:?}"), format!("{wb:?}")));
            }
            // with a call log: the counter advanced by the port's own draws only (the effects' draws are presentation)
            let want = match &calls {
                Some(_) => a.snap.rand.wrapping_add(rf::logic_draws(by_rec.get(&(i + 1)).map_or(&[][..], |v| &v[..]))),
                None => b.snap.rand,
            };
            if rand != want {
                diffs.push(("random counter".into(), rand.to_string(), want.to_string()));
            }
            if calls.is_some() {
                let md = |r: &rf::Rec| [0, 1].map(|k| u32::from_le_bytes(r.snap.f(k)[0x1C..0x20].try_into().unwrap_or([0; 4])));
                let theirs: Vec<_> = by_rec
                    .get(&(i + 1))
                    .map(|v| v.iter().filter_map(|c| rf::call_request(c, md(a)).or_else(|| rf::call_request(c, md(b)))).collect())
                    .unwrap_or_default();
                let ours = rf::comparable(&reqs);
                if ours != theirs {
                    diffs.push(("requests".into(), format!("{ours:?}"), format!("{theirs:?}")));
                }
            }
            let cam = rf::cam_after(a, &reqs);
            if cam != b.snap.cam {
                diffs.push(("camera words".into(), format!("{cam:02x?}"), format!("{:02x?}", b.snap.cam)));
            }
            let sname = format!("{st:?}");
            port = Some((fs, walls, rand));
            if only.as_ref().is_some_and(|o| !sname.starts_with(o.as_str())) {
                continue;
            }
            let e = stages.entry(sname.clone()).or_default();
            e.0 += 1;
            steps += 1;
            if !diffs.is_empty() {
                e.1 += 1;
                bad += 1;
                if verbose {
                    println!("{name} frame {} {sname}: {} fields differ", a.snap.frame, diffs.len());
                    for (p, x, y) in diffs.iter().take(40) {
                        println!("    {p}: port {x} original {y}");
                    }
                }
            }
            for (p, x, y) in diffs {
                let e = fields.entry((sname.clone(), p)).or_insert((0, format!("{name} frame {}: port {x} original {y}", a.snap.frame)));
                e.0 += 1;
            }
        }
    }
    if std::env::var("COVER").is_ok() {
        println!("decision sections reached (section: steps):");
        let line: Vec<String> = sections.iter().map(|(a, n)| format!("{a:?}:{n}")).collect();
        for l in line.chunks(8) {
            println!("  {}", l.join(" "));
        }
        println!("events in the original:");
        for (k, n) in &census {
            println!("  {k:40} {n}");
        }
        println!("states requested per character:");
        for (c, s) in &states {
            let v: Vec<String> = s.iter().map(|x| x.to_string()).collect();
            println!("  {c} ({}): {}", v.len(), v.join(" "));
        }
    }
    println!("frames {} mode: {steps} stage steps, {bad} with a difference", if carried { "carried" } else { "independent" });
    if no_data > 0 {
        println!("{no_data} stage steps skipped: a fighter's data is not in the BR1 install");
    }
    for (s, (n, b)) in &stages {
        println!("  {s:12} {n:6} steps, {b:6} differ");
    }
    for (why, n) in &excluded {
        println!("  excluded: {why}: {n}");
    }
    if !fields.is_empty() {
        println!("differing fields (stage, path: steps, first case):");
        for ((s, p), (n, first)) in &fields {
            let first: String = first.chars().take(200).collect();
            println!("  {s:12} {p:36} {n:5}  {first}");
        }
    }
    if bad > 0 { Err(format!("{bad} of {steps} stage steps differ")) } else { Ok(()) }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

/// `br1test BR1_DIR ai DUMP_DIR…` (S10; DUMP_DIR = a harness run with RAM dumps `fN.ram` / `fN.spad` at frame pairs N,
/// N + 1, as `.research/br1/s10/runs.sh` makes them): for every CPU side of dump N whose AI runs (F.01 != 0, F.00 &
/// 0x8080 == 0), the port's AI (`fight::br1::ai`) from the AI block, both fighter blocks, the hips, S.AA, the random
/// table and the timer of dump N, against the AI block of dump N + 1 field by field. `URI=1` adds the synthetic check
/// of the guard-stun push (each side forced into state 3 in guard stun at level 0xFF, run as MIT, as URI
/// human and as URI in beast form); `SYNTH=1` prints the block after the synthetic patches of
/// `.research/br1/s10/ai_synth.py` instead (states 6, 10..21: diff the two). `V=1` prints every mismatching frame.
#[cfg(not(target_arch = "wasm32"))]
fn ai(root: &str, dirs: &[String]) -> Result<(), String> {
    use open_kumite_beast::fight::br1::ai::{Br1Ai, Ctx, Table};
    use open_kumite_beast::fight::br1::object::{Block, Br1Fighter, Node};
    use open_kumite_beast::fight::br1::res::Br1Res;
    use open_kumite_beast::formats::br1_anm::CHARACTERS;
    use std::collections::BTreeMap;
    let rd = |p: &str| std::fs::read(format!("{root}/{p}"));
    let (cmn, tables) = (
        rd("engine/CMN/CMN.ACT").map_err(|e| e.to_string())?,
        pollster::block_on(open_kumite_beast::fight::br1::res::Br1Tables::load(&open_kumite_beast::assets::Assets::new(root.to_string())))?,
    );
    let res: Vec<Option<Br1Res>> =
        CHARACTERS.iter().map(|c| rd(&format!("engine/{c}/{c}31.DAT")).ok().and_then(|a| Br1Res::new(c, a, cmn.clone(), &tables).ok())).collect();
    let verbose = std::env::var("V").is_ok();
    let u32at = |b: &[u8], o: usize| b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
    let ram = |addr: u32| (addr & 0x1F_FFFF) as usize;
    // the AI block at RAM address `a` (pc and stack less the ANM's base)
    let block = |m: &[u8], a: u32, base: u32| {
        let o = ram(a);
        let h = |k: usize| i16::from_le_bytes([m[o + k], m[o + k + 1]]);
        let w = |k: usize| u32at(m, o + k);
        Br1Ai {
            idx: m[o + 4],
            pad: h(6) as u16,
            level: h(8) as u16,
            flags: h(0xA) as u16,
            pc: w(0x24).wrapping_sub(base),
            state: h(0x28),
            t2a: h(0x2A),
            t2c: h(0x2C),
            action: w(0x30) as i32,
            band: w(0x34) as i32,
            res: w(0x38) as i32,
            regs: std::array::from_fn(|k| h(0x3C + 2 * k)),
            stack: std::array::from_fn(|k| w(0x4C + 4 * k).wrapping_sub(base)),
            depth: h(0x6C) as u16,
        }
    };
    // the fields the port keeps, as (name, value) pairs (r0..r7: r8.. overlap the stack; the stack up to the depth)
    let fields = |a: &Br1Ai| {
        let mut v = vec![
            ("idx", a.idx as i64),
            ("pad", a.pad as i64),
            ("level", a.level as i64),
            ("flags", a.flags as i64),
            ("pc", a.pc as i64),
            ("state", a.state as i64),
            ("t2a", a.t2a as i64),
            ("t2c", a.t2c as i64),
            ("action", a.action as i64),
            ("band", a.band as i64),
            ("res", a.res as i64),
            ("depth", a.depth as i64),
        ];
        const R: [&str; 8] = ["r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7"];
        const S: [&str; 8] = ["s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7"];
        v.extend((0..8).map(|k| (R[k], a.regs[k] as i64)));
        v.extend((0..(a.depth as usize).min(8)).map(|k| (S[k], a.stack[k] as i64)));
        v
    };
    let (mut total, mut good) = (0usize, 0usize);
    let mut states: BTreeMap<i16, (usize, usize)> = BTreeMap::new();
    let mut pads: BTreeMap<u16, usize> = BTreeMap::new();
    let mut ops = 0usize;
    // frames at a round loop step before the inputs
    let mut skipped = 0usize;
    // the synthetic Uriko check: per (chr, form) the frames run and those with pad 0x2000
    let mut uri: BTreeMap<(u8, u8), (usize, usize)> = BTreeMap::new();
    for dir in dirs {
        let mut frames: Vec<u32> = std::fs::read_dir(dir)
            .map_err(|e| format!("{dir}: {e}"))?
            .filter_map(|e| e.ok()?.file_name().to_str()?.strip_prefix('f')?.strip_suffix(".spad")?.parse().ok())
            .collect();
        frames.sort();
        for &f in &frames {
            let load = |n: u32| -> Result<(Vec<u8>, Vec<u8>), String> {
                let r = |e: &str| std::fs::read(format!("{dir}/f{n}.{e}")).map_err(|e| format!("{dir}/f{n}: {e}"));
                Ok((r("spad")?, r("ram")?))
            };
            let (sp, ma) = load(f)?;
            for side in 0..2 {
                let fb = 0xF4 + 0xEC * side;
                if sp[fb + 1] == 0 || (sp[fb] | sp[fb + 1]) & 0x80 != 0 {
                    continue;
                }
                let me_blk = Block::read(&sp[fb..fb + 0xEC]);
                let Some(r) = res.get(me_blk.chr as usize).and_then(Option::as_ref) else { continue };
                let (base, md, s, a, op) = (u32at(&sp, fb + 0x2C), u32at(&sp, fb + 0x1C), u32at(&sp, fb + 0x30), u32at(&sp, fb + 0x34), u32at(&sp, fb + 0x28));
                let ob = (op.wrapping_sub(0x1F80_0000) as usize).min(sp.len() - 0xEC);
                let opp = Block::read(&sp[ob..ob + 0xEC]);
                let hips = |mdp: u32| u32at(&ma, ram(mdp + 0x178)) as i32;
                let mut me = Br1Fighter { f: me_blk, ..Default::default() };
                me.anim.id = i16::from_le_bytes([ma[ram(s + 0xAA)], ma[ram(s + 0xAB)]]);
                me.forms[0].md = vec![Node::default(); 5];
                me.forms[0].md[4].wt[1] = hips(md);
                let opp_hips = hips(u32at(&sp, ob + 0x1C));
                let t = ram(0x801F_0EC0);
                let table = Table(std::array::from_fn(|k| ma[t + k]));
                let timer = i16::from_le_bytes([sp[0x90], sp[0x91]]);
                let before = block(&ma, a, base);
                let c = Ctx { me: &me, opp: &opp, opp_hips, anm: &r.anm.bytes, tables: &r.tables, rnd: &table, timer };
                if std::env::var("SYNTH").is_ok() {
                    // `.research/br1/s10/ai_synth.py`'s cases (name, state, level, F.50 set / clear, F.54 set, F.60 set)
                    let mut cases: Vec<(String, i16, u16, u32, u32, u32, u32)> = vec![
                        ("s6a".into(), 6, 0x60, 0x8000, 0, 0, 0),
                        ("s6b".into(), 6, 0xF0, 0x8000, 0, 0, 0),
                        ("s6off".into(), 6, 0x60, 0, 0x8000, 0, 0),
                        ("s4to6".into(), 4, 0x60, 0x8000, 0x3_0000, 0, 0x1000),
                    ];
                    for st in 10..22 {
                        for lv in [0, 0xF0] {
                            cases.push((format!("s{st}"), st, lv, 0, 0, 0, 0));
                        }
                    }
                    cases.extend([
                        ("s14gs".into(), 14, 0x60, 0, 0, 0x200_0000, 0),
                        ("s14st".into(), 14, 0x60, 0, 0, 0x2000_0000, 0x82_1000),
                        ("s21rec".into(), 21, 0x60, 0x1000, 0, 0, 0),
                        ("s11air".into(), 11, 0x60, 0x200_0000, 0, 0x400_0000, 0),
                    ]);
                    let name = std::path::Path::new(dir).file_name().and_then(|n| n.to_str()).unwrap_or("");
                    for (case, st, lv, s50, c50, s54, s60) in cases {
                        let mut x = before.clone();
                        (x.state, x.level) = (st, lv);
                        let mut m = me.clone();
                        m.f.f50 = (m.f.f50 | s50) & !c50;
                        m.f.f54 |= s54;
                        m.f.f60 |= s60;
                        x.think(&Ctx { me: &m, ..c });
                        println!(
                            "{name} f{f} P{} {case}: idx {} pad {:#x} flags {:#x} pc {:#x} state {} t2a {} t2c {} action {} band {} res {} depth {} r0 {}",
                            side + 1,
                            x.idx,
                            x.pad,
                            x.flags,
                            x.pc,
                            x.state,
                            x.t2a,
                            x.t2c,
                            x.action,
                            x.band,
                            x.res,
                            x.depth,
                            x.regs[0]
                        );
                    }
                    continue;
                }
                if std::env::var("URI").is_ok() {
                    for (chr, form) in [(4u8, 1u8), (8, 1), (8, 0x11)] {
                        let mut x = before.clone();
                        (x.state, x.level) = (3, 0xFF);
                        let mut m = me.clone();
                        (m.f.chr, m.f.form, m.f.hit_stun) = (chr, form, 0x20);
                        m.f.f54 = (m.f.f54 | 0x200_0000) & !0x80;
                        m.f.f50 &= 0x3FFF_FFFF;
                        x.think(&Ctx { me: &m, ..c });
                        let e = uri.entry((chr, form)).or_default();
                        e.0 += 1;
                        e.1 += (x.pad & 0x2000 != 0) as usize;
                    }
                }
                if !frames.contains(&(f + 1)) {
                    continue;
                }
                let (sb, mb) = load(f + 1)?;
                let want = block(&mb, a, base);
                // the round loop's work before the inputs: a round start's AI init (0x80184FF0: the index back to
                // player · 128), FIGHT!'s end of the intro lock (F.58 0x10000) between the dumps
                let f58 = |b: &[u8]| u32at(b, fb + 0x58);
                if (want.idx == me.f.player << 7 && before.idx != want.idx) || (f58(&sp) ^ f58(&sb)) & 0x1_0000 != 0 {
                    skipped += 1;
                    continue;
                }
                let mut got = before.clone();
                got.think(&c);
                total += 1;
                let (gf, wf) = (fields(&got), fields(&want));
                let diff: Vec<String> = wf
                    .iter()
                    .filter(|w| !gf.contains(w))
                    .map(|(n, v)| format!("{n} {:#x}/{v:#x}", gf.iter().find(|g| g.0 == *n).map_or(-1, |g| g.1)))
                    .collect();
                let e = states.entry(want.state).or_default();
                e.0 += 1;
                *pads.entry(want.pad).or_default() += 1;
                ops += (before.pc != want.pc) as usize;
                if diff.is_empty() {
                    good += 1;
                } else {
                    e.1 += 1;
                    if verbose || total - good <= 10 {
                        println!("{dir} f{f} P{} {} state {} → {}: {}", side + 1, CHARACTERS[me.f.chr as usize], before.state, want.state, diff.join(", "));
                    }
                }
            }
        }
    }
    println!("{good} / {total} AI frames as the original's (the block after the frame); by the state after it (frames, mismatching): {states:?}");
    println!("distinct pads {}: {pads:x?}; frames whose VM position moved: {ops}; skipped at a round start or FIGHT!: {skipped}", pads.len());
    for ((chr, form), (n, hit)) in &uri {
        println!("synthetic guard-stun push, {} form {form:#04x}: pad 0x2000 on {hit} / {n} frames", CHARACTERS[*chr as usize]);
    }
    if good < total { Err(format!("{} of {total} AI frames differ", total - good)) } else { Ok(()) }
}
