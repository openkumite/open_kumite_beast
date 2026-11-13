//! Gamepads without a mapping: raw button codes in the HID order, the browser's pads; the user's SDL mapping file.
//!
//! Engine addition.
//!
//! The pure half of the gamepads (`docs/gamepads.md`; `app::gamepads` reads the devices): a pad that the SDL
//! database knows comes through gilrs's named buttons; one that it does not know gets the default layout here from
//! its raw button numbers (`hid_button_bit`, the order of a DualShock 4 / DualSense and most generic HID pads), per
//! backend (`mac_code_bit`, `linux_code_bit`, `windows_code_bit`); the browser's pads by the Gamepad API's
//! "standard" layout or by the same raw order (`web_standard_bits`, `web_raw_bits`); and the check of the user's
//! `gamecontrollerdb.txt` (`UserMappings`).

use crate::fight::input::pad::*;

const START: u16 = 0x800;
const SELECT: u16 = 0x100;

/// A gamepad's raw button number (1-based, HID order) → pad bits in the default layout: 1 □, 2 ×, 3 ○, 4 △, 5 L1,
/// 6 R1, 7 L2, 8 R2, 9 SELECT, 10 START; the rest (sticks' clicks, the home button…) nothing.
pub(super) fn hid_button_bit(n: u32) -> u16 {
    match n {
        1 => SQUARE,
        2 => CROSS,
        3 => CIRCLE,
        4 => TRIANGLE,
        5 => L1,
        6 => R1,
        7 => L2,
        8 => R2,
        9 => SELECT,
        10 => START,
        _ => 0,
    }
}

/// macOS (IOKit): a raw code (gilrs `Code::into_u32`, usage page << 16 | usage) → bits; the button page (9) holds the
/// buttons by number.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn mac_code_bit(code: u32) -> u16 {
    if code >> 16 == 9 { hid_button_bit(code & 0xFFFF) } else { 0 }
}

/// Linux (evdev): a raw code (type << 16 | code) → bits. A joystick's buttons (BTN_TRIGGER 0x120…) are numbered as
/// HID's; a gamepad's (BTN_SOUTH 0x130…) are named by the kernel's driver and taken as named, as SDL does.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn linux_code_bit(code: u32) -> u16 {
    if code >> 16 != 1 {
        return 0;
    }
    match code & 0xFFFF {
        c @ 0x120..=0x12F => hid_button_bit(c - 0x11F),
        0x130 => CROSS,
        0x131 => CIRCLE,
        0x133 => TRIANGLE,
        0x134 => SQUARE,
        0x136 => L1,
        0x137 => R1,
        0x138 => L2,
        0x139 => R2,
        0x13A => SELECT,
        0x13B => START,
        _ => 0,
    }
}

/// Windows (Windows.Gaming.Input `RawGameController`): a raw code (kind << 16 | index, buttons kind 0) → bits; the
/// buttons by index from 0.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn windows_code_bit(code: u32) -> u16 {
    if code < 32 { hid_button_bit(code + 1) } else { 0 }
}

/// The current backend's raw code → bits (nothing where gilrs has no backend).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) fn raw_code_bit(code: u32) -> u16 {
    #[cfg(target_os = "macos")]
    return mac_code_bit(code);
    #[cfg(target_os = "linux")]
    return linux_code_bit(code);
    #[cfg(target_os = "windows")]
    return windows_code_bit(code);
    #[allow(unreachable_code)]
    {
        let _ = code;
        0
    }
}

/// A stick's x / y (y down, as the Gamepad API's) → d-pad bits past half way.
fn stick_bits(x: f64, y: f64) -> u16 {
    let mut v = 0;
    if x < -0.5 {
        v |= LEFT;
    }
    if x > 0.5 {
        v |= RIGHT;
    }
    if y < -0.5 {
        v |= UP;
    }
    if y > 0.5 {
        v |= DOWN;
    }
    v
}

/// A browser pad with the Gamepad API's "standard" mapping (buttons pressed, axes) → bits: 0 ×, 1 ○, 2 □, 3 △, 4 L1,
/// 5 R1, 6 L2, 7 R2, 8 SELECT, 9 START, 12..15 the d-pad; axes 0 / 1 the left stick.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn web_standard_bits(buttons: &[bool], axes: &[f64]) -> u16 {
    const BITS: [u16; 16] = [CROSS, CIRCLE, SQUARE, TRIANGLE, L1, R1, L2, R2, SELECT, START, 0, 0, UP, DOWN, LEFT, RIGHT];
    let mut v = 0;
    for (i, _) in buttons.iter().enumerate().filter(|b| *b.1) {
        v |= BITS.get(i).copied().unwrap_or(0);
    }
    v | stick_bits(axes.first().copied().unwrap_or(0.0), axes.get(1).copied().unwrap_or(0.0))
}

