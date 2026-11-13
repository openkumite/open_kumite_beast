//! The front end's and the HUD's number tables, typed: the select's cursor and layout, the VS screen's announcer
//! pauses, the movies' lengths, the arcade ranks, the controller config's actions, STORY mode's fights and pages, the
//! screens' banner widths and the mode menu's half widths, the HUD's sprites, from the converted `tables.okt`. The
//! tables' loader installs them (`Assets::br2_tables`), as the text (`text`) and the fonts (`output::font`).
//!
//! State: static data (not serialised; one per process: a test that reads them installs them first,
//! `formats::tables::export_tables`). Without them every table is zeros. Neither the front end nor a fight runs without
//! them (`frontend::load` and `fight_data` fail on them: the setup screen asks for a new conversion); before they are
//! installed a settings load finds no default button map (`default_buttons`: nothing bound, the map stays unset), and
//! the banners' width is at least 1 whatever the data (`title_width`), so bad data cannot stall a scroll.

use crate::formats::tables::Tables;
use std::sync::{OnceLock, RwLock};

/// The tables (each by the name of its `tables.okt` table).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameTables {
    /// `select.neighbours`: per cell the cell up, right, down, left
    pub neighbours: [[u8; 4]; 11],
    /// `select.faces`: per cell its face icon's top-left corner
    pub faces: [(i32, i32); 11],
    /// `select.plates`: per character the name plate's x of the name and of the beast type
    pub plates: [(i32, i32); 11],
    /// `select.story_beast_x`, `select.story_title_x`, `select.story_name_x`: the STORY select's x offsets
    pub story_beast_x: [i32; 11],
    pub story_title_x: [i32; 11],
    pub story_name_x: [i32; 11],
    /// `select.stage_half`: the stage names' half widths
    pub stage_half: [i32; 9],
    /// `select.normal_order`: the NORMAL select's faces left to right
    pub normal_order: [u8; 11],
    /// `vs.announcer_delays`: per character the pauses after the name and after the beast word
    pub announcer_delays: [(u32, u32); 11],
    /// `movie.records`: each movie's last frame (+8 of its record)
    pub movie_last: [usize; 4],
    /// `progress.rank`: DIFFICULTY × fight → the CPU level / 16
    pub rank: [[u8; 16]; 8],
    /// `menu.cc_values`: the controller config's actions
    pub cc_values: [u16; 10],
    /// `input.default_buttons`: the default button map (the logical bits of □ × ○ △ R1 R2 L1 L2)
    pub default_buttons: [u16; 8],
    /// `menu.mode_half`: the mode menu's half widths (ARCADE .. OPTIONS)
    pub mode_half: [i32; 7],
    /// `screen.title_widths`: the banner width of each text of the list `screen_title`
    pub title_widths: [i32; 24],
    /// `story.fight_count`, `story.stages`, `story.opponents`, `story.script`: STORY mode per character
    pub story_count: [u8; 11],
    pub story_stages: [[u8; 8]; 11],
    pub story_opponents: [[u8; 8]; 11],
    pub story_script: [[u8; 25]; 11],
    /// `hud.data`: the HUD's block (the names' x, the sprites' rectangles, read by offset: `fight::hud`)
    pub hud: Vec<u8>,
    /// `hud.frame`: the HUD frames' pieces (u, v, w, h)
    pub frame: [[u8; 4]; 8],
    /// `hud.popup`: the pop-ups' palettes and CLUTs
    pub popup: [u16; 20],
}

impl GameTables {
    /// The banner width `k` (`title_widths`), at least 1: the banners' scroll and repeats step by it, so a zero or
    /// negative width of bad data (or none) cannot stall them.
    pub fn title_width(&self, k: usize) -> i32 {
        self.title_widths[k].max(1)
    }

