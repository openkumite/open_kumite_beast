//! Layering check (engine addition, restructure stage 1), ENFORCING since
//! stage 4: it scans `src/**/*.rs` for module paths (`crate::…`, `super::…`, a `mod.rs`'s child modules, `std::fs` /
//! `std::env`, the platform crates), puts every file into its layer directory (ARCHITECTURE.md, Layers) and checks the edges
//! between those directories against the target layer rules. It fails on a forbidden edge that `EXCEPTIONS` does not
//! list, on an L2 file naming a platform crate or `std::fs` outside test code that `EXTERNAL_EXCEPTIONS` does not
//! list, on a file missing from `FILES`, and on an exception that matches nothing any more (remove it: the lists only
//! shrink). Env reads and hash maps in L2 are listed in the report, not enforced (rules 2 / 5).
//!
//! `cargo test --release --test layering -- --nocapture` shows the report; it is also written to
//! `$CARGO_TARGET_TMPDIR/layering_report.txt`.
//!
//! The directories are where each file goes as a whole. Items that a later stage moves out of a file count as the
//! file they are in today, so the edges that stage removes are exceptions named after it.
//!
//! `super::` resolves from the inline module (`mod name { … }`) it is written in; code inside a `#[cfg(test)]` /
//! `#[cfg(all(test, …))]` inline module is test code, and so is a whole file that its parent declares that way (`mod
//! tests;`, the tests in a sibling `tests.rs`: `test_file`). A module imported by name (`use crate::base::decode_limit;`) is an edge at its `use`; its later uses (`decode_limit::x`)
//! are not scanned again.

mod common;
use common::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// The layer directories (ARCHITECTURE.md, Layers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Dir {
    Base,
    Platform,
    Formats,
    Assets,
    Output,
    Fight,
    Frontend,
    Session,
    Render,
    Audio,
    Net,
    Present,
    App,
}
use Dir::*;

impl Dir {
    /// L0 base / platform … L5 app.
    fn layer(self) -> u8 {
        match self {
            Base | Platform => 0,
            Formats | Assets | Output => 1,
            Fight | Frontend | Session => 2,
            Render | Audio | Net => 3,
            Present => 4,
            App => 5,
        }
    }

    /// The order inside L2: session → frontend → fight.
    fn l2_rank(self) -> u8 {
        match self {
            Session => 2,
            Frontend => 1,
            _ => 0,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Base => "base",
            Platform => "platform",
            Formats => "formats",
            Assets => "assets",
            Output => "output",
            Fight => "fight",
            Frontend => "frontend",
            Session => "session",
            Render => "render",
            Audio => "audio",
            Net => "net",
            Present => "present",
            App => "app",
        }
    }
}

