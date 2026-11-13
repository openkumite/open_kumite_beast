//! The option struct `opt`, the save file (`Card`) and the engine's preferences (`Prefs`).
//!
//! State: serialised: `Settings` is part of `GameState` (in the card); `Card` / `Prefs` are files and replay headers
//!
//! The option struct `opt` (defaults from the static tables): the OPTIONS / SOUND OPTIONS values,
//! the CUSTOM settings block (opt+0x38) and the unlock bits. Saved where the memory card would be: a JSON file
//! in the user's config directory on desktop, `localStorage["okb_save"]` on the web (moved once from `br2_save`) (`platform::storage`).

use serde::{Deserialize, Serialize};

pub use crate::fight::rules::Custom;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// opt.7 DIFFICULTY 0..7 (shown 1..8)
    pub difficulty: u8,
    /// opt.8 ATTACK LEVEL 0..7 (HP 100 % … 55 %)
    pub attack_level: u8,
    /// opt.9 TIME LIMIT 0..4: 20 / 40 / 60 / 90 s / infinite
    pub time_limit: u8,
    /// opt.5 = opt.6 MATCH POINT 0..3 (rounds to win − 1)
    pub match_point: u8,
    /// opt.0x11 CHARACTER SELECT: 0 NORMAL, 1 QUICK
    pub char_select: u8,
    /// opt.0x25 DISPLAY TYPE: NORMAL / WIDE 1..3
    pub display: u8,
    /// opt.0x24 VIBRATION
    pub vibration: u8,
    /// opt.0xC OUTPUT: 0 MONO, 1 STEREO
    pub output: u8,
    /// opt.0xD BGM SELECT: 0 CONSOLE (character themes), 1 ARCADE (stage themes)
    pub bgm_select: u8,
    /// opt.0xE MASTER VOLUME 0..100
    pub master: u8,
    /// opt.0xF BGM VOLUME 0..100
    pub bgm_volume: u8,
    pub custom: Custom,
    /// opt.0x44 CUSTOM item unlocks
    pub custom_unlock: u32,
    /// opt.0x4C selectable characters
    pub characters: u32,
    /// opt.0x50 fourth costume per character (arcade clears)
    pub costume4: u32,
    /// opt.0x60 bits 16..26 arcade cleared with c, bits 0..10 story cleared
    pub cleared: u32,
    /// CONTROLLER CONFIG. (save +0x3F0 per player): logical bits of □ × ○ △ R1 R2 L1 L2; `UNSET` is unset (the
    /// defaults'), which a load resolves to the game data's default map (`resolve_buttons`)
    pub buttons: [[u16; 8]; 2],
    /// opt.0x28 MOVIES unlocked: bit 0 OPENING, 1 STORY, 2 ENDING1, 3 ENDING2 (set when the ending plays them)
    pub movies: u32,
    /// opt.0x54..0x5C PICTURES unlocked (bit n = picture n of /STY/PIC, set when story mode shows it)
    pub pictures: [u32; 3],
    /// opt.0x264: the PLAY DATA statistics per character (0x24 bytes each)
    pub stats: [PlayStats; 11],
    /// opt.0..2: the last name entered (default of the next NAME ENTRY)
    pub last_name: [u8; 3],
    /// the rankings (+0x84 of opt): 0 arcade (clear time), 1 VS RANKING (`c` = the best streak
    /// fighter+0xE0, a tie ranks above; filled at the NAME ENTRY exit of arcade / versus), 2 survival (wins)
    pub ranking: [[RankRec; 10]; 3],
    /// opt.0x22 AUTO SAVE (MEMORY CARD screen): show the auto-save screen on the way back to the title
    pub auto_save: u8,
    /// opt.0x20 (default 1, saved with opt, no menu item and no writer in the game): the attract sound. Main
    /// modes 3..7 (logos, opening, title, demo, ranking) set the SE mode (opt.20 ? 1 : 2); the demo's
    /// stage BGM and the ranking's music play only when it is set
    pub attract_sound: u8,
}

/// PLAY DATA record of a character (opt.0x264 + 0x24 n), kept at a play, a win and
/// loss: +0 plays, +2 wins, +4/+6 wins / matches against the CPU, +8/+A against humans, +C the best
/// win streak, +E wins against each character (human opponents).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PlayStats {
    pub plays: u16,
    pub wins: u16,
    pub com_wins: u16,
    pub com_games: u16,
    pub hum_wins: u16,
    pub hum_games: u16,
    pub best: u16,
    pub vs: [u16; 11],
}

