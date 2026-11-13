//! The MEMORY CARD screen's dialogs: questions, messages, the card op and its result.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The MEMORY CARD screen's dialogs (`auto`): the questions, the "NOW …" messages,
//! the card op and its result, over the MEMORY CARD menu or alone on black (the auto-save, mode 5 entry).
//! The save file is MEMORY CARD SLOT 1, always inserted and formatted; the other card results are only reachable
//! through the debug fault (`BR2_MCFAULT`).

use crate::assets::text;
use crate::frontend::options::{Card, CardError, Settings};
use crate::output::font::Fonts;
use crate::output::prim::Prim;
use serde::{Deserialize, Serialize};

const RIGHT: u16 = 0x2000;
const LEFT: u16 = 0x8000;
const CONFIRM: u16 = 0xE0; // □ × ○
const BACK: u16 = 0x10; // △

/// The ops (`s0`); 5 = MEMORY CARD CHANGED, saved as op 1.
pub const LOAD: u8 = 0;
pub const SAVE: u8 = 1;
pub const FORMAT: u8 = 2;
pub const OVERWRITE: u8 = 3;
pub const CHECK: u8 = 4;
pub const CHANGED: u8 = 5;

/// What the caller does after a dialog frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Stay,
    /// the dialog is over (menu: back to menu state 0; auto: leave the screen)
    Done,
    /// LOAD completed, after the button: apply the volumes / stereo, refresh the OPTIONS values
    Loaded,
}

/// Debug (`BR2_MCFAULT=name`): one card result forced on the first matching op, so that the messages a JSON save
/// cannot produce can be compared with the reference frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fault {
    /// bit n = op n
    ops: u8,
    code: i32,
}

impl Fault {
    /// A `BR2_MCFAULT` value: nocard, unformatted / unfmt, newcard, changed, full, other, nodata, dataerr / vererr.
    pub fn parse(v: &str) -> Option<Self> {
        let any = 0x3F;
        let save = 1 << SAVE | 1 << OVERWRITE | 1 << CHANGED;
        let (ops, code) = match v {
            "nocard" => (any, 1),
            "unformatted" | "unfmt" => (any, 4),
            "newcard" => (any, 3),
            "changed" => (save, 3),
            "full" => (save, 7),
            "other" => (save, 6),
            "nodata" => (1 << LOAD, 5),
            "dataerr" | "vererr" => (1 << LOAD, 2),
            _ => return None,
        };
        Some(Self { ops, code })
    }
}

/// What this frame shows (the state handler's draw calls).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum View {
    None,
    Question,
    Busy,
    /// the result header alone (results that go straight on to a question)
    Header,
    Result,
}

/// The dialog states (menu descriptor state byte, 0x10 and up) and their variables.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CardDialog {
    /// `s0`
    op: u8,
    st: u8,
    /// `s4`: 0 YES, 1 NO, −1 no question
    yes: i8,
    /// `s2`: the last result, −1 before any op
    result: i32,
    /// auto-dismiss timer of the auto-save's COMPLETED
    t: i32,
    pub auto: bool,
    /// the menu item: 1 AUTO SAVE, 2 LOAD, 3 SAVE
    item: usize,
    /// engine: frames the "NOW …" text stays up while the (instant) op stands for the card's access time
    hold: i32,
    view: View,
}

impl CardDialog {
    /// The menu's LOAD / SAVE confirm, or AUTO SAVE switched ON: state 0x10.
    pub fn menu(item: usize) -> Self {
        Self { op: CHECK, st: 0x10, yes: 1, result: -1, t: -1, auto: false, item, hold: 0, view: View::None }
    }

    /// The auto-save: SAVE without the question.
    pub fn auto() -> Self {
        Self { op: SAVE, st: 0x11, yes: -1, result: -1, t: -1, auto: true, item: 3, hold: 0, view: View::None }
    }

    /// States 0x12..0x14: the menu banner stops (the banner scroll (0, 0, 0, 7)).
    pub fn busy(&self) -> bool {
        (0x12..=0x14).contains(&self.st)
    }

