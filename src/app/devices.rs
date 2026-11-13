//! Keyboard → raw PS1 pad bits; gamepads → players; Android's key codes and joystick axes.
//!
//! Engine addition.

use winit::keyboard::KeyCode;

/// Keyboard → raw PS1 pad bits for player 2 (arrows + numpad).
pub(super) fn key_bit2(k: KeyCode) -> u16 {
    use crate::fight::input::pad::*;
    match k {
        KeyCode::ArrowUp => UP,
        KeyCode::ArrowDown => DOWN,
        KeyCode::ArrowLeft => LEFT,
        KeyCode::ArrowRight => RIGHT,
        KeyCode::Numpad1 => SQUARE,
        KeyCode::Numpad2 => CROSS,
        KeyCode::Numpad3 => CIRCLE,
        KeyCode::Numpad4 => TRIANGLE,
        KeyCode::Numpad5 => R1,
        KeyCode::Numpad6 => L1,
        KeyCode::NumpadEnter => 0x800,
        KeyCode::NumpadDecimal => 0x100,
        _ => 0,
    }
}

/// Keyboard → raw PS1 pad bits for player 1.
pub(super) fn key_bit(k: KeyCode) -> u16 {
    use crate::fight::input::pad::*;
    match k {
        KeyCode::KeyW => UP,
        KeyCode::KeyS => DOWN,
        KeyCode::KeyA => LEFT,
        KeyCode::KeyD => RIGHT,
        KeyCode::KeyJ => SQUARE,
        KeyCode::KeyK => CROSS,
        KeyCode::KeyL => CIRCLE,
        KeyCode::KeyI => TRIANGLE,
        KeyCode::KeyU => R1,
        KeyCode::KeyO => L1,
        KeyCode::Enter => 0x800,
        KeyCode::Backspace => 0x100,
        _ => 0,
    }
}

/// An Android key code (`KeyEvent.getKeyCode()`) → raw pad bits (engine addition, `app::android`): a gamepad's buttons
/// and d-pad, and the desktop's keyboard layout for player 1 (W A S D, J K L I, U O, Enter, Backspace).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(super) fn android_key_bit(code: i32) -> u16 {
    use crate::fight::input::pad::*;
    match code {
        // KEYCODE_DPAD_UP / DOWN / LEFT / RIGHT (also a keyboard's arrows), KEYCODE_W / S / A / D
        19 | 51 => UP,
        20 | 47 => DOWN,
        21 | 29 => LEFT,
        22 | 32 => RIGHT,
        // BUTTON_X (west) / J, BUTTON_A (south) / K / DPAD_CENTER, BUTTON_B (east) / L, BUTTON_Y (north) / I
        99 | 38 => SQUARE,
        96 | 39 | 23 => CROSS,
        97 | 40 => CIRCLE,
        100 | 37 => TRIANGLE,
        // BUTTON_L1 / O, BUTTON_R1 / U, BUTTON_L2, BUTTON_R2
        102 | 43 => L1,
        103 | 49 => R1,
        104 => L2,
        105 => R2,
        // BUTTON_START / ENTER, BUTTON_SELECT / DEL (Backspace)
        108 | 66 => 0x800,
        109 | 67 => 0x100,
        _ => 0,
    }
}

/// An Android joystick's axes (left stick x / y, hat x / y, left / right trigger; y down) → pad bits (`app::android`).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(super) fn android_axes_bits(a: [f32; 6]) -> u16 {
    use crate::fight::input::pad::*;
    let [x, y, hx, hy, lt, rt] = a;
    let mut v = 0;
    if x < -0.5 || hx < -0.5 {
        v |= LEFT;
    }
    if x > 0.5 || hx > 0.5 {
        v |= RIGHT;
    }
    if y < -0.5 || hy < -0.5 {
        v |= UP;
    }
    if y > 0.5 || hy > 0.5 {
        v |= DOWN;
    }
    if lt > 0.5 {
        v |= L2;
    }
    if rt > 0.5 {
        v |= R2;
    }
    v
}

