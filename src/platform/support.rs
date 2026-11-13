//! Support and debug tools: the logger, crash reports, the support report, the overlay text.
//!
//! Engine addition.
//!
//! Support and debug tools: a logger that keeps the last lines in memory, crash reports, the support report
//! (F8), the build stamp and the on-screen overlay text (F1).
//!
//! Keys (desktop and web): F1 overlay · F2 / F3 leave P2 / P1 at 1 HP · F4 round timer to 1 s · F6 full beast gauges ·
//! F7 screenshot · F8 support report · F9 pause · F10 fast forward ×4 · F11 unlock everything (movies, pictures too) · `.` single step
//! while paused · `` ` `` performance overlay (`OKB_PERF=1` / `?perf=1` at start). Env / URL: `OKB_LOG=debug`
//! (`?log=debug`), `OKB_DEBUG=1` (`?debug=1`, `--debug`) shows the overlay at start and turns on the debug tools
//! (`app::tools`, `TOOLS_HELP`).

use std::collections::VecDeque;
use std::sync::Mutex;

pub use crate::base::build::VERSION;
use crate::base::trace::is_muted;

static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const KEEP: usize = 400;

struct Logger;

static LOGGER: Logger = Logger;
/// chosen level of our crate (log::LevelFilter as usize)
static LEVEL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(3);

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        // our crate (the library and the `okbeast` binary) at the chosen level, the desktop WebRTC stack's crates (`webrtc`, `rtc`, `rtc_*`: their ICE and
        // DTLS warn of routine things at every connect) at error, everything else (wgpu, winit, matchbox, …) at warn
        const WEBRTC: [&str; 2] = ["webrtc", "rtc"];
        let t = m.target();
        let krate = t.split("::").next().unwrap_or(t);
        let max = if krate == env!("CARGO_CRATE_NAME") || krate == "okbeast" {
            LEVEL.load(std::sync::atomic::Ordering::Relaxed)
        } else if WEBRTC.iter().any(|c| krate.strip_prefix(c).is_some_and(|r| r.is_empty() || r.starts_with('_'))) {
            log::Level::Error as usize
        } else {
            log::Level::Warn as usize
        };
        (m.level() as usize) <= max
    }

    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) || is_muted() {
            return;
        }
        let line = format!("{:9.3} {:5} {}: {}", uptime(), r.level(), r.target(), r.args());
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        eprintln!("{line}");
        #[cfg(target_os = "android")]
        android_log(r.level(), &line);
        #[cfg(target_arch = "wasm32")]
        {
            let v = wasm_bindgen::JsValue::from_str(&line);
            match r.level() {
                log::Level::Error => web_sys::console::error_1(&v),
                log::Level::Warn => web_sys::console::warn_1(&v),
                _ => web_sys::console::log_1(&v),
            }
        }
        if let Ok(mut l) = LINES.lock() {
            if l.len() >= KEEP {
                l.pop_front();
            }
            l.push_back(line);
        }
    }

    fn flush(&self) {}
}

/// A log line to logcat (tag `okb`; Android has no stderr for apps).
#[cfg(target_os = "android")]
#[allow(unsafe_code)] // Android: the liblog FFI below (SAFETY note at the call)
fn android_log(level: log::Level, line: &str) {
    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32;
    }
    // ANDROID_LOG_DEBUG 3, INFO 4, WARN 5, ERROR 6
    let prio = match level {
        log::Level::Error => 6,
        log::Level::Warn => 5,
        log::Level::Info => 4,
        _ => 3,
    };
    let text = std::ffi::CString::new(line.replace('\0', " ")).unwrap_or_default();
    // SAFETY: both strings are NUL-terminated and live across the call
    unsafe {
        __android_log_write(prio, c"okb".as_ptr(), text.as_ptr());
    }
}

/// Where the crash and support reports go: the working directory (desktop), the save's folder (Android; made here,
/// a panic may come before anything was saved).
#[cfg(not(target_arch = "wasm32"))]
fn report_path(name: String) -> String {
    #[cfg(target_os = "android")]
    let name = {
        let p = crate::platform::storage::debug_path(&name);
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        p.display().to_string()
    };
    name
}

/// Seconds since the logger started.
fn uptime() -> f64 {
    static START: Mutex<Option<f64>> = Mutex::new(None);
    let now = crate::now();
    let mut s = START.lock().unwrap_or_else(|e| e.into_inner());
    now - *s.get_or_insert(now)
}

