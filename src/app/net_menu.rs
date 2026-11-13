//! The NETWORK menu (the mode menu's NETWORK): HOST, JOIN, SETTINGS, the code and address entry, the connect.
//!
//! Engine addition.
//!
//! The NETWORK menu (engine addition, `docs/network_menu.md`): HOST, JOIN, SETTINGS and EXIT; JOIN asks for the
//! host's room code (or, by direct IP, its address) on an on-screen character picker (the pads, the touch controls
//! and gamepads drive it: no OS keyboard); SETTINGS edits the connection (ROOM CODE through a matchbox signaling
//! server, or DIRECT IP over UDP), the signaling server, the STUN / TURN server (none by default: nothing is sent to
//! a third party unless the player sets one), the direct-IP host's port and the input delay. HOST and JOIN hand the
//! app a `Cmd`; the connect's screen (the room code or the addresses, where the connect is, △ to cancel) and its
//! messages (a failure, the other player leaving) are views of the page. Built in the list menus' style
//! (`menus::frame`, the scrolling banner, menu font palettes 0 / 1 / 3, the cursor `>`) with their sounds (0x0A move,
//! 0x0B value, 0x0C confirm, 0x0D back), as the replay list (`replay_menu`). The page and its settings (`NetPrefs`,
//! `net.json` / localStorage) live outside the game state: the session waits on the mode menu while it is up, and
//! nothing of it reaches a replay, a save or the netplay handshake.

use crate::assets::text;
use crate::assets::text::Label::{self, Game, Port};
use crate::fight::input::pad::{DOWN, LEFT, RIGHT, START, TRIANGLE, UP};
use crate::frontend::Pads;
use crate::output::font::Fonts;
use crate::output::prim::{self, Prim};
use serde::{Deserialize, Serialize};

/// □ × ○ (the list menus' confirm)
const CONFIRM: u16 = 0xE0;
/// Menu font palettes: highlighted, normal, orange.
const HL: u16 = 0;
const NORMAL: u16 = 1;
const ORANGE: u16 = 3;
/// The frame's colour set (`menus::COLOURS`: PLAY DATA's).
const STYLE: usize = 5;
/// Banner: the text width (112) + 64, as the list menus'.
const BANNER: (&str, i32) = ("NETWORK", 176);
/// A view's key help (8×8 menu font), above the frame's legend and clear of its emblem.
const HELP_Y: i32 = 376;
/// The value column of SETTINGS, and the characters of a value shown there (8×8 menu font).
const VAL_X: i32 = 296;
const VAL_CHARS: usize = 16;

/// This build's ways to connect: rooms (WebRTC through a signaling server) and direct IP (UDP: not in the browser).
pub const ROOMS: bool = cfg!(feature = "netplay-webrtc");
pub const DIRECT: bool = cfg!(not(target_arch = "wasm32"));
/// The longest text setting (`net::webrtc`'s URL cap).
const MAX_TEXT: usize = 256;
/// The direct-IP host's default port.
pub const DEFAULT_PORT: u16 = 7000;
/// The longest room code the picker takes (`net::webrtc::MAX_ROOM` allows 16; the host's are `CODE_LEN`).
const MAX_CODE: usize = 8;
/// A host's room code: this many characters of `CODE_SET`.
#[cfg_attr(not(feature = "netplay-webrtc"), allow(dead_code))]
pub const CODE_LEN: usize = 4;
/// Room codes: letters and digits that cannot be taken for each other (no 0 / O / Q / D-like pairs, 1 / I / L,
/// 2 / Z, 5 / S, 8 / B, 6 / G).
pub const CODE_SET: &[u8] = b"ACEFHJKMNPRTUVWXY3479";
/// A direct-IP address (IPv4 and the port), a port, and the text settings' characters (the letters typed in the
/// case SHIFT shows).
const ADDR_SET: &[u8] = b"0123456789.:";
const PORT_SET: &[u8] = b"0123456789";
const TEXT_SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.:/-_@";

/// The NETWORK settings (`net.json` beside the save, localStorage `okb_net` in the browser, moved once from `br2_net`): this app's own, never
/// part of the game state, a replay or the handshake.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetPrefs {
    /// HOST / JOIN by direct IP (UDP) rather than by room code
    pub direct: bool,
    /// the matchbox signaling server (`ws://HOST:PORT`); empty: not set
    pub signal: String,
    /// a STUN server (`stun:HOST:PORT`); empty: none
    pub stun: String,
    /// a TURN server (`turn:HOST:PORT`) with its user and password; empty: none
    pub turn: String,
    pub turn_user: String,
    pub turn_pass: String,
    /// the UDP port a direct-IP host listens on
    pub port: u16,
    /// the address the last direct-IP JOIN went to
    pub join: String,
    /// the input delay in frames (1..=`net::MAX_DELAY`); 0 = AUTO, the host's choice from the round trip
    pub delay: u32,
}

impl Default for NetPrefs {
    fn default() -> Self {
        Self {
            direct: !ROOMS,
            signal: String::new(),
            stun: String::new(),
            turn: String::new(),
            turn_user: String::new(),
            turn_pass: String::new(),
            port: DEFAULT_PORT,
            join: String::new(),
            delay: 0,
        }
    }
}

