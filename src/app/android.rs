//! The app on Android: the activity's input, the touch controls and their page, the lifecycle.
//!
//! Engine addition.
//!
//! The app on Android (engine addition, `docs/android.md`): the input the activity forwards (`android_input`: fingers
//! to the touch controls or their page, keys and gamepad axes to pad bits, the back key), the touch controls' quads
//! (`touch_screen`), the TOUCH CONTROLS page's frame (`touch_menu_tick`), the setup screen's touches and back key
//! (`Droid::touches`, `Droid::back`), and the lifecycle (`android_idle`: the background; `android_resumed`,
//! `android_suspended`, `android_focus`; `android_failed`: a load that failed). Every pad bit made here reaches the session as the keyboard's do (`App::pads`, the latch); a
//! second gamepad is player 2's (`PadSlots`).

use super::devices::{PadSlots, android_axes_bits, android_key_bit};
use super::touch::{Cmd, Menu, Touch, TouchPrefs};
use super::{App, FRAME};
use crate::fight::input::pad::TRIANGLE;
use crate::platform::android::{self as platform, Input};
use winit::event_loop::{ActiveEventLoop, ControlFlow};

const START: u16 = 0x800;
/// `KeyEvent.KEYCODE_BACK`
const KEYCODE_BACK: i32 = 4;

/// The app's Android state.
pub(super) struct Droid {
    /// the app is not in front (focus lost, or no window): no steps, no sound
    pub inactive: bool,
    /// the game could not start or go on (`UserEvent::Failed`): the activity's dialog is up (`android_failed`),
    /// nothing is stepped, loaded or drawn
    pub failed: bool,
    /// a native window is there (from `resumed` to `suspended`; the first `resumed` comes with one): the GPU
    /// surface can be made
    window: bool,
    /// per player: keys held (player 1: the keyboard and the back key too; the gamepads' buttons by `slots`) and the
    /// gamepads' stick / hat / triggers as pad bits
    keys: [u16; 4],
    axes: [u16; 4],
    /// the gamepads' players
    slots: PadSlots<i32>,
    /// what the back key pressed (START in a fight, else △), let go with it
    back_bit: u16,
    pub touch: Touch,
    /// the TOUCH CONTROLS page (the session waits while it is up)
    pub menu: Option<Menu>,
    /// the setup screen (`app::setup`): its touch events since its last frame (phase, finger, window pixels;
    /// `setup_touch`), and the back key pressed
    pub touches: Vec<(u8, i32, [f32; 2])>,
    pub back: bool,
}

impl Droid {
    pub fn new() -> Self {
        Self {
            inactive: false,
            failed: false,
            window: true,
            keys: [0; 4],
            axes: [0; 4],
            slots: PadSlots::default(),
            back_bit: 0,
            touch: Touch::new(TouchPrefs::from_stored(crate::platform::storage::read_touch_prefs())),
            menu: None,
            touches: Vec::new(),
            back: false,
        }
    }

    /// The pad bits from Android's devices: keys, gamepad axes, the touch controls (player 1's; none under the page);
    /// players 3 / 4 gamepads only (the team battle).
    pub fn bits(&self) -> [u16; 4] {
        let touch = if self.menu.is_none() { self.touch.bits() } else { 0 };
        std::array::from_fn(|k| self.keys[k] | self.axes[k] | if k == 0 { touch } else { 0 })
    }

    /// No steps, no loads, no sound: not in front.
    pub fn frozen(&self) -> bool {
        self.inactive
    }

    /// The player of a key or axes event: a gamepad's by its slot (None: a fifth one), else player 1.
    fn player(&mut self, pad: bool, dev: i32) -> Option<usize> {
        if pad { self.pad_player(dev) } else { Some(0) }
    }

    /// A gamepad's player, given one when it is new (`PadSlots`).
    fn pad_player(&mut self, dev: i32) -> Option<usize> {
        let had = self.slots.player_of(dev);
        let p = self.slots.player(dev, 0);
        if had.is_none() {
            match p {
                Some(p) => log::info!("android: gamepad {dev} is player {}", p + 1),
                None => log::info!("android: gamepad {dev} not read (four are in use)"),
            }
        }
        p
    }
}

