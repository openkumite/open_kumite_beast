//! The setup screen: missing or unusable game data, and how the player gives the game its data (engine addition).
//!
//! When the quick check (`assets::manifest`) finds the game data folder unusable at start, or the start cannot read
//! it, the app shows this screen instead of the game: only the renderer exists, drawing untextured quads
//! (`draw.rs`), nothing of the session has started. `screen.rs` is the pages' state machine; `pickers.rs` the
//! desktop's ways to name a folder (the scan list, BROWSE; drag and drop and the typed path come through the window's
//! events here); `job.rs` checks the source (on a thread) and copies it in on a thread; `plan.rs` what both copy;
//! `web.rs` the browser's folder input and copy into its private storage (OPFS), under the same names. Once installed
//! the app goes on to its normal start (`App::start_up`) without a restart. Opened from the game (OPTIONS → CHANGE
//! GAME DATA, simulation v9) it stands over the waiting session, BACK TO THE GAME closes it, and a new install
//! restarts the app (the loaded data would be stale). An install at start ends on a page offering the optional
//! disc of the imported game and Quake III Arena's maps (`Page::Done`; their installs `Kind::Br1`, `Kind::Q3`: `job` /
//! `convert` / `q3` / `web` into their own folders, `platform::game_data::Dirs::sub`; their state here an `Extra`):
//! START THE GAME reads them as the start does. Opened from the game, an install or a removal of any of them restarts
//! the app once it has ended (the session's start is what offers them).
//! Debug hooks: OKB_SETUP_FROM=PATH drops PATH onto the window at UI frame 30 and installs it at frame 60 (or once
//! checked), then starts the game a second after the done page; OKB_SETUP_BR1=PATH adds PATH as the imported game's
//! disc wherever ADD BLOODY ROAR 1 DISC is offered (after that install, or over the game; a failed check or install
//! ends it, and a OKB_SETUP_FROM run quits); OKB_SETUP_Q3=PATH the same for QUAKE III ARENA MAPS (after the imported
//! game's disc's, when both are set); OKB_SETUP_SCAN=dirs (the scan's folders), OKB_SETUP_SLOW=ms (a pause
//! per copied file); OKB_UISCRIPT / OKB_UISHOTS / OKB_UIEXIT drive and shoot it by its UI frames.

#[cfg(all(feature = "converter", not(target_arch = "wasm32")))]
mod convert;
mod draw;
#[cfg(feature = "converter")]
mod imports;
#[cfg(not(target_arch = "wasm32"))]
mod job;
#[cfg(not(target_arch = "wasm32"))]
mod pickers;
mod plan;
#[cfg(all(feature = "converter", not(target_arch = "wasm32")))]
mod q3;
mod screen;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(all(target_arch = "wasm32", feature = "converter"))]
mod worker;
#[cfg(target_arch = "wasm32")]
use web::{self as job, self as pickers, Dirs};
#[cfg(all(target_arch = "wasm32", feature = "converter"))]
pub(super) use worker::worker_main;

use super::{App, UserEvent};
use crate::assets::extras::Q3Verdict;
use crate::assets::manifest::Br1Verdict;
use crate::frontend::Pads;
#[cfg(not(target_arch = "wasm32"))]
use crate::platform::game_data::Dirs;
use crate::render;
pub(super) use screen::Act;
use screen::{Cmd, Kind, Page};
use std::path::PathBuf;
use winit::event::{ElementState, KeyEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};

/// An optional install's state on the screen (`Kind::EXTRAS`): its folders, its debug hook, its check and removal
/// under way (`V`: its quick check's verdict).
struct Extra<V> {
    /// its folders (`Dirs::sub`; the browser: its next OPFS folder)
    dirs: Dirs,
    /// its debug hook (OKB_SETUP_BR1, OKB_SETUP_Q3)
    hook: Hook,
    /// the browser: its quick check under way (the verdict lands here)
    #[cfg(target_arch = "wasm32")]
    check: Option<std::rc::Rc<std::cell::RefCell<Option<V>>>>,
    /// the browser: its install's end waits for that check (the job page till then)
    #[cfg(target_arch = "wasm32")]
    installed: bool,
    /// a REMOVE under way (its answer lands here)
    #[cfg(target_arch = "wasm32")]
    remove: Option<std::rc::Rc<std::cell::RefCell<Option<Result<(), String>>>>>,
    /// a REMOVE under way on a thread (a slow disk does not stop the window)
    #[cfg(not(target_arch = "wasm32"))]
    remove: Option<std::thread::JoinHandle<Result<(), String>>>,
    #[cfg(not(target_arch = "wasm32"))]
    _verdict: std::marker::PhantomData<V>,
}

