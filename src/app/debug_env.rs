//! The app's debug env hooks, read once at start (desktop only).
//!
//! Engine addition.
//!
//! The app's debug env hooks (engine additions), read once at start on the desktop; all off in the browser but
//! `?nointro=1`. The
//! ones that set up the session go into its `SessionStart` (`DebugEnv::start`).

use crate::session::{Entry, FightDebug, SessionStart};

#[derive(Default)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct DebugEnv {
    /// OKB_SCRIPT: scripted pad 1 of the fight (same syntax as the `sim` binary)
    pub script: Vec<u16>,
    /// OKB_SCRIPT2: scripted pad 2 of the fight (with OKB_P2=human)
    pub script2: Vec<u16>,
    /// OKB_SCRIPT3 / OKB_SCRIPT4 (engine addition, the team battle): scripted pads 3 / 4 of the team battle (with
    /// OKB_TEAM_HUMANS or joined on its select)
    pub script3: Vec<u16>,
    pub script4: Vec<u16>,
    /// OKB_FESCRIPT .. OKB_FESCRIPT4: scripted pads 1 .. 4 of the front end (S = START, H = L1; pads 3 / 4: the team
    /// battle's screens)
    pub fescript: [Vec<u16>; 4],
    /// OKB_P2DMG=d@frame[,d@frame…]: P2's damage set to d at those fight frames (`Cheat::P2Dmg`)
    pub p2dmg: Vec<(i16, u64)>,
    /// OKB_FORCEMOVE=frame:move: P1's move request at that fight frame (`Cheat::ForceMove`; the effect
    /// debug line logged from that frame on)
    pub forcemove: Option<(u64, i16)>,
    /// OKB_BREAK=side: that wall broken at fight frame 330 (`Cheat::Break`)
    pub brk: Option<usize>,
    /// OKB_FAST=n: n fight steps per frame until close to a OKB_SHOTS frame (16 when n does not parse)
    pub fast: Option<i32>,
    /// OKB_FEFAST=n: n front-end steps per frame while the next OKB_FESHOTS frame is far (`App::fe_fast`; 16 when
    /// n does not parse)
    pub fefast: Option<i32>,
    /// OKB_SHOTS=n,m,…: fight frames to save
    pub shots: Vec<u64>,
    /// OKB_FESHOTS=n,m,…: front-end frames to save (counted from the start)
    pub feshots: Vec<u32>,
    /// OKB_FEEXIT: quit 5 frames after the last OKB_FESHOTS frame
    pub feexit: bool,
    /// OKB_VRAMDUMP=path: the front end's VRAM written there on every upload
    pub vramdump: Option<String>,
    /// OKB_LOCKSTEP: exactly one 60 Hz step per frame that reached the screen, whatever the clock says
    /// (reproducible OKB_SHOTS / OKB_FESHOTS runs; an occluded window waits)
    pub lockstep: bool,
    /// OKB_MUTE: no audio output
    pub mute: bool,
    /// OKB_WINDOW=WxH: the window's inner size in physical pixels (`WxHpt`: in logical points, i.e. × the
    /// display's scale factor — the HiDPI case)
    pub window: Option<WindowSize>,
    /// OKB_WINDOWPOS=x,y: the window's position in logical points (two netplay windows side by side)
    pub pos: Option<(i32, i32)>,
    /// OKB_SEED=n: the front end's random seed instead of the clock (reproducible screenshot runs)
    pub seed: Option<u64>,
    /// OKB_FESCRIPT, OKB_SCRIPT or OKB_NOINTRO set: the front end starts at the title, not the logos
    pub nointro: bool,
    /// OKB_NAMETEST=mode:value (`SessionStart::entry`)
    pub nametest: Option<(i32, i32)>,
    /// OKB_EPILOGUE=c (`SessionStart::entry`)
    pub epilogue: Option<usize>,
    /// OKB_TEAM_ARCADE=n (engine addition, TEAM ARCADE A4; `SessionStart::entry`): a TEAM ARCADE run at stage n's loading
    /// screen; the humans' fighters P1 / P3 of OKB_TEAM_CHARS, the partner pad 2 with OKB_TEAM_HUMANS >= 2
    pub team_arcade: Option<(u8, [u8; 2], bool)>,
    /// BR2_MCFAULT=name: a forced memory card result
    pub mcfault: Option<crate::frontend::memcard::Fault>,
    /// OKB_P2, OKB_WATCH, OKB_MODEL / OKB_CFLAGS / OKB_PRACTICE, OKB_CONT, OKB_GAUGE, OKB_P2DMG=d
    pub fight: FightDebug,
    /// OKB_DEBUGKEYS=frame:key,…: the debug tools' keys pressed at those drawn frames (`tools::parse_keys`; with
    /// OKB_DEBUG only)
    pub debugkeys: Vec<(u64, super::tools::Action)>,
    /// OKB_RECORD=path: the whole session's recording written there as a replay file at exit
    pub record: Option<String>,
    /// OKB_SIM_VERSION=n: the session runs simulation version n (an older one's logic, as its replays; 1..=
    /// `session::SIM_VERSION`, anything else is warned about and ignored)
    pub sim_version: Option<u32>,
    /// OKB_UISCRIPT: scripted pad 1 of the replay list and the playback controls, by their own 60 Hz frames
    /// (`App::ui_frame`)
    pub uiscript: Vec<u16>,
    /// OKB_UISHOTS=n,m,…: those UI frames saved as `okb_ui_<n>.png`; OKB_UIEXIT: quit 5 UI frames after the last
    pub uishots: Vec<u64>,
    pub uiexit: bool,
    /// OKB_NOPLAYUI: no playback overlay (a playback's frames as the recording's)
    pub noplayui: bool,
    /// OKB_NETSCRIPT: this side's scripted pad in netplay, by session frame (`netplay`)
    pub netscript: Vec<u16>,
    /// OKB_NETSHOTS=n,m,…: netplay session frames saved as `okb_net_<n>.png` (the fight or the front end)
    pub netshots: Vec<u32>,
    /// OKB_NETCODE=CODE: the NETWORK menu's HOST takes this room code instead of a random one (scripted joins)
    #[cfg_attr(not(feature = "netplay-webrtc"), allow(dead_code))]
    pub netcode: Option<String>,
    /// OKB_NETBIND=ADDR: a direct-IP HOST of the NETWORK menu listens on this address (e.g. 127.0.0.1) instead of
    /// every interface, and shows it as its address
    pub netbind: Option<std::net::IpAddr>,
    /// OKB_NETEXIT=n: n UI frames after a netplay session of the NETWORK menu ended and the menu is back, that frame
    /// saved as `okb_net_end.png` and the app quits (the end of scripted netplay runs)
    pub netexit: Option<u64>,
    /// OKB_SETUP_FROM=PATH: the setup screen takes PATH as dropped onto the window at UI frame 30 and installs it at
    /// frame 60 (`setup`); OKB_SETUP_SCAN=dirs: the folders its scan looks in (a path list; empty: none);
    /// OKB_SETUP_SLOW=ms: a pause after each copied file (screenshots of the progress)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub setup_from: Option<std::path::PathBuf>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub setup_scan: Option<String>,
    /// OKB_SETUP_BR1=PATH: the setup screen adds this disc image or converted folder of the imported game (after OKB_SETUP_FROM's
    /// install at start, or over the game from OPTIONS → CHANGE GAME DATA)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub setup_br1: Option<std::path::PathBuf>,
    /// OKB_SETUP_Q3=PATH (engine addition): the same for QUAKE III ARENA MAPS (Quake III Arena's baseq3 folder, the game's
    /// folder, a .pk3 or a converted folder; after OKB_SETUP_BR1's when both are set)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub setup_q3: Option<std::path::PathBuf>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub setup_slow: u64,
    /// OKB_TEAM=2v2|ffa (engine addition, debug; the menus' TEAM BATTLE is the way in since stage T4): the app starts a
    /// team battle
    /// instead of the title, with OKB_TEAM_CHARS=P1,P2,P3,P4 (codes, default YUG,ALI,BAK,ULI), OKB_TEAM_STAGE (default
    /// ST0, one of `fight::stage::TEAM_STAGES`) and OKB_TEAM_HUMANS=n (pads 1..n human, the others CPUs; default 1)
    pub team: Option<TeamDebug>,
}