/// One ranking record (16 bytes): name, character, value (+8: clear time in frames), +0xC / +0xE (fighter
/// +0xE0 / +0xE6, the win counters).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RankRec {
    pub name: [u8; 3],
    pub chr: u8,
    pub value: i32,
    pub c: i16,
    pub e: i16,
}

/// The default rankings (the static tables' type 0 and type 5): "HUD".
pub fn default_ranking() -> [[RankRec; 10]; 3] {
    let ab = [5i16, 4, 3, 2, 1, 1, 1, 1, 1, 1];
    let t0: [RankRec; 10] =
        std::array::from_fn(|i| RankRec { name: *b"HUD", chr: i as u8, value: if i == 9 { 356_400 } else { 36_000 * (i as i32 + 1) }, c: ab[i], e: ab[i] });
    let chr5 = [8u8, 7, 6, 5, 4, 3, 2, 1, 0, 10];
    let t5: [RankRec; 10] = std::array::from_fn(|i| RankRec { name: *b"HUD", chr: chr5[i], value: 72_000 + 18_000 * i as i32, c: ab[i], e: ab[i] });
    [t0, t5, t0]
}

/// An unset button map (`Settings::buttons`) until the game data's default map replaces it: no action's value (an
/// entry of zeros is CONTROLLER CONFIG.'s "nothing", which a player may set on every button and save).
pub const UNSET: [u16; 8] = [0xFFFF; 8];

impl Default for Settings {
    fn default() -> Self {
        Self {
            difficulty: 3,
            attack_level: 3,
            time_limit: 2,
            match_point: 1,
            char_select: 0,
            display: 0,
            vibration: 0,
            output: 1,
            bgm_select: 0,
            master: 100,
            bgm_volume: 60,
            custom: Custom::default(),
            custom_unlock: 0x0C40,
            characters: 0x1FF,
            costume4: 0,
            cleared: 0,
            buttons: [UNSET; 2],
            movies: 1,
            stats: [PlayStats::default(); 11],
            pictures: [0; 3],
            last_name: [0; 3],
            ranking: default_ranking(),
            auto_save: 0,
            attract_sound: 1,
        }
    }
}

/// How the picture fits the window (a engine-only display preference, the PS1 is 4:3 only).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Aspect {
    /// the 3D scene fills the window (more stage on the sides), HUD and front end in a centred 4:3 area
    #[default]
    #[serde(rename = "wide")]
    Wide,
    /// exactly the console game: everything in a centred 4:3 area, black bars around it
    #[serde(rename = "4:3")]
    Four3,
}

impl Aspect {
    /// The renderer's window layout for this preference.
    pub fn fit(self) -> crate::output::layout::Fit {
        match self {
            Aspect::Wide => crate::output::layout::Fit::Fill,
            Aspect::Four3 => crate::output::layout::Fit::Classic,
        }
    }
}

/// Engine-only preferences, kept apart from the memory card data (`Settings`, save.json): `prefs.json` beside it
/// on desktop, `localStorage["okb_prefs"]` on the web (moved once from `br2_prefs`). The OPTIONS → GRAPHICS screen (engine addition) edits them;
/// the defaults are the engine's look before that screen existed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Prefs {
    /// SCREEN MODE: FULL (Wide) / CLASSIC (Four3); also F5
    pub aspect: Aspect,
    /// WINDOW: borderless fullscreen (also Alt+Enter)
    pub fullscreen: bool,
    /// WINDOW SIZE: index into `WINDOW_SIZES` (desktop, windowed)
    pub window: u8,
    /// VSYNC (desktop)
    pub vsync: bool,
    /// FRAME CAP: at most 60 drawn frames a second (desktop; the game logic is a fixed 60 Hz step either way)
    pub frame_cap: bool,
    /// RENDER QUALITY: index into `RENDER_SCALES` (the 3D scene's resolution against the window)
    pub render_scale: u8,
    /// HUD SIZE: index into `HUD_SCALES`
    pub hud_size: u8,
    /// PERF OVERLAY (also the ` key)
    pub perf: bool,
}

