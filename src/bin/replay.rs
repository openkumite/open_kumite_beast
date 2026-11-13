//! Headless replay check: re-simulates replay files and compares every checkpoint.
//!
//! Headless replay check: `replay ../okb_export FILE.okbr…` re-simulates each file as the app plays it back
//! (`session::replay::simulate`), compares every checkpoint and prints a summary per file; exit 0 when every file
//! plays in sync, 1 otherwise (2: usage).
//! `replay --trace ../okb_export FILE` (a replay or a `sim` / `fetest` STEPLOG step log): only the state hash after
//! every step on stdout (`frame hash`, as the STATEHASH trace), to diff across targets (game-state stage 7). On
//! wasm32 the same trace is the exported `trace_file(file, assets_url)`.
//! `replay --video OUT [options] [EXPORT_DIR] FILE` (desktop): the replay drawn and mixed without a window into a
//! video file through the system's ffmpeg (`open_kumite_beast::render_video`, `docs/replay_video.md`).

/// `replay --video`'s usage.
#[cfg(not(target_arch = "wasm32"))]
const VIDEO_USAGE: &str = "replay --video OUT.mp4 [--size 4k|1440p|1080p|720p|WxH] [--aspect 4:3|wide] [--codec h264|ffv1]
               [--bitrate MBIT/S] [--ffmpeg PATH] [EXPORT_DIR] FILE.okbr
  the whole replay as a 60 fps video with its sound (default 4k = 3840x2160, at most 8192 a side and
  8192x4320 in all; the stored SCREEN MODE; H.264 + AAC, --bitrate up to 1000; ffv1: lossless, into a .mkv);
  EXPORT_DIR defaults to ../okb_export";

/// `replay --video …` (the arguments after `--video`): the video made, or the exit code.
#[cfg(not(target_arch = "wasm32"))]
fn video(args: &[String]) -> i32 {
    use open_kumite_beast::frontend::options::Aspect;
    use open_kumite_beast::{VideoCodec, VideoOptions};
    let usage = |why: &str| {
        eprintln!("replay --video: {why}\nusage: {VIDEO_USAGE}");
        2
    };
    let mut o = VideoOptions {
        export: "../okb_export".into(),
        replay: Default::default(),
        out: Default::default(),
        size: (3840, 2160),
        aspect: None,
        codec: VideoCodec::H264,
        bitrate: None,
        ffmpeg: None,
    };
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned().ok_or(format!("{a} needs a value"));
        let r = match a.as_str() {
            "--size" => value().and_then(|v| {
                o.size = match v.to_ascii_lowercase().as_str() {
                    "4k" | "2160p" => (3840, 2160),
                    "1440p" => (2560, 1440),
                    "1080p" => (1920, 1080),
                    "720p" => (1280, 720),
                    s => s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))).ok_or(format!("--size {v}?"))?,
                };
                Ok(())
            }),
            "--aspect" => value().and_then(|v| {
                o.aspect = Some(match v.as_str() {
                    "4:3" | "43" | "4/3" => Aspect::Four3,
                    "wide" | "WIDE" => Aspect::Wide,
                    _ => return Err(format!("--aspect {v}? (4:3 or wide)")),
                });
                Ok(())
            }),
            "--codec" => value().and_then(|v| {
                o.codec = match v.as_str() {
                    "h264" => VideoCodec::H264,
                    "ffv1" => VideoCodec::Ffv1,
                    _ => return Err(format!("--codec {v}? (h264 or ffv1)")),
                };
                Ok(())
            }),
            "--bitrate" => value().and_then(|v| {
                // Mbit/s, above 0 and at most 1000
                let m: f64 = v.parse().ok().filter(|m: &f64| *m > 0.0 && *m <= 1000.0).ok_or(format!("--bitrate {v}? (0 to 1000 Mbit/s)"))?;
                o.bitrate = Some(((m * 1e6) as u64).max(1));
                Ok(())
            }),
            "--ffmpeg" => value().map(|v| o.ffmpeg = Some(v.into())),
            f if f.starts_with("--") => Err(format!("unknown option {f}")),
            f => {
                files.push(f.to_string());
                Ok(())
            }
        };
        if let Err(e) = r {
            return usage(&e);
        }
    }
    // OUT [EXPORT_DIR] FILE: the arguments that are not options, in order
    match files.as_slice() {
        [] => return usage("no output file"),
        [out, f] => (o.out, o.replay) = (out.into(), f.into()),
        [out, e, f] => (o.out, o.export, o.replay) = (out.into(), e.clone(), f.into()),
        _ => return usage("an output file, then one replay file (after the export folder, if given)"),
    }
    match open_kumite_beast::render_video(&o, true) {
        Ok(s) => {
            println!(
                "{}: {} frames ({:.2} s at 60 fps), {} sound samples at {} Hz, {} {}x{}, made in {:.1} s ({:.1} fps)",
                o.out.display(),
                s.frames,
                s.frames as f64 / 60.0,
                s.samples,
                open_kumite_beast::VIDEO_RATE,
                s.encoder,
                o.size.0,
                o.size.1,
                s.secs,
                s.frames as f64 / s.secs.max(1e-6)
            );
            0
        }
        Err(e) => {
            eprintln!("replay --video: {e}");
            1
        }
    }
}