/// A browser's hat switch axis (a value -1 + 2·k/7 per direction k from up clockwise; centred outside -1..1) → d-pad
/// bits.
fn hat_bits(v: f64) -> u16 {
    if !(-1.05..=1.05).contains(&v) {
        return 0;
    }
    const DIRS: [u16; 8] = [UP, UP | RIGHT, RIGHT, DOWN | RIGHT, DOWN, DOWN | LEFT, LEFT, UP | LEFT];
    DIRS[(((v + 1.0) * 3.5).round() as usize).min(7)]
}

/// A browser's axis with a value outside -1..1 is a hat switch at rest (`web_raw_bits`).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn is_hat_value(v: f64) -> bool {
    !(-1.05..=1.05).contains(&v)
}

/// A browser pad without the standard mapping (the raw device: buttons in HID order) → bits by the default layout:
/// button i as HID number i + 1; axes 0 / 1 the stick, and the axes in `hats` (a bit per axis, seen outside -1..1)
/// hat switches.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn web_raw_bits(buttons: &[bool], axes: &[f64], hats: u32) -> u16 {
    let mut v = 0;
    for (i, _) in buttons.iter().enumerate().filter(|b| *b.1) {
        v |= hid_button_bit(i as u32 + 1);
    }
    let axis = |i: usize| if hats >> i & 1 == 0 { axes.get(i).copied().unwrap_or(0.0) } else { 0.0 };
    v |= stick_bits(axis(0), axis(1));
    for (i, &a) in axes.iter().enumerate().take(32) {
        if hats >> i & 1 != 0 {
            v |= hat_bits(a);
        }
    }
    v
}

/// 64-bit FNV-1a: a pad's model (its GUID and name) as `PadSlots` compares them.
pub(super) fn model_hash(parts: &[&[u8]]) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    for p in parts {
        for &b in *p {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3);
        }
        h = (h ^ 0xFF).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// SDL's names for a gamepad's buttons and axes (a mapping line's keys; gilrs rejects a line with another).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const SDL_KEYS: [&str; 35] = [
    "a",
    "b",
    "back",
    "c",
    "dpdown",
    "dpleft",
    "dpright",
    "dpup",
    "guide",
    "leftshoulder",
    "leftstick",
    "lefttrigger",
    "leftx",
    "lefty",
    "leftz",
    "misc1",
    "misc2",
    "misc3",
    "misc4",
    "misc5",
    "paddle1",
    "paddle2",
    "paddle3",
    "paddle4",
    "rightshoulder",
    "rightstick",
    "righttrigger",
    "rightx",
    "righty",
    "rightz",
    "start",
    "touchpad",
    "x",
    "y",
    "z",
];

/// The platform name of SDL mapping lines (`platform:` field) on this build, as gilrs compares it.
#[cfg_attr(any(target_arch = "wasm32", target_os = "android"), allow(dead_code))]
pub(super) const SDL_PLATFORM: &str = if cfg!(target_os = "macos") {
    "Mac OS X"
} else if cfg!(target_os = "linux") {
    "Linux"
} else if cfg!(target_os = "windows") {
    "Windows"
} else {
    "Unknown"
};

/// One SDL mapping line checked: its GUID (32 lowercase hex digits; `xinput` as gilrs reads it, all zeros), name,
/// platform, and the line as gilrs gets it (SDL's `crc:` field, which gilrs does not know, left out).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Debug, PartialEq)]
pub(super) struct MappingLine {
    pub guid: String,
    pub name: String,
    pub platform: Option<String>,
    pub line: String,
}

/// An SDL mapping line (`GUID,name,key:value,…`) checked as gilrs parses it; Err: why gilrs would not use it.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(super) fn check_mapping_line(line: &str) -> Result<MappingLine, String> {
    let mut fields = line.split(',');
    let guid = fields.next().unwrap_or("").trim();
    let guid = if guid == "xinput" {
        "0".repeat(32)
    } else {
        let g = guid.replace('-', "").to_ascii_lowercase();
        if g.len() != 32 || !g.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("\"{guid}\" is not a GUID (32 hex digits)"));
        }
        g
    };
    let Some(name) = fields.next() else { return Err("no name after the GUID".into()) };
    let mut platform = None;
    let mut keep = vec![line.split(',').next().unwrap_or("").to_string(), name.to_string()];
    let mut mapped = 0;
    for f in fields {
        if f.is_empty() {
            continue;
        }
        let Some((key, value)) = f.split_once(':') else { return Err(format!("\"{f}\" is not key:value")) };
        if value.contains(':') {
            return Err(format!("\"{f}\" is not key:value"));
        }
        if key == "crc" {
            continue;
        }
        keep.push(f.to_string());
        if value.is_empty() {
            continue;
        }
        if key == "platform" {
            platform = Some(value.to_string());
            continue;
        }
        let k = key.strip_prefix(['+', '-']).unwrap_or(key);
        if SDL_KEYS.binary_search(&k).is_err() {
            return Err(format!("\"{key}\" is not one of SDL's button / axis names"));
        }
        if !sdl_value_ok(value) {
            return Err(format!("\"{f}\": the value is not bN, aN, +aN, -aN (~) or hN.M"));
        }
        mapped += 1;
    }
    if mapped == 0 {
        return Err("no buttons or axes mapped".into());
    }
    Ok(MappingLine { guid, name: name.to_string(), platform, line: keep.join(",") + "," })
}