/// OKB_TEAM's battle (`DebugEnv::team`).
#[derive(Clone, Debug)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) struct TeamDebug {
    pub mode: crate::fight::team::TeamMode,
    pub chars: [String; 4],
    pub stage: String,
    pub humans: usize,
}

impl TeamDebug {
    /// The battle the OKB_TEAM* values ask for (None: OKB_TEAM unset or not 2v2 / ffa).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn parse(mode: Option<String>, chars: Option<String>, stage: Option<String>, humans: Option<String>) -> Option<Self> {
        let mode = match mode?.to_ascii_lowercase().as_str() {
            "2v2" => crate::fight::team::TeamMode::Vs2,
            "ffa" => crate::fight::team::TeamMode::Ffa,
            m => {
                log::warn!("OKB_TEAM={m}: 2v2 or ffa; ignored");
                return None;
            }
        };
        let mut c = ["YUG", "ALI", "BAK", "ULI"].map(String::from);
        for (k, v) in chars.unwrap_or_default().split(',').map(|v| v.trim().to_ascii_uppercase()).filter(|v| !v.is_empty()).take(4).enumerate() {
            c[k] = v;
        }
        let humans = humans.and_then(|v| v.parse().ok()).unwrap_or(1usize).min(4);
        Some(TeamDebug { mode, chars: c, stage: stage.unwrap_or_else(|| "ST0".into()).to_ascii_uppercase(), humans })
    }

    /// Fighter `k`'s costume: the next one for each earlier fighter of the same character (the automatic change of a
    /// duplicate).
    pub fn costumes(&self) -> [u8; 4] {
        std::array::from_fn(|k| (self.chars[..k].iter().filter(|c| **c == self.chars[k]).count() % 4) as u8)
    }
}

