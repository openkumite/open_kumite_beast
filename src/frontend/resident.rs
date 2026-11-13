//! The front end's VRAM as the state keeps it: which TIMs the logic loaded where.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The front end's VRAM as the state keeps it: which TIMs the logic loaded where (`Resident`, part of the state),
//! and where a TIM's bytes come from (`tim_bytes`). The image itself is the presentation's
//! (`present::front_vram::VramView`), rebuilt from these loads at any time (a loaded save state, a rewind).

use super::FrontData;
use crate::formats::vram::Vram;
use crate::session::FileCache;
use serde::{Deserialize, Serialize};

/// Where a TIM comes from: an entry of a front-end file (`FrontData`) or of a fetched file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tim {
    Init(u8),
    Title(u8),
    Se00,
    P00(u8),
    P01(u8),
    Sel00(u8),
    Sel01(u8),
    Sb(u8),
    Scea,
    Hudson,
    /// entry `.1` of the fetched container `.0` (story pages, pictures, epilogues)
    File(String, u8),
    /// engine addition (the import S9, `assets::br1_art`): imported cell k's select portrait, the imported
    /// row's faces, cell k's VS picture
    Br1Select(u8),
    Br1Faces,
    Br1Vs(u8),
    /// engine addition (the import S10): the fetched file `.0`, itself a TIM (an imported picture)
    Raw(String),
}

/// One `Vram::load_tim`: the TIM, its CLUT and pixel positions, and the rects (x, y, w, h) it writes
/// (`Vram::tim_rects`; None when one crosses the VRAM edge, where `Vram::blit` wraps).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Load {
    pub tim: Tim,
    pub clut: Option<(u16, u16)>,
    pub pix: Option<(u16, u16)>,
    rects: Option<[Option<[u16; 4]>; 2]>,
}

/// The TIMs loaded into VRAM since the front end started, in order: the VRAM is these loads over a cleared one.
/// A load drops the earlier ones whose writes it covers whole, so the list stays as short as the set of places
/// written (the image is the same).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resident {
    loads: Vec<Load>,
}

impl Resident {
    /// Most loads a state may hold (`session::load_state`; the real lists stay far below: at most 38 on the
    /// fetest arcade and select routes, 2026-11-01).
    pub const MAX: usize = 512;

    /// `Vram::load_tim(bytes, clut, pix)` of `tim` (whose bytes are `bytes`).
    pub fn load(&mut self, tim: Tim, bytes: &[u8], clut: Option<(usize, usize)>, pix: Option<(usize, usize)>) {
        let r = Vram::tim_rects(bytes, clut, pix);
        let inside = |r: &Option<[usize; 4]>| r.is_none_or(|r| r[0] + r[2] <= 1024 && r[1] + r[3] <= 512);
        let rects = (inside(&r[0]) && inside(&r[1])).then(|| r.map(|r| r.map(|r| r.map(|v| v as u16))));
        if let Some(new) = rects {
            let covered = |a: &[u16; 4]| new.iter().flatten().any(|b| b[0] <= a[0] && b[1] <= a[1] && a[0] + a[2] <= b[0] + b[2] && a[1] + a[3] <= b[1] + b[3]);
            self.loads.retain(|l| !l.rects.is_some_and(|old| old.iter().flatten().all(covered)));
        }
        let pos = |p: Option<(usize, usize)>| p.map(|(x, y)| (x as u16, y as u16));
        self.loads.push(Load { tim, clut: pos(clut), pix: pos(pix), rects });
        debug_assert!(self.loads.len() <= Self::MAX, "{} VRAM loads: the state would not load again", self.loads.len());
    }

    pub fn loads(&self) -> &[Load] {
        &self.loads
    }

    /// The fetched files the loads read (`Front::file_cached` keeps them in the app's store).
    pub fn uses(&self, name: &str) -> bool {
        self.files().any(|n| n == name)
    }

    /// The names of the fetched files the loads read.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.loads.iter().filter_map(|l| match &l.tim {
            Tim::File(n, _) | Tim::Raw(n) => Some(n.as_str()),
            _ => None,
        })
    }
}

/// A TIM's bytes from the static data or a fetched file (None when it is not there).
pub(crate) fn tim_bytes<'a>(tim: &Tim, data: &'a FrontData, files: &FileCache) -> Option<std::borrow::Cow<'a, [u8]>> {
    use std::borrow::Cow::Borrowed;
    let entry = |v: &'a [Option<Vec<u8>>], k: u8| v.get(k as usize).and_then(|e| e.as_deref()).map(Borrowed);
    match tim {
        Tim::Init(k) => entry(&data.init, *k),
        Tim::Title(k) => entry(&data.title, *k),
        Tim::Se00 => Some(Borrowed(&data.se00)),
        Tim::P00(k) => entry(&data.p00, *k),
        Tim::P01(k) => entry(&data.p01, *k),
        Tim::Sel00(k) => entry(&data.sel00, *k),
        Tim::Sel01(k) => entry(&data.sel01, *k),
        Tim::Sb(k) => data.sb.get(*k as usize).map(|b| Borrowed(b.as_slice())),
        Tim::Scea => data.scea.as_deref().map(Borrowed),
        Tim::Hudson => data.hudson.as_deref().map(Borrowed),
        Tim::Br1Select(k) => data.br1.select.get(*k as usize).map(|b| Borrowed(b.as_slice())),
        Tim::Br1Faces => Some(Borrowed(data.br1.faces.as_slice())),
        Tim::Br1Vs(k) => data.br1.vs.get(*k as usize).map(|b| Borrowed(b.as_slice())),
        Tim::File(name, k) => {
            let c = crate::formats::container::read(files.get(name)?);
            c.into_iter().nth(*k as usize).flatten().map(std::borrow::Cow::Owned)
        }
        Tim::Raw(name) => files.get(name).map(|b| std::borrow::Cow::Owned(b.to_vec())),
    }
}