/// Gamepads → players (engine addition; `app::gamepads` on desktop and the web, `app::android`): the first gamepad
/// used (a button pressed) is player 1's (with the keyboard and the touch controls), the second distinct one player
/// 2's, the third and fourth players 3 and 4's (the team battle's pads 3 / 4, `FrameInput::ext`: only its screens read
/// them); a fifth is not read while four are connected. A gamepad keeps its player while connected, and the player
/// stays reserved for it when it goes away, so it gets it back when it comes back under the same id. A gamepad
/// without a player that is used takes, in this order: the player a gamepad of the same model went away from (a pad
/// coming back under a new id, as on macOS), then of players 1 / 2 and only then of 3 / 4: a player never taken, a
/// player whose gamepad went away.
#[derive(Clone, Copy, Debug)]
pub(super) struct PadSlots<K>([Option<Slot<K>>; SLOTS]);

/// The players a gamepad can have (four: the team battle).
pub(super) const SLOTS: usize = 4;

/// A player's gamepad: its id, model (a hash of what identifies the kind of pad; Android: 0) and whether it is there.
#[derive(Clone, Copy, Debug)]
struct Slot<K> {
    dev: K,
    model: u64,
    connected: bool,
}

impl<K> Default for PadSlots<K> {
    fn default() -> Self {
        Self(std::array::from_fn(|_| None))
    }
}

impl<K: Copy + PartialEq> PadSlots<K> {
    /// The player (0 .. 3) of gamepad `dev` (of model `model`) as it is used: its own, or one taken as above; None
    /// while all four players' gamepads are connected.
    pub fn player(&mut self, dev: K, model: u64) -> Option<usize> {
        if let Some(k) = self.reconnected(dev) {
            return Some(k);
        }
        let away = |s: &Option<Slot<K>>, same: bool| s.is_some_and(|s| !s.connected && (!same || s.model == model));
        // players 1 / 2 before 3 / 4 (only the team battle reads those): in a 1v1 a new pad replaces one gone away
        let free = |s: &Self, r: std::ops::Range<usize>| r.clone().find(|&k| s.0[k].is_none()).or_else(|| r.clone().find(|&k| away(&s.0[k], false)));
        let k = (0..SLOTS).find(|&k| away(&self.0[k], true)).or_else(|| free(self, 0..2)).or_else(|| free(self, 2..SLOTS))?;
        self.0[k] = Some(Slot { dev, model, connected: true });
        Some(k)
    }

    /// The player of gamepad `dev` if it has one (also while it is away).
    pub fn player_of(&self, dev: K) -> Option<usize> {
        self.0.iter().position(|s| s.is_some_and(|s| s.dev == dev))
    }

    /// Player `k`'s gamepad while it is connected.
    pub fn dev(&self, k: usize) -> Option<K> {
        self.0[k].filter(|s| s.connected).map(|s| s.dev)
    }

    /// Gamepad `dev` is connected (again, under the same id): its player if it has one.
    pub fn reconnected(&mut self, dev: K) -> Option<usize> {
        let k = self.player_of(dev)?;
        if let Some(s) = &mut self.0[k] {
            s.connected = true;
        }
        Some(k)
    }

    /// Gamepad `dev` went away: the player it had, now kept for it until another gamepad needs it.
    pub fn removed(&mut self, dev: K) -> Option<usize> {
        let k = self.0.iter().position(|s| s.is_some_and(|s| s.dev == dev && s.connected))?;
        if let Some(s) = &mut self.0[k] {
            s.connected = false;
        }
        Some(k)
    }
}

#[cfg(test)]
mod tests {
    use super::{PadSlots, android_axes_bits, android_key_bit};
    use crate::fight::input::pad::*;

    #[test]
    fn android_keys_and_axes() {
        assert_eq!(android_key_bit(96), CROSS);
        assert_eq!(android_key_bit(99), SQUARE);
        assert_eq!(android_key_bit(108), 0x800);
        // the back key is the app's (`app::android`)
        assert_eq!(android_key_bit(4), 0);
        assert_eq!(android_axes_bits([0.0, 0.0, -1.0, 1.0, 0.0, 0.0]), LEFT | DOWN);
        assert_eq!(android_axes_bits([0.7, -0.7, 0.0, 0.0, 0.9, 0.0]), RIGHT | UP | L2);
        assert_eq!(android_axes_bits([0.3, 0.3, 0.0, 0.0, 0.2, 0.2]), 0);
    }