/// WINDOW SIZE presets (logical points; clamped to the monitor).
pub const WINDOW_SIZES: [(u32, u32); 5] = [(960, 720), (1280, 720), (1600, 900), (1920, 1080), (2560, 1440)];
/// RENDER QUALITY: the 3D scene's internal resolution as a fraction of the window's.
pub const RENDER_SCALES: [f32; 4] = [0.5, 1.0, 1.5, 2.0];
/// HUD SIZE: SMALL / NORMAL / LARGE.
pub const HUD_SCALES: [f32; 3] = [0.8, 1.0, 1.25];

impl Default for Prefs {
    fn default() -> Self {
        Self { aspect: Aspect::Wide, fullscreen: false, window: 1, vsync: true, frame_cap: false, render_scale: 1, hud_size: 1, perf: false }
    }
}

impl Prefs {
    pub fn window_size(&self) -> (u32, u32) {
        WINDOW_SIZES[(self.window as usize).min(WINDOW_SIZES.len() - 1)]
    }

    pub fn render_scale(&self) -> f32 {
        RENDER_SCALES[(self.render_scale as usize).min(RENDER_SCALES.len() - 1)]
    }

    pub fn hud_scale(&self) -> f32 {
        HUD_SCALES[(self.hud_size as usize).min(HUD_SCALES.len() - 1)]
    }
}

impl Prefs {
    /// The stored preferences (`platform::storage::read_prefs`; defaults when missing or unreadable); env
    /// `OKB_ASPECT=4:3` / `wide` overrides.
    pub fn from_stored(text: Option<String>) -> Self {
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let mut p: Self = text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        #[cfg(not(target_arch = "wasm32"))]
        match crate::base::env::var("ASPECT").as_deref() {
            Ok("4:3" | "43" | "4/3") => p.aspect = Aspect::Four3,
            Ok("wide" | "WIDE") => p.aspect = Aspect::Wide,
            _ => {}
        }
        p
    }

    /// The text `platform::storage::write_prefs` stores; None when it could not be encoded.
    pub fn to_stored(&self) -> Option<String> {
        serde_json::to_string_pretty(self).ok()
    }
}

/// Why a LOAD failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardError {
    /// no save file: NO BLOODY ROAR 2 DATA DETECTED !
    NoData,
    /// the file does not parse: DATA ERROR !!
    DataError,
}

impl Settings {
    /// Each unset button map (`UNSET`) becomes the game data's default map (`input.default_buttons`; without the data
    /// the maps stay unset); a map the player set stays, one of nothing but "nothing" too.
    pub fn resolve_buttons(&mut self) {
        let d = crate::assets::game_tables::default_buttons();
        if d == [0; 8] {
            // no game data yet: the map stays unset, so a save made now cannot keep "nothing bound"
            return;
        }
        for b in &mut self.buttons {
            if *b == UNSET {
                *b = d;
            }
        }
    }

    fn resolved(mut self) -> Self {
        self.resolve_buttons();
        self
    }

    /// OPTIONS → DEFAULT (the defaults for 7, 8, 9, 5/6, 0x11; display and vibration 0).
    pub fn options_default(&mut self) {
        let d = Self::default();
        (self.difficulty, self.attack_level, self.time_limit, self.match_point, self.char_select, self.display, self.vibration) =
            (d.difficulty, d.attack_level, d.time_limit, d.match_point, d.char_select, d.display, d.vibration);
    }

    pub fn sound_default(&mut self) {
        let d = Self::default();
        (self.output, self.bgm_select, self.master, self.bgm_volume) = (d.output, d.bgm_select, d.master, d.bgm_volume);
    }

    /// MEMORY CARD SLOT 1 holds a main-game file (CHECK: 0xF "exists", else 0 "no file").
    pub fn exists(card: &Card) -> bool {
        card.file.is_some()
    }

    /// LOAD: no file → result 5, unreadable → 2 (DATA ERROR).
    pub fn try_load(card: &Card) -> Result<Self, CardError> {
        let text = card.file.as_deref().ok_or(CardError::NoData)?;
        let mut s = serde_json::from_slice::<Settings>(text).map_err(|e| {
            log::warn!("settings unreadable ({e})");
            CardError::DataError
        })?;
        s.resolve_buttons();
        Ok(s)
    }