    /// One frame. `s` = opt (LOAD replaces it, a failure with the cursor on AUTO SAVE clears opt.22), `dirty` =
    /// save+0x410, `card` = MEMORY CARD SLOT 1.
    pub fn tick(&mut self, press: u16, rep: u16, s: &mut Settings, dirty: &mut bool, card: &mut Card, sounds: &mut Vec<u16>) -> Out {
        self.view = View::None;
        // NO by default; LOAD asks first, SAVE / AUTO SAVE check the card without a question
        if self.st == 0x10 {
            self.yes = 1;
            (self.op, self.st) = if self.item == 2 { (LOAD, 0x11) } else { (CHECK, 0x12) };
        }
        // what comes next
        if self.st == 0x16 {
            match (self.result, self.op) {
                (0, LOAD) => return Out::Loaded,
                (0, SAVE | OVERWRITE | CHANGED) => return Out::Done,
                (4, op) if op != LOAD => (self.op, self.yes, self.st) = (FORMAT, 1, 0x11),
                _ => {
                    if !self.auto && self.item == 1 {
                        s.auto_save = 0;
                    }
                    if !(self.auto && self.op == SAVE) {
                        return Out::Done;
                    }
                    (self.yes, self.st) = (1, 0x11);
                }
            }
        }
        // the question
        if self.st == 0x11 {
            if self.yes < 0 {
                self.st = 0x12;
            } else {
                self.view = View::Question;
                if rep & (LEFT | RIGHT) != 0 {
                    self.yes ^= 1;
                    sounds.push(0x0B);
                } else if press & BACK != 0 {
                    sounds.push(0x0D);
                    self.st = 0x20;
                } else if press & CONFIRM != 0 {
                    sounds.push(0x0C);
                    if self.yes == 0 {
                        self.st = 0x12;
                    } else if self.auto && matches!(self.op, FORMAT | OVERWRITE | CHANGED) {
                        (self.op, self.yes) = (SAVE, 1);
                    } else {
                        self.st = 0x20;
                    }
                }
                return Out::Stay;
            }
        }
        match self.st {
            // "NOW …" for two frames
            0x12 => {
                self.view = View::Busy;
                self.st = 0x13;
            }
            0x13 => {
                self.view = View::Busy;
                self.st = 0x14;
                self.hold = match self.op {
                    LOAD => 15,
                    CHECK => 20,
                    _ => 30,
                };
            }
            // the op (blocking on the console, whose screen keeps the 0x13 frame); this frame draws nothing
            0x14 => {
                if self.hold > 0 {
                    self.hold -= 1;
                    self.view = View::Busy;
                    return Out::Stay;
                }
                self.result = self.run(s, dirty, card);
                self.t = if self.auto { 120 } else { -1 };
                self.st = 0x15;
            }
            // the result
            0x15 => {
                // engine: the app stores the file after the op's step; a failed store is the SAVE's ERROR ! (the
                // file stays unsaved)
                if self.result == 0 && matches!(self.op, SAVE | OVERWRITE | CHANGED) && card.take_write_failed() {
                    self.result = 6;
                    *dirty = true;
                }
                self.view = View::Header;
                match (self.result, self.op) {
                    (0 | 3, CHECK) => (self.op, self.yes, self.st) = (SAVE, 1, 0x11),
                    (0xF, _) => (self.op, self.yes, self.st) = (OVERWRITE, 1, 0x11),
                    (3, _) => (self.op, self.yes, self.st) = (CHANGED, 1, 0x11),
                    (0, FORMAT) => (self.op, self.yes, self.st) = (if self.item == 2 { LOAD } else { SAVE }, -1, 0x11),
                    _ => {
                        self.view = View::Result;
                        if press & CONFIRM != 0 {
                            sounds.push(0x0C);
                            self.st = 0x16;
                        } else if press & BACK != 0 {
                            sounds.push(0x0D);
                            self.st = 0x16;
                        } else if self.auto && self.result == 0 {
                            self.t -= 1;
                            if self.t < 0 {
                                self.st = 0x16;
                            }
                        }
                    }
                }
            }
            // declined
            0x20 => self.st = 0x21,
            _ => {
                if !self.auto && self.item == 1 {
                    s.auto_save = 0;
                }
                return Out::Done;
            }
        }
        Out::Stay
    }

