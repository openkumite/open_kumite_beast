//! The game's text: every word the engine shows of a game's own comes from that game's converted text table
//! (`text.strings` of its `engine/tables.okt`, ok_br2_converter `docs/tables.md`), looked up by key; the engine's own
//! words stay in the code. The tables' loaders install it (`Assets::br2_tables`, `Assets::br1_tables`); a key nothing
//! installed (tests and tools without converted data) or the table lacks reads a neutral placeholder (`placeholder`).
//! The store is one per process: a test that reads a game's text installs it first (`formats::tables::export_tables`,
//! `br1_export_tables`; data-gated), as another test may not have yet.
//!
//! State: static data (not serialised). The serialised state never holds a text of the table: it holds ids, and where a
//! state's serialised form is a text (the round's message) that form is the installed text.
//!
//! Determinism: the text is the simulation's input data as the other tables of `tables.okt` are (the round's serialised
//! message, the name entry's letters and the censor list come from it), so two runs' states hash alike only with the
//! same text (netplay peers, a replay and its recording). The data format (`manifest::FORMAT_MIN..=FORMAT_MAX`) pins
//! it: a converter that changes `text.strings` for the same disc must bump the format.
//!
//! Keys: a list's entries are `<list>.<n>` from 0 (the main game: `names` each character's name, then its beast type;
//! `stages`, `modes`, `screen_title`, `story_title`, `displays`, `cc_names`, `cc_icons`, `movies`, `censor`,
//! `ordinal`, `se_banks`; the imported game: `names`, `stages`), the single texts keys made of their words (`you_win`,
//! `press_start`; `_n` marks a value filled in, `fill`).

use crate::formats::tables::Tables;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, RwLock};

/// The table's name in `tables.okt`.
pub const TABLE: &str = "text.strings";

/// Whose text: the main game's (the game's own) or the imported game's (its import, engine addition).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
    Br2,
    Br1,
}

/// An installed table: its bytes (a second install of the same bytes is a no-op) and its texts by key (leaked: a
/// game's text is installed once a run, again only when its data changes).
struct Installed {
    bytes: Vec<u8>,
    map: BTreeMap<&'static str, &'static str>,
}

static STORE: RwLock<[Option<Installed>; 2]> = RwLock::new([None, None]);

/// The entries of a text table's bytes (`key NUL text NUL` …); Err when they are not that.
pub fn parse(b: &[u8]) -> Result<Vec<(&str, &str)>, String> {
    let bad = || format!("tables.okt: {TABLE} is not key/text pairs (convert the game data again)");
    let Some((&0, body)) = b.split_last() else {
        return if b.is_empty() { Ok(Vec::new()) } else { Err(bad()) };
    };
    let parts: Vec<&str> = body.split(|&c| c == 0).map(std::str::from_utf8).collect::<Result<_, _>>().map_err(|_| bad())?;
    if !parts.len().is_multiple_of(2) || parts.chunks(2).any(|p| p[0].is_empty()) {
        return Err(bad());
    }
    Ok(parts.chunks(2).map(|p| (p[0], p[1])).collect())
}

/// Installs `game`'s text from its tables; Err when they have no text table or a broken one (data of an older
/// converter).
pub fn install(game: Game, tables: &Tables) -> Result<(), String> {
    install_bytes(game, tables.get(TABLE)?.bytes())
}

/// Installs `game`'s text from a text table's bytes (`install`); an empty table installs nothing (the tests' tables: a
/// game's converted table is never empty).
pub fn install_bytes(game: Game, b: &[u8]) -> Result<(), String> {
    let mut store = STORE.write().unwrap_or_else(|e| e.into_inner());
    if b.is_empty() || store[game as usize].as_ref().is_some_and(|i| i.bytes == b) {
        return Ok(());
    }
    let map = parse(b)?.into_iter().map(|(k, t)| (leak(k), leak(t))).collect();
    store[game as usize] = Some(Installed { bytes: b.to_vec(), map });
    Ok(())
}

/// `game`'s text is installed.
pub fn installed(game: Game) -> bool {
    STORE.read().unwrap_or_else(|e| e.into_inner())[game as usize].is_some()
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

/// The main game's text `key`.
pub fn get(key: &str) -> &'static str {
    lookup(Game::Br2, key)
}

/// Entry `k` of the main game's list `list` (`<list>.<k>`).
pub fn item(list: &str, k: usize) -> &'static str {
    lookup(Game::Br2, &format!("{list}.{k}"))
}