    /// The settings on the card, or the defaults.
    pub fn load(card: &Card) -> Self {
        match Self::try_load(card) {
            Ok(s) => {
                log::info!("settings loaded");
                s
            }
            Err(CardError::DataError) => {
                log::warn!("using the defaults");
                Self::default().resolved()
            }
            Err(CardError::NoData) => Self::default().resolved(),
        }
    }

    /// SAVE: the file written to the card (the app stores it, `Card::persist`); false when it could not be
    /// encoded (result 6, ERROR !).
    pub fn save(&self, card: &mut Card) -> bool {
        let Ok(text) = serde_json::to_string_pretty(self) else { return false };
        card.file = Some(text.into_bytes());
        card.dirty = true;
        true
    }
}

/// The virtual memory card in MEMORY CARD SLOT 1: the main-game file's bytes (save.json's text). The logic reads and
/// writes only this (`Settings::load` / `try_load` / `exists` / `save`); the app fills it from the disk /
/// localStorage once at start (`platform::storage::read_card`) and stores it after every step that wrote it
/// (`take_dirty`, `platform::storage::store_card`, `stored`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    /// the file, `None` = NO BLOODY ROAR 2 DATA
    file: Option<Vec<u8>>,
    /// written since the last step's end (`take_dirty`)
    dirty: bool,
    /// the app's last store of the file failed (the memory card dialog's SAVE then shows ERROR !)
    write_failed: bool,
    /// debug BR2_MCFAULT: a forced card result (taken by the first matching op)
    pub fault: Option<crate::frontend::memcard::Fault>,
}

impl Card {
    pub fn new(file: Option<Vec<u8>>, fault: Option<crate::frontend::memcard::Fault>) -> Self {
        Self { file, dirty: false, write_failed: false, fault }
    }

    /// The file's bytes, as the app stores them.
    pub fn file(&self) -> Option<&[u8]> {
        self.file.as_deref()
    }

    /// Whether the app's last store failed (taken: the dialog's SAVE result shows it once).
    pub fn take_write_failed(&mut self) -> bool {
        std::mem::take(&mut self.write_failed)
    }