/// A mapping's value: `bN` a button, `aN` / `+aN` / `-aN` an axis (`~` inverted), `hN.M` a hat's direction.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn sdl_value_ok(v: &str) -> bool {
    let num = |s: &str| !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u16>().is_ok();
    if let Some(n) = v.strip_prefix('b') {
        return num(n);
    }
    if let Some(h) = v.strip_prefix('h') {
        return h.split_once('.').is_some_and(|(a, b)| num(a) && num(b));
    }
    let a = v.strip_prefix(['+', '-']).unwrap_or(v);
    a.strip_prefix('a').is_some_and(|n| num(n.strip_suffix('~').unwrap_or(n)))
}

/// The user's SDL mapping file (`gamecontrollerdb.txt` beside the save) checked: the lines for this platform as
/// gilrs gets them, their GUIDs and names (to tell which mapping a pad got), the lines for other platforms (left out,
/// counted) and the lines gilrs would not use (line number, why).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Debug, Default)]
pub(super) struct UserMappings {
    pub text: String,
    pub pads: Vec<(String, String)>,
    pub other_platforms: usize,
    pub errors: Vec<(usize, String)>,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
impl UserMappings {
    /// `text` (the file; a byte order mark, blank lines and `#` comments skipped) for platform `platform`.
    pub fn parse(text: &str, platform: &str) -> Self {
        let mut m = Self::default();
        for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match check_mapping_line(line) {
                Ok(l) if l.platform.as_deref().is_some_and(|p| p != platform) => m.other_platforms += 1,
                Ok(l) => {
                    m.text.push_str(&l.line);
                    m.text.push('\n');
                    m.pads.push((l.guid, l.name));
                }
                Err(e) => m.errors.push((i + 1, e)),
            }
        }
        m
    }

    /// Whether this file holds the mapping named `name` for GUID `guid` (32 lowercase hex digits).
    pub fn has(&self, guid: &str, name: &str) -> bool {
        self.pads.iter().any(|(g, n)| g == guid && n == name)
    }