/// A OKB_WINDOW size.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) enum WindowSize {
    Physical(u32, u32),
    Logical(u32, u32),
}

impl WindowSize {
    /// `WxH` (physical pixels) or `WxHpt` (logical points).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn parse(s: &str) -> Option<Self> {
        let (s, logical) = match s.strip_suffix("pt") {
            Some(s) => (s, true),
            None => (s, false),
        };
        let (w, h) = s.split_once(['x', 'X'])?;
        let (w, h) = (w.trim().parse::<u32>().ok().filter(|&w| w > 0)?, h.trim().parse::<u32>().ok().filter(|&h| h > 0)?);
        Some(if logical { WindowSize::Logical(w, h) } else { WindowSize::Physical(w, h) })
    }
}

/// A debug hook's value by its name (`OKB_<k>`; an earlier build's `BR2_<k>` too: `base::env`): the environment's;
/// on Android (no environment) the launch intent's `okb_env` extra (`platform::android::debug_var`:
/// `adb shell am start … --es okb_env 'OKB_SETUP_FROM=…;OKB_MUTE=1'`).
#[cfg(not(target_arch = "wasm32"))]
fn get(k: &str) -> Option<String> {
    let k = k.strip_prefix(crate::base::env::PREFIX).unwrap_or(k);
    #[cfg(target_os = "android")]
    if let Some((_, v)) = crate::base::env::pick(k, crate::platform::android::debug_var) {
        return Some(v);
    }
    crate::base::env::var(k).ok()
}