/// The trace of a replay or a step log: the lines `frame hash`, one per step, up to the end or the error.
async fn trace(assets: &open_kumite_beast::assets::Assets, file: &[u8]) -> (String, Result<(), String>) {
    use open_kumite_beast::session::{determinism_trace, hash, replay};
    let mut out = String::new();
    let mut put = |frame: u32, h: u64| {
        out += &determinism_trace::line(frame, h);
        out.push('\n');
    };
    let r = if determinism_trace::is_step_log(file) {
        match determinism_trace::StepLog::decode(file) {
            Ok(l) => determinism_trace::run(&l, assets, put).await.map(|_| ()),
            Err(e) => Err(e),
        }
    } else {
        match replay::Replay::decode(file) {
            Ok(r) => replay::simulate_async(assets, r, |s| put(s.frame, hash(s))).await.map(|_| ()),
            Err(e) => Err(e),
        }
    };
    (out, r)
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    use open_kumite_beast::platform::storage::read_capped;
    use open_kumite_beast::session::replay::{MAX_BYTES, Replay, simulate};
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // warnings on stderr (a replay of other game data: `replay::content_warning`)
    open_kumite_beast::platform::support::init(&open_kumite_beast::base::env::var("LOG").unwrap_or("warn".into()));
    if args.first().is_some_and(|a| a == "--video") {
        std::process::exit(video(&args[1..]));
    }
    let tracing = args.first().is_some_and(|a| a == "--trace");
    if tracing {
        args.remove(0);
    }
    if args.len() < 2 || tracing && args.len() != 2 {
        eprintln!("usage: replay EXPORT_DIR FILE.okbr…\n       replay --trace EXPORT_DIR FILE\n       {VIDEO_USAGE}");
        std::process::exit(2);
    }
    let assets = open_kumite_beast::assets::Assets::new(args[0].clone());
    if tracing {
        let (t, r) = match read_capped(args[1].as_ref(), MAX_BYTES) {
            Ok(b) => pollster::block_on(trace(&assets, &b)),
            Err(e) => (String::new(), Err(e)),
        };
        print!("{t}");
        if let Err(e) = r {
            eprintln!("{}: FAIL: {e}", args[1]);
            std::process::exit(1);
        }
        return;
    }
    let mut bad = 0;
    for path in &args[1..] {
        let t = std::time::Instant::now();
        let r = match read_capped(path.as_ref(), MAX_BYTES).and_then(|b| Replay::decode(&b).map(|r| (b.len(), r))) {
            Ok(x) => x,
            Err(e) => {
                println!("{path}: FAIL: {e}");
                bad += 1;
                continue;
            }
        };
        let (bytes, r) = r;
        let h = &r.header;
        let what = match &h.spec {
            Some(m) => format!(
                "{} {} vs {} stage {} {:?} rounds {}-{}",
                h.mode,
                open_kumite_beast::assets::roster::code(m.chr[0]),
                open_kumite_beast::assets::roster::code(m.chr[1]),
                m.stage,
                h.outcome,
                h.rounds[0],
                h.rounds[1]
            ),
            None => format!("{:?}", h.start.entry),
        };
        println!(
            "{path}: {what}; engine {} v{}, {} steps (watched from {}), {} checkpoints, {bytes} bytes",
            r.engine,
            r.sim_version,
            h.frames,
            h.from,
            r.body.checks.len()
        );
        match simulate(&assets, r, |_| {}) {
            Ok(s) => println!(
                "{path}: PASS: {} steps, {} checkpoints matched ({}), final hash {:016x}, {:.1} s",
                s.frames,
                s.checks,
                if s.strict { "full state + sync" } else { "sync only: another build" },
                s.hash,
                t.elapsed().as_secs_f64()
            ),
            Err(e) => {
                println!("{path}: FAIL: {e}");
                bad += 1;
            }
        }
    }
    std::process::exit(if bad > 0 { 1 } else { 0 });
}

/// The wasm `replay --trace`: the trace of `file` (a replay or a step log) with the data under `assets_url` (the
/// export folder's URL, ending in `/`), and a last line `FAIL: why` when it stopped early.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub async fn trace_file(file: Vec<u8>, assets_url: String) -> String {
    let (t, r) = trace(&open_kumite_beast::assets::Assets::new(assets_url), &file).await;
    match r {
        Ok(()) => t,
        Err(e) => format!("{t}FAIL: {e}\n"),
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