impl NetPrefs {
    /// The stored text (`platform::storage::read_net_prefs`); defaults when missing or unreadable, each value
    /// brought into range (a hand-edited file).
    pub fn from_stored(text: Option<String>) -> Self {
        let mut p: Self = text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for s in [&mut p.signal, &mut p.stun, &mut p.turn, &mut p.turn_user, &mut p.turn_pass, &mut p.join] {
            if s.len() > MAX_TEXT || s.chars().any(char::is_control) {
                s.clear();
            }
        }
        p.direct = (p.direct || !ROOMS) && DIRECT;
        if p.port == 0 {
            p.port = DEFAULT_PORT;
        }
        if p.delay > crate::net::MAX_DELAY {
            p.delay = 0;
        }
        p
    }

    pub fn to_stored(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }

    /// The input delay asked for (None: AUTO).
    pub fn delay(&self) -> Option<u32> {
        (self.delay > 0).then_some(self.delay)
    }

    /// A way to connect is set up: by room code a signaling server is needed.
    pub fn ready(&self) -> Result<(), &'static str> {
        if self.direct || !self.signal.trim().is_empty() { Ok(()) } else { Err("SET THE SIGNALING SERVER IN SETTINGS FIRST.") }
    }
}

/// A host's room code from `seed` (`CODE_LEN` characters of `CODE_SET`).
#[cfg_attr(not(feature = "netplay-webrtc"), allow(dead_code))]
pub fn room_code(mut seed: u64) -> String {
    (0..CODE_LEN)
        .map(|_| {
            // splitmix64: every character from fresh bits
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            CODE_SET[((z ^ (z >> 31)) % CODE_SET.len() as u64) as usize] as char
        })
        .collect()
}

/// What the app must do after a frame of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    None,
    /// back to the mode menu
    Close,
    /// HOST with the settings as they are
    Host,
    /// JOIN this room code, or by direct IP this address
    Join(String),
    /// the connect under way is cancelled (△ on its screen)
    Cancel,
    /// the settings changed: store them
    Prefs(NetPrefs),
}

/// A setting or an entry the picker edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Signal,
    Stun,
    Turn,
    TurnUser,
    TurnPass,
    Port,
    /// JOIN's room code / address (not a setting)
    Code,
    Addr,
}

/// SETTINGS' rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Connection,
    Edit(Field),
    Delay,
    Exit,
}

/// The rows of this build's SETTINGS: CONNECTION when it has both ways, the room's servers when it has rooms, the
/// port when it has direct IP.
fn settings_rows() -> Vec<Row> {
    let mut v = Vec::new();
    if ROOMS && DIRECT {
        v.push(Row::Connection);
    }
    if ROOMS {
        v.extend([Field::Signal, Field::Stun, Field::Turn, Field::TurnUser, Field::TurnPass].map(Row::Edit));
    }
    if DIRECT {
        v.push(Row::Edit(Field::Port));
    }
    v.extend([Row::Delay, Row::Exit]);
    v
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Signal => "SIGNALING SERVER",
            Field::Stun => "STUN SERVER",
            Field::Turn => "TURN SERVER",
            Field::TurnUser => "TURN USER",
            Field::TurnPass => "TURN PASSWORD",
            Field::Port => "HOST PORT",
            Field::Code => "ROOM CODE",
            Field::Addr => "HOST ADDRESS",
        }
    }

    /// The picker's characters, the longest text, the letters typed in lower case first.
    fn picker(self) -> (&'static [u8], usize, bool) {
        match self {
            Field::Signal | Field::Stun | Field::Turn => (TEXT_SET, MAX_TEXT, true),
            Field::TurnUser | Field::TurnPass => (TEXT_SET, MAX_TEXT, false),
            Field::Port => (PORT_SET, 5, false),
            Field::Code => (CODE_SET, MAX_CODE, false),
            Field::Addr => (ADDR_SET, 21, false),
        }
    }

    /// What the text first typed is (a scheme a URL setting needs, when empty).
    fn start(self) -> &'static str {
        match self {
            Field::Signal => "ws://",
            Field::Stun => "stun:",
            Field::Turn => "turn:",
            _ => "",
        }
    }

    fn get(self, p: &NetPrefs) -> String {
        match self {
            Field::Signal => p.signal.clone(),
            Field::Stun => p.stun.clone(),
            Field::Turn => p.turn.clone(),
            Field::TurnUser => p.turn_user.clone(),
            Field::TurnPass => p.turn_pass.clone(),
            Field::Port => p.port.to_string(),
            Field::Code => String::new(),
            Field::Addr => p.join.clone(),
        }
    }

    /// The edited text into `p`; Err with why it does not check out (the edit stays open).
    fn set(self, p: &mut NetPrefs, text: &str) -> Result<(), String> {
        let t = text.trim().to_string();
        // a scheme alone is no server
        let t = if t == self.start() { String::new() } else { t };
        #[cfg(feature = "netplay-webrtc")]
        let check = |r: Result<String, String>| r.map(|_| ());
        match self {
            #[cfg(feature = "netplay-webrtc")]
            Field::Signal if !t.is_empty() => check(crate::net::webrtc::signal_url(&t))?,
            #[cfg(feature = "netplay-webrtc")]
            Field::Stun if !t.is_empty() => crate::net::webrtc::Room::new("ws://x", "A").and_then(|r| r.with_ice(Some(&t), None)).map(|_| ())?,
            #[cfg(feature = "netplay-webrtc")]
            Field::Turn if !t.is_empty() => crate::net::webrtc::Room::new("ws://x", "A").and_then(|r| r.with_ice(None, Some((&t, "u", "p")))).map(|_| ())?,
            Field::Port => match t.parse::<u16>() {
                Ok(n) if n > 0 => {}
                _ => return Err("A PORT: 1 TO 65535.".into()),
            },
            _ => {}
        }
        match self {
            Field::Signal => p.signal = t,
            Field::Stun => p.stun = t,
            Field::Turn => p.turn = t,
            Field::TurnUser => p.turn_user = t,
            Field::TurnPass => p.turn_pass = t,
            Field::Port => p.port = t.parse().unwrap_or(DEFAULT_PORT),
            Field::Code | Field::Addr => {}
        }
        Ok(())
    }
}