/// Every file under `src/` (but `lib.rs`, `main.rs`, `bin/`) and the directory it belongs to. A file that
/// stage 2 or 5 splits is listed under the part that keeps the file; `MIXED` names the parts that leave.
const FILES: &[(&str, Dir)] = &[
    ("base/mod.rs", Base),
    ("base/serde_util.rs", Base),
    ("base/trace.rs", Base),
    ("base/decode_limit.rs", Base),
    ("base/libc_rand.rs", Base),
    ("base/portable.rs", Base),
    ("base/hash.rs", Base),
    ("base/build.rs", Base),
    ("base/env.rs", Base),
    ("platform/mod.rs", Platform),
    ("platform/clock.rs", Platform),
    ("platform/game_data.rs", Platform),
    ("platform/storage.rs", Platform),
    ("platform/support.rs", Platform),
    ("platform/android.rs", Platform),
    ("formats/mod.rs", Formats),
    ("formats/act_ani.rs", Formats),
    ("formats/br1_anm.rs", Formats),
    ("formats/container.rs", Formats),
    ("formats/gltf_model.rs", Formats),
    ("formats/gltf_pose.rs", Formats),
    ("formats/movie.rs", Formats),
    ("formats/music.rs", Formats),
    ("formats/pcm.rs", Formats),
    ("formats/q3_map.rs", Formats),
    ("formats/tables.rs", Formats),
    ("formats/xa.rs", Formats),
    ("formats/vab.rs", Formats),
    ("formats/vram.rs", Formats),
    ("assets/mod.rs", Assets),
    ("assets/br1_art.rs", Assets),
    ("assets/extras.rs", Assets),
    ("assets/game_tables.rs", Assets),
    ("assets/manifest.rs", Assets),
    ("assets/roster.rs", Assets),
    ("assets/source.rs", Assets),
    ("assets/text.rs", Assets),
    ("frontend/data.rs", Assets),
    ("output/mod.rs", Output),
    ("output/backdrop.rs", Output),
    ("output/font.rs", Output),
    ("output/gte.rs", Output),
    ("output/layout.rs", Output),
    ("output/prim.rs", Output),
    ("output/psx_quad.rs", Output),
    ("output/sound.rs", Output),
    ("output/sprite.rs", Output),
    ("output/tiny_font.rs", Output),
    ("fight/mod.rs", Fight),
    ("fight/ai.rs", Fight),
    ("fight/anim/mod.rs", Fight),
    ("fight/anim/act_clips.rs", Fight),
    ("fight/anim/playback.rs", Fight),
    ("fight/anim/pose.rs", Fight),
    ("fight/anim/spring.rs", Fight),
    ("fight/br1/address_map.rs", Fight),
    ("fight/br1/ai.rs", Fight),
    ("fight/br1/bridge.rs", Fight),
    ("fight/br1/decision.rs", Fight),
    ("fight/br1/frame.rs", Fight),
    ("fight/br1/frame_tests.rs", Fight),
    ("fight/br1/hit.rs", Fight),
    ("fight/br1/init.rs", Fight),
    ("fight/br1/input.rs", Fight),
    ("fight/br1/mod.rs", Fight),
    ("fight/br1/object.rs", Fight),
    ("fight/br1/physics.rs", Fight),
    ("fight/br1/pose.rs", Fight),
    ("fight/br1/reference.rs", Fight),
    ("fight/br1/reference_frame.rs", Fight),
    ("fight/br1/res.rs", Fight),
    ("fight/br1/script/mod.rs", Fight),
    ("fight/br1/script/ops.rs", Fight),
    ("fight/br1/script/tests.rs", Fight),
    ("fight/br1/throw.rs", Fight),
    ("fight/camera.rs", Fight),
    ("fight/duel/mod.rs", Fight),
    ("fight/duel/br1.rs", Fight),
    ("fight/duel/frame.rs", Fight),
    ("fight/duel/hits.rs", Fight),
    ("fight/duel/setup.rs", Fight),
    ("fight/fight_data.rs", Fight),
    ("fight/stage/extra.rs", Fight),
    ("fight/stage/br1.rs", Fight),
    ("fight/fighter.rs", Fight),
    ("fight/fx/mod.rs", Fight),
    ("fight/fx/effect_models.rs", Fight),
    ("fight/fx/host.rs", Fight),
    ("fight/fx/hit_effects/mod.rs", Fight),
    ("fight/fx/hit_effects/quads.rs", Fight),
    ("fight/fx/hit_effects/sprites.rs", Fight),
    ("fight/fx/hit_effects/tests.rs", Fight),
    ("fight/fx/hit_effects/timeline.rs", Fight),
    ("fight/fx/hit_effects/trig.rs", Fight),
    ("fight/fx/hit_effects/update.rs", Fight),
    ("fight/fx/move_effects.rs", Fight),
    ("fight/fx/native_tasks.rs", Fight),
    ("fight/fx/script_link.rs", Fight),
    ("fight/fx/script_vm/mod.rs", Fight),
    ("fight/fx/script_vm/anims.rs", Fight),
    ("fight/fx/script_vm/interp.rs", Fight),
    ("fight/fx/script_vm/mat3.rs", Fight),
    ("fight/fx/script_vm/mem.rs", Fight),
    ("fight/fx/script_vm/ops.rs", Fight),
    ("fight/fx/script_vm/tests.rs", Fight),
    ("fight/fx/script_vm/timeline.rs", Fight),
    ("fight/hit.rs", Fight),
    ("fight/hud.rs", Fight),
    ("fight/input.rs", Fight),
    ("fight/input_script.rs", Fight),
    ("fight/move_decision/mod.rs", Fight),
    ("fight/move_decision/reaction.rs", Fight),
    ("fight/move_decision/world.rs", Fight),
    ("fight/move_script/head_turn.rs", Fight),
    ("fight/move_script/mod.rs", Fight),
    ("fight/move_script/sound.rs", Fight),
    ("fight/move_script/step.rs", Fight),
    ("fight/move_script/throw_damage.rs", Fight),
    ("fight/pause.rs", Fight),
    ("fight/physics.rs", Fight),
    ("fight/round.rs", Fight),
    ("fight/rules.rs", Fight),
    ("fight/scene.rs", Fight),
    ("fight/side.rs", Fight),
    ("fight/sim/mod.rs", Fight),
    ("fight/sim/setup.rs", Fight),
    ("fight/sim/br1.rs", Fight),
    ("fight/sim/br1_tests.rs", Fight),
    ("fight/sim/frame.rs", Fight),
    ("fight/sim/ko_replay.rs", Fight),
    ("fight/stage/mod.rs", Fight),
    ("fight/stage/anim.rs", Fight),
    ("fight/team/mod.rs", Fight),
    ("fight/team/arcade.rs", Fight),
    ("fight/team/br1.rs", Fight),
    ("fight/team/br1_tests.rs", Fight),
    ("fight/team/frame.rs", Fight),
    ("fight/team/hud.rs", Fight),
    ("fight/team/round.rs", Fight),
    ("fight/team/target.rs", Fight),
    ("fight/team/tests.rs", Fight),
    ("fight/team/watch.rs", Fight),
    ("fight/throw.rs", Fight),
    ("fight/version.rs", Fight),
    ("frontend/mod.rs", Frontend),
    ("frontend/screen.rs", Frontend),
    ("frontend/tick.rs", Frontend),
    ("frontend/draw.rs", Frontend),
    ("frontend/check.rs", Frontend),
    ("frontend/title.rs", Frontend),
    ("frontend/select.rs", Frontend),
    ("frontend/vs.rs", Frontend),
    ("frontend/team.rs", Frontend),
    ("frontend/team_arcade.rs", Frontend),
    ("frontend/movie.rs", Frontend),
    ("frontend/progress.rs", Frontend),
    ("frontend/story_flow.rs", Frontend),
    ("frontend/menus/mod.rs", Frontend),
    ("frontend/menus/actions.rs", Frontend),
    ("frontend/menus/controller.rs", Frontend),
    ("frontend/menus/draw.rs", Frontend),
    ("frontend/menus/flow.rs", Frontend),
    ("frontend/serial.rs", Frontend),
    ("frontend/resident.rs", Frontend),
    ("frontend/memcard.rs", Frontend),
    ("frontend/options.rs", Frontend),
    ("frontend/ranking.rs", Frontend),
    ("frontend/story_screens.rs", Frontend),
    ("session/mod.rs", Session),
    ("session/start.rs", Session),
    ("session/state.rs", Session),
    ("session/step.rs", Session),
    ("session/snapshot.rs", Session),
    ("session/hash.rs", Session),
    ("session/tests.rs", Session),
    ("session/replay/mod.rs", Session),
    ("session/replay/inputs.rs", Session),
    ("session/replay/sync.rs", Session),
    ("session/replay/recorder.rs", Session),
    ("session/replay/player.rs", Session),
    ("session/replay/headless.rs", Session),
    ("session/replay/tests.rs", Session),
    ("session/debug_history.rs", Session),
    ("session/determinism_trace.rs", Session),
    ("session/fight.rs", Session),
    ("session/rollback.rs", Session),
    ("present/mod.rs", Present),
    ("present/fight_scene/mod.rs", Present),
    ("present/fight_scene/scene.rs", Present),
    ("present/fight_scene/arena.rs", Present),
    ("present/fight_scene/stage_cull.rs", Present),
    ("present/fight_scene/stage_draw.rs", Present),
    ("present/fight_scene/debug_hud.rs", Present),
    ("present/fight_scene/team.rs", Present),
    ("present/front_vram.rs", Present),
    ("present/backdrop.rs", Present),
    ("present/br1_fighter.rs", Present),
    ("present/br1_fx.rs", Present),
    ("present/br1_props.rs", Present),
    ("present/br1_stage_bg.rs", Present),
    ("present/drive_grid.rs", Present),
    ("present/fighter_lighting.rs", Present),
    ("render/mod.rs", Render),
    ("render/pipelines.rs", Render),
    ("render/models.rs", Render),
    ("render/lightmapped.rs", Render),
    ("render/instances.rs", Render),
    ("render/frame.rs", Render),
    ("render/targets.rs", Render),
    ("render/gpu_timer.rs", Render),
    ("render/atlas_cache.rs", Render),
    ("render/offscreen.rs", Render),
    ("audio/mod.rs", Audio),
    ("audio/mixer.rs", Audio),
    ("net/mod.rs", Net),
    ("net/connect.rs", Net),
    ("net/handshake.rs", Net),
    ("net/link.rs", Net),
    ("net/proto.rs", Net),
    ("net/udp.rs", Net),
    ("net/webrtc.rs", Net),
    ("app/mod.rs", App),
    ("app/events.rs", App),
    ("app/stepping.rs", App),
    ("app/requests.rs", App),
    ("app/keys.rs", App),
    ("app/overlay.rs", App),
    ("app/draw.rs", App),
    ("app/capture.rs", App),
    ("app/debug_env.rs", App),
    ("app/devices.rs", App),
    ("app/gamepads.rs", App),
    ("app/pad_layout.rs", App),
    ("app/netplay.rs", App),
    ("app/perf.rs", App),
    ("app/playback.rs", App),
    ("app/replay_menu.rs", App),
    ("app/net_menu.rs", App),
    ("app/replay_store.rs", App),
    ("app/replays.rs", App),
    ("app/tools.rs", App),
    ("app/web_files.rs", App),
    ("app/android.rs", App),
    ("app/touch.rs", App),
    ("app/video.rs", App),
    ("app/video_ffmpeg.rs", App),
    ("app/setup/mod.rs", App),
    ("app/setup/screen.rs", App),
    ("app/setup/draw.rs", App),
    ("app/setup/imports.rs", App),
    ("app/setup/job.rs", App),
    ("app/setup/pickers.rs", App),
    ("app/setup/convert.rs", App),
    ("app/setup/plan.rs", App),
    ("app/setup/q3.rs", App),
    ("app/setup/web.rs", App),
    ("app/setup/worker.rs", App),
];

