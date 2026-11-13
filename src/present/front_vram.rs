//! The front end's VRAM as drawn: the state's loads replayed into an image, a playing movie over it.
//!
//! State: presentation (reads the state, never writes it)
//!
//! The front end's VRAM as drawn (presentation: owned by the app, outside the state): `frontend::Resident`'s loads
//! replayed into a VRAM image for the renderer, with a playing movie's frame over it, its decoded stream and its
//! music. The image is rebuilt from the state at any time (a loaded save state, a rewind).

use crate::formats::vram::Vram;
use crate::frontend::resident::{Load, tim_bytes};
use crate::frontend::{Front, FrontData};
use crate::session::FileCache;
use std::sync::Arc;

/// The movies' first audio sector in their stream: their audio starts this many seconds after the first frame
/// (`VramView::step`).
const MOVIE_XA_LEAD: f64 = 7.0 / crate::formats::movie::SECTORS_PER_SEC;

/// A playing movie's parsed stream.
struct Media {
    n: usize,
    movie: crate::formats::movie::Movie,
}

/// The front end's VRAM as drawn: `Resident`'s loads replayed, a movie frame over them (presentation, outside
/// the state; the app uploads `vram` when `sync` says it changed).
#[derive(Default)]
pub struct VramView {
    pub vram: Vram,
    /// the loads in `vram`, and the movie frame over them
    applied: Vec<Load>,
    frame: Option<usize>,
    /// `Front::vram_gen` of the image (0 = none yet)
    seen: u32,
    /// the image must be checked against the state and go up again (`invalidate`)
    stale: bool,
    movie: Option<Media>,
}

impl VramView {
    /// After a step: the movie's media follow the state (parsed from the fetched files when one starts). Returns
    /// the movie's music to start and where (seconds in), when a movie started or was found running (a loaded
    /// state): the movie's own audio, outside the state.
    pub fn step(&mut self, f: &Front, files: &FileCache) -> Option<(Arc<crate::formats::music::Music>, f64)> {
        if !self.follow(f, files) {
            return None;
        }
        let (n, t) = f.movie_playing()?;
        let [_, audio] = crate::frontend::movie::files(n);
        let bytes = files.get(&audio)?;
        let music = crate::formats::music::decode(bytes.clone()).map(Arc::new)?;
        // the music started with the movie (`Front::fetched`), whose start step already ticked it once: frame t
        // shows over the music (t - 1) / 60 s in. The disc's audio starts with the stream's sector 7 (every
        // /MOV/*.STR: one audio sector in 8, from 7, each 2016 frames at 37.8 kHz = 8 sectors' time; frames are
        // timed by their sector at 150 a second, `Movie::frame_at`), 7 / 150 s after the first frame: a `.xa`'s
        // first sample is played then (a negative start waits). The MP3 of older exports has its encoder's delay
        // (1105 samples at 32 kHz) in front instead and stays as it was.
        let lead = if bytes.starts_with(crate::formats::xa::MAGIC) { MOVIE_XA_LEAD } else { 0.0 };
        Some((music, t.saturating_sub(1) as f64 / 60.0 - lead))
    }

    /// The movie's stream follows the state, without its music (the debug tools' rewind); true when a movie was
    /// parsed now (started or found running).
    pub fn follow(&mut self, f: &Front, files: &FileCache) -> bool {
        let Some((n, _)) = f.movie_playing() else {
            self.movie = None;
            return false;
        };
        if self.movie.as_ref().is_some_and(|m| m.n == n) {
            return false;
        }
        let [bsf, _] = crate::frontend::movie::files(n);
        let Some(movie) = files.get(&bsf).and_then(|b| crate::formats::movie::Movie::parse(b.clone())) else {
            log::warn!("movie {bsf} not in the store");
            return false;
        };
        self.movie = Some(Media { n, movie });
        true
    }

    /// Before drawing: the image brought to the state (the loads since it was last built, or all of them again;
    /// the movie's frame due now over them). True when it changed and must be uploaded.
    pub fn sync(&mut self, f: &Front, data: &FrontData, files: &FileCache) -> bool {
        let mut changed = false;
        if self.stale || f.vram_gen() != self.seen {
            self.stale = false;
            let loads = f.resident().loads();
            if self.frame.is_none() && loads.starts_with(&self.applied) {
                for l in &loads[self.applied.len()..] {
                    Self::apply(&mut self.vram, l, data, files);
                }
            } else {
                self.vram = Vram::default();
                for l in loads {
                    Self::apply(&mut self.vram, l, data, files);
                }
                self.frame = None;
            }
            self.applied = loads.to_vec();
            self.seen = f.vram_gen();
            changed = true;
        }
        // the frame at the stream position (150 sectors/s) into VRAM (0, 0) as 15-bit pixels
        if let (Some((n, t)), Some(m)) = (f.movie_playing(), &self.movie)
            && m.n == n
            && t > 0
        {
            let k = m.movie.frame_at(t as f64 / 60.0);
            if self.frame != Some(k) {
                self.frame = Some(k);
                let px = m.movie.decode(k);
                let (w, h) = (m.movie.w.min(1024), m.movie.h);
                for (r, row) in px.chunks(m.movie.w.max(1)).enumerate().take(h.min(512)) {
                    self.vram.words[r * 1024..r * 1024 + w.min(row.len())].copy_from_slice(&row[..w.min(row.len())]);
                }
                changed = true;
            }
        }
        changed
    }

    /// The renderer's VRAM was replaced or the state put back (the debug tools' load and rewind): the next `sync`
    /// checks the loads again and reports the image changed.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// The movie's media forgotten: the next `step` parses them again and returns its music at the state's frame.
    pub fn forget_movie(&mut self) {
        self.movie = None;
    }

    fn apply(vram: &mut Vram, l: &Load, data: &FrontData, files: &FileCache) {
        let pos = |p: Option<(u16, u16)>| p.map(|(x, y)| (x as usize, y as usize));
        match tim_bytes(&l.tim, data, files) {
            Some(b) => vram.load_tim(&b, pos(l.clut), pos(l.pix)),
            None => log::warn!("VRAM: {:?} not in the store", l.tim),
        }
    }
}