/// A debug hook adding an optional install (module header).
#[derive(Default)]
struct Hook {
    /// the source to add
    from: Option<PathBuf>,
    /// its install was started: its end (installed, or the error page) ends the hook
    started: bool,
}

impl<V> Extra<V> {
    fn new(dirs: Dirs, from: Option<PathBuf>) -> Self {
        Self {
            dirs,
            hook: Hook { from, started: false },
            #[cfg(target_arch = "wasm32")]
            check: None,
            #[cfg(target_arch = "wasm32")]
            installed: false,
            remove: None,
            #[cfg(not(target_arch = "wasm32"))]
            _verdict: std::marker::PhantomData,
        }
    }
}

/// The screen, its job and its pickers.
pub(super) struct Run {
    ui: screen::Setup,
    dirs: Dirs,
    /// The imported game's disc's install
    br1: Extra<Br1Verdict>,
    /// Quake III Arena's maps' install (engine addition, the extra stages)
    q3: Extra<Q3Verdict>,
    job: Option<job::Job>,
    /// the source being copied (TRY AGAIN after a failed copy)
    src: Option<PathBuf>,
    browse: Option<pickers::Browse>,
    scan: Option<std::thread::JoinHandle<(Vec<PathBuf>, Vec<PathBuf>)>>,
    /// the source being checked (`job::inspect` on a thread: a slow drive does not stop the window)
    check: Option<(PathBuf, std::thread::JoinHandle<Result<screen::Found, String>>)>,
    /// the rows' rectangles as last drawn (clicks)
    hits: Vec<draw::Hit>,
    /// a finger on the screen (touch screens: taps and drags)
    drag: screen::Drag,
    mouse: [f32; 2],
    pads: Pads,
    frames: u64,
    /// debug OKB_SETUP_FROM / OKB_SETUP_SLOW; `scripted`: OKB_SETUP_FROM drove an install at start
    from: Option<PathBuf>,
    scripted: bool,
    /// the UI frame the done page was last entered (OKB_SETUP_FROM goes on from it a second later: a screenshot)
    done_since: Option<u64>,
    slow_ms: u64,
    /// opened from the game's OPTIONS: the session waits under it
    in_game: bool,
}

impl App {
    /// OPTIONS → CHANGE GAME DATA (`Request::ChangeData`): the setup screen over the waiting session. OPTIONS values
    /// changed before it and not yet left with EXIT (the menu's scratch copy) are not kept by an install's (or a
    /// removal's) restart.
    pub(super) fn change_data(&mut self) {
        if !self.cfg.as_ref().is_some_and(|c| c.own_data) {
            self.toast = Some(("this run reads --assets: change the data there".into(), 240));
            return;
        }
        self.enter_setup("Change the game data or the optional data: the game restarts after each install or removal.".into(), Act::Close);
    }

