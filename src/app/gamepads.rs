//! Gamepads → the four players' pad bits (desktop: gilrs; the web: the Gamepad API), rumble, the connect log.
//!
//! Engine addition.
//!
//! `Pads` (`docs/gamepads.md`): read once per redraw (`App::pads`), outside the deterministic session. A pad's
//! buttons come through its SDL mapping (gilrs's bundled database, the user's `gamecontrollerdb.txt` beside the
//! save, `SDL_GAMECONTROLLERCONFIG`) or, without one, the default layout of `pad_layout`; a pad that does not look
//! like a gamepad is not read. Players follow `PadSlots`: the first gamepad used is player 1's, the second player
//! 2's, the third and fourth players 3 and 4's (the team battle's pads), each keeps its player while away. Every pad
//! connecting, getting a player or going away is logged.
//! The rumble (`Rumble`, desktop) goes to each player's gamepad.

use super::devices::{PadSlots, SLOTS};
use super::pad_layout;

/// How a desktop pad's buttons are read.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    /// an SDL mapping: gilrs's named buttons
    Mapped,
    /// no mapping, but the system's own gamepad layout (Windows' Xbox-compatible pads): named as well
    Driver,
    /// no mapping: the raw buttons in the default layout (`pad_layout::raw_code_bit`)
    Raw,
}

/// What `Pads` knows of a connected desktop pad.
#[cfg(not(target_arch = "wasm32"))]
struct Info {
    name: String,
    layout: Layout,
    model: u64,
    /// read at all (false: it does not look like a gamepad)
    gamepad: bool,
    /// "not read" said once
    told: bool,
}

/// The gamepads and their players (desktop: gilrs).
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct Pads {
    gilrs: Option<gilrs::Gilrs>,
    /// the user's `gamecontrollerdb.txt` and `SDL_GAMECONTROLLERCONFIG`, checked (which mapping a pad got)
    user: pad_layout::UserMappings,
    env: pad_layout::UserMappings,
    known: std::collections::HashMap<gilrs::GamepadId, Info>,
    slots: PadSlots<gilrs::GamepadId>,
    rumble: Rumble,
}

#[cfg(not(target_arch = "wasm32"))]
impl Pads {
    /// gilrs with the user's mappings (`gamecontrollerdb.txt` beside the save; lines gilrs would not use are logged).
    pub fn new() -> Self {
        let path = crate::platform::storage::pad_mappings_path();
        let user = match crate::platform::storage::read_pad_mappings() {
            None => pad_layout::UserMappings::default(),
            Some(Err(e)) => {
                log::warn!("gamepads: could not read {}: {e}", path.display());
                pad_layout::UserMappings::default()
            }
            Some(Ok(text)) => {
                let m = pad_layout::UserMappings::parse(&text, pad_layout::SDL_PLATFORM);
                for (line, e) in &m.errors {
                    log::warn!("gamepads: {}:{line}: {e}; line not used", path.display());
                }
                log::info!(
                    "gamepads: {} mapping(s) for {} from {} ({} for other systems left out)",
                    m.pads.len(),
                    pad_layout::SDL_PLATFORM,
                    path.display(),
                    m.other_platforms
                );
                m
            }
        };
        let env = std::env::var("SDL_GAMECONTROLLERCONFIG").map(|t| pad_layout::UserMappings::parse(&t, pad_layout::SDL_PLATFORM)).unwrap_or_default();
        for (line, e) in &env.errors {
            log::warn!("gamepads: SDL_GAMECONTROLLERCONFIG line {line}: {e}; gilrs will not use it");
        }
        // gilrs adds its bundled database and then SDL_GAMECONTROLLERCONFIG over these: a line of the user's file
        // counts for the pads the database does not know
        let gilrs = gilrs::GilrsBuilder::new().add_mappings(&user.text).build().ok();
        Self { gilrs, user, env, known: Default::default(), slots: PadSlots::default(), rumble: Rumble::default() }
    }