/// The width of the picker's SHIFT, DEL and OK cells.
const SPECIAL_W: i32 = 96;

/// A picker cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Char(u8),
    Shift,
    Del,
    Ok,
}

/// The on-screen character picker (engine addition, for pads and the touch controls): a grid of the field's
/// characters with SHIFT (letters' case, where they are typed), DEL and OK; the d-pad moves (wrapping), □ × ○
/// enter the cell, △ deletes the last character (on an empty text: cancels), START is OK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picker {
    pub buf: String,
    set: &'static [u8],
    max: usize,
    /// letters typed in lower case (SHIFT); None: the set has no letters
    lower: Option<bool>,
    /// the cell under the cursor
    cur: usize,
    cols: usize,
}

impl Picker {
    fn new(text: &str, set: &'static [u8], max: usize, lower: bool) -> Self {
        let letters = set.iter().any(u8::is_ascii_alphabetic) && set != CODE_SET;
        let buf: String = text.chars().filter(|c| set.contains(&(c.to_ascii_uppercase() as u8)) && c.is_ascii()).take(max).collect();
        let cols = match set.len() {
            n if n > 24 => 14,
            n if n > 12 => 11,
            n => n,
        };
        Self { buf, set, max, lower: letters.then_some(lower), cur: 0, cols }
    }

    fn cells(&self) -> Vec<Cell> {
        let mut v: Vec<Cell> = self.set.iter().map(|&c| Cell::Char(c)).collect();
        // the specials on a row of their own
        while !v.len().is_multiple_of(self.cols) {
            v.push(Cell::Char(0));
        }
        if self.lower.is_some() {
            v.push(Cell::Shift);
        }
        v.extend([Cell::Del, Cell::Ok]);
        v
    }

    /// One frame: Some(Some(text)) at OK, Some(None) on △ with nothing typed.
    fn tick(&mut self, rep: u16, press: u16, sounds: &mut Vec<u16>) -> Option<Option<String>> {
        let cells = self.cells();
        let n = cells.len();
        let rows = n.div_ceil(self.cols);
        // the specials' row: its cells are `SPECIAL_W` wide (this many grid columns each), so there a column is a
        // special's index, and UP / DOWN go to the characters drawn above or below it
        let (srow, span) = (rows - 1, (SPECIAL_W / 32) as usize);
        let ns = n - srow * self.cols;
        let (mut r, mut c) = (self.cur / self.cols, self.cur % self.cols);
        let width = if r == srow { ns } else { self.cols };
        let moved = rep & (UP | DOWN | LEFT | RIGHT) != 0;
        if rep & RIGHT != 0 {
            c = (c + 1) % width;
        }
        if rep & LEFT != 0 {
            c = (c + width - 1) % width;
        }
        let r0 = r;
        if rep & DOWN != 0 {
            r = (r + 1) % rows;
        }
        if rep & UP != 0 {
            r = (r + rows - 1) % rows;
        }
        if r != r0 && r == srow {
            c = (c / span).min(ns - 1);
        } else if r != r0 && r0 == srow {
            c = (c * span + span / 2).min(self.cols - 1);
        }
        if moved {
            sounds.push(0x0A);
            let mut k = (r * self.cols + c).min(n - 1);
            // the blank cells after the last character: the row's last real one
            while cells[k] == Cell::Char(0) {
                k -= 1;
            }
            self.cur = k;
        }
        if press & START != 0 {
            sounds.push(0x0C);
            return Some(Some(self.buf.clone()));
        }
        if press & TRIANGLE != 0 {
            sounds.push(0x0D);
            if self.buf.pop().is_none() {
                return Some(None);
            }
            return None;
        }
        if press & CONFIRM == 0 {
            return None;
        }
        match cells[self.cur] {
            Cell::Char(0) => {}
            Cell::Char(ch) => {
                if self.buf.len() < self.max {
                    sounds.push(0x0C);
                    let ch = if self.lower == Some(true) { ch.to_ascii_lowercase() } else { ch };
                    self.buf.push(ch as char);
                } else {
                    sounds.push(0x0D);
                }
            }
            Cell::Shift => {
                sounds.push(0x0B);
                self.lower = self.lower.map(|l| !l);
            }
            Cell::Del => {
                sounds.push(0x0D);
                self.buf.pop();
            }
            Cell::Ok => {
                sounds.push(0x0C);
                return Some(Some(self.buf.clone()));
            }
        }
        None
    }