    #[test]
    fn android_gamepads_to_players() {
        let mut s = PadSlots::default();
        // the first gamepad used is player 1's, the second player 2's, the third and fourth players 3 and 4's (the team
        // battle's pads 3 / 4), a fifth none; each keeps its player
        assert_eq!(s.player(7, 0), Some(0));
        assert_eq!(s.player(9, 0), Some(1));
        assert_eq!(s.player(7, 0), Some(0));
        assert_eq!(s.player(12, 0), Some(2));
        assert_eq!(s.player(13, 0), Some(3));
        assert_eq!(s.player(14, 0), None);
        assert_eq!(s.player_of(14), None);
        assert_eq!(s.player(9, 0), Some(1));
        // player 1's going away: player 2's stays, the next new one is player 1's
        assert_eq!(s.removed(7), Some(0));
        assert_eq!(s.removed(7), None);
        assert_eq!(s.removed(3), None);
        assert_eq!(s.player(9, 0), Some(1));
        assert_eq!(s.player(14, 0), Some(0));
        assert_eq!(s.dev(0), Some(14));
    }

    #[test]
    fn gamepads_keep_their_players() {
        const PS5: u64 = 5;
        const XBOX: u64 = 360;
        let mut s = PadSlots::default();
        assert_eq!(s.player('a', PS5), Some(0));
        assert_eq!(s.player('b', XBOX), Some(1));
        // player 1's pad goes away: player 2's stays player 2, and a third pad now gets player 1
        assert_eq!(s.removed('a'), Some(0));
        assert_eq!((s.dev(0), s.dev(1)), (None, Some('b')));
        assert_eq!(s.player('b', XBOX), Some(1));
        // the same pad back under its id: player 1 again, before it is even used
        assert_eq!(s.reconnected('a'), Some(0));
        assert_eq!(s.dev(0), Some('a'));
        // back under a new id (macOS): the player its model left
        assert_eq!(s.removed('a'), Some(0));
        assert_eq!(s.removed('b'), Some(1));
        assert_eq!(s.player('c', XBOX), Some(1));
        assert_eq!(s.player('d', PS5), Some(0));
        assert_eq!(s.player_of('a'), None);
        // no player free: a pad of another model takes the one a pad away left, which then has none
        let mut s = PadSlots::default();
        s.player('a', PS5);
        s.player('b', XBOX);
        s.removed('a');
        // (players 1 / 2 go first: in a 1v1, player 1's new pad of another model is player 1, not 3)
        assert_eq!(s.player('c', 7), Some(0));
        assert_eq!(s.reconnected('a'), None);
        // (the pad back, used: players 1 / 2 held, it takes player 3)
        assert_eq!(s.player('a', PS5), Some(2));
        // a player never taken goes before one kept for a pad away
        let mut s = PadSlots::default();
        s.player('a', PS5);
        s.removed('a');
        assert_eq!(s.player('c', 7), Some(1));
        assert_eq!(s.reconnected('a'), Some(0));
        assert_eq!((s.player('e', 8), s.player('f', 9)), (Some(2), Some(3)));
        assert_eq!(s.player('g', 10), None);
        // players 3 / 4 are taken only once players 1 and 2 are held (connected or kept for a pad away)
        let mut s = PadSlots::default();
        assert_eq!((s.player('a', 1), s.player('b', 2)), (Some(0), Some(1)));
        s.removed('a');
        s.removed('b');
        assert_eq!(s.player('c', 3), Some(0));
        assert_eq!(s.player('d', 4), Some(1));
        assert_eq!(s.player('e', 5), Some(2));
        s.removed('c');
        assert_eq!(s.player('f', 6), Some(0));
        assert_eq!(s.player('g', 7), Some(3));
        assert_eq!(s.player('h', 8), None);
    }
}
