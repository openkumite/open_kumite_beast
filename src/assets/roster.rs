//! Character codes and names shared by the fight, the front end and the HUD.
//!
//! State: static data

use super::text;

/// Character ids (fighter +3), in the order of the sound banks CHARA_00 .. CHARA_10.
pub const CHARACTERS: [&str; 11] = ["YUG", "ALI", "LON", "ULI", "MAR", "BUS", "BAK", "JEN", "STU", "GAD", "RON"];

/// Engine addition (the import S9): the select's imported cells, character numbers `BR1_FIRST..`
/// (`MatchSpec::chr`): their fighter codes (`assets::manifest::BR1_FIGHTERS`), their entries in the imported game's
/// `names` list (its Uriko named "<name> (BR1)", the user's choice, beside the main game's), and beast types: the
/// imported game's Fox by its name, its Uriko by the main game's (her `names` entry 7), the engine's own words for the others.
pub const BR1_FIRST: u8 = 11;
pub const BR1_CODES: [&str; 4] = ["MIT", "FOX", "GRE", "URI"];
const BR1_NAME: [usize; 4] = [3, 5, 2, 8];
const BR1_SUFFIX: [&str; 4] = ["", "", "", " (BR1)"];
/// Character numbers a `MatchSpec` may hold: the main game's 11, then the imported cells.
pub const SELECTABLE: usize = 15;

/// Character `c`'s fighter code: the main game's, or an imported cell's (`BR1_CODES`); YUG out of range.
pub fn code(c: u8) -> &'static str {
    match c.checked_sub(BR1_FIRST) {
        Some(k) => BR1_CODES.get(k as usize).copied().unwrap_or(CHARACTERS[0]),
        None => CHARACTERS[c as usize],
    }
}

/// The main game's character `c`'s name (text `names`, entry 2c).
pub fn br2_name(c: usize) -> &'static str {
    text::item("names", 2 * c)
}

/// The main game's 11 names (`br2_name`).
pub fn br2_names() -> [&'static str; 11] {
    std::array::from_fn(br2_name)
}

/// Character `c`'s name (`br2_name`, an imported cell's `br1_name`; "" out of range).
pub fn name(c: u8) -> &'static str {
    match c.checked_sub(BR1_FIRST) {
        Some(k) => BR1_CODES.get(k as usize).and_then(|c| br1_name(c)).unwrap_or(""),
        None => br2_name(c as usize),
    }
}

/// Character `c`'s beast type (text `names`, entry 2c + 1; an imported cell's: `BR1_CODES`; "" out of range).
pub fn beast(c: u8) -> &'static str {
    match c.checked_sub(BR1_FIRST) {
        Some(0) => "BOAR",
        Some(1) => text::br1_item("names", BR1_NAME[1]),
        Some(2) => "OX",
        Some(3) => text::item("names", 7),
        Some(_) => "",
        None => text::item("names", 2 * c as usize + 1),
    }
}

/// Character `c` is an imported cell.
pub fn is_br1(c: u8) -> bool {
    (BR1_FIRST..SELECTABLE as u8).contains(&c)
}

/// The imported fighter `code`'s name (`BR1_CODES`), None for any other code.
pub fn br1_name(code: &str) -> Option<&'static str> {
    static MADE: std::sync::Mutex<Vec<(String, &'static str)>> = std::sync::Mutex::new(Vec::new());
    let k = BR1_CODES.iter().position(|c| *c == code)?;
    let name = text::br1_item("names", BR1_NAME[k]);
    if BR1_SUFFIX[k].is_empty() {
        return Some(name);
    }
    let full = format!("{name}{}", BR1_SUFFIX[k]);
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(_, s)) = made.iter().find(|(f, _)| *f == full) {
        return Some(s);
    }
    let s: &'static str = Box::leak(full.clone().into_boxed_str());
    made.push((full, s));
    Some(s)
}