    /// Whether this file has a line for GUID `guid`.
    pub fn has_guid(&self, guid: &str) -> bool {
        self.pads.iter().any(|(g, _)| g == guid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_codes_in_hid_order() {
        // macOS: button page usages 1.. ; other pages (the hat, the triggers' axes) nothing
        assert_eq!(mac_code_bit(9 << 16 | 1), SQUARE);
        assert_eq!(mac_code_bit(9 << 16 | 2), CROSS);
        assert_eq!(mac_code_bit(9 << 16 | 4), TRIANGLE);
        assert_eq!(mac_code_bit(9 << 16 | 8), R2);
        assert_eq!(mac_code_bit(9 << 16 | 10), START);
        assert_eq!(mac_code_bit(9 << 16 | 13), 0);
        assert_eq!(mac_code_bit(1 << 16 | 0x39), 0);
        assert_eq!(mac_code_bit(2 << 16 | 0xC4), 0);
        // Linux: a joystick's BTN_TRIGGER.. by number, a gamepad's BTN_SOUTH.. by the kernel's names
        assert_eq!(linux_code_bit(1 << 16 | 0x120), SQUARE);
        assert_eq!(linux_code_bit(1 << 16 | 0x123), TRIANGLE);
        assert_eq!(linux_code_bit(1 << 16 | 0x129), START);
        assert_eq!(linux_code_bit(1 << 16 | 0x130), CROSS);
        assert_eq!(linux_code_bit(1 << 16 | 0x134), SQUARE);
        assert_eq!(linux_code_bit(1 << 16 | 0x13B), START);
        assert_eq!(linux_code_bit(1 << 16 | 0x220), 0);
        assert_eq!(linux_code_bit(3 << 16 | 0x120), 0);
        // Windows: raw buttons by index from 0; gilrs's d-pad codes (u32::MAX - 3..) nothing
        assert_eq!(windows_code_bit(0), SQUARE);
        assert_eq!(windows_code_bit(5), R1);
        assert_eq!(windows_code_bit(9), START);
        assert_eq!(windows_code_bit(u32::MAX - 3), 0);
        assert_eq!(windows_code_bit(1 << 16), 0);
    }

    #[test]
    fn browser_pads() {
        let mut b = [false; 17];
        b[0] = true;
        b[9] = true;
        b[14] = true;
        assert_eq!(web_standard_bits(&b, &[0.0, 0.8, 0.0, 0.0]), CROSS | START | LEFT | DOWN);
        // the same buttons raw: index 0 is HID button 1 (□), 9 is 10 (START), 14 nothing
        assert_eq!(web_raw_bits(&b, &[0.0, 0.0], 0), SQUARE | START);
        assert_eq!(web_raw_bits(&[false, true, false, true], &[-0.9, -0.9], 0), CROSS | TRIANGLE | LEFT | UP);
        // a hat at axis 9: centred (Chrome's 9/7) nothing, -1 up, 3/7 down-right… ; its axes are no stick
        let mut axes = [0.0; 10];
        axes[9] = 9.0 / 7.0;
        assert!(is_hat_value(axes[9]) && !is_hat_value(-1.0));
        assert_eq!(web_raw_bits(&[], &axes, 1 << 9), 0);
        axes[9] = -1.0;
        assert_eq!(web_raw_bits(&[], &axes, 1 << 9), UP);
        axes[9] = -1.0 + 2.0 * 3.0 / 7.0;
        assert_eq!(web_raw_bits(&[], &axes, 1 << 9), DOWN | RIGHT);
        axes[9] = 1.0;
        assert_eq!(web_raw_bits(&[], &axes, 1 << 9), UP | LEFT);
        // axis 0 as a hat: up, not the stick's left
        assert_eq!(web_raw_bits(&[], &[-1.0, 0.0], 1), UP);
    }

    #[test]
    fn mapping_lines() {
        let ps =
            "030000004c050000e60c000000010000,PS5 Controller,a:b1,b:b2,x:b0,y:b3,leftx:a0,lefty:a1,-leftx:-a0,dpup:h0.1,lefttrigger:+a3~,platform:Mac OS X,";
        let l = check_mapping_line(ps).unwrap();
        assert_eq!(l.guid, "030000004c050000e60c000000010000");
        assert_eq!(l.name, "PS5 Controller");
        assert_eq!(l.platform.as_deref(), Some("Mac OS X"));
        assert_eq!(l.line, ps);
        // `xinput`, SDL's crc left out
        let l = check_mapping_line("xinput,XInput Controller,a:b0,crc:1234,").unwrap();
        assert_eq!(l.guid, "0".repeat(32));
        assert_eq!(l.line, "xinput,XInput Controller,a:b0,");
        let err = |s: &str| check_mapping_line(s).unwrap_err();
        assert!(err("0300004c05,Pad,a:b0").contains("GUID"));
        assert!(err("030000004c050000e60c000000010000").contains("name"));
        assert!(err("030000004c050000e60c000000010000,Pad,a=b0").contains("key:value"));
        assert!(err("030000004c050000e60c000000010000,Pad,jump:b0").contains("SDL"));
        assert!(err("030000004c050000e60c000000010000,Pad,a:x0").contains("value"));
        assert!(err("030000004c050000e60c000000010000,Pad,dpup:h0").contains("value"));
        assert!(err("030000004c050000e60c000000010000,Pad,platform:Linux").contains("no buttons"));
    }

    #[test]
    fn mapping_file() {
        let text = "\u{feff}# my pads\n\n\
            03000000aaaa0000bbbb000000000000,Pad A,a:b0,b:b1,platform:Mac OS X,\n\
            03000000aaaa0000bbbb000000000000,Pad A,a:b0,b:b1,platform:Linux,\n\
            03000000cccc0000dddd000000000000,Pad B,a:b2,\n\
            nonsense\n";
        let m = UserMappings::parse(text, "Mac OS X");
        assert_eq!(m.other_platforms, 1);
        assert_eq!(m.errors.len(), 1);
        assert_eq!(m.errors[0].0, 6);
        assert_eq!(m.pads.len(), 2);
        assert!(m.has("03000000aaaa0000bbbb000000000000", "Pad A"));
        assert!(!m.has("03000000aaaa0000bbbb000000000000", "Pad B"));
        assert!(m.has_guid("03000000cccc0000dddd000000000000"));
        assert_eq!(m.text.lines().count(), 2);
    }

    #[test]
    fn models() {
        assert_eq!(model_hash(&[b"ab", b"c"]), model_hash(&[b"ab", b"c"]));
        assert_ne!(model_hash(&[b"ab", b"c"]), model_hash(&[b"a", b"bc"]));
    }
}