    /// The four players' pad bits as the gamepads are now (pads 3 / 4: the team battle's); the events since the last
    /// call give players and log.
    pub fn poll(&mut self) -> [u16; SLOTS] {
        let Some(g) = self.gilrs.as_mut() else { return [0; SLOTS] };
        let mut used = Vec::new();
        while let Some(ev) = g.next_event() {
            match ev.event {
                // its mapping may be another now: known again below
                gilrs::EventType::Connected => {
                    self.known.remove(&ev.id);
                }
                gilrs::EventType::Disconnected => {
                    let name = self.known.remove(&ev.id).map(|i| i.name).unwrap_or_default();
                    match self.slots.removed(ev.id) {
                        Some(k) => log::info!("gamepad {} \"{name}\" went away (player {} kept for it)", ev.id, k + 1),
                        None => log::info!("gamepad {} \"{name}\" went away", ev.id),
                    }
                }
                gilrs::EventType::ButtonPressed(..) if !used.contains(&ev.id) => used.push(ev.id),
                _ => {}
            }
        }
        for (id, pad) in g.gamepads() {
            if !self.known.contains_key(&id) {
                let i = Self::info(&pad, &self.user, &self.env);
                if i.gamepad
                    && let Some(k) = self.slots.reconnected(id)
                {
                    log::info!("gamepad {id} \"{}\" is player {} again", i.name, k + 1);
                }
                self.known.insert(id, i);
            }
        }
        for id in used {
            let Some(i) = self.known.get_mut(&id) else { continue };
            if !i.gamepad || (0..SLOTS).any(|k| self.slots.dev(k) == Some(id)) {
                continue;
            }
            match self.slots.player(id, i.model) {
                Some(k) => log::info!("gamepad {id} \"{}\" is player {}", i.name, k + 1),
                None if !i.told => log::info!("gamepad {id} \"{}\" not read: all four players have a gamepad", i.name),
                None => {}
            }
            i.told = true;
        }
        std::array::from_fn(|k| {
            let Some(id) = self.slots.dev(k) else { return 0 };
            match (g.connected_gamepad(id), self.known.get(&id)) {
                (Some(pad), Some(i)) => pad_bits(&pad, i.layout),
                _ => 0,
            }
        })
    }

    /// A pad seen connected: its layout and whether it is read, logged.
    fn info(pad: &gilrs::Gamepad, user: &pad_layout::UserMappings, env: &pad_layout::UserMappings) -> Info {
        use gilrs::Button;
        let name = pad.name().to_string();
        let uuid = pad.uuid();
        let guid: String = uuid.iter().map(|b| format!("{b:02x}")).collect();
        let (layout, source) = if pad.mapping_source() == gilrs::MappingSource::SdlMappings {
            let m = pad.map_name().unwrap_or("");
            let from = if env.has(&guid, m) {
                "SDL_GAMECONTROLLERCONFIG"
            } else if user.has(&guid, m) {
                "the user's gamecontrollerdb.txt"
            } else {
                "the SDL database"
            };
            (Layout::Mapped, format!("mapping \"{m}\" from {from}"))
        } else if cfg!(target_os = "windows") && uuid == [0; 16] {
            (Layout::Driver, "no mapping, the system's gamepad layout".to_string())
        } else {
            (Layout::Raw, "no mapping, the default layout (buttons in HID order)".to_string())
        };
        if layout == Layout::Mapped && user.has_guid(&guid) && !source.contains("gamecontrollerdb") {
            log::warn!(
                "gamepads: the gamecontrollerdb.txt line for {guid} is not used: gilrs's own database or \
                 SDL_GAMECONTROLLERCONFIG comes first; put the line in SDL_GAMECONTROLLERCONFIG to replace it"
            );
        }
        // a device without a mapping that has few buttons is no gamepad (a 3D mouse, a pedal set); Linux's backend
        // takes only joysticks and gamepads, and names a joystick's buttons apart from these
        let buttons = [
            Button::South,
            Button::East,
            Button::North,
            Button::West,
            Button::C,
            Button::Z,
            Button::LeftTrigger,
            Button::LeftTrigger2,
            Button::RightTrigger,
            Button::RightTrigger2,
            Button::Select,
            Button::Start,
            Button::Mode,
            Button::LeftThumb,
            Button::RightThumb,
        ]
        .iter()
        .filter(|&&b| pad.button_code(b).is_some())
        .count();
        let gamepad = layout != Layout::Raw || cfg!(target_os = "linux") || buttons >= 4;
        let id = pad.id();
        let i = Info { name, layout, model: pad_layout::model_hash(&[&uuid, pad.name().as_bytes()]), gamepad, told: false };
        log::info!("gamepad {id} \"{}\" connected (GUID {guid}): {source}", i.name);
        if !gamepad {
            log::info!("gamepad {id} \"{}\" not read: {buttons} buttons, no gamepad (an SDL mapping for it makes it one)", i.name);
        }
        i
    }

