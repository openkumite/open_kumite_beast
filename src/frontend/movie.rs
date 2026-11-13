//! Movies: play, per frame, draw and where the front end goes after.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)

use super::{Event, Front, Screen, Text};
use crate::frontend::menus::{self, Menu};
use crate::output::layout::Anchor;
use crate::output::prim::{self, Prim, ft4};
use crate::output::sound::Queue;
use serde::{Deserialize, Serialize};

/// A movie being played: the frame counter and what it ends at. The stream, its frames in VRAM
/// and its music are the presentation's (`VramView` follows `t`).
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct MoviePlay {
    /// frames since the start
    pub(super) t: u32,
    /// the frame count `t` ends at: the first one whose stream frame (150 sectors/s) is the last frame
    /// (the movie table +8) or the stream's last
    pub(super) end: u32,
    /// the frame size (pixels)
    pub(super) w: u16,
    pub(super) h: u16,
    pub(super) skippable: bool,
}

impl MoviePlay {
    /// Longest movie followed (an hour): `end` of a stream whose frame table never gets there.
    const MAX_T: u32 = 60 * 60 * 60;

    /// `last` = the movie's last frame number.
    pub(super) fn new(m: &crate::formats::movie::Movie, last: usize, skippable: bool) -> Self {
        let lim = last.min(m.frames.len());
        let end = (1..Self::MAX_T).find(|&t| m.frame_at(t as f64 / 60.0) + 1 >= lim).unwrap_or(Self::MAX_T);
        Self { t: 0, end, w: m.w.min(1024) as u16, h: m.h.min(512) as u16, skippable }
    }

    /// Engine addition (simulation v21): an imported movie whose stream is missing, black for its fixed `end`.
    pub(super) fn black(end: u32, skippable: bool) -> Self {
        Self { t: 0, end, w: 0, h: 0, skippable }
    }
}

/// Where the front end goes after a movie.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)] // Copy, short-lived
pub(super) enum After {
    Menu {
        menu: Menu,
        parent: Option<Menu>,
    },
    /// STORY: the character select
    Select,
    Title,
    /// the opening movie of the boot / attract loop (main mode 4): always skippable, not marked as seen
    Boot,
    /// the arcade ending: CONGRATULATIONS (then a waiting NAME ENTRY)
    Congrats {
        to_custom: bool,
    },
    /// engine addition (the import S10): an imported fighter's arcade ending: its staff roll, movie `n`
    /// (`BR1_MOVIES`), then CONGRATULATIONS
    Staff {
        n: u8,
    },
}

/// Engine addition (the import S10): the imported game's install's movies after the main game's four (movie numbers
/// 4..): Mitsuko's, Fox's and Greg's endings and the two staff rolls (the imported game's file table: 8 MITUKO,
/// 10 MITUKOST "STAFF ROLL B", 6 FOX, 2 GREG, 9 ALICESTF "STAFF ROLL A"); played to their last frame. Their "seen" bits
/// (opt.28's 4..8: skippable once seen) are beside the main game's.
pub(crate) const BR1_MOVIES: [&str; 5] = ["MITUKO", "MITUKOST", "FOX", "GREG", "ALICESTF"];

/// The imported game's STAFF ROLL A (`BR1_MOVIES`' ALICESTF), after every ending but Mitsuko's.
pub(super) const BR1_STAFF_A: usize = menus::MOVIE_FILES.len() + 4;

/// Imported cell `k`'s (`roster::BR1_CODES`) arcade ending movie and staff roll (the imported game's jump table
/// MIT MITUKO + STAFF ROLL B, FOX FOX + A, GRE GREG + A); None for Uriko (no movie of her own).
pub(super) fn br1_ending(k: u8) -> Option<(usize, usize)> {
    let b = menus::MOVIE_FILES.len();
    match k {
        0 => Some((b, b + 1)),
        1 => Some((b + 2, BR1_STAFF_A)),
        2 => Some((b + 3, BR1_STAFF_A)),
        _ => None,
    }
}

/// The movies by number: the main game's (`menus::MOVIE_FILES`), then the imported game's.
pub(crate) const COUNT: usize = menus::MOVIE_FILES.len() + BR1_MOVIES.len();

/// Movie `n`'s name (`files`).
pub(crate) fn name(n: usize) -> &'static str {
    let b = menus::MOVIE_FILES.len();
    if n < b { menus::MOVIE_FILES[n] } else { BR1_MOVIES.get(n - b).copied().unwrap_or("") }
}

/// Movie `n`'s last frame number: the main game's table, an imported movie's stream to its end.
pub(super) fn last(n: usize) -> usize {
    crate::assets::game_tables::get().movie_last.get(n).copied().unwrap_or(usize::MAX)
}