    /// The grid from `y` (menu font, 32 px a cell), the cell under the cursor on a darkened box in palette 0.
    fn draw(&self, f: &Fonts, out: &mut Vec<Prim>, y: i32) {
        let x0 = 256 - (self.cols as i32 * 32) / 2;
        for (k, cell) in self.cells().into_iter().enumerate() {
            let (r, c) = ((k / self.cols) as i32, (k % self.cols) as i32);
            let hl = k == self.cur;
            let (label, w) = match cell {
                Cell::Char(0) => continue,
                Cell::Char(ch) => ((ch as char).to_string(), 32),
                // the menu font's lower-case cells are pad icons: the case by name
                Cell::Shift => (if self.lower == Some(true) { "LOWER" } else { "UPPER" }.to_string(), SPECIAL_W),
                Cell::Del => ("DEL".into(), SPECIAL_W),
                Cell::Ok => (text::get("ok").into(), SPECIAL_W),
            };
            // the specials on the last row, wider
            let specials = self.set.len().div_ceil(self.cols) * self.cols;
            let x = if matches!(cell, Cell::Char(_)) { x0 + 32 * c } else { x0 + SPECIAL_W * (k - specials) as i32 };
            let yy = y + 28 * r;
            if hl {
                out.push(prim::f4(x, yy - 4, w - 2, 24, [96, 96, 96]).blend(crate::output::sprite::Blend::Sub));
            }
            let tw = f.menu_width(&label);
            f.menu_text(out, &label, x + (w - 2 - tw) / 2, yy, if hl { HL } else { NORMAL });
        }
    }
}