    /// The step's end: whether the file was written since the last call (`session::Out::card_dirty`; the app
    /// then stores it with `platform::storage::store_card`).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// The app's store of the file after the last step (`session::FrameInput`'s card bits, applied at the start
    /// of the next step): a failure is kept in `write_failed`, which the memory card dialog's SAVE result (state
    /// 0x15, the step after the op) turns into result 6, ERROR !, as on the PS1.
    pub fn stored(&mut self, ok: bool) {
        self.write_failed = !ok;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_from_before_graphics_keep_the_defaults() {
        // a prefs.json written before the GRAPHICS screen: only the aspect
        let p: Prefs = serde_json::from_str(r#"{ "aspect": "4:3" }"#).unwrap();
        assert_eq!(p, Prefs { aspect: Aspect::Four3, ..Prefs::default() });
        // the defaults are the earlier look: 1280×720, vsync, no cap, scale 1, HUD 1, no overlay
        let d = Prefs::default();
        assert_eq!((d.aspect, d.fullscreen, d.window_size(), d.vsync, d.frame_cap), (Aspect::Wide, false, (1280, 720), true, false));
        assert_eq!((d.render_scale(), d.hud_scale(), d.perf), (1.0, 1.0, false));
        let back: Prefs = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn screen_mode_starts_wide_until_one_is_saved() {
        // every target (Android included): nothing saved, or a prefs.json without the aspect, starts in WIDE; a
        // saved CLASSIC is kept
        if crate::base::env::var_os("ASPECT").is_some() {
            return;
        }
        assert_eq!(Prefs::from_stored(None).aspect, Aspect::Wide);
        assert_eq!(Prefs::from_stored(Some(r#"{ "vsync": false }"#.into())).aspect, Aspect::Wide);
        assert_eq!(Prefs::from_stored(Some("not json".into())).aspect, Aspect::Wide);
        let classic = Prefs { aspect: Aspect::Four3, ..Prefs::default() };
        assert_eq!(Prefs::from_stored(classic.to_stored()), classic);
    }

    #[test]
    fn card_round_trips_the_save_json_format() {
        let mut s = Settings {
            difficulty: 5,
            cleared: 0x0001_0002,
            last_name: *b"ABC",
            buttons: [[0x40, 0x80, 0x20, 0x408, 0xC0, 0, 0x02, 0x01]; 2],
            ..Settings::default()
        };
        s.stats[3].wins = 7;
        s.ranking[1][0].value = 1234;
        // SAVE writes save.json's text (pretty JSON) to the card, in memory only
        let mut card = Card::default();
        assert!(!Settings::exists(&card));
        assert_eq!(Settings::try_load(&card), Err(CardError::NoData));
        assert!(s.save(&mut card));
        let text = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(card.file(), Some(text.as_bytes()));
        assert!(text.starts_with("{\n  \"difficulty\": 5,\n  \"attack_level\": 3,\n"));
        assert!(card.dirty);
        assert!(Settings::exists(&card));
        assert_eq!(Settings::try_load(&card), Ok(s));
        // a card read from such a file loads the same settings and saves them back byte for byte
        let mut read = Card::new(Some(text.clone().into_bytes()), None);
        assert!(!read.dirty);
        assert_eq!(Settings::load(&read), s);
        assert!(Settings::load(&read).save(&mut read));
        assert_eq!(read.file(), Some(text.as_bytes()));
        // an older file without the later fields keeps their defaults; a broken file is DATA ERROR
        let old = Card::new(Some(br#"{ "difficulty": 6, "auto_save": 1 }"#.to_vec()), None);
        // (its button maps unset: the game data's default map when installed)
        let mut want = Settings { difficulty: 6, auto_save: 1, ..Settings::default() };
        want.resolve_buttons();
        assert_eq!(Settings::try_load(&old), Ok(want));
        assert_eq!(Settings::try_load(&Card::new(Some(b"{ not json".to_vec()), None)), Err(CardError::DataError));
    }

    #[test]
    fn a_failed_store_is_the_saves_error() {
        use crate::frontend::memcard::{CardDialog, Out};
        // the auto-save dialog: SAVE (no question), the op after the "NOW SAVING" hold; `stored` = the app's store
        let run = |stored: bool| {
            let (mut s, mut dirty, mut card, mut snd) = (Settings::default(), true, Card::default(), Vec::new());
            let mut d = CardDialog::auto();
            let mut frames = 0;
            while !card.dirty {
                assert_eq!(d.tick(0, 0, &mut s, &mut dirty, &mut card, &mut snd), Out::Stay);
                frames += 1;
                assert!(frames < 60);
            }
            // the app stores the file after the op's step
            card.dirty = false;
            card.write_failed = !stored;
            d.tick(0, 0, &mut s, &mut dirty, &mut card, &mut snd);
            assert!(!card.write_failed);
            // COMPLETED leaves by itself; ERROR ! waits for the button, then asks again (the auto-save's retry)
            let mut out = Out::Stay;
            for _ in 0..130 {
                out = d.tick(0, 0, &mut s, &mut dirty, &mut card, &mut snd);
            }
            if out == Out::Stay {
                d.tick(0x40, 0x40, &mut s, &mut dirty, &mut card, &mut snd);
                out = d.tick(0, 0, &mut s, &mut dirty, &mut card, &mut snd);
            }
            (dirty, out)
        };
        assert_eq!(run(true), (false, Out::Done));
        assert_eq!(run(false), (true, Out::Stay));
    }

    /// An unset button map (the defaults', an old file's explicit map aside) loads as the game data's default map
    /// (data-gated); a map the player set loads unchanged.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn unset_buttons_resolve_to_the_table() {
        let Some(t) = crate::formats::tables::export_tables() else {
            eprintln!("skipped: no okb_export");
            return;
        };
        let table: [u16; 8] = std::array::from_fn(|k| t.table("input.default_buttons").u16(2 * k));
        assert_ne!(table, UNSET);
        assert!(!crate::frontend::menus::cc_values().contains(&UNSET[0]));
        let mine = [0x40, 0x80, 0x20, 0x408, 0xC0, 0, 0x02, 0x01];
        let mut card = Card::default();
        assert!(Settings { buttons: [mine, UNSET], ..Settings::default() }.save(&mut card));
        assert_eq!(Settings::load(&card).buttons, [mine, table]);
        // a map of nothing but "nothing" is the player's
        assert!(Settings { buttons: [[0; 8], UNSET], ..Settings::default() }.save(&mut card));
        assert_eq!(Settings::load(&card).buttons, [[0; 8], table]);
        assert_eq!(Settings::load(&Card::default()).buttons, [table; 2]);
    }
}