/// Files whose items go to more than one directory (stage 2 / 5), for the report's header.
const MIXED: &[(&str, &str)] = &[("lib.rs", "re-exports: now / perf_now / clock_seed (platform/clock.rs), the app's run …")];

/// The crate root's items (`lib.rs`): where they live (the re-exports of `app`, the clock functions).
const ROOT_ITEMS: &[(&str, Dir)] =
    &[("Config", App), ("run", App), ("session_start", App), ("web_main", App), ("now", Platform), ("perf_now", Platform), ("clock_seed", Platform)];

/// The items of `session::start` that `session` re-exports: `frontend` may use them (rule 1), by either path.
const SESSION_START: &[&str] = &[
    "SessionStart",
    "Entry",
    "FightDebug",
    "CustomTest",
    "FileCache",
    "Cached",
    "browser_build",
    "SIM_VERSION",
    "sim_version_env",
    "sim_version_now",
    "parse_sim_version",
];

/// The forbidden edges that stay for now: (source file, path as written, what removes it). One entry covers the
/// path's uses in test code too.
const EXCEPTIONS: &[(&str, &str, &str)] = &[];

/// The L2 files' platform names (rule 2) outside test code that stay: (file, name, why). `std::fs` in test code is
/// allowed.
const EXTERNAL_EXCEPTIONS: &[(&str, &str, &str)] = &[
    ("session/hash.rs", "std::fs", "rule 2's listed exception: the debug-only file output (HashTrace::from_env, OKB_STATEHASH)"),
    ("session/rollback.rs", "std::fs", "rule 2's listed exception: the debug-only file output (the rollback test's log and dumps)"),
];