impl App {
    /// The input the activity forwarded since the last frame, in order; a press shorter than a frame still reaches
    /// one step (the latch).
    pub(super) fn android_input(&mut self) {
        for ev in platform::take_input() {
            match ev {
                Input::Insets(i) => self.droid.touch.set_insets(i),
                // the setup screen: its fingers (a tap chooses the row under it, a drag scrolls); the back key goes back
                Input::Touch { phase, id, x, y } if self.setup.is_some() => self.droid.touches.push((phase, id, [x, y])),
                Input::Key { code: KEYCODE_BACK, down, .. } if self.setup.is_some() => self.droid.back |= down,
                // the page: a finger coming down on a row chooses it
                Input::Touch { phase, x, y, .. } if self.droid.menu.is_some() => {
                    if phase == 0 {
                        self.menu_tap(x, y);
                    }
                }
                Input::Touch { phase, id, x, y } => {
                    let before = self.droid.touch.bits();
                    self.droid.touch.event(phase, id, [x, y]);
                    self.latch[0] |= self.droid.touch.bits() & !before;
                    if std::mem::take(&mut self.droid.touch.opt_tapped) {
                        let held = self.pads();
                        self.droid.menu = Some(Menu::new(self.droid.touch.prefs, held));
                    }
                }
                Input::Key { code: KEYCODE_BACK, down, .. } => self.back_key(down),
                Input::Key { code, down, pad, dev } => {
                    let bit = android_key_bit(code);
                    let Some(p) = self.droid.player(pad && bit != 0, dev) else { continue };
                    if bit != 0 && p == 0 {
                        // player 1's controller or keyboard in use: the touch controls step aside until the next touch
                        self.droid.touch.shown = false;
                    }
                    if down {
                        self.droid.keys[p] |= bit;
                        if p < 2 {
                            // pads 3 / 4 have no latch: read as held
                            self.latch[p] |= bit;
                        }
                    } else {
                        self.droid.keys[p] &= !bit;
                    }
                }
                Input::Axes { dev, axes } => {
                    let bits = android_axes_bits(axes);
                    // a stick at rest assigns no player (some devices report axes without being used)
                    let p = if bits != 0 { self.droid.pad_player(dev) } else { self.droid.slots.player_of(dev) };
                    let Some(p) = p else { continue };
                    if bits != 0 && p == 0 {
                        self.droid.touch.shown = false;
                    }
                    if p < 2 {
                        self.latch[p] |= bits & !self.droid.axes[p];
                    }
                    self.droid.axes[p] = bits;
                }
                Input::Removed(dev) => {
                    if let Some(p) = self.droid.slots.removed(dev) {
                        log::info!("android: gamepad {dev} (player {}) went away", p + 1);
                        self.droid.keys[p] = 0;
                        self.droid.axes[p] = 0;
                    }
                }
            }
        }
    }

    /// The back key / gesture: the page closes; in a fight START (the pause menu), elsewhere △ (back).
    fn back_key(&mut self, down: bool) {
        if !down {
            self.droid.keys[0] &= !self.droid.back_bit;
            self.droid.back_bit = 0;
            return;
        }
        if self.droid.menu.is_some() {
            self.close_menu(&[0x0D]);
            return;
        }
        let bit = if (self.state.fight.is_some() || self.state.team.is_some()) && self.player.is_none() && self.browser.is_none() { START } else { TRIANGLE };
        self.droid.back_bit = bit;
        self.droid.keys[0] |= bit;
        self.latch[0] |= bit;
    }

    fn play_ui(&mut self, sounds: &[u16]) {
        if let Some(f) = &self.state.front {
            let mut q = crate::output::sound::Queue::new(false);
            for &id in sounds {
                f.play_se(&mut q, id);
            }
            self.audio.apply(&mut q);
        }
    }

    /// The page closes; the buttons held then reach the session only once let go and pressed again.
    fn close_menu(&mut self, sounds: &[u16]) {
        self.play_ui(sounds);
        self.droid.menu = None;
        let pads = self.pads();
        self.swallow = pads[0] | pads[1];
    }

    fn menu_cmd(&mut self, cmd: Cmd, sounds: Vec<u16>) {
        match cmd {
            Cmd::None => self.play_ui(&sounds),
            Cmd::Close => self.close_menu(&sounds),
            Cmd::Prefs(p) => {
                self.play_ui(&sounds);
                self.droid.touch.prefs = p;
                if let Ok(text) = serde_json::to_string_pretty(&p) {
                    crate::platform::storage::write_touch_prefs(&text);
                }
            }
        }
    }

    /// A tap at window pixel (x, y) on the page (the 512 × 480 space over the core 4:3 rectangle).
    fn menu_tap(&mut self, x: f32, y: f32) {
        let Some(r) = &self.renderer else { return };
        let vp = r.viewport();
        let (hx, hy) = ((x - vp.core[0] as f32) / vp.unit[0], (y - vp.core[1] as f32) / vp.unit[1]);
        let Some(m) = &mut self.droid.menu else { return };
        let mut sounds = Vec::new();
        let cmd = m.tap(hx, hy, &mut sounds);
        self.menu_cmd(cmd, sounds);
    }