    /// The motors' state to each player's gamepad (`Rumble`).
    pub fn rumble(&mut self, vib: [[(u8, u8); 2]; 2]) {
        let Some(g) = self.gilrs.as_mut() else { return };
        let ids = [self.slots.dev(0), self.slots.dev(1)];
        self.rumble.update(g, vib, ids);
    }
}

/// A desktop pad's state → pad bits: the d-pad (buttons, or a hat gilrs left as axes) and the left stick; the buttons
/// by name (`Mapped`, `Driver`) or by raw number (`Raw`).
#[cfg(not(target_arch = "wasm32"))]
fn pad_bits(pad: &gilrs::Gamepad, layout: Layout) -> u16 {
    use crate::fight::input::pad::*;
    use gilrs::{Axis, Button};
    let mut v = 0;
    for (b, bit) in [(Button::DPadUp, UP), (Button::DPadDown, DOWN), (Button::DPadLeft, LEFT), (Button::DPadRight, RIGHT)] {
        if pad.is_pressed(b) {
            v |= bit;
        }
    }
    if layout == Layout::Raw {
        for (code, b) in pad.state().buttons() {
            if b.is_pressed() {
                v |= pad_layout::raw_code_bit(code.into_u32());
            }
        }
    } else {
        for (b, bit) in [
            (Button::West, SQUARE),
            (Button::South, CROSS),
            (Button::East, CIRCLE),
            (Button::North, TRIANGLE),
            (Button::RightTrigger, R1),
            (Button::LeftTrigger, L1),
            (Button::RightTrigger2, R2),
            (Button::LeftTrigger2, L2),
            (Button::Start, 0x800),
            (Button::Select, 0x100),
        ] {
            if pad.is_pressed(b) {
                v |= bit;
            }
        }
    }
    // gilrs's y is up
    for (x, y) in [(Axis::LeftStickX, Axis::LeftStickY), (Axis::DPadX, Axis::DPadY)] {
        let (x, y) = (pad.value(x), pad.value(y));
        if x < -0.5 {
            v |= LEFT;
        }
        if x > 0.5 {
            v |= RIGHT;
        }
        if y > 0.5 {
            v |= UP;
        }
        if y < -0.5 {
            v |= DOWN;
        }
    }
    v
}

/// Gamepad rumble from the DualShock motor state: strong = large motor / 255, weak = small on/off;
/// on each player's gamepad (`PadSlots`).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Rumble {
    effects: [Option<gilrs::ff::Effect>; 2],
    last: [((u8, u8), Option<gilrs::GamepadId>); 2],
}

#[cfg(not(target_arch = "wasm32"))]
impl Rumble {
    fn update(&mut self, g: &mut gilrs::Gilrs, vib: [[(u8, u8); 2]; 2], ids: [Option<gilrs::GamepadId>; 2]) {
        use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
        for port in 0..2 {
            let v = (vib[port][0].1, vib[port][1].1);
            if (v, ids[port]) == self.last[port] {
                continue;
            }
            self.last[port] = (v, ids[port]);
            self.effects[port] = None; // dropping an effect stops it
            let Some(id) = ids[port] else { continue };
            if v == (0, 0) || g.connected_gamepad(id).is_none_or(|p| !p.is_ff_supported()) {
                continue;
            }
            let sched = Replay { play_for: Ticks::from_ms(5000), ..Default::default() };
            let strong =
                BaseEffect { kind: BaseEffectType::Strong { magnitude: (v.0 as u32 * 0xFFFF / 0xFF) as u16 }, scheduling: sched, ..Default::default() };
            let weak = BaseEffect { kind: BaseEffectType::Weak { magnitude: if v.1 != 0 { 0xC000 } else { 0 } }, scheduling: sched, ..Default::default() };
            if let Ok(e) = EffectBuilder::new().add_effect(strong).add_effect(weak).gamepads(&[id]).finish(g) {
                let _ = e.play();
                self.effects[port] = Some(e);
            }
        }
    }
}