/// Install the logger (`level`: error / warn / info / debug / trace) and the crash handler.
pub fn init(level: &str) {
    let level: log::LevelFilter = level.parse().unwrap_or(log::LevelFilter::Info);
    LEVEL.store(level as usize, std::sync::atomic::Ordering::Relaxed);
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level.max(log::LevelFilter::Warn));
    uptime();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = format!("PANIC: {info}\n\n{}", report("crash"));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = report_path(format!("okb_crash_{}.txt", stamp()));
            if std::fs::write(&path, &text).is_ok() {
                eprintln!("crash report written to {path}");
            }
        }
        #[cfg(target_arch = "wasm32")]
        web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&text));
        prev(info);
    }));
    log::info!("OpenKumiteBeast {VERSION} on {}", platform());
}

/// The kept log lines, oldest first.
pub fn recent() -> Vec<String> {
    LINES.lock().map(|l| l.iter().cloned().collect()).unwrap_or_default()
}

pub fn platform() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let ua = web_sys::window().and_then(|w| w.navigator().user_agent().ok()).unwrap_or_default();
        format!("web ({ua})")
    }
    #[cfg(not(target_arch = "wasm32"))]
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// A timestamp for file names (seconds since 1970).
pub fn stamp() -> u64 {
    crate::now() as u64
}

/// The support report: build, platform, `state` (the game's own description) and the log tail.
pub fn report(state: &str) -> String {
    let mut s =
        format!("OpenKumiteBeast {VERSION}\nplatform: {}\nuptime: {:.1} s\n\n== state ==\n{state}\n\n== log (last {KEEP} lines) ==\n", platform(), uptime());
    for l in recent() {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

/// Write the report (desktop: `br2_report_<time>.txt` in the working directory; web: the browser console)
/// and return where it went.
pub fn save_report(state: &str) -> String {
    let text = report(state);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = report_path(format!("okb_report_{}.txt", stamp()));
        match std::fs::write(&path, &text) {
            Ok(()) => path,
            Err(e) => format!("(could not write {path}: {e})"),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&text));
        "the browser console".to_string()
    }
}

pub const HELP: &str =
    "F1 overlay  F2/F3 1HP P2/P1  F4 time 1s  F6 gauge  F7 shot  F8 report  F9 pause  . step  F10 fast  F11 unlock  F5 aspect WIDE/4:3  ` perf";

/// The debug tools' keys (`OKB_DEBUG=1` / `--debug` / `?debug=1`; `app::tools`), added to the overlay's help.
pub const TOOLS_HELP: &str = "P pause (every screen)  . step  F12 save slot  Shift+F12 load  Shift+F1..F4 slot 1..4  hold Backspace rewind (Shift x2)";

/// The process's resident memory in bytes: `task_info(MACH_TASK_BASIC_INFO)` on macOS (declared here, no crate),
/// `VmRSS` of `/proc/self/status` on Linux and Android, None elsewhere.
#[allow(unsafe_code)] // macOS: the task_info FFI below (SAFETY note at the call)
pub fn rss() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        // struct mach_task_basic_info (MACH_TASK_BASIC_INFO = 20, count = 12 natural_t)
        #[repr(C)]
        #[derive(Default)]
        struct Info {
            virtual_size: u64,
            resident_size: u64,
            resident_size_max: u64,
            user_time: [i32; 2],
            system_time: [i32; 2],
            policy: i32,
            suspend_count: i32,
        }
        unsafe extern "C" {
            static mach_task_self_: u32;
            fn task_info(task: u32, flavor: i32, info: *mut Info, count: *mut u32) -> i32;
        }
        let mut info = Info::default();
        let mut count = (std::mem::size_of::<Info>() / 4) as u32;
        // SAFETY: `info` is a writable mach_task_basic_info of `count` natural_t words, as the call expects;
        // mach_task_self_ is the task port the system sets up before main
        let kr = unsafe { task_info(mach_task_self_, 20, &mut info, &mut count) };
        (kr == 0).then_some(info.resident_size)
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // VmRSS is in kB whatever the page size
        let s = std::fs::read_to_string("/proc/self/status").ok()?;
        let kb = s.lines().find_map(|l| l.strip_prefix("VmRSS:"))?.split_whitespace().next()?.parse::<u64>().ok()?;
        Some(kb * 1024)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_has_build_state_and_log() {
        super::init("debug");
        log::info!("hello from the test");
        let r = super::report("STATE LINE");
        assert!(r.contains(super::VERSION));
        assert!(r.contains("STATE LINE"));
        assert!(r.contains("hello from the test"));
        let path = super::save_report("STATE LINE");
        assert!(std::fs::read_to_string(&path).unwrap().contains("hello from the test"));
        let _ = std::fs::remove_file(path);
    }
}