/// The connect's screen: what runs (HOST / JOIN), what the other player needs shown big (the room code, or this
/// machine's address) under its label, the lines under it, where the connect is (the app's, every frame).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conn {
    pub title: &'static str,
    pub big: Option<(&'static str, String)>,
    pub info: Vec<String>,
    pub status: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum View {
    /// HOST JOIN SETTINGS EXIT (the cursor's row)
    Main(usize),
    /// SETTINGS (the cursor's row of `settings_rows`)
    Settings(usize),
    /// a setting or JOIN's code / address in the picker; why the last OK was refused
    Edit(Field, Picker, Option<String>),
    /// the connect under way
    Connecting,
    /// a message (its title and text): any button goes back to the main rows (`NetMenu::row`)
    Message(String, String),
}

/// The NETWORK menu.
pub struct NetMenu {
    pub prefs: NetPrefs,
    view: View,
    pub conn: Conn,
    /// the main rows' cursor as last left (where a message or a cancelled connect goes back to)
    row: usize,
    banner_x: i32,
    frame: u32,
    pads: Pads,
}

const MAIN: [Label; 4] = [Port("HOST"), Port("JOIN"), Port("SETTINGS"), Game("exit")];

impl NetMenu {
    /// The page with `prefs`, the cursor on HOST; `held` = the pads as it opens (the press that opened it is not a
    /// new one).
    pub fn new(prefs: NetPrefs, held: [u16; 2]) -> Self {
        let mut pads = Pads::default();
        pads.update(held);
        Self { prefs, view: View::Main(0), conn: Conn::default(), row: 0, banner_x: 0, frame: 0, pads }
    }

    /// The connect's screen up (`conn` filled by the app).
    pub fn connecting(&mut self, conn: Conn) {
        self.conn = conn;
        self.view = View::Connecting;
    }

    /// A message over the main rows (a failed connect, a session that ended); the cursor goes back where it was.
    pub fn message(&mut self, title: &str, text: &str) {
        self.view = View::Message(title.into(), text.into());
    }

    /// One 60 Hz frame with both pads; the sounds (`play(0x400, id)`) go into `sounds`.
    pub fn tick(&mut self, raw: [u16; 2], sounds: &mut Vec<u16>) -> Cmd {
        self.frame = self.frame.wrapping_add(1);
        self.pads.update(raw);
        self.banner_x -= 4;
        while self.banner_x < -BANNER.1 {
            self.banner_x += BANNER.1;
        }
        let (press, rep) = (self.pads.any_press(), self.pads.any_rep());
        let step = |n: usize, c: usize, sounds: &mut Vec<u16>| {
            if rep & DOWN != 0 {
                sounds.push(0x0A);
                (c + 1) % n
            } else if rep & UP != 0 {
                sounds.push(0x0A);
                (c + n - 1) % n
            } else {
                c
            }
        };
        match self.view.clone() {
            View::Main(c) => {
                if press & TRIANGLE != 0 {
                    sounds.push(0x0D);
                    return Cmd::Close;
                }
                let c = step(MAIN.len(), c, sounds);
                self.view = View::Main(c);
                self.row = c;
                if press & CONFIRM == 0 {
                    return Cmd::None;
                }
                match c {
                    0 | 1 => {
                        if let Err(why) = self.prefs.ready() {
                            sounds.push(0x0D);
                            self.message(MAIN[c].text(), why);
                            return Cmd::None;
                        }
                        sounds.push(0x0C);
                        if c == 0 {
                            return Cmd::Host;
                        }
                        let f = if self.prefs.direct { Field::Addr } else { Field::Code };
                        let (set, max, lower) = f.picker();
                        self.view = View::Edit(f, Picker::new(&f.get(&self.prefs), set, max, lower), None);
                        Cmd::None
                    }
                    2 => {
                        sounds.push(0x0C);
                        self.view = View::Settings(0);
                        Cmd::None
                    }
                    _ => {
                        sounds.push(0x0D);
                        Cmd::Close
                    }
                }
            }
            View::Settings(c) => {
                let rows = settings_rows();
                if press & TRIANGLE != 0 {
                    sounds.push(0x0D);
                    self.view = View::Main(2);
                    return Cmd::None;
                }
                let c = step(rows.len(), c, sounds);
                self.view = View::Settings(c);
                let before = self.prefs.clone();
                match rows[c] {
                    Row::Connection if rep & (LEFT | RIGHT) != 0 || press & CONFIRM != 0 => {
                        sounds.push(0x0B);
                        self.prefs.direct = !self.prefs.direct;
                    }
                    Row::Delay if rep & (LEFT | RIGHT) != 0 => {
                        sounds.push(0x0B);
                        let n = crate::net::MAX_DELAY + 1;
                        self.prefs.delay = if rep & RIGHT != 0 { (self.prefs.delay + 1) % n } else { (self.prefs.delay + n - 1) % n };
                    }
                    Row::Edit(Field::Port) if rep & (LEFT | RIGHT) != 0 => {
                        sounds.push(0x0B);
                        self.prefs.port = if rep & RIGHT != 0 { self.prefs.port.checked_add(1).unwrap_or(1) } else { self.prefs.port.saturating_sub(1).max(1) };
                    }
                    Row::Edit(f) if press & CONFIRM != 0 => {
                        sounds.push(0x0C);
                        let (set, max, lower) = f.picker();
                        let text = f.get(&self.prefs);
                        let text = if text.is_empty() { f.start().to_string() } else { text };
                        self.view = View::Edit(f, Picker::new(&text, set, max, lower), None);
                    }
                    Row::Exit if press & CONFIRM != 0 => {
                        sounds.push(0x0D);
                        self.view = View::Main(2);
                    }
                    _ => {}
                }
                if self.prefs != before { Cmd::Prefs(self.prefs.clone()) } else { Cmd::None }
            }
            View::Edit(f, mut p, why) => {
                let back = |f: Field| match f {
                    Field::Code | Field::Addr => View::Main(1),
                    _ => View::Settings(settings_rows().iter().position(|r| *r == Row::Edit(f)).unwrap_or(0)),
                };
                match p.tick(rep, press, sounds) {
                    None => {
                        self.view = View::Edit(f, p, why);
                        Cmd::None
                    }
                    Some(None) => {
                        self.view = back(f);
                        Cmd::None
                    }
                    Some(Some(text)) => match f {
                        Field::Code | Field::Addr if text.trim().is_empty() => {
                            self.view = View::Edit(f, p, Some("NOTHING TYPED YET.".into()));
                            Cmd::None
                        }
                        Field::Code => Cmd::Join(text),
                        Field::Addr => {
                            self.prefs.join = text.clone();
                            Cmd::Join(text)
                        }
                        _ => match f.set(&mut self.prefs, &text) {
                            Ok(()) => {
                                self.view = back(f);
                                Cmd::Prefs(self.prefs.clone())
                            }
                            Err(e) => {
                                self.view = View::Edit(f, p, Some(e.to_uppercase()));
                                Cmd::None
                            }
                        },
                    },
                }
            }
            View::Connecting => {
                if press & TRIANGLE != 0 {
                    sounds.push(0x0D);
                    self.view = View::Main(self.row);
                    return Cmd::Cancel;
                }
                Cmd::None
            }
            View::Message(..) => {
                if press & (CONFIRM | TRIANGLE | START) != 0 {
                    sounds.push(0x0D);
                    self.view = View::Main(self.row);
                }
                Cmd::None
            }
        }
    }

    /// The main rows' note: how HOST and JOIN connect.
    fn how(&self) -> Vec<String> {
        let p = &self.prefs;
        let mut v = Vec::new();
        if p.direct {
            v.push(format!("BY DIRECT IP (UDP), HOST PORT {}", p.port));
            v.push("THE OTHER PLAYER NEEDS YOUR ADDRESS.".into());
        } else if p.signal.trim().is_empty() {
            v.push("BY ROOM CODE: NO SIGNALING SERVER SET.".into());
            v.push("SET ONE IN SETTINGS.".into());
        } else {
            v.push("BY ROOM CODE THROUGH".into());
            v.push(cut(&p.signal, 50));
        }
        let ice = if p.stun.is_empty() && p.turn.is_empty() { "NONE: THIS NETWORK ONLY".to_string() } else { "SET".into() };
        if !p.direct {
            v.push(format!("STUN / TURN {ice}"));
        }
        v.push(format!("INPUT DELAY {}", delay_label(p.delay)));
        v
    }

    /// The page: the frame, the banner, then the view.
    pub fn draw(&self, f: &Fonts, out: &mut Vec<Prim>) {
        crate::frontend::menus::frame(out, f, STYLE);
        crate::frontend::menus::banner(f, out, BANNER.0, BANNER.1, self.banner_x, 48, 7);
        let title = match &self.view {
            View::Settings(_) => "SETTINGS",
            View::Edit(Field::Code | Field::Addr, ..) => "JOIN",
            View::Edit(..) => "SETTINGS",
            View::Connecting => self.conn.title,
            _ => BANNER.0,
        };
        f.menu_text(out, title, 48, 48, HL);
        let row = |out: &mut Vec<Prim>, label: &str, y: i32, hl: bool| {
            if hl {
                f.menu_text(out, ">", 32, y, HL);
            }
            f.menu_text(out, label, 48, y, if hl { HL } else { NORMAL });
        };
        match &self.view {
            View::Main(c) => {
                for (k, l) in MAIN.iter().enumerate() {
                    row(out, l.text(), 80 + 24 * k as i32, k == *c);
                }
                for (k, l) in self.how().iter().enumerate() {
                    f.menu_small(out, l, 48, 200 + 14 * k as i32, NORMAL);
                }
            }
            View::Settings(c) => {
                let mut y = 80;
                for (k, r) in settings_rows().into_iter().enumerate() {
                    let hl = k == *c;
                    let pal = if hl { HL } else { NORMAL };
                    match r {
                        Row::Connection => {
                            row(out, "CONNECTION", y, hl);
                            f.menu_text(out, if self.prefs.direct { "DIRECT IP" } else { "ROOM CODE" }, VAL_X, y, pal);
                        }
                        Row::Edit(fl) => {
                            row(out, fl.label(), y, hl);
                            let v = fl.get(&self.prefs);
                            let v = match fl {
                                _ if v.is_empty() => if fl == Field::Signal { "NOT SET" } else { "NONE" }.to_string(),
                                Field::TurnPass => "*".repeat(v.len().min(VAL_CHARS)),
                                _ => cut(&v, VAL_CHARS),
                            };
                            f.menu_small(out, &v, VAL_X, y + 4, pal);
                        }
                        Row::Delay => {
                            row(out, "INPUT DELAY", y, hl);
                            f.menu_text(out, &delay_label(self.prefs.delay), VAL_X, y, pal);
                        }
                        Row::Exit => row(out, text::get("exit"), y, hl),
                    }
                    y += 24;
                }
                let help = match settings_rows()[*c] {
                    Row::Connection | Row::Delay => "LEFT/RIGHT: CHANGE   TRIANGLE: BACK",
                    Row::Edit(Field::Port) => "LEFT/RIGHT: CHANGE   OK: TYPE IT   TRIANGLE: BACK",
                    Row::Edit(_) => "OK: EDIT   TRIANGLE: BACK",
                    Row::Exit => "OK: BACK",
                };
                f.menu_small(out, help, 48, HELP_Y, NORMAL);
                if !self.prefs.direct && settings_rows()[*c] == Row::Edit(Field::Stun) {
                    f.menu_small(out, "NONE: ONLY THIS MACHINE AND THE LAN CONNECT.", 48, HELP_Y - 20, ORANGE);
                }
            }
            View::Edit(fl, p, why) => {
                f.menu_text(out, fl.label(), 48, 80, ORANGE);
                match fl {
                    Field::Code => {
                        // the code in the tall menu font, a blinking cursor after it
                        let mut x = 48;
                        for ch in p.buf.chars() {
                            f.menu_text_tall(out, &ch.to_string(), x, 108, HL);
                            x += 24;
                        }
                        if self.frame & 8 == 0 {
                            f.menu_text_tall(out, "_", x, 108, 5);
                        }
                    }
                    _ => {
                        let shown = if *fl == Field::TurnPass { "*".repeat(p.buf.len()) } else { p.buf.clone() };
                        let lines = super::wrap(&shown, 48);
                        let n = lines.len();
                        for (k, l) in lines.iter().enumerate().skip(n.saturating_sub(3)) {
                            let y = 112 + 12 * (k + 3 - n.max(3)) as i32;
                            f.menu_small(out, l, 48, y, HL);
                            if k == n - 1 && self.frame & 8 == 0 {
                                f.menu_small(out, "_", 48 + 9 * l.chars().count() as i32, y, 5);
                            }
                        }
                    }
                }
                p.draw(f, out, 176);
                if let Some(w) = why {
                    f.menu_small(out, &cut(w, 50), 48, 156, ORANGE);
                }
                f.menu_small(out, "OK: ENTER   TRIANGLE: DELETE   START: DONE", 48, HELP_Y, NORMAL);
            }
            View::Connecting => {
                let mut y = 80;
                if let Some((label, text)) = &self.conn.big {
                    f.menu_text(out, label, 48, y, ORANGE);
                    if text.len() <= MAX_CODE {
                        // a room code, letter-spaced in the tall menu font
                        let mut x = 48;
                        for ch in text.chars() {
                            f.menu_text_tall(out, &ch.to_string(), x, y + 28, HL);
                            x += 28;
                        }
                        y += 76;
                    } else {
                        f.menu_text(out, text, 48, y + 24, HL);
                        y += 56;
                    }
                }
                for l in &self.conn.info {
                    f.menu_small(out, l, 48, y, NORMAL);
                    y += 14;
                }
                y += 14;
                for l in &self.conn.status {
                    for w in super::wrap(l, 50) {
                        f.menu_small(out, &w, 48, y, HL);
                        y += 14;
                    }
                }
                f.menu_small(out, "TRIANGLE: CANCEL", 48, HELP_Y, NORMAL);
            }
            View::Message(t, text) => {
                f.menu_text(out, t, 48, 80, ORANGE);
                for (k, l) in super::wrap(&text.to_uppercase(), 50).iter().enumerate().take(16) {
                    f.menu_small(out, l, 48, 112 + 14 * k as i32, NORMAL);
                }
                f.menu_small(out, "OK: BACK", 48, HELP_Y, NORMAL);
            }
        }
    }
}

/// A direct-IP JOIN's host address as typed on the picker: `A.B.C.D:PORT`, or `A.B.C.D` for `DEFAULT_PORT`; an
/// address only, never looked up as a name (no resolver is asked, the frame loop never waits on one).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn host_address(text: &str) -> Result<std::net::SocketAddr, String> {
    let a = text.parse::<std::net::SocketAddr>().ok().or_else(|| text.parse::<std::net::IpAddr>().ok().map(|ip| std::net::SocketAddr::new(ip, DEFAULT_PORT)));
    match a {
        Some(a) if a.port() != 0 && !a.ip().is_unspecified() => Ok(a),
        _ => Err(format!("{text} is not a host's address: type it as A.B.C.D:PORT (or A.B.C.D for port {DEFAULT_PORT})")),
    }
}