    /// The setup screen instead of the game (`reason`: why; `leave`: QUIT at start, BACK TO THE GAME over a
    /// session); the renderer is made first when there is none (`UserEvent::SetupReady`).
    pub(super) fn enter_setup(&mut self, reason: String, leave: Act) {
        log::warn!("setup: {reason}");
        let in_game = leave == Act::Close;
        // no key held or latched before reaches the screen (a press during the start's check would choose row 0), and
        // over the game the keys' releases go to the screen: nothing may stay held for the session
        (self.keys, self.keys2, self.latch) = (0, 0, [0; 2]);
        let dirs = Dirs::app();
        let over = self.env.setup_scan.clone();
        let skip = dirs.root.clone();
        #[cfg(not(target_arch = "wasm32"))]
        let scan = std::thread::Builder::new().name("okb setup scan".into()).spawn(move || pickers::scan(over.as_deref(), &skip)).ok();
        // the browser: no scan, no QUIT (the page stays)
        #[cfg(target_arch = "wasm32")]
        let scan = {
            let _ = (over, skip);
            None
        };
        let leave = (cfg!(not(target_arch = "wasm32")) || leave == Act::Close).then_some(leave);
        let br1 = Extra::new(extra_dirs(&dirs, Kind::Br1), self.env.setup_br1.clone());
        let q3 = Extra::new(extra_dirs(&dirs, Kind::Q3), self.env.setup_q3.clone());
        self.setup = Some(Box::new(Run {
            ui: screen::Setup { convert: cfg!(feature = "converter"), ..screen::Setup::new(reason, pickers::can_browse(), leave) },
            dirs,
            br1,
            q3,
            job: None,
            src: None,
            browse: None,
            scan,
            check: None,
            hits: Vec::new(),
            drag: screen::Drag::default(),
            mouse: [0.0; 2],
            pads: Pads::default(),
            frames: 0,
            from: self.env.setup_from.clone(),
            scripted: self.env.setup_from.is_some(),
            done_since: None,
            slow_ms: self.env.setup_slow,
            in_game,
        }));
        // over the game (its data usable): the optional installs can be added, replaced or removed
        if in_game && let Some(run) = &mut self.setup {
            run.check_extras();
        }
        if self.renderer.is_some() {
            return;
        }
        let Some(window) = self.window.clone() else { return };
        let proxy = self.proxy.clone();
        let fut = async move {
            let ev = match render::Renderer::new(window).await {
                Ok(r) => UserEvent::SetupReady(Box::new(r)),
                Err(e) => UserEvent::Failed(e),
            };
            let _ = proxy.send_event(ev);
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(fut);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(fut);
    }

    /// The renderer for the setup screen is there.
    pub(super) fn setup_ready(&mut self, mut r: render::Renderer) {
        if let Some((w, h)) = self.surface_size() {
            r.resize(w, h);
        }
        r.set_vsync(true);
        self.renderer = Some(r);
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// A key on the setup screen: the path field takes text while the cursor is on it; arrows, W / S, Enter, K,
    /// Escape, Backspace work the rows.
    pub(super) fn setup_key(&mut self, ev: &KeyEvent, el: &ActiveEventLoop) {
        use crate::fight::input::pad::{CROSS, DOWN, TRIANGLE, UP};
        let Some(run) = &mut self.setup else { return };
        if ev.state != ElementState::Pressed {
            return;
        }
        let typing = run.ui.typing();
        let cmd = match &ev.logical_key {
            Key::Named(NamedKey::ArrowUp) => run.ui.pad(UP),
            Key::Named(NamedKey::ArrowDown) | Key::Named(NamedKey::Tab) => run.ui.pad(DOWN),
            Key::Named(NamedKey::Enter) if !ev.repeat => run.ui.pad(CROSS),
            Key::Named(NamedKey::Escape) if !ev.repeat => run.ui.back(true),
            Key::Named(NamedKey::Backspace) if typing => {
                run.ui.erase();
                Cmd::None
            }
            Key::Named(NamedKey::Backspace) if !ev.repeat => run.ui.back(false),
            _ if typing => {
                if let Some(t) = &ev.text {
                    run.ui.text(t);
                }
                Cmd::None
            }
            Key::Character(c) if !ev.repeat => match c.to_ascii_lowercase().as_str() {
                "w" => run.ui.pad(UP),
                "s" => run.ui.pad(DOWN),
                "k" => run.ui.pad(CROSS),
                "i" => run.ui.pad(TRIANGLE),
                _ => Cmd::None,
            },
            _ => Cmd::None,
        };
        self.setup_cmd(cmd, el);
    }

    /// The pointer moved (window pixels).
    pub(super) fn setup_pointer(&mut self, at: [f32; 2]) {
        if let Some(run) = &mut self.setup {
            run.mouse = at;
        }
    }

    /// A click or tap at `at` (None: where the mouse is): the row under it chosen; a row marked `^` / `v` (more rows
    /// above or below) brings the next one into view instead (`draw::Hit::scroll`).
    pub(super) fn setup_click(&mut self, at: Option<[f32; 2]>, el: &ActiveEventLoop) {
        let Some(run) = &mut self.setup else { return };
        let p = at.unwrap_or(run.mouse);
        let Some(hit) = run.hits.iter().find(|h| p[0] >= h.rect[0] && p[0] < h.rect[2] && p[1] >= h.rect[1] && p[1] < h.rect[3]) else { return };
        if hit.scroll != 0 {
            let k = (hit.row as i64 + hit.scroll as i64).max(0) as usize;
            run.ui.reveal(k);
            return;
        }
        let cmd = run.ui.choose(hit.row);
        self.setup_cmd(cmd, el);
    }

    /// A touch event on the setup screen (`phase` 0 down, 1 move, 2 up, 3 cancel; window pixels): a tap chooses (or
    /// scrolls, `setup_click`) when the finger goes up in place, a drag moves the cursor a row per row's height.
    pub(super) fn setup_touch(&mut self, phase: u8, id: i32, at: [f32; 2], el: &ActiveEventLoop) {
        let Some(run) = &mut self.setup else { return };
        let row_h = run.hits.first().map_or(40.0, |h| h.rect[3] - h.rect[1]);
        match run.drag.event(phase, id, at, row_h) {
            Some(screen::Gesture::Tap(p)) => self.setup_click(Some(p), el),
            Some(screen::Gesture::Rows(n)) => run.ui.scroll(n),
            None => {}
        }
    }

    /// A file or folder dropped onto the window.
    pub(super) fn setup_dropped(&mut self, p: PathBuf, el: &ActiveEventLoop) {
        if let Some(run) = &mut self.setup {
            let cmd = run.ui.dropped(p);
            self.setup_cmd(cmd, el);
        }
    }

    fn setup_cmd(&mut self, cmd: Cmd, el: &ActiveEventLoop) {
        let Some(run) = &mut self.setup else { return };
        match cmd {
            Cmd::None => {}
            Cmd::Quit => self.setup_quit(el),
            Cmd::Close => {
                self.setup = None;
                // the buttons that closed it reach the session only once released and pressed again; no catch-up
                self.swallow = self.pads()[0] | self.pads()[1];
                self.last = None;
                log::info!("setup: closed, back to the game");
            }
            // the browser: a chooser never answered (no `cancel` event) gives way to a new one
            Cmd::Browse => {
                // the native picker takes the focus, and the finger that chose the row may never send its up here (on
                // the web each touch has a new id, so `Drag` could not recover by id): its gesture is forgotten
                run.drag = screen::Drag::default();
                if run.browse.is_none() || cfg!(target_arch = "wasm32") {
                    run.browse = None;
                    #[cfg(target_arch = "wasm32")]
                    {
                        run.browse = pickers::Browse::open(run.ui.disc, run.ui.kind);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        run.browse = pickers::Browse::open(run.ui.disc, run.ui.kind);
                    }
                }
            }
            // the browser: its pick was checked when chosen (`web::inspect` gives that answer)
            #[cfg(target_arch = "wasm32")]
            Cmd::Check(p) => match job::inspect(&p, &run.dirs.root) {
                Ok(f) => run.ui.found(f),
                Err(e) => run.ui.failed(e, None),
            },
            #[cfg(not(target_arch = "wasm32"))]
            Cmd::Check(p) if run.check.is_none() => {
                let (q, data, kind) = (p.clone(), run.dirs.root.clone(), run.ui.kind);
                match std::thread::Builder::new().name("okb setup check".into()).spawn(move || job::inspect(&q, &data, kind)) {
                    Ok(h) => {
                        run.ui.checking(p.clone());
                        run.check = Some((p, h));
                    }
                    Err(e) => run.ui.failed(e.to_string(), Some(p)),
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Cmd::Check(_) => {}
            Cmd::Install(p) => {
                // the browser: the next `data-<n>` / `br1-<n>` from the pointers as they are now (a REMOVE or an
                // earlier install moved them)
                #[cfg(target_arch = "wasm32")]
                if run.ui.kind == Kind::Br2 {
                    run.dirs = Dirs::app();
                }
                (run.br1.dirs, run.q3.dirs) = (extra_dirs(&run.dirs, Kind::Br1), extra_dirs(&run.dirs, Kind::Q3));
                let kind = run.ui.kind;
                let dirs = match kind {
                    Kind::Br1 => run.br1.dirs.clone(),
                    Kind::Q3 => run.q3.dirs.clone(),
                    Kind::Br2 => run.dirs.clone(),
                };
                log::info!("setup: installing {} into {} ({kind:?})", p.display(), dirs.root.display());
                run.src = Some(p.clone());
                run.job = Some(job::Job::copy(p, dirs, job::Hooks { slow_ms: run.slow_ms, before_verify: None }, kind));
            }
            Cmd::Cancel => {
                if let Some(j) = &run.job {
                    j.cancel();
                }
            }
            Cmd::Start => self.setup_start(),
            Cmd::Remove(kind) => {
                let started = match kind {
                    Kind::Br1 => run.br1.start_remove(kind),
                    Kind::Q3 => run.q3.start_remove(kind),
                    Kind::Br2 => Ok(()),
                };
                if let Err(e) = started {
                    run.removed(kind, Err(e));
                }
            }
        }
    }

    /// The app quits from the setup screen (QUIT, the window closed, OKB_UIEXIT): a running copy is cancelled and
    /// waited for, so that no `.part` is left behind.
    pub(super) fn setup_quit(&mut self, el: &ActiveEventLoop) {
        self.setup_stop();
        #[cfg(target_os = "android")]
        {
            let _ = el;
            crate::platform::android::quit();
        }
        #[cfg(not(target_os = "android"))]
        el.exit();
    }

    /// The setup screen ends with the app (QUIT, the window closed, the app told to quit: macOS Cmd+Q): a running
    /// copy is cancelled and waited for, a folder dialog still open closed.
    pub(super) fn setup_stop(&mut self) {
        // the screen itself stays until the loop ends (its redraws, not the game's, until then)
        let Some(run) = self.setup.as_mut() else { return };
        if let Some(j) = run.job.take() {
            log::info!("setup: quitting: the copy cancelled");
            j.stop();
        }
        run.browse = None;
    }

    /// One redraw of the setup screen: the pads, the pickers' and the job's news, the picture (OKB_UISHOTS).
    pub(super) fn setup_redraw(&mut self, el: &ActiveEventLoop) {
        // Android: the activity's input (fingers and the back key come here, keys and gamepads through the pads)
        #[cfg(target_os = "android")]
        {
            self.android_input();
            // not in front (no window: the renderer has no surface): nothing drawn, and an install that finished
            // meanwhile starts the game only on the return, when the surface is there again
            if self.droid.failed || self.droid.frozen() {
                // a failed start's fingers are dropped (they would pile up, every move one)
                if self.droid.failed {
                    self.droid.touches.clear();
                }
                self.last = None;
                return;
            }
            for (phase, id, p) in std::mem::take(&mut self.droid.touches) {
                self.setup_touch(phase, id, p, el);
            }
            if std::mem::take(&mut self.droid.back) {
                let cmd = self.setup.as_mut().map_or(Cmd::None, |r| r.ui.back(true));
                self.setup_cmd(cmd, el);
            }
            if self.setup.is_none() {
                return;
            }
        }
        // the presses since the last frame latched in (a press shorter than a frame is still one, as in the menus: an
        // Android key event, a gamepad's)
        let pads = self.pads();
        let pads = [pads[0] | self.latch[0], pads[1] | self.latch[1]];
        self.latch = [0; 2];
        let pads = self.ui_pads(pads);
        let frame = self.ui_frame;
        let Some(run) = &mut self.setup else { return };
        run.frames += 1;
        run.pads.update(pads);
        // up / down repeat when held, the other buttons do not (a held × must not go on to INSTALL)
        let moves = crate::fight::input::pad::UP | crate::fight::input::pad::DOWN;
        let mut cmds = vec![run.ui.pad(run.pads.any_rep() & moves | run.pads.any_press() & !moves)];
        if run.scan.as_ref().is_some_and(|h| h.is_finished())
            && let Some(found) = run.scan.take().and_then(|h| h.join().ok())
        {
            log::info!("setup: found {:?} and the disc images {:?}", found.0, found.1);
            run.ui.set_found(found);
        }
        if run.check.as_ref().is_some_and(|c| c.1.is_finished())
            && let Some((p, h)) = run.check.take()
        {
            match h.join().unwrap_or_else(|_| Err("the check stopped unexpectedly".into())) {
                Ok(f) => {
                    log::info!("setup: {} found: {} ({} bytes)", f.path.display(), f.what, f.bytes);
                    run.ui.found(f);
                }
                Err(e) => {
                    log::warn!("setup: {}: {e}", p.display());
                    run.ui.failed(e, Some(p));
                }
            }
        }
        if let Some(a) = run.browse.as_mut().and_then(|b| b.poll()) {
            run.browse = None;
            cmds.push(run.ui.picked(a));
        }
        // debug OKB_SETUP_FROM: dropped at UI frame 30, installed at 60 (or once its check has ended)
        if let Some(p) = &run.from {
            if frame == 30 {
                cmds.push(run.ui.dropped(p.clone()));
            } else if frame >= 60 && matches!(run.ui.page, Page::Confirm(_)) {
                cmds.push(run.ui.choose(0));
                run.from = None;
            }
        }
        // debug OKB_SETUP_BR1 / OKB_SETUP_Q3, one at a time: ADD (BLOODY ROAR 1 DISC, QUAKE III ARENA MAPS) once
        // offered, the path dropped on its page, installed once checked
        let hooked = Kind::EXTRAS.into_iter().find(|&k| run.hook(k).is_some_and(|h| h.from.is_some()));
        if run.from.is_none()
            && run.job.is_none()
            && let Some(kind) = hooked
            && let Some((p, started)) = run.hook(kind).and_then(|h| Some((h.from.clone()?, h.started)))
        {
            let add = run.ui.rows().iter().position(|r| r.1 == Act::Add(kind));
            let mut end = false;
            match (&run.ui.page, run.ui.kind, add) {
                // its check or its install failed: the hook ends and the run quits (it would wait on the error page)
                (Page::Error { msg, .. }, k, _) if k == kind => {
                    log::error!("setup: the {} hook {}: {msg}", kind.data(), p.display());
                    end = true;
                    cmds.push(Cmd::Quit);
                }
                // its install ended (the job is gone, not on its error page): installed
                _ if started => end = true,
                (Page::Intro | Page::Done, Kind::Br2, Some(k)) => cmds.push(run.ui.choose(k)),
                (Page::Intro, k, _) if k == kind => cmds.push(run.ui.dropped(p)),
                (Page::Confirm(_), k, _) if k == kind => {
                    cmds.push(run.ui.choose(0));
                    if let Some(h) = run.hook(kind) {
                        h.started = true;
                    }
                }
                _ => {}
            }
            if end && let Some(h) = run.hook(kind) {
                h.from = None;
            }
        } else if run.scripted && hooked.is_none() && run.job.is_none() && run.ui.page == Page::Done {
            // OKB_SETUP_FROM's install done (and the optional installs' hooks): the game starts a second later
            if run.done_since.is_some_and(|f| frame >= f + 60) {
                cmds.push(Cmd::Start);
            }
        }
        if run.ui.page != Page::Done {
            run.done_since = None;
        } else if run.done_since.is_none() {
            run.done_since = Some(frame);
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(v) = run.br1.check.as_ref().and_then(|c| c.borrow_mut().take()) {
            log::info!("setup: the imported game's data: {v:?}");
            run.br1.check = None;
            if std::mem::take(&mut run.br1.installed) {
                run.ui.br1_done(v);
            } else {
                run.ui.br1 = Some(v);
            }
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(v) = run.q3.check.as_ref().and_then(|c| c.borrow_mut().take()) {
            log::info!("setup: Quake III Arena maps: {v:?}");
            run.q3.check = None;
            if std::mem::take(&mut run.q3.installed) {
                run.ui.q3_done(v);
            } else {
                run.ui.q3 = Some(v);
            }
        }
        // an optional install removed over the game: the app restarts as after an install (`setup_done`)
        let mut removed = false;
        if let Some(r) = run.br1.poll_remove() {
            removed |= r.is_ok();
            run.removed(Kind::Br1, r);
        }
        if let Some(r) = run.q3.poll_remove() {
            removed |= r.is_ok();
            run.removed(Kind::Q3, r);
        }
        if removed && run.in_game && run.job.is_none() {
            self.setup = None;
            self.restart(el);
            return;
        }
        let mut done = false;
        if let Some(j) = &mut run.job {
            run.ui.progress = j.now();
            match j.poll() {
                None => {}
                Some(Ok(())) => done = true,
                Some(Err(e)) => {
                    log::warn!("setup: {e:?}");
                    run.job = None;
                    run.ui.failed(e.message(), run.src.clone());
                }
            }
        }
        for c in cmds {
            self.setup_cmd(c, el);
        }
        // the pick page left (BACK, a drop): its folder dialog goes (the browser: a pending chooser and its listeners;
        // not Android, whose picker covers the app and answers once it is back)
        if let Some(run) = &mut self.setup
            && run.ui.page != Page::Pick
            && cfg!(not(target_os = "android"))
        {
            run.browse = None;
        }
        if done {
            self.setup_done(el);
            if self.setup.is_none() {
                return;
            }
        }
        self.setup_draw(el);
    }

    /// The data is installed. Opened from the game: the app restarts (any kind). At start: an optional install goes
    /// back to the page it was opened from; the main game's to the page offering the optional ones, START THE GAME
    /// goes on (`setup_start`).
    fn setup_done(&mut self, el: &ActiveEventLoop) {
        let Some(run) = self.setup.as_mut() else { return };
        run.job = None;
        // opened from the game: every install restarts the app (the session's start has read what was installed;
        // the page's header says so), the optional ones too
        if run.in_game {
            log::info!("setup: {} installed: restarting", run.ui.kind.data());
            self.setup = None;
            self.restart(el);
            return;
        }
        if run.ui.kind == Kind::Q3 {
            log::info!("setup: Quake III Arena maps installed in {}", run.q3.dirs.root.display());
            // the done page with the installed maps' own verdict (the browser: once its check answers)
            run.check_q3();
            #[cfg(not(target_arch = "wasm32"))]
            run.ui.q3_done(run.ui.q3.clone().unwrap_or(Q3Verdict::Ok));
            #[cfg(target_arch = "wasm32")]
            {
                run.q3.installed = true;
            }
            return;
        }
        if run.ui.kind == Kind::Br1 {
            log::info!("setup: the imported game's data installed in {}", run.br1.dirs.root.display());
            // the done page with the installed data's own verdict (the browser: once its check answers)
            run.check_br1();
            #[cfg(not(target_arch = "wasm32"))]
            run.ui.br1_done(run.ui.br1.clone().unwrap_or(Br1Verdict::Ok));
            #[cfg(target_arch = "wasm32")]
            {
                run.br1.installed = true;
            }
            return;
        }
        log::info!("setup: game data installed in {}", run.dirs.root.display());
        run.ui.br2_done(Br1Verdict::Absent);
        run.check_extras();
    }

    /// START THE GAME after an install at start: the setup screen goes, the game starts as it would have with the
    /// data there.
    fn setup_start(&mut self) {
        let Some(run) = self.setup.take() else { return };
        if let Some(cfg) = &mut self.cfg {
            #[cfg(not(target_arch = "wasm32"))]
            {
                cfg.assets = crate::assets::Assets::new(run.dirs.root.clone());
            }
            #[cfg(target_arch = "wasm32")]
            {
                cfg.assets = crate::assets::Assets::opfs(run.dirs.root.to_string_lossy());
            }
        }
        self.start_up();
    }

    /// The app started again as it was started (the same arguments and environment, but a debug OKB_SETUP_FROM,
    /// OKB_SETUP_BR1 or OKB_SETUP_Q3, which would install again), this one ending as EXIT GAME ends it.
    fn restart(&mut self, el: &ActiveEventLoop) {
        // Android: the activity starts again (the process ends there; the card and preferences were saved on change)
        #[cfg(target_os = "android")]
        {
            let _ = el;
            crate::platform::android::restart();
            return;
        }
        // the browser: the page loads again
        #[cfg(target_arch = "wasm32")]
        {
            let _ = el;
            if let Some(w) = web_sys::window() {
                let _ = w.location().reload();
            }
            return;
        }
        // the preferences written before the new process can read them
        #[allow(unreachable_code)]
        if self.saves() {
            crate::platform::storage::write_prefs(self.prefs.to_stored());
        }
        let exe = std::env::current_exe();
        match exe.and_then(|e| {
            std::process::Command::new(e)
                .args(std::env::args_os().skip(1))
                .env_remove("OKB_SETUP_FROM")
                .env_remove("OKB_SETUP_BR1")
                .env_remove("OKB_SETUP_Q3")
                .env_remove("BR2_SETUP_FROM")
                .env_remove("BR2_SETUP_BR1")
                .env_remove("BR2_SETUP_Q3")
                .spawn()
        }) {
            Ok(_) => {
                log::info!("setup: restarting");
                self.exit_req = true;
                el.exit();
            }
            Err(e) => {
                log::warn!("setup: could not restart: {e}");
                // the session goes on: a new start offers the optional installs as they are now (as `start_up`; a
                // removal deleted their files)
                if let Some(assets) = self.cfg.as_ref().map(|c| c.assets.clone())
                    && self.player.is_none()
                    && !self.net_ui.netplay
                {
                    self.start.br1 = crate::assets::manifest::br1_install_id(&assets);
                    self.start.set_extras(crate::assets::extras::present(&assets));
                }
                self.toast = Some(("the change takes effect from the next start".into(), 600));
            }
        }
    }

    fn setup_draw(&mut self, el: &ActiveEventLoop) {
        let (Some(run), Some(r)) = (&mut self.setup, &mut self.renderer) else { return };
        let win = [r.viewport().win[0] as f32, r.viewport().win[1] as f32];
        let (quads, hits) = draw::draw(&run.ui, win, crate::platform::support::VERSION, run.frames / 30 % 2 == 0);
        run.hits = hits;
        r.screen = quads;
        // nothing of the game's picture (over a waiting session: its afterimage, its ambient fill)
        (r.afterimage, r.ambient) = (0, None);
        let shot = cfg!(not(target_arch = "wasm32")) && self.env.uishots.contains(&self.ui_frame) && self.ui_shot_done != self.ui_frame;
        #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
        if let Some((w, h, px)) = r.frame_capture(&[], &[], &[], draw::CLEAR, shot) {
            self.ui_shot_done = self.ui_frame;
            #[cfg(not(target_arch = "wasm32"))]
            {
                let path = format!("okb_ui_{:05}.png", self.ui_frame);
                let _ = image::save_buffer(&path, &px, w, h, image::ExtendedColorType::Rgba8);
                log::info!("saved {path}");
            }
        }
        if self.env.uiexit && self.env.uishots.iter().max().is_some_and(|&m| self.ui_frame > m + 5) {
            self.setup_quit(el);
        }
    }
}

impl Run {
    /// The optional installs' quick checks into the screen.
    fn check_extras(&mut self) {
        self.check_br1();
        self.check_q3();
    }

    /// The debug hook of the optional install `kind` (None: the main game's).
    fn hook(&mut self, kind: Kind) -> Option<&mut Hook> {
        match kind {
            Kind::Br1 => Some(&mut self.br1.hook),
            Kind::Q3 => Some(&mut self.q3.hook),
            Kind::Br2 => None,
        }
    }

    /// Quake III Arena's maps' quick check into the screen (`ui.q3`; the browser: its light check, when it has
    /// answered).
    fn check_q3(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let v = crate::assets::extras::check_dir_q3(&self.q3.dirs.root).0;
            log::info!("setup: Quake III Arena maps {}: {v:?}", self.q3.dirs.root.display());
            self.ui.q3 = Some(v);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let cell: std::rc::Rc<std::cell::RefCell<Option<Q3Verdict>>> = Default::default();
            let c = cell.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let dir = crate::platform::game_data::web::Q3.pointer();
                *c.borrow_mut() = Some(crate::assets::extras::check_opfs_q3(dir.as_deref(), true).await.0);
            });
            self.q3.check = Some(cell);
        }
    }

    /// The imported game's data's quick check into the screen (`ui.br1`; the browser: when its check has answered).
    fn check_br1(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let v = crate::assets::manifest::check_dir_br1(&self.br1.dirs.root);
            log::info!("setup: the imported game's data {}: {v:?}", self.br1.dirs.root.display());
            self.ui.br1 = Some(v);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let cell: std::rc::Rc<std::cell::RefCell<Option<Br1Verdict>>> = Default::default();
            let c = cell.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let dir = crate::platform::game_data::web::BR1.pointer();
                *c.borrow_mut() = Some(crate::assets::manifest::check_opfs_br1(dir.as_deref()).await);
            });
            self.br1.check = Some(cell);
        }
    }