impl DebugEnv {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read() -> Self {
        let var = |k: &str| get(k);
        let script = |k: &str| var(k).map(|s| crate::fight::input_script::parse_script(&s)).unwrap_or_default();
        fn list<T: std::str::FromStr>(k: &str) -> Vec<T> {
            get(k).map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default()
        }
        DebugEnv {
            script: script("OKB_SCRIPT"),
            script2: script("OKB_SCRIPT2"),
            script3: script("OKB_SCRIPT3"),
            script4: script("OKB_SCRIPT4"),
            fescript: ["OKB_FESCRIPT", "OKB_FESCRIPT2", "OKB_FESCRIPT3", "OKB_FESCRIPT4"].map(script),
            p2dmg: var("OKB_P2DMG")
                .map(|v| v.split(',').filter_map(|e| e.split_once('@').and_then(|(d, f)| Some((d.parse::<i16>().ok()?, f.parse::<u64>().ok()?)))).collect())
                .unwrap_or_default(),
            forcemove: var("OKB_FORCEMOVE").and_then(|v| v.split_once(':').and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<i16>().ok()?)))),
            brk: var("OKB_BREAK").and_then(|v| v.parse::<usize>().ok()),
            fast: var("OKB_FAST").map(|v| v.parse().unwrap_or(16).max(1)),
            fefast: var("OKB_FEFAST").map(|v| v.parse().unwrap_or(16).max(1)),
            shots: list("OKB_SHOTS"),
            feshots: list("OKB_FESHOTS"),
            feexit: var("OKB_FEEXIT").is_some(),
            vramdump: var("OKB_VRAMDUMP"),
            lockstep: var("OKB_LOCKSTEP").is_some(),
            mute: var("OKB_MUTE").is_some(),
            window: var("OKB_WINDOW").and_then(|v| WindowSize::parse(&v)),
            pos: var("OKB_WINDOWPOS").and_then(|v| v.split_once(',').and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))),
            seed: var("OKB_SEED").and_then(|v| v.parse().ok()),
            nointro: ["OKB_FESCRIPT", "OKB_SCRIPT", "OKB_NOINTRO"].iter().any(|k| var(k).is_some()),
            nametest: var("OKB_NAMETEST").map(|v| v.split_once(':').map(|(a, b)| (a.parse().unwrap_or(0), b.parse().unwrap_or(7323))).unwrap_or((0, 7323))),
            epilogue: var("OKB_EPILOGUE").and_then(|v| v.parse().ok()),
            team_arcade: var("OKB_TEAM_ARCADE").and_then(|v| v.parse::<u8>().ok()).map(|stage| {
                // 1..=TEAM_ARCADE_MAX_STAGES, the range a replay's header takes (`replay::check_header`; the ladder clamps it to its own length)
                let stage = stage.clamp(1, crate::frontend::TEAM_ARCADE_MAX_STAGES);
                let t = TeamDebug::parse(Some("2v2".into()), var("OKB_TEAM_CHARS"), None, var("OKB_TEAM_HUMANS"));
                let code = |k: usize| t.as_ref().and_then(|t| crate::assets::roster::CHARACTERS.iter().position(|c| *c == t.chars[k]));
                (stage, [code(0).unwrap_or(0) as u8, code(2).unwrap_or(6) as u8], t.is_some_and(|t| t.humans >= 2))
            }),
            mcfault: var("OKB_MCFAULT").and_then(|v| crate::frontend::memcard::Fault::parse(&v)),
            fight: FightDebug {
                p2_human: var("OKB_P2").is_some_and(|v| v == "human"),
                watch: var("OKB_WATCH").is_some(),
                custom: var("OKB_MODEL").and_then(|v| v.parse().ok()).map(|model| crate::session::CustomTest {
                    model,
                    flags: var("OKB_CFLAGS").and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0),
                    practice: var("OKB_PRACTICE").is_some(),
                }),
                cont: var("OKB_CONT").is_some(),
                gauge: var("OKB_GAUGE").and_then(|v| v.parse().ok()),
                p2dmg: var("OKB_P2DMG").and_then(|v| v.parse().ok()),
            },
            debugkeys: var("OKB_DEBUGKEYS").map(|v| super::tools::parse_keys(&v)).unwrap_or_default(),
            record: var("OKB_RECORD").filter(|v| !v.is_empty()),
            sim_version: var("OKB_SIM_VERSION")
                .and_then(|v| crate::session::parse_sim_version("OKB_SIM_VERSION", &v).inspect_err(|e| log::warn!("{e}: ignored")).ok()),
            uiscript: script("OKB_UISCRIPT"),
            uishots: list("OKB_UISHOTS"),
            uiexit: var("OKB_UIEXIT").is_some(),
            noplayui: var("OKB_NOPLAYUI").is_some(),
            netscript: script("OKB_NETSCRIPT"),
            netshots: list("OKB_NETSHOTS"),
            netcode: var("OKB_NETCODE").filter(|v| !v.is_empty()),
            netexit: var("OKB_NETEXIT").and_then(|v| v.parse().ok()),
            setup_from: var("OKB_SETUP_FROM").filter(|v| !v.is_empty()).map(std::path::PathBuf::from),
            setup_scan: var("OKB_SETUP_SCAN"),
            setup_br1: var("OKB_SETUP_BR1").filter(|v| !v.is_empty()).map(std::path::PathBuf::from),
            setup_q3: var("OKB_SETUP_Q3").filter(|v| !v.is_empty()).map(std::path::PathBuf::from),
            setup_slow: var("OKB_SETUP_SLOW").and_then(|v| v.parse().ok()).unwrap_or(0),
            netbind: var("OKB_NETBIND").and_then(|v| v.parse().inspect_err(|_| log::warn!("OKB_NETBIND={v}: not an IP address: ignored")).ok()),
            team: TeamDebug::parse(var("OKB_TEAM"), var("OKB_TEAM_CHARS"), var("OKB_TEAM_STAGE"), var("OKB_TEAM_HUMANS")),
        }
    }

    /// The browser: only `?nointro=1` (the title at once, as OKB_NOINTRO: scripted browser runs).
    #[cfg(target_arch = "wasm32")]
    pub fn read() -> Self {
        let q = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
        DebugEnv { nointro: q.trim_start_matches('?').split('&').any(|kv| kv == "nointro=1"), ..DebugEnv::default() }
    }

    /// The session's start: `direct` = the --fight match (p1, p2, stage), `locked` = the game's own progression
    /// (`SessionStart::locked`), `extras` = the extra stages whose files the game data has (`assets::extras::present`), `br1` = the
    /// content id of the imported game's install (`assets::manifest::br1_install_id`), the seed from OKB_SEED or the clock, the stored save, the boot screen and the debug
    /// setups of these hooks. A NAME ENTRY test wins over an epilogue test, an epilogue test over a TEAM ARCADE start.
    pub fn start(&self, direct: Option<(String, String, String)>, locked: bool, extras: u8, br1: Option<u32>) -> SessionStart {
        let intro = !self.nointro;
        let entry = match (direct, self.nametest, self.epilogue, self.team_arcade) {
            (Some((p1, p2, stage)), ..) => Entry::DirectFight { p1, p2, stage },
            (None, Some((mode, value)), ..) => Entry::NameTest { intro, mode, value },
            (None, None, Some(chr), _) => Entry::Epilogue { intro, chr },
            (None, None, None, Some((stage, chr, partner))) => Entry::TeamArcade { stage, chr, partner },
            (None, None, None, None) if intro => Entry::Intro,
            (None, None, None, None) => Entry::Title,
        };
        let mut s = SessionStart {
            sim_version: self.sim_version.unwrap_or(crate::session::SIM_VERSION),
            seed: self.seed.unwrap_or_else(crate::clock_seed),
            card: crate::platform::storage::read_card(),
            unlock: false,
            locked,
            entry,
            card_fault: self.mcfault,
            fight: self.fight,
            browser: crate::session::browser_build(),
            network: network_build(),
            // the GRAPHICS rows of an always-full-screen app (engine addition)
            android: cfg!(target_os = "android"),
            extra_stage: false,
            extra_q3dm6: false,
            br1,
            // the game data's, set by the app once its start read it (`App::content_found`)
            content: None,
            netplay: false,
        };
        s.set_extras(extras);
        s
    }
}

/// This build can play over a network: UDP everywhere but the browser, rooms with `netplay-webrtc` (the mode menu's
/// NETWORK, `SessionStart::network`).
pub(super) fn network_build() -> bool {
    super::net_menu::DIRECT || super::net_menu::ROOMS
}

#[cfg(test)]
mod tests {
    use super::WindowSize;

    #[test]
    fn window_size_parse() {
        assert_eq!(WindowSize::parse("1280x720"), Some(WindowSize::Physical(1280, 720)));
        assert_eq!(WindowSize::parse("640X360pt"), Some(WindowSize::Logical(640, 360)));
        assert_eq!(WindowSize::parse("0x720"), None);
        assert_eq!(WindowSize::parse("1280"), None);
        assert_eq!(WindowSize::parse("axb"), None);
    }
}