    pub fn from_tables(t: &Tables) -> Self {
        let tab = |n: &str| t.table(n);
        let u8s = |n: &str, k: usize| tab(n).u8(k);
        let (faces, plates, delays, movies, rank) =
            (tab("select.faces"), tab("select.plates"), tab("vs.announcer_delays"), tab("movie.records"), tab("progress.rank"));
        let (stages, opps, script) = (tab("story.stages"), tab("story.opponents"), tab("story.script"));
        Self {
            neighbours: std::array::from_fn(|c| std::array::from_fn(|d| tab("select.neighbours").u8(4 * c + d))),
            faces: std::array::from_fn(|c| (faces.i16(4 * c) as i32, faces.i16(4 * c + 2) as i32)),
            plates: std::array::from_fn(|c| (plates.u8(2 * c) as i32, plates.u8(2 * c + 1) as i32)),
            story_beast_x: std::array::from_fn(|c| u8s("select.story_beast_x", c) as i32),
            story_title_x: std::array::from_fn(|c| u8s("select.story_title_x", c) as i32),
            story_name_x: std::array::from_fn(|c| u8s("select.story_name_x", c) as i32),
            stage_half: std::array::from_fn(|k| u8s("select.stage_half", k) as i32),
            normal_order: std::array::from_fn(|k| u8s("select.normal_order", k)),
            announcer_delays: std::array::from_fn(|c| (delays.u8(2 * c) as u32, delays.u8(2 * c + 1) as u32)),
            movie_last: std::array::from_fn(|k| movies.u32(28 * k + 8) as usize),
            rank: std::array::from_fn(|d| std::array::from_fn(|f| rank.u8(16 * d + f))),
            cc_values: std::array::from_fn(|k| tab("menu.cc_values").u16(2 * k)),
            default_buttons: std::array::from_fn(|k| tab("input.default_buttons").u16(2 * k)),
            mode_half: std::array::from_fn(|k| u8s("menu.mode_half", k) as i32),
            title_widths: std::array::from_fn(|k| tab("screen.title_widths").u32(4 * k) as i32),
            story_count: std::array::from_fn(|c| u8s("story.fight_count", c)),
            story_stages: std::array::from_fn(|c| std::array::from_fn(|k| stages.u8(8 * c + k))),
            story_opponents: std::array::from_fn(|c| std::array::from_fn(|k| opps.u8(8 * c + k))),
            story_script: std::array::from_fn(|c| std::array::from_fn(|k| script.u8(25 * c + k))),
            hud: tab("hud.data").bytes().to_vec(),
            frame: std::array::from_fn(|k| std::array::from_fn(|i| tab("hud.frame").u8(4 * k + i))),
            popup: std::array::from_fn(|k| tab("hud.popup").u16(2 * k)),
        }
    }
}

static STORE: RwLock<Option<&'static GameTables>> = RwLock::new(None);

/// Installs the tables of `t` (by `Assets::br2_tables`); the same tables again is a no-op, others (new game data)
/// replace them (the old ones leaked: once a data change).
pub fn install(t: &Tables) {
    let g = GameTables::from_tables(t);
    let mut s = STORE.write().unwrap_or_else(|e| e.into_inner());
    if s.is_some_and(|old| *old == g) {
        return;
    }
    *s = Some(Box::leak(Box::new(g)));
}

/// The default button map (`GameTables::default_buttons`: zeros, nothing bound, without the game data).
pub fn default_buttons() -> [u16; 8] {
    get().default_buttons
}

/// The installed tables (zeros without them).
pub fn get() -> &'static GameTables {
    static EMPTY: OnceLock<GameTables> = OnceLock::new();
    match *STORE.read().unwrap_or_else(|e| e.into_inner()) {
        Some(g) => g,
        None => EMPTY.get_or_init(GameTables::default),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The test data's tables (data-gated) are the values the engine had in its code before (checked against them
    /// once; but for two banner widths, MOVIES AND PICTURES and PLAY DATA, which the engine had as their text width + 64),
    /// by the CRC-32 of their debug form (no table data here).
    #[test]
    fn tables_as_before() {
        let Some(t) = crate::formats::tables::export_tables() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let g = GameTables::from_tables(&t);
        assert_eq!(crate::base::hash::crc32(format!("{g:?}").as_bytes()), 0x505D_5B60);
    }

    /// Without the tables every value is 0.
    #[test]
    fn no_tables_zeros() {
        let g = GameTables::from_tables(&Tables::default());
        assert_eq!(g, GameTables::default());
    }
}