/// The imported game's text `key`.
pub fn br1(key: &str) -> &'static str {
    lookup(Game::Br1, key)
}

/// Entry `k` of the imported game's list `list`.
pub fn br1_item(list: &str, k: usize) -> &'static str {
    lookup(Game::Br1, &format!("{list}.{k}"))
}

/// `game`'s text `key`, or its placeholder (logged once per key when the game's table is installed but lacks it: labels
/// are looked up every frame).
pub fn lookup(game: Game, key: &str) -> &'static str {
    static WARNED: Mutex<BTreeSet<(usize, String)>> = Mutex::new(BTreeSet::new());
    let store = STORE.read().unwrap_or_else(|e| e.into_inner());
    match store[game as usize].as_ref().map(|i| i.map.get(key)) {
        Some(Some(t)) => t,
        Some(None) => {
            if WARNED.lock().unwrap_or_else(|e| e.into_inner()).insert((game as usize, key.to_owned())) {
                log::warn!("text: no {key} in the {game:?} text table");
            }
            placeholder(game, key)
        }
        None => placeholder(game, key),
    }
}

/// The text of a key nothing installed: "FIGHTER n" for a character's name (`names`: the main game's even entries,
/// every one of the imported game's), "STAGE n" for a stage's (`stages`), "?" for any other (n counts from 1).
pub fn placeholder(game: Game, key: &str) -> &'static str {
    static MADE: Mutex<BTreeMap<String, &'static str>> = Mutex::new(BTreeMap::new());
    let num = |list: &str| key.strip_prefix(list).and_then(|r| r.strip_prefix('.')).and_then(|n| n.parse::<usize>().ok());
    let text = match (num("names"), num("stages")) {
        (Some(n), _) if game == Game::Br1 => format!("FIGHTER {}", n + 1),
        (Some(n), _) if n % 2 == 0 => format!("FIGHTER {}", n / 2 + 1),
        (_, Some(n)) => format!("STAGE {}", n + 1),
        _ => return "?",
    };
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    made.entry(text.clone()).or_insert_with(|| leak(&text))
}

/// `template` with its value marks (`%s`, `%u`, `%d`) replaced by `values` in order (a mark without a value: removed).
pub fn fill(template: &str, values: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::new();
    let mut rest = template;
    let mut values = values.iter();
    while let Some(k) = rest.find('%') {
        out += &rest[..k];
        match rest[k + 1..].chars().next() {
            Some('s' | 'u' | 'd') => {
                if let Some(v) = values.next() {
                    out += &v.to_string();
                }
                rest = &rest[k + 2..];
            }
            _ => {
                out.push('%');
                rest = &rest[k + 1..];
            }
        }
    }
    out + rest
}

/// A text the engine shows: one of the game's (a key of the main game's text table) or the engine's own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Game(&'static str),
    Port(&'static str),
}

impl Label {
    pub fn text(self) -> &'static str {
        match self {
            Label::Game(key) => get(key),
            Label::Port(t) => t,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_placeholders() {
        assert_eq!(parse(b"a.0\0X Y\0b\0\0").unwrap(), [("a.0", "X Y"), ("b", "")]);
        assert_eq!(parse(b"").unwrap(), []);
        assert!(parse(b"a\0X").is_err());
        assert!(parse(b"a\0").is_err());
        assert!(parse(b"a\0\xFF\0").is_err());
        assert!(parse(b"\0X\0").is_err());
        assert_eq!(placeholder(Game::Br2, "names.4"), "FIGHTER 3");
        assert_eq!(placeholder(Game::Br2, "names.5"), "?");
        assert_eq!(placeholder(Game::Br1, "names.5"), "FIGHTER 6");
        assert_eq!(placeholder(Game::Br2, "stages.2"), "STAGE 3");
        assert_eq!(placeholder(Game::Br2, "you_win"), "?");
        assert_eq!(placeholder(Game::Br2, "namesx.1"), "?");
    }

    #[test]
    fn fill_marks() {
        assert_eq!(fill("ABC? %u", &[&7]), "ABC? 7");
        assert_eq!(fill("CARD %s.", &[&"A"]), "CARD A.");
        assert_eq!(fill("100%", &[]), "100%");
        assert_eq!(fill("%s-%s", &[&1]), "1-");
    }
}