fn delay_label(d: u32) -> String {
    if d == 0 { "AUTO".into() } else { format!("{d} FRAMES") }
}

/// `s` cut to `n` characters ("..." at the end when cut).
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}...", s.chars().take(n.saturating_sub(3)).collect::<String>()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: u16 = 0x40;

    /// Frames of the page with these pads (each press then a release); the commands that were not None.
    fn press(m: &mut NetMenu, keys: &[u16]) -> Vec<Cmd> {
        let mut out = Vec::new();
        for &k in keys {
            out.push(m.tick([k, 0], &mut Vec::new()));
            out.push(m.tick([0, 0], &mut Vec::new()));
        }
        out.into_iter().filter(|c| *c != Cmd::None).collect()
    }

    /// The picker: moves wrap, cells enter, DEL / △ delete, △ on nothing cancels, OK and START finish; the cap.
    #[test]
    fn picker_typing() {
        let mut s = Vec::new();
        let mut p = Picker::new("", CODE_SET, 3, false);
        // A, then RIGHT twice: E
        assert_eq!(p.tick(0, OK, &mut s), None);
        p.tick(RIGHT, 0, &mut s);
        p.tick(RIGHT, 0, &mut s);
        p.tick(0, OK, &mut s);
        assert_eq!(p.buf, "AE");
        // LEFT from the first column wraps to the row's last; DOWN from the last row wraps to the first
        p.cur = 0;
        p.tick(LEFT, 0, &mut s);
        assert_eq!(p.cur, p.cols - 1);
        p.tick(UP, 0, &mut s);
        assert_eq!(p.cells()[p.cur], Cell::Ok);
        // the cap: a third character, then no more
        p.cur = 1;
        p.tick(0, OK, &mut s);
        p.tick(0, OK, &mut s);
        assert_eq!(p.buf, "AEC");
        // △ deletes, START finishes
        p.tick(0, TRIANGLE, &mut s);
        assert_eq!(p.tick(0, START, &mut s), Some(Some("AE".into())));
        let mut p = Picker::new("", CODE_SET, 3, false);
        assert_eq!(p.tick(0, TRIANGLE, &mut s), Some(None));
        // foreign characters of a stored value are dropped
        assert_eq!(Picker::new("1.2.3.4:70x0", ADDR_SET, 21, false).buf, "1.2.3.4:700");
    }

    /// SHIFT types the letters in the other case; the code picker has none.
    #[test]
    fn picker_shift() {
        let mut s = Vec::new();
        let mut p = Picker::new("", TEXT_SET, 64, true);
        p.tick(0, OK, &mut s);
        let shift = p.cells().iter().position(|c| *c == Cell::Shift).unwrap();
        p.cur = shift;
        p.tick(0, OK, &mut s);
        p.cur = 1;
        p.tick(0, OK, &mut s);
        assert_eq!(p.buf, "aB");
        assert!(!Picker::new("", CODE_SET, 4, false).cells().contains(&Cell::Shift));
    }

    /// HOST and JOIN wait for a signaling server by room code; by direct IP JOIN asks for the address.
    #[test]
    fn host_and_join() {
        let mut m = NetMenu::new(NetPrefs { direct: false, ..Default::default() }, [0; 2]);
        assert_eq!(press(&mut m, &[OK]), []);
        assert!(matches!(m.view, View::Message(..)));
        press(&mut m, &[OK]);
        m.prefs.signal = "ws://127.0.0.1:3536".into();
        assert_eq!(press(&mut m, &[OK]), [Cmd::Host]);
        m.view = View::Main(1);
        // JOIN: K (index of K in CODE_SET), then OK by START
        assert_eq!(press(&mut m, &[OK]), []);
        let k = CODE_SET.iter().position(|&c| c == b'K').unwrap();
        for _ in 0..k {
            press(&mut m, &[RIGHT]);
        }
        assert_eq!(press(&mut m, &[OK, START]), [Cmd::Join("K".into())]);
        // by direct IP: the last address prefilled
        let mut m = NetMenu::new(NetPrefs { direct: true, join: "127.0.0.1:7000".into(), ..Default::default() }, [0; 2]);
        assert_eq!(press(&mut m, &[DOWN, OK, START]), [Cmd::Join("127.0.0.1:7000".into())]);
        // the connect's screen: △ cancels
        m.connecting(Conn::default());
        assert_eq!(press(&mut m, &[TRIANGLE]), [Cmd::Cancel]);
        assert_eq!(press(&mut m, &[TRIANGLE]), [Cmd::Close]);
    }

    /// SETTINGS: the delay and the port by LEFT / RIGHT, each change stored; a bad port refused in the picker.
    #[test]
    fn settings_edit() {
        let mut m = NetMenu::new(NetPrefs::default(), [0; 2]);
        press(&mut m, &[DOWN, DOWN, OK]);
        assert_eq!(m.view, View::Settings(0));
        let rows = settings_rows();
        let delay = rows.iter().position(|r| *r == Row::Delay).unwrap();
        for _ in 0..delay {
            press(&mut m, &[DOWN]);
        }
        assert_eq!(press(&mut m, &[RIGHT, RIGHT]).last(), Some(&Cmd::Prefs(NetPrefs { delay: 2, ..NetPrefs::default() })));
        press(&mut m, &[LEFT, LEFT, LEFT]);
        assert_eq!(m.prefs.delay, crate::net::MAX_DELAY);
        if DIRECT {
            m.view = View::Settings(rows.iter().position(|r| *r == Row::Edit(Field::Port)).unwrap());
            press(&mut m, &[OK]);
            // the stored 7000 with a 0 typed: refused
            let zero = PORT_SET.iter().position(|&c| c == b'0').unwrap();
            if let View::Edit(_, p, _) = &mut m.view {
                p.cur = zero;
            }
            assert_eq!(press(&mut m, &[OK, START]), []);
            assert!(matches!(&m.view, View::Edit(_, _, Some(_))));
            press(&mut m, &[TRIANGLE, TRIANGLE, START]);
            assert_eq!(m.prefs.port, 700);
        }
    }

    /// The specials' row: UP / DOWN go between a special and the characters drawn over it, LEFT / RIGHT wrap
    /// among the specials.
    #[test]
    fn picker_specials() {
        let mut s = Vec::new();
        // TEXT_SET: 42 characters in 14 columns, then UPPER (columns 0..2), DEL (3..5), OK (6..8)
        let mut p = Picker::new("", TEXT_SET, 64, false);
        let at = |p: &Picker| p.cells()[p.cur];
        p.cur = 14 * 2 + 7;
        p.tick(DOWN, 0, &mut s);
        assert_eq!(at(&p), Cell::Ok);
        p.tick(RIGHT, 0, &mut s);
        assert_eq!(at(&p), Cell::Shift);
        p.tick(LEFT, 0, &mut s);
        assert_eq!(at(&p), Cell::Ok);
        p.tick(LEFT, 0, &mut s);
        assert_eq!(at(&p), Cell::Del);
        p.tick(UP, 0, &mut s);
        assert_eq!(p.cur, 14 * 2 + 4);
        p.tick(DOWN, 0, &mut s);
        p.tick(DOWN, 0, &mut s);
        assert_eq!(p.cur, 4);
    }

    /// A message goes back to the main row it came from.
    #[test]
    fn message_returns_to_the_row() {
        let mut m = NetMenu::new(NetPrefs { direct: true, join: "1.2.3.4".into(), ..Default::default() }, [0; 2]);
        press(&mut m, &[DOWN, OK]);
        m.message("NOT CONNECTED", "no");
        press(&mut m, &[OK]);
        assert_eq!(m.view, View::Main(1));
    }

    /// JOIN's address: an IP with or without its port, never a name.
    #[test]
    fn host_addresses() {
        assert_eq!(host_address("192.168.1.5:7001").unwrap().to_string(), "192.168.1.5:7001");
        assert_eq!(host_address("192.168.1.5").unwrap().port(), DEFAULT_PORT);
        for bad in ["12.34.:7000", "1..2:7000", "localhost:7000", "0.0.0.0:7000", "1.2.3.4:0", "1.2.3.4:", ""] {
            assert!(host_address(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn stored_prefs() {
        assert_eq!(NetPrefs::from_stored(None), NetPrefs::default());
        let p = NetPrefs::from_stored(Some(r#"{"signal": "ws://a:1", "port": 0, "delay": 99, "stun": "x\ny"}"#.into()));
        assert_eq!((p.signal.as_str(), p.port, p.delay, p.stun.as_str()), ("ws://a:1", DEFAULT_PORT, 0, ""));
        let q = NetPrefs::from_stored(p.to_stored());
        assert_eq!(q, p);
        // codes from the unambiguous set
        let c = room_code(12345);
        assert_eq!(c.len(), CODE_LEN);
        assert!(c.bytes().all(|b| CODE_SET.contains(&b)));
        assert_ne!(room_code(1), room_code(2));
    }
}