    /// The card routines on the JSON save.
    fn run(&self, s: &mut Settings, dirty: &mut bool, card: &mut Card) -> i32 {
        let op = if self.op == CHANGED { SAVE } else { self.op };
        if let Some(f) = card.fault
            && f.ops >> op & 1 != 0
        {
            card.fault = None;
            return f.code;
        }
        match op {
            LOAD => match Settings::try_load(card) {
                Ok(n) => {
                    *s = n;
                    *dirty = false;
                    0
                }
                Err(CardError::NoData) => 5,
                Err(CardError::DataError) => 2,
            },
            CHECK => {
                if Settings::exists(card) {
                    0xF
                } else {
                    0
                }
            }
            FORMAT => 0,
            _ => {
                if s.save(card) {
                    *dirty = false;
                    0
                } else {
                    6
                }
            }
        }
    }

    /// The messages (x 16, from y 208, 32 per call, an embedded newline 16; menu font palettes 0 white, 1 grey,
    /// 2 red, 3 orange).
    pub fn draw(&self, f: &Fonts, out: &mut Vec<Prim>) {
        // a template of the slot (`%s`: SLOT 1)
        let slot = |key: &str| text::fill(text::get(key), &[&text::get("slot_1")]);
        let mut y = 208;
        let mut say = |out: &mut Vec<Prim>, s: &str, pal: u16| {
            for l in s.split('\n') {
                f.menu_text(out, l, 16, y, pal);
                y += 16;
            }
            y += 16;
        };
        match self.view {
            View::None => {}
            View::Question => {
                let yes_no = |out: &mut Vec<Prim>, y: i32| {
                    f.menu_text(out, text::get("yes"), 48, y, if self.yes == 0 { 0 } else { 1 });
                    f.menu_text(out, text::get("no"), 144, y, if self.yes == 0 { 1 } else { 0 });
                };
                match self.op {
                    LOAD => {
                        say(out, &slot("load_from_memory_card_n_ask"), 0);
                        yes_no(out, 240);
                    }
                    FORMAT => {
                        say(out, &slot("format_memory_card_n_ask"), 0);
                        yes_no(out, 240);
                    }
                    OVERWRITE => {
                        say(out, &slot("saving_to_memory_card_n"), 0);
                        say(out, text::get("over_write_ok"), 0);
                        yes_no(out, 272);
                    }
                    CHANGED => {
                        say(out, text::get("memory_card_changed"), 2);
                        say(out, &slot("save_to_memory_card_n_ask"), 0);
                        yes_no(out, 272);
                        f.menu_text(out, text::get("1_block_is_required_to_save"), 16, 304, 3);
                    }
                    _ => {
                        say(out, &slot("save_to_memory_card_n_ask"), 0);
                        yes_no(out, 240);
                        f.menu_text(out, text::get("1_block_is_required_to_save"), 16, 272, 3);
                    }
                }
            }
            View::Busy => {
                let busy = match self.op {
                    // the slot on a line of its own ("%s...")
                    LOAD => format!("{}\n{}...", text::get("now_loading_from_memory_card"), text::get("slot_1")),
                    FORMAT => slot("now_formatting_memory_card_n"),
                    CHECK => slot("now_checking_memory_card_n"),
                    _ => slot("now_saving_to_memory_card_n"),
                };
                say(out, &busy, 0);
                say(out, text::get("do_not_remove_memory_card"), 3);
            }
            View::Header | View::Result => {
                let head = match self.op {
                    LOAD => slot("load_from_memory_card_n_done"),
                    FORMAT => slot("format_memory_card_n_done"),
                    CHECK => slot("checking_memory_card_n"),
                    _ => slot("save_to_memory_card_n_done"),
                };
                say(out, &head, 0);
                if self.view == View::Header {
                    return;
                }
                let (msg, pal) = match self.result {
                    0 => (text::get("completed").to_string(), 3),
                    1 => (format!("{}\n{}", text::get("no_memory_card_is_inserted_in"), slot("memory_card_n")), 2),
                    7 => (
                        format!(
                            "{}\n{}\n{}",
                            text::get("there_is_insufficient_memory_card"),
                            text::get("blocks_and_1_block_is_required"),
                            text::get("to_save")
                        ),
                        2,
                    ),
                    4 if self.op != LOAD => (text::get("unformatted_memory_card").to_string(), 2),
                    4 | 5 => (text::get("no_bloody_roar_2_data_detected").to_string(), 2),
                    2 => (text::get("data_error").to_string(), 2),
                    _ => (text::get("error").to_string(), 2),
                };
                say(out, &msg, pal);
            }
        }
    }
}