/// External names L2 must not use (rule 2), and the ones that are only listed (env reads, hash maps: rules 2 / 5).
/// (`std::fs` and `std::env` are found by path.)
const L2_FORBIDDEN: &[&str] = &["wgpu", "winit", "cpal", "gilrs", "web_sys", "js_sys"];
const L2_LISTED: &[(&str, &str)] = &[("HashMap", "HashMap (rule 5: no iteration order)"), ("HashSet", "HashSet (rule 5: no iteration order)")];

/// The paths of a use tree / an expression path starting at `s` (after `prefix::`): `a::b`, `a::{b, c::d}`, `*`,
/// `x as y`. Each path is its segments; `self` inside braces stands for the path so far.
fn expand(s: &str) -> Vec<Vec<String>> {
    fn skip_ws(b: &[u8], mut i: usize) -> usize {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    }
    fn tree(b: &[u8], mut i: usize, prefix: &[String], out: &mut Vec<Vec<String>>) -> usize {
        i = skip_ws(b, i);
        if b.get(i) == Some(&b'{') {
            i += 1;
            loop {
                i = skip_ws(b, i);
                match b.get(i) {
                    None => return i,
                    Some(b'}') => return i + 1,
                    Some(b',') => i += 1,
                    _ => {
                        let j = tree(b, i, prefix, out);
                        if j == i {
                            return i; // not a use tree
                        }
                        i = j;
                    }
                }
            }
        }
        if b.get(i) == Some(&b'*') {
            out.push(prefix.to_vec());
            return i + 1;
        }
        let start = i;
        while i < b.len() && is_ident(b[i]) {
            i += 1;
        }
        if i == start {
            if !prefix.is_empty() {
                out.push(prefix.to_vec());
            }
            return i;
        }
        let seg = std::str::from_utf8(&b[start..i]).expect("ASCII ident").to_string();
        let mut path = prefix.to_vec();
        if seg != "self" {
            path.push(seg);
        }
        let j = skip_ws(b, i);
        if b[j..].starts_with(b"::") && b.get(skip_ws(b, j + 2)) != Some(&b'<') {
            return tree(b, j + 2, &path, out);
        }
        out.push(path);
        // `x as y`
        if b[j..].starts_with(b"as") && b.get(j + 2).is_some_and(|c| c.is_ascii_whitespace()) {
            let k = skip_ws(b, j + 2);
            let mut e = k;
            while e < b.len() && is_ident(b[e]) {
                e += 1;
            }
            return e;
        }
        i
    }
    let mut out = Vec::new();
    tree(s.as_bytes(), 0, &[], &mut out);
    out
}

/// One file's module path (`session/replay.rs` → `session::replay`, `output/mod.rs` → `output`).
fn module_of(rel: &str) -> Vec<String> {
    let rel = rel.strip_suffix(".rs").unwrap_or(rel);
    let mut v: Vec<String> = rel.split('/').map(str::to_string).collect();
    if v.last().is_some_and(|l| l == "mod" || l == "lib") {
        v.pop();
    }
    v
}

/// A reference from a file to a module path, at a line.
struct Edge {
    file: &'static str,
    line: usize,
    test: bool,
    from: Dir,
    to: Dir,
    /// `crate::a::b` as written after resolving `super`
    path: String,
    /// the module part of `path` (a `FILES` module) and the item after it
    module: String,
    item: String,
}

/// The module (from the crate root) that the code at byte `at` of file module `me` is in: `me` plus the inline modules
/// around `at`, outermost first.
fn module_at(me: &[String], mods: &[InlineMod], at: usize) -> Vec<String> {
    let mut m = me.to_vec();
    let mut inside: Vec<&InlineMod> = mods.iter().filter(|x| x.open < at && at < x.close).collect();
    inside.sort_by_key(|x| x.open);
    m.extend(inside.iter().map(|x| x.name.clone()));
    m
}