    /// The removal's answer into the screen.
    fn removed(&mut self, kind: Kind, r: Result<(), String>) {
        match &r {
            Ok(()) => log::info!("setup: the {} removed", kind.data()),
            Err(e) => log::warn!("setup: the {} not removed: {e}", kind.data()),
        }
        self.ui.removed(kind, r);
    }
}

impl<V> Extra<V> {
    /// REMOVE: its folder deleted on a thread (its answer polled each frame, `poll_remove`); the browser: under the
    /// install lock in a task. Nothing while one runs; Err: the thread could not start.
    fn start_remove(&mut self, kind: Kind) -> Result<(), String> {
        if self.remove.is_some() {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = kind;
            let dirs = self.dirs.clone();
            let h = std::thread::Builder::new().name("okb setup remove".into()).spawn(move || remove_extra(&dirs)).map_err(|e| e.to_string())?;
            self.remove = Some(h);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let cell: std::rc::Rc<std::cell::RefCell<Option<Result<(), String>>>> = Default::default();
            let c = cell.clone();
            wasm_bindgen_futures::spawn_local(async move {
                *c.borrow_mut() = Some(remove_extra(web::slot(kind)).await);
            });
            self.remove = Some(cell);
        }
        Ok(())
    }

    /// The removal's answer, once it is there.
    fn poll_remove(&mut self) -> Option<Result<(), String>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let h = self.remove.take_if(|h| h.is_finished())?;
            Some(h.join().unwrap_or_else(|_| Err("the removal stopped".into())))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let r = self.remove.as_ref()?.borrow_mut().take()?;
            self.remove = None;
            Some(r)
        }
    }
}