/// A browser pad as `Pads` last saw it.
#[cfg(target_arch = "wasm32")]
struct WebPad {
    /// the Gamepad API's index and the hash of its id (the browser may give a freed index to another pad)
    key: (u32, u64),
    name: String,
    standard: bool,
    /// the axes seen outside -1..1: hat switches (`pad_layout::web_raw_bits`)
    hats: u32,
    buttons: Vec<bool>,
    axes: Vec<f64>,
    told: bool,
}

/// The gamepads and their players (the web: `navigator.getGamepads()`, read directly: gilrs's web backend hides
/// whether a pad has the standard mapping and cuts a hat switch's values).
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub(super) struct Pads {
    known: Vec<WebPad>,
    slots: PadSlots<(u32, u64)>,
}

#[cfg(target_arch = "wasm32")]
impl Pads {
    pub fn new() -> Self {
        Self::default()
    }

    /// The four players' pad bits as the browser's pads are now (the Gamepad API's four); new presses give players,
    /// all logged.
    pub fn poll(&mut self) -> [u16; SLOTS] {
        use wasm_bindgen::JsCast;
        let Some(list) = web_sys::window().and_then(|w| w.navigator().get_gamepads().ok()) else { return [0; SLOTS] };
        let mut seen = Vec::new();
        for v in list.iter() {
            let Ok(gp) = v.dyn_into::<web_sys::Gamepad>() else { continue };
            if !gp.connected() {
                continue;
            }
            let name = gp.id();
            let key = (gp.index(), pad_layout::model_hash(&[name.as_bytes()]));
            seen.push(key);
            let buttons: Vec<bool> = gp.buttons().iter().map(|b| b.unchecked_into::<web_sys::GamepadButton>().pressed()).collect();
            let axes: Vec<f64> = gp.axes().iter().map(|a| a.as_f64().unwrap_or(0.0)).collect();
            let k = match self.known.iter().position(|p| p.key == key) {
                Some(k) => k,
                None => {
                    let standard = gp.mapping() == web_sys::GamepadMappingType::Standard;
                    let layout = if standard { "the standard layout" } else { "no standard mapping, the default layout (buttons in HID order)" };
                    log::info!("gamepad {} \"{name}\" connected: {layout}", key.0);
                    if let Some(p) = self.slots.reconnected(key) {
                        log::info!("gamepad {} \"{name}\" is player {} again", key.0, p + 1);
                    }
                    self.known.push(WebPad { key, name, standard, hats: 0, buttons: Vec::new(), axes: Vec::new(), told: false });
                    self.known.len() - 1
                }
            };
            let p = &mut self.known[k];
            let pressed = buttons.iter().enumerate().any(|(i, &b)| b && !p.buttons.get(i).copied().unwrap_or(false));
            for (i, &a) in axes.iter().enumerate().take(32) {
                if pad_layout::is_hat_value(a) {
                    p.hats |= 1 << i;
                }
            }
            p.buttons = buttons;
            p.axes = axes;
            if pressed && (0..SLOTS).all(|k| self.slots.dev(k) != Some(key)) {
                match self.slots.player(key, key.1) {
                    Some(n) => log::info!("gamepad {} \"{}\" is player {}", key.0, p.name, n + 1),
                    None if !p.told => log::info!("gamepad {} \"{}\" not read: all four players have a gamepad", key.0, p.name),
                    None => {}
                }
                p.told = true;
            }
        }
        let slots = &mut self.slots;
        self.known.retain(|p| {
            if seen.contains(&p.key) {
                return true;
            }
            match slots.removed(p.key) {
                Some(n) => log::info!("gamepad {} \"{}\" went away (player {} kept for it)", p.key.0, p.name, n + 1),
                None => log::info!("gamepad {} \"{}\" went away", p.key.0, p.name),
            }
            false
        });
        std::array::from_fn(|n| {
            let Some(key) = self.slots.dev(n) else { return 0 };
            let Some(p) = self.known.iter().find(|p| p.key == key) else { return 0 };
            if p.standard { pad_layout::web_standard_bits(&p.buttons, &p.axes) } else { pad_layout::web_raw_bits(&p.buttons, &p.axes, p.hats) }
        })
    }

    /// No rumble in the browser.
    pub fn rumble(&mut self, _vib: [[(u8, u8); 2]; 2]) {}
}
