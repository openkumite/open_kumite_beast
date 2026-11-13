//! State hashes: FNV-1a 64 of the state's bytes, by section, and the STATEHASH trace.
//!
//! Engine addition.
//! State: none (hashes the serialised `GameState`: its bytes are the state's format)
//!
//! State hashes: FNV-1a 64 of the state's postcard bytes (`hash`), by part for a desync report (`section_hashes`),
//! and the debug STATEHASH trace (`HashTrace`).

use super::GameState;
use crate::base::hash::{FNV_BASIS, fnv1a};
use serde::Serialize;
use std::io::Write;

/// FNV-1a 64 of the state's postcard bytes (the debug STATEHASH trace, desync checks). Like the simulation, the bytes
/// depend on the installed game data, its text included (`assets::text`: the round's message is serialised as its
/// text): peers and replays agree only on the same converted data, which its format pins.
pub fn hash(s: &GameState) -> u64 {
    hash_of(s)
}

/// The hashes of the state's parts, for a desync report: the whole state, the fight's match by part, its HUD,
/// the front end, its memory card, the backdrop and the session's own fields.
pub fn section_hashes(s: &GameState) -> Vec<(&'static str, u64)> {
    let mut v = vec![("state", hash(s))];
    if let Some(g) = &s.fight {
        let m = &g.m;
        v.extend([
            ("fighters[0]", hash_of(&m.fighters[0])),
            ("fighters[1]", hash_of(&m.fighters[1])),
            ("world", hash_of(&m.world)),
            ("round", hash_of(&m.round)),
            ("ai", hash_of(&m.ai)),
            ("fx", hash_of(&m.fx)),
            ("vm", hash_of(&m.vm)),
            ("fxn", hash_of(&m.fxn)),
            ("cam", hash_of(&m.cam)),
            ("replay", hash_of(&m.replay)),
            ("br1", hash_of(&m.br1)),
            ("match", hash_of(m)),
            ("hud", hash_of(&g.hud)),
            ("fight", hash_of(g)),
        ]);
    }
    if let Some(f) = &s.front {
        v.extend([("front", hash_of(f)), ("card", hash_of(&f.save.memory_card))]);
    }
    if let Some(g) = &s.backdrop {
        v.push(("backdrop", hash_of(g)));
    }
    if let Some(t) = &s.team {
        v.extend([
            ("team.fighters", hash_of(&t.fighters)),
            ("team.world", hash_of(&t.world)),
            ("team.round", hash_of(&t.round)),
            ("team.ai", hash_of(&t.ai)),
            ("team.fx", hash_of(&t.fx)),
            ("team.vm", hash_of(&t.vm)),
            ("team.cam", hash_of(&t.cam)),
            ("team", hash_of(t)),
        ]);
        if let Some(b) = &t.br1 {
            v.push(("team.br1", hash_of(b)));
        }
    }
    v.push((
        "session",
        hash_of(&(
            s.frame,
            s.music,
            s.music_volume,
            s.spec,
            s.pending,
            &s.fetching,
            s.debug,
            s.sim_version,
            s.browser,
            s.network,
            s.android,
            s.extra_stage,
            s.extra_q3dm6,
            s.br1,
            s.team_spec,
            s.netplay,
        )),
    ));
    if let Some(c) = &s.team_config {
        v.push(("team_config", hash_of(c)));
    }
    v
}

/// Debug STATEHASH=1 (`OKB_STATEHASH` in the app): `frame hash` after every step to stderr, or to the file named
/// by any other value.
pub struct HashTrace(Option<Box<dyn std::io::Write>>);

impl HashTrace {
    pub fn from_env(var: &str) -> Self {
        let out: Option<Box<dyn std::io::Write>> = match std::env::var(var).ok().filter(|v| !v.is_empty() && v != "0") {
            Some(v) if v == "1" => Some(Box::new(std::io::stderr())),
            Some(path) => match std::fs::File::create(&path) {
                Ok(f) => Some(Box::new(std::io::BufWriter::new(f))),
                Err(e) => {
                    log::warn!("{var}: {path}: {e}");
                    None
                }
            },
            None => None,
        };
        Self(out)
    }

    pub fn log(&mut self, s: &GameState) {
        if let Some(w) = &mut self.0 {
            let _ = writeln!(w, "{} {:016x}", s.frame, hash(s));
        }
    }
}

/// FNV-1a 64 of `x`'s postcard bytes, hashed as they are serialised (no buffer).
fn hash_of<T: Serialize>(x: &T) -> u64 {
    postcard::serialize_with_flavor(x, Fnv1a64(FNV_BASIS)).expect("the state serialises")
}

/// FNV-1a, 64 bits, as a postcard output.
struct Fnv1a64(u64);

impl postcard::ser_flavors::Flavor for Fnv1a64 {
    type Output = u64;

    fn try_extend(&mut self, b: &[u8]) -> postcard::Result<()> {
        self.0 = fnv1a(self.0, b);
        Ok(())
    }

    fn try_push(&mut self, x: u8) -> postcard::Result<()> {
        self.try_extend(&[x])
    }

    fn finalize(self) -> postcard::Result<u64> {
        Ok(self.0)
    }
}