/// Engine addition (the import S11, simulation v21): the frames an imported movie (`BR1_MOVIES`) plays, fixed:
/// what `MoviePlay::new` gives each of the disc's streams (`tests::br1_movie_lengths_as_the_disc`), so that an ending's
/// length never depends on the install's movie files (they are not part of its content id). Before v21 the stream's
/// own frame table decides.
pub(super) const BR1_MOVIE_END: [u32; 5] = [2175, 5951, 2956, 3860, 5951];

/// The imported game's movie `n`'s fixed length from simulation v21 (None: a main-game movie, or an older version).
pub(super) fn br1_end(n: usize, g: crate::fight::version::Gates) -> Option<u32> {
    let k = n.checked_sub(menus::MOVIE_FILES.len())?;
    BR1_MOVIE_END.get(k).copied().filter(|_| g.br1_fixed_movies)
}

/// Movie `n`'s files: the stream and its music (by its MP3 name: the fetch reads the `.xa` beside it first,
/// `Assets::load_music`).
pub(crate) fn files(n: usize) -> [String; 2] {
    let f = name(n);
    if n < menus::MOVIE_FILES.len() {
        [format!("video/{f}.bsf"), format!("video/{f}.mp3")]
    } else {
        // The imported game's install's (`assets::BR1_PREFIX`)
        let p = crate::assets::BR1_PREFIX;
        [format!("{p}movies/{f}.bsf"), format!("{p}movies/{f}.mp3")]
    }
}

impl Front {
    /// Start movie `n` (file table 8 + n), then `then`.
    pub(super) fn play_movie(&mut self, n: usize, then: After, audio: &mut Queue) -> Event {
        audio.stop_music();
        self.screen = Screen::Movie { n, then };
        Event::Fetch(files(n).to_vec())
    }

    pub(super) fn after_movie(&mut self, then: After, audio: &mut Queue) {
        // the ending's movie unlock and the story pictures set the dirty flag
        if !matches!(then, After::Menu { .. } | After::Boot) {
            self.save.dirty = true;
        }
        self.save.settings.save(&mut self.save.memory_card);
        match then {
            After::Menu { menu, parent } => self.screen = Screen::Menu { menu, parent },
            After::Select => {
                self.boot_vram();
                self.enter_select(audio);
            }
            After::Title | After::Boot => {
                self.boot_vram();
                self.to_title(audio);
            }
            After::Congrats { to_custom } => {
                self.boot_vram();
                self.screen = Screen::Message { t: 300, text: Text::Congratulations, to_custom };
            }
            // engine addition (the import S10): the staff roll after an imported ending
            After::Staff { n } => self.pending_event = Some(self.play_movie(n as usize, After::Congrats { to_custom: false }, audio)),
        }
    }

    /// One frame of a movie (its frame due at the stream position goes into VRAM (0, 0): `VramView`); START / × / △
    /// skip (mask 0x850), the end is the last frame number.
    pub(super) fn tick_movie(&mut self, press: u16, audio: &mut Queue) -> bool {
        let Some(mp) = &mut self.movie else { return false };
        mp.t += 1;
        if (mp.skippable && press & 0x850 != 0) || mp.t >= mp.end {
            audio.stop_music();
            self.movie = None;
            self.restore_vram();
            return true;
        }
        false
    }

    pub(super) fn draw_movie(&self, out: &mut Vec<Prim>) {
        out.push(prim::f4(0, 0, 512, 480, [0, 0, 0]));
        let Some(mp) = &self.movie else { return };
        if mp.t == 0 || mp.w == 0 {
            return;
        }
        let (w, h) = (mp.w as i32, mp.h as i32);
        // 15-bit pages at x 0 and 128 (a page is 256 pixels wide); the 320-pixel display fills the screen
        let y = (240 - h) / 2 * 2;
        let half = w / 2;
        let sx = 512 * half / 320;
        // engine addition: the window around the frame gets its ambient fill
        out.push(ft4(0, y, sx, h * 2, (0, 0, half, h), 0x100, 0).at(Anchor::AMBIENT));
        out.push(ft4(sx, y, 512 - sx, h * 2, (half - 128, 0, w - half, h), 0x102, 0).at(Anchor::AMBIENT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BR1_MOVIE_END` is what `MoviePlay::new` gives the disc's streams (the imported game's install beside the engine;
    /// data-gated).
    #[test]
    fn br1_movie_lengths_as_the_disc() {
        let mut ends = Vec::new();
        for n in 0..BR1_MOVIES.len() {
            let Some(b) = crate::formats::br1_anm::tests::br1_file(&format!("movies/{}.bsf", BR1_MOVIES[n])) else {
                eprintln!("skipped: no okb_export/br1");
                return;
            };
            let m = crate::formats::movie::Movie::parse(b).expect("a stream");
            ends.push(MoviePlay::new(&m, last(menus::MOVIE_FILES.len() + n), false).end);
        }
        eprintln!("{ends:?}");
        assert_eq!(ends, BR1_MOVIE_END);
    }
}