/// REMOVE (an optional install): its folder deleted (under the install lock; on the removal's thread).
#[cfg(not(target_arch = "wasm32"))]
fn remove_extra(dirs: &Dirs) -> Result<(), String> {
    let _held = dirs.lock().map_err(|e| e.to_string())?.ok_or("another copy of the game is installing its data: try again when it is done")?;
    dirs.remove().map_err(|e| e.to_string())
}

/// REMOVE in the browser: the slot's pointer cleared under the install lock (another tab's install cannot switch it
/// meanwhile), then its folder deleted (at the next start if the tab closes first). Quake III Arena's maps, which a
/// session may still be reading: the folder they replaced before goes now, theirs only at the next start
/// (`web::forget_old`).
#[cfg(target_arch = "wasm32")]
async fn remove_extra(slot: crate::platform::game_data::web::Slot) -> Result<(), String> {
    use crate::platform::game_data::web;
    let _held = web::install_lock().await.ok_or("another tab of the game is installing its data: try again when it is done")?;
    let root = web::root().await.ok();
    if slot == web::Q3
        && let Some(r) = &root
    {
        slot.forget_replaced(r).await;
    }
    slot.switch_to(None)?;
    if slot != web::Q3
        && let Some(r) = &root
    {
        slot.forget_replaced(r).await;
    }
    Ok(())
}

/// The folders of the install `kind` beside the main game's `dirs` (`Dirs::sub`; the browser: its slot's next
/// folder); the main game's own: `dirs`.
fn extra_dirs(dirs: &Dirs, kind: Kind) -> Dirs {
    match kind.dir() {
        None => dirs.clone(),
        #[cfg(not(target_arch = "wasm32"))]
        Some(name) => dirs.sub(name),
        #[cfg(target_arch = "wasm32")]
        Some(_) => Dirs::of(kind),
    }
}