struct Scan {
    edges: Vec<Edge>,
    /// L2 files' external names: (file, line, what, forbidden, in test code)
    external: Vec<(&'static str, usize, String, bool, bool)>,
    unmapped: Vec<String>,
    scanned: usize,
}

fn scan(src_dir: &Path) -> Scan {
    let mut all = Vec::new();
    collect(src_dir, src_dir, &mut all);
    let modules: BTreeMap<Vec<String>, (&'static str, Dir)> = FILES.iter().map(|&(f, d)| (module_of(f), (f, d))).collect();
    let mut out = Scan { edges: Vec::new(), external: Vec::new(), unmapped: Vec::new(), scanned: 0 };
    for rel in &all {
        if rel == "lib.rs" || rel == "main.rs" || rel.starts_with("bin/") {
            continue;
        }
        let Some(&(file, from)) = FILES.iter().find(|(f, _)| f == rel) else {
            out.unmapped.push(rel.clone());
            continue;
        };
        out.scanned += 1;
        let text = std::fs::read_to_string(src_dir.join(rel)).expect("read a source file");
        let code = strip_comments(&text);
        let mods = inline_mods(&structure(&text));
        let me = module_of(rel);
        let whole_test = test_file(src_dir, rel);
        // a `mod.rs`'s child modules, named without a prefix
        let children: Vec<String> = if rel.ends_with("mod.rs") {
            modules.keys().filter(|m| m.len() == me.len() + 1 && m.starts_with(&me)).map(|m| m[me.len()].clone()).collect()
        } else {
            Vec::new()
        };
        let b = code.as_bytes();
        let starts: Vec<usize> = std::iter::once(0).chain(b.iter().enumerate().filter(|(_, c)| **c == b'\n').map(|(i, _)| i + 1)).collect();
        let mut seen = BTreeSet::new();
        let mut i = 0;
        while i < b.len() {
            if !is_ident(b[i]) || ident_before(b, i) || (i >= 2 && &b[i - 2..i] == b"::") {
                i += 1;
                continue;
            }
            let start = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            let word = &code[start..i];
            let line = line_of(&starts, start);
            let test = whole_test || mods.iter().any(|m| m.test && m.open < start && start < m.close);
            let after = {
                let mut j = i;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                j
            };
            // L2: the platform crates, std::fs / std::env (`use std::{env, fs}` too), hash maps
            if from.layer() == 2 {
                let t = if test { " (test)" } else { "" };
                if L2_FORBIDDEN.contains(&word) {
                    out.external.push((file, line, format!("{word}{t}"), true, test));
                }
                if let Some((_, what)) = L2_LISTED.iter().find(|(x, _)| *x == word) {
                    out.external.push((file, line, format!("{what}{t}"), false, test));
                }
                if word == "std" && code[after..].starts_with("::") {
                    for p in expand(&code[after + 2..]) {
                        match p.first().map(String::as_str) {
                            Some("fs") => out.external.push((file, line, format!("std::fs{t}"), true, test)),
                            Some("env") => out.external.push((file, line, format!("std::env (rule 2: env reads only through base::trace){t}"), false, test)),
                            _ => {}
                        }
                    }
                }
            }
            if !code[after..].starts_with("::") {
                continue;
            }
            let base: Vec<String> = match word {
                "crate" => Vec::new(),
                "super" => {
                    // super::super::…, from the inline module the path is in
                    let mut m = module_at(&me, &mods, start);
                    m.pop();
                    let mut k = after + 2;
                    while code[k..].trim_start().starts_with("super::") {
                        let skipped = code[k..].len() - code[k..].trim_start().len();
                        k += skipped + "super::".len();
                        m.pop();
                    }
                    let paths = expand(&code[k..]);
                    push_edges(&mut out.edges, &mut seen, &modules, file, from, line, test, &m, paths);
                    continue;
                }
                w if children.iter().any(|c| c == w) => {
                    let mut m = me.clone();
                    m.push(w.to_string());
                    m
                }
                _ => continue,
            };
            let paths = expand(&code[after + 2..]);
            push_edges(&mut out.edges, &mut seen, &modules, file, from, line, test, &base, paths);
        }
    }
    out
}

/// The edges of `paths` (relative to module `base`) from `file`, one per (path, test) and file.
fn push_edges(
    edges: &mut Vec<Edge>,
    seen: &mut BTreeSet<(String, bool)>,
    modules: &BTreeMap<Vec<String>, (&'static str, Dir)>,
    file: &'static str,
    from: Dir,
    line: usize,
    test: bool,
    base: &[String],
    paths: Vec<Vec<String>>,
) {
    for p in paths {
        let full: Vec<String> = base.iter().cloned().chain(p).collect();
        // the longest module prefix; the crate root's items by name
        let (module, to) = match (1..=full.len()).rev().find_map(|n| modules.get(&full[..n]).map(|&(_, d)| (n, d))) {
            Some((n, d)) => (n, d),
            None => match full.first().and_then(|f| ROOT_ITEMS.iter().find(|(r, _)| r == f)) {
                Some(&(_, d)) => (0, d),
                None => continue, // a std / extern path through `crate::` re-exports, a macro …
            },
        };
        let path = format!("crate::{}", full.join("::"));
        if !seen.insert((path.clone(), test)) {
            continue;
        }
        edges.push(Edge { file, line, test, from, to, path, module: full[..module].join("::"), item: full.get(module).cloned().unwrap_or_default() });
    }
}

// ---------------------------------------------------------------------------------------------- the rules

/// None when the target rules allow the edge, else the rule it breaks.
fn violation(e: &Edge) -> Option<&'static str> {
    let (from, to) = (e.from, e.to);
    if from == to {
        return None;
    }
    if from.layer() == 2 && to == Platform {
        return Some("rule 2: L2 names platform");
    }
    if matches!(from, Render | Audio) && to.layer() >= 2 {
        return Some("rule 3: render / audio use only L0-L1");
    }
    if to.layer() > from.layer() {
        return Some("rule 1: a lower layer uses a higher one");
    }
    if from.layer() == 2 && to.layer() == 2 && to.l2_rank() > from.l2_rank() {
        if from == Frontend && to == Session && (e.module == "session::start" || e.module == "session" && SESSION_START.contains(&e.item.as_str())) {
            return None; // session::start
        }
        return Some("rule 1: L2 order session → frontend → fight");
    }
    None
}

/// What removes a forbidden edge (`EXCEPTIONS`).
fn exception(e: &Edge) -> Option<&'static str> {
    exception_in(EXCEPTIONS, e)
}

fn exception_in(exceptions: &[(&str, &str, &'static str)], e: &Edge) -> Option<&'static str> {
    exceptions.iter().find(|(file, path, _)| *file == e.file && *path == e.path).map(|x| x.2)
}

/// Why an L2 file may name a platform crate or `std::fs` there (`EXTERNAL_EXCEPTIONS`; test code may).
fn external_exception(file: &str, what: &str, test: bool) -> Option<&'static str> {
    if test {
        return Some("test code");
    }
    EXTERNAL_EXCEPTIONS.iter().find(|(f, w, _)| *f == file && *w == what).map(|x| x.2)
}

fn report(s: &Scan) -> String {
    let mut r = String::new();
    let _ = writeln!(r, "layering report (ENFORCED, ARCHITECTURE.md, Layers): {} files scanned", s.scanned);
    let _ = writeln!(r, "\nfiles split later (counted as the directory that keeps the file):");
    for (f, what) in MIXED {
        let _ = writeln!(r, "  {f:24} {what}");
    }
    if !s.unmapped.is_empty() {
        let _ = writeln!(r, "\nfiles not in the table (add them to FILES): {}", s.unmapped.join(", "));
    }
    // directory matrix: references (distinct paths) and files
    let mut matrix: BTreeMap<(Dir, Dir), (usize, BTreeSet<&str>, usize)> = BTreeMap::new();
    for e in s.edges.iter().filter(|e| e.from != e.to) {
        let m = matrix.entry((e.from, e.to)).or_default();
        m.0 += 1;
        m.1.insert(e.file);
        if violation(e).is_some() {
            m.2 += 1;
        }
    }
    let _ = writeln!(r, "\ncross-directory edges (from → to: distinct paths in files; L = layer; ✗ = paths the target rules forbid):");
    for ((from, to), (n, files, bad)) in &matrix {
        let mark = if *bad > 0 { format!("  ✗ {bad}") } else { String::new() };
        let _ = writeln!(r, "  L{} {:9} → L{} {:9} {n:4} in {:2} files{mark}", from.layer(), from.name(), to.layer(), to.name(), files.len());
    }
    // the violations by rule and directory pair
    let mut bad: Vec<(&Edge, &str)> = s.edges.iter().filter_map(|e| violation(e).map(|v| (e, v))).collect();
    bad.sort_by_key(|(e, v)| (*v, e.from, e.to, e.file, e.line));
    let listed = bad.iter().filter(|(e, _)| exception(e).is_some()).count();
    let tests = bad.iter().filter(|(e, _)| e.test).count();
    let _ =
        writeln!(r, "\nedges the target rules forbid: {} ({listed} listed exceptions, {} not listed; {tests} in test code):", bad.len(), bad.len() - listed);
    let mut last = ("", Base, Base);
    for (e, v) in &bad {
        if (*v, e.from, e.to) != last {
            let _ = writeln!(r, "  {v}: {} → {}", e.from.name(), e.to.name());
            last = (v, e.from, e.to);
        }
        let _ = writeln!(
            r,
            "    src/{}:{} {}{}{}",
            e.file,
            e.line,
            e.path,
            if e.test { " (test)" } else { "" },
            exception(e).map_or(String::from("  [NOT LISTED]"), |f| format!("  [{f}]"))
        );
    }
    let external: BTreeSet<_> = s.external.iter().collect();
    let forbidden: Vec<_> = external.iter().filter(|x| x.3).collect();
    let _ = writeln!(r, "\nL2 files naming platform crates or std::fs (rule 2): {}", forbidden.len());
    for (f, l, what, _, test) in &forbidden {
        let why = external_exception(f, what.trim_end_matches(" (test)"), *test).unwrap_or("NOT LISTED");
        let _ = writeln!(r, "    src/{f}:{l} {what}  [{why}]");
    }
    let env: Vec<_> = external.iter().filter(|x| !x.3).collect();
    let _ = writeln!(r, "\nL2 env reads and hash maps (listed, rules 2 / 5): {}", env.len());
    for (f, l, what, _, _) in &env {
        let _ = writeln!(r, "    src/{f}:{l} {what}");
    }
    r
}

/// What the enforcing test fails on, one line each: forbidden edges and L2 platform names without an exception,
/// exceptions that match nothing, files missing from `FILES`.
fn failures(s: &Scan) -> Vec<String> {
    failures_with(s, EXCEPTIONS)
}

/// `failures` with `exceptions` for `EXCEPTIONS` (the self-test's own list).
fn failures_with(s: &Scan, exceptions: &[(&str, &str, &'static str)]) -> Vec<String> {
    let mut out = Vec::new();
    for e in s.edges.iter().filter(|e| exception_in(exceptions, e).is_none()) {
        if let Some(v) = violation(e) {
            out.push(format!("src/{}:{} {}: {v} (fix it, or add it to EXCEPTIONS with the stage that removes it)", e.file, e.line, e.path));
        }
    }
    for (file, path, _) in exceptions {
        if !s.edges.iter().any(|e| e.file == *file && e.path == *path && violation(e).is_some()) {
            out.push(format!("EXCEPTIONS: {file} {path} is no forbidden edge any more: remove the entry"));
        }
    }
    for (f, l, what, forbidden, test) in &s.external {
        if *forbidden && external_exception(f, what.trim_end_matches(" (test)"), *test).is_none() {
            out.push(format!("src/{f}:{l} {what}: rule 2, an L2 file names a platform crate or std::fs (fix it, or add it to EXTERNAL_EXCEPTIONS)"));
        }
    }
    for (file, what, _) in EXTERNAL_EXCEPTIONS {
        if !s.external.iter().any(|(f, _, w, forbidden, test)| f == file && w == what && *forbidden && !*test) {
            out.push(format!("EXTERNAL_EXCEPTIONS: {file} {what} is not used outside test code any more: remove the entry"));
        }
    }
    out.extend(s.unmapped.iter().map(|f| format!("src/{f}: not in FILES (add it with its directory)")));
    out
}

#[test]
fn layering_enforced() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let s = scan(&src);
    assert!(s.scanned > 0, "no source files under {}", src.display());
    let r = report(&s);
    println!("{r}");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("layering_report.txt");
    if let Err(e) = std::fs::write(&out, &r) {
        eprintln!("layering report: {}: {e}", out.display());
    }
    let f = failures(&s);
    assert!(f.is_empty(), "layering (ARCHITECTURE.md, Layers): {} finding(s)\n{}", f.len(), f.join("\n"));
}

#[test]
fn enforcement_flags_unlisted_and_stale() {
    let edge = |file: &'static str, from: Dir, to: Dir, path: &str, module: &str, item: &str| Edge {
        file,
        line: 1,
        test: false,
        from,
        to,
        path: path.to_string(),
        module: module.to_string(),
        item: item.to_string(),
    };
    // the test's exception list (`EXCEPTIONS` is empty since stage 5k)
    let ex: &[(&str, &str, &str)] = &[("frontend/data.rs", "crate::audio::Audio", "the self-test")];
    // every exception matched, plus one allowed edge: no finding
    let mut edges: Vec<Edge> = ex
        .iter()
        .map(|&(file, path, _)| {
            let from = FILES.iter().find(|(f, _)| *f == file).expect("an exception's file is in FILES").1;
            let to = if path.starts_with("crate::audio") {
                Audio
            } else if path.starts_with("crate::session") {
                Session
            } else if path.starts_with("crate::platform") {
                Platform
            } else if path.starts_with("crate::formats") {
                Formats
            } else {
                Fight
            };
            edge(file, from, to, path, "", path.rsplit("::").next().unwrap_or(""))
        })
        .collect();
    edges.push(edge("fight/sim/mod.rs", Fight, Base, "crate::base::portable", "base::portable", ""));
    let external = EXTERNAL_EXCEPTIONS.iter().map(|&(f, w, _)| (f, 1, w.to_string(), true, false)).collect();
    let mut s = Scan { edges, external, unmapped: Vec::new(), scanned: 1 };
    assert_eq!(failures_with(&s, ex), Vec::<String>::new());
    // an unlisted forbidden edge, a file outside FILES, std::fs in test code (allowed)
    s.edges.push(edge("fight/sim/mod.rs", Fight, Render, "crate::render::Renderer", "render", "Renderer"));
    s.unmapped.push("fight/new.rs".to_string());
    s.external.push(("fight/physics.rs", 2, "std::fs (test)".to_string(), true, true));
    let f = failures_with(&s, ex);
    assert_eq!(f.len(), 2, "{f:?}");
    assert!(f[0].contains("crate::render::Renderer") && f[1].contains("fight/new.rs"));
    // a stale exception and a stale external exception
    s.edges.retain(|e| e.path != ex[0].1);
    s.external.retain(|x| x.0 != EXTERNAL_EXCEPTIONS[0].0 || x.4);
    let f = failures_with(&s, ex);
    assert!(f.iter().any(|l| l.starts_with("EXCEPTIONS:") && l.contains(ex[0].1)), "{f:?}");
    assert!(f.iter().any(|l| l.starts_with("EXTERNAL_EXCEPTIONS:") && l.contains(EXTERNAL_EXCEPTIONS[0].0)), "{f:?}");
}

#[test]
fn use_trees_expand() {
    let p = |s: &str| expand(s).into_iter().map(|v| v.join("::")).collect::<Vec<_>>();
    assert_eq!(p("a::b::c;"), ["a::b::c"]);
    assert_eq!(p("{a, b::{self, c}, d::*};"), ["a", "b", "b::c", "d"]);
    assert_eq!(p("{\n    x as y,\n    z::w,\n};"), ["x", "z::w"]);
    assert_eq!(p("debug::is_muted() && x"), ["debug::is_muted"]);
    assert_eq!(p("rollback::Rollback::<T>::from_env(\"X\")"), ["rollback::Rollback"]);
}

#[test]
fn comments_are_blanked_strings_kept() {
    let s = "a // crate::x\n/* crate::y /* nested */ */ \"crate::z // no\" r#\"crate::w\"# '\"' '\\'' b // crate::v";
    let c = strip_comments(s);
    assert!(!c.contains("crate::x") && !c.contains("crate::y"));
    assert!(c.contains("\"crate::z // no\"") && c.contains("crate::w") && c.contains("'\"' '\\'' b") && !c.contains("crate::v"));
    assert_eq!(c.lines().count(), s.lines().count());
}

#[test]
fn raw_byte_and_c_strings() {
    // a raw byte string with a quote inside, and a raw C string ending in a backslash: the comments after them
    let s = "let a = br#\"x \" y\"#; // crate::x\nlet b = cr\"\\\"; // crate::y\nlet c = b\"q\"; crate::z";
    let c = strip_comments(s);
    assert!(!c.contains("crate::x") && !c.contains("crate::y"), "{c}");
    assert!(c.contains("br#\"x \" y\"#") && c.contains("cr\"\\\"") && c.contains("crate::z"));
    // an identifier ending in `br` / `cr` before a string is not a raw string prefix
    assert!(strip_comments("abr\"\\\"\" // crate::v").contains("abr\"\\\"\"") && !strip_comments("abr\"\\\"\" // crate::v").contains("crate::v"));
    // `structure` blanks what is inside the literals (their braces), not the quotes
    assert_eq!(structure("f(\"{\", '}', br#\"{\"#) { '\\'' }"), "f(\" \", ' ', br#\" \"#) { '  ' }");
}

#[test]
fn inline_modules_and_super() {
    let src = "mod a {\n    mod b {\n        use super::X;\n    }\n    fn f() { let s = \"}\"; }\n}\n#[cfg(test)]\nmod tests;\nfn g() {}\n\
               #[cfg(all(test, not(target_arch = \"wasm32\")))]\n#[allow(dead_code)]\npub(crate) mod t2 {\n    use super::Y;\n}\nfn h() {}\n";
    let mods = inline_mods(&structure(src));
    let names: Vec<(&str, bool)> = mods.iter().map(|m| (m.name.as_str(), m.test)).collect();
    // `mod tests;` is a file module: not inline, and not test code here
    assert_eq!(names, [("a", false), ("b", false), ("t2", true)]);
    let me = vec!["session".to_string(), "debug".to_string()];
    let at = |pat: &str| src.find(pat).expect("in the source");
    assert_eq!(module_at(&me, &mods, at("super::X")).join("::"), "session::debug::a::b");
    assert_eq!(module_at(&me, &mods, at("fn f")).join("::"), "session::debug::a");
    assert_eq!(module_at(&me, &mods, at("super::Y")).join("::"), "session::debug::t2");
    // the string's brace does not close `a`: `fn g` is outside every module, `fn h` after the test module's end
    assert_eq!(module_at(&me, &mods, at("fn g")).join("::"), "session::debug");
    let test_at = |pat: &str| mods.iter().any(|m| m.test && m.open < at(pat) && at(pat) < m.close);
    assert!(test_at("super::Y") && !test_at("fn h") && !test_at("fn g") && !test_at("super::X"));
}

#[test]
fn test_files() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(test_file(&src, "session/tests.rs"));
    assert!(!test_file(&src, "session/start.rs") && !test_file(&src, "session/mod.rs") && !test_file(&src, "base/hash.rs"));
}

#[test]
fn cfg_test_attributes() {
    assert!(cfg_test_before("x;\n#[cfg(test)]\n"));
    assert!(cfg_test_before("#[cfg( all(test, feature = \"x\") )]\n#[allow(unused)]\npub(in crate::a) "));
    assert!(!cfg_test_before("#[cfg(not(test))]\n"));
    assert!(!cfg_test_before("#[cfg(test)]\nfn f() {}\n"));
    assert!(!cfg_test_before("let x = f(a)"));
}