    /// One 60 Hz frame of the page with the pads (keys and gamepads move its cursor).
    pub(super) fn touch_menu_tick(&mut self, pads: [u16; 2]) {
        let Some(m) = &mut self.droid.menu else { return };
        let mut sounds = Vec::new();
        let cmd = m.tick(pads, &mut sounds);
        self.menu_cmd(cmd, sounds);
    }

    /// The touch controls for this frame's `Renderer::screen` (OPT offered on the front end's own screens).
    pub(super) fn touch_screen(&mut self) {
        let in_fight = self.state.fight.is_some() || self.state.team.is_some();
        // not in netplay: the page stops this side's steps, which would stall the other player
        let front =
            !in_fight && self.state.front.is_some() && self.browser.is_none() && self.net_ui.menu.is_none() && self.net.is_none() && self.player.is_none();
        let Some(r) = &mut self.renderer else { return };
        let t = &mut self.droid.touch;
        t.set_window(r.config.width, r.config.height);
        // the fight HUD at the window's edges, clear of the cutout
        r.set_hud_insets(t.insets());
        t.opt = front;
        t.fight = in_fight;
        r.screen = if self.droid.menu.is_some() { Vec::new() } else { t.quads(crate::output::font::fonts()) };
    }

    /// `about_to_wait` on Android: true while there is nothing to draw (the app not in front, a failure's dialog up),
    /// the loop then waking a few times a second.
    pub(super) fn android_idle(&mut self, el: &ActiveEventLoop) -> bool {
        if !self.droid.failed && !self.droid.frozen() {
            return false;
        }
        el.set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + std::time::Duration::from_secs_f64(FRAME * 12.0)));
        true
    }

    /// The game cannot start or go on (no GPU, a match that did not load): the activity says why and ends the app
    /// cleanly (`platform::show_fatal`); nothing runs meanwhile. Not `ActiveEventLoop::exit`: the process
    /// would end under the activity's UI thread.
    pub(super) fn android_failed(&mut self, e: &str) {
        log::error!("the game cannot go on: {e}");
        if std::mem::replace(&mut self.droid.failed, true) {
            return;
        }
        self.audio.suspend();
        platform::show_fatal(e);
    }

    /// A new native window (back from the background): the GPU surface made again, the frame clock restarted.
    pub(super) fn android_resumed(&mut self) {
        log::info!("android: a new window");
        self.droid.window = true;
        if let (Some(r), Some(w)) = (&mut self.renderer, &self.window) {
            if let Err(e) = r.resume_surface(w.clone()) {
                log::error!("the GPU surface could not be made again: {e}");
            }
            self.droid.touch.set_window(r.config.width, r.config.height);
        }
        self.android_focus(true);
    }

    /// The native window went (home, the screen off): the surface dropped, nothing stepped or drawn.
    pub(super) fn android_suspended(&mut self) {
        log::info!("android: the window went");
        self.droid.window = false;
        if let Some(r) = &mut self.renderer {
            r.suspend();
        }
        self.android_focus(false);
    }

    /// The app in front (`true`) or not: no steps and no sound output while not; the inputs let go, the frame clock
    /// restarted (no catch-up burst) when back.
    pub(super) fn android_focus(&mut self, on: bool) {
        if on == !self.droid.inactive {
            return;
        }
        let was = self.droid.frozen();
        self.droid.inactive = !on;
        self.freeze_changed(was);
    }

    /// Frozen (`Droid::frozen`) now, or not, after `was`: the inputs let go; the audio closed, or opened again with
    /// the frame clock restarted (no catch-up burst) when it ends.
    fn freeze_changed(&mut self, was: bool) {
        let now = self.droid.frozen();
        if now == was {
            return;
        }
        let on = !now;
        self.droid.keys = [0; 4];
        self.droid.axes = [0; 4];
        self.droid.back_bit = 0;
        self.droid.touch.release_all();
        self.latch = [0; 2];
        self.last = None;
        if on {
            // what came in while away (the dialog's taps) is dropped, the insets kept
            for ev in platform::take_input() {
                match ev {
                    Input::Insets(i) => self.droid.touch.set_insets(i),
                    Input::Removed(dev) => {
                        self.droid.slots.removed(dev);
                    }
                    _ => {}
                }
            }
            if !self.env.mute && self.store.front.is_some() {
                self.audio.start();
            }
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        } else {
            self.audio.suspend();
        }
        log::info!("android: {}", if on { "running" } else { "in the background" });
    }
}
