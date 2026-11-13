//! The app's replay files: the store on disk or in localStorage, its rotation, export and import.
//!
//! Engine addition.
//!
//! Where the app keeps its replay files (game-state plan stage 6b): the desktop's `<config dir>/replays` folder, or
//! in the browser the page's localStorage (base64 text under `br2r/<name>`, a key prefix kept from earlier builds, at most `WEB_BUDGET` of it beside the
//! memory card; in memory only when there is none, or when a write is refused). Each finished match is added
//! (`Store::add`), then the rotation keeps the newest `KEEP_LAST` and every KEEP-marked one. The replay list reads
//! the headers (`Store::list`, newest first); files that are not replays this build reads are left alone. Export /
//! import (desktop): copies to `replays/export`, and the `.okbr` (and earlier `.br2r`) files found in `replays/import` (moved to
//! `import/done` once taken; a recording the store has already is not added again); the browser downloads and
//! uploads (`app::web_files`).

use crate::session::replay::{Header, Replay};

/// Match files kept besides the KEEP-marked ones.
pub const KEEP_LAST: usize = 20;

/// A replay of the store, as the list shows it.
#[derive(Clone, Debug)]
pub struct Item {
    /// its file name (desktop) / storage name (browser), `<date>-<time>_<P1>-<P2>.okbr`
    pub key: String,
    pub header: Header,
    /// when it was written (seconds since 1970; 0 when unknown)
    pub written: u64,
}

/// The list's order: newest first (the header's time, then when it was written, then the name).
pub fn sort_newest(items: &mut [Item]) {
    items.sort_by(|a, b| (b.header.time, b.written, &b.key).cmp(&(a.header.time, a.written, &a.key)));
}

/// The rotation: the items beyond the newest `keep` that are not KEEP-marked, oldest first.
pub fn rotated_out(items: &[Item], keep: usize) -> Vec<String> {
    let mut v: Vec<&Item> = items.iter().filter(|i| !i.header.keep).collect();
    v.sort_by(|a, b| (a.header.time, a.written, &a.key).cmp(&(b.header.time, b.written, &b.key)));
    let n = v.len().saturating_sub(keep);
    v[..n].iter().map(|i| i.key.clone()).collect()
}

/// `<date>-<time UTC>_<P1>-<P2>`, e.g. `20231114-221320_YUG-ALI` (no extension).
pub fn base_name(r: &Replay) -> String {
    let (y, mo, d, h, mi, s) = utc(r.header.time);
    let sides = r
        .header
        .spec
        .map(|m| m.chr.map(|c| if (c as usize) < crate::assets::roster::SELECTABLE { crate::assets::roster::code(c) } else { "X" }))
        .map(|[a, b]| format!("_{a}-{b}"))
        // a team battle (engine addition): its four codes
        .or_else(|| {
            r.header.team.map(|t| {
                format!("_{}", t.chr.map(|c| if (c as usize) < crate::assets::roster::SELECTABLE { crate::assets::roster::code(c) } else { "X" }).join("-"))
            })
        });
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}{}", sides.unwrap_or_default())
}

/// Seconds since 1970 as the UTC date and time (year, month, day, hour, minute, second).
pub fn utc(t: u64) -> (i64, u32, u32, u32, u32, u32) {
    // days to the civil date (H. Hinnant's algorithm)
    let z = (t / 86_400) as i64 + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    let s = (t % 86_400) as u32;
    (y, m, d, s / 3600, s / 60 % 60, s % 60)
}

/// An imported file's replay: every check of a replay file (a newer one is refused with its message), then
/// marked KEEP (an imported replay is older than the newest matches; the rotation would drop it at once).
pub fn import(bytes: &[u8]) -> Result<Replay, String> {
    let mut r = Replay::decode(bytes)?;
    r.header.keep = true;
    Ok(r)
}

/// The same recording (its name and KEEP mark aside): the version, when it was saved, its part and every step.
pub fn same(a: &Replay, b: &Replay) -> bool {
    let (x, y) = (&a.header, &b.header);
    (a.sim_version, x.time, x.frames, x.from, x.fight_from) == (b.sim_version, y.time, y.frames, y.from, y.fight_from)
        && (a.body.inputs == b.body.inputs && a.body.checks == b.body.checks)
}

/// A write that went through: None = stored for good; Some(why) = in the store's memory only, until the page closes
/// (the browser's storage refused it).
pub type Kept = Option<String>;

/// What an IMPORT did with a file.
#[derive(Debug, PartialEq, Eq)]
pub enum Imported {
    /// added under this name (`Kept`)
    Added(String, Kept),
    /// the store has it already (under this name)
    There(String),
}

impl Store {
    /// IMPORT of a file's bytes: validated in full (`import`), then added unless the store has the same recording.
    pub fn import(&mut self, bytes: &[u8]) -> Result<Imported, String> {
        let r = import(bytes)?;
        let (h, items) = (&r.header, self.list());
        let like = items.iter().filter(|i| (i.header.time, i.header.frames, i.header.from) == (h.time, h.frames, h.from));
        for i in like {
            if self.read(&i.key).and_then(|b| Replay::decode(&b)).is_ok_and(|o| same(&o, &r)) {
                return Ok(Imported::There(i.key.clone()));
            }
        }
        self.add(&r).map(|(key, kept)| Imported::Added(key, kept))
    }
}

/// The replay with a new name (trimmed; empty = none) or KEEP mark, re-encoded (the header is checked again).
pub fn edit(bytes: &[u8], f: impl FnOnce(&mut Header)) -> Result<Vec<u8>, String> {
    let mut r = Replay::decode(bytes)?;
    f(&mut r.header);
    if let Some(n) = &mut r.header.name {
        *n = n.trim().to_string();
    }
    if r.header.name.as_ref().is_some_and(|n| n.is_empty()) {
        r.header.name = None;
    }
    let b = r.encode();
    Replay::decode_header(&b)?;
    Ok(b)
}

// ------------------------------------------------------------------ desktop: the replays folder

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Store {
    dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl Store {
    pub fn new() -> Self {
        Self { dir: crate::platform::storage::replay_dir() }
    }

    #[cfg(test)]
    pub fn at(dir: std::path::PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, key: &str) -> Result<std::path::PathBuf, String> {
        // a key is a file name of the folder, never a path
        if key.is_empty() || key.contains(['/', '\\']) || key.starts_with('.') {
            return Err(format!("replay name {key:?}"));
        }
        Ok(self.dir.join(key))
    }

    /// The replays of the folder, newest first.
    pub fn list(&self) -> Vec<Item> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut v: Vec<Item> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(is_replay))
            .filter_map(|p| {
                let (_, _, header) = Replay::decode_header(&crate::platform::storage::read_capped(&p, crate::session::replay::MAX_BYTES).ok()?).ok()?;
                let written = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
                Some(Item { key: p.file_name()?.to_str()?.to_string(), header, written: written.map_or(0, |d| d.as_secs()) })
            })
            .collect();
        sort_newest(&mut v);
        v
    }

    pub fn read(&self, key: &str) -> Result<Vec<u8>, String> {
        crate::platform::storage::read_capped(&self.path(key)?, crate::session::replay::MAX_BYTES)
    }

    pub fn write(&mut self, key: &str, b: &[u8]) -> Result<Kept, String> {
        let p = self.path(key)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        // a new file beside it, then renamed over it: a crash never leaves half a replay
        let tmp = self.dir.join(format!(".{key}.tmp"));
        std::fs::write(&tmp, b).and_then(|_| std::fs::rename(&tmp, &p)).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(None)
    }

    pub fn remove(&mut self, key: &str) -> Result<(), String> {
        let p = self.path(key)?;
        std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))
    }

    fn exists(&self, key: &str) -> bool {
        self.dir.join(key).exists()
    }

    /// A new replay under a name of its own, then the rotation; its name (always stored: `Kept` None).
    pub fn add(&mut self, r: &Replay) -> Result<(String, Kept), String> {
        let key = self.free_key(&base_name(r));
        let kept = self.write(&key, &r.encode())?;
        self.prune(KEEP_LAST);
        Ok((key, kept))
    }

    fn free_key(&self, base: &str) -> String {
        let mut key = format!("{base}.{EXT}");
        let mut k = 2;
        while self.exists(&key) {
            key = format!("{base}_{k}.{EXT}");
            k += 1;
        }
        key
    }

    /// The rotation (`rotated_out`); the number deleted.
    pub fn prune(&mut self, keep: usize) -> usize {
        let items = self.list();
        let mut n = 0;
        for key in rotated_out(&items, keep) {
            match self.remove(&key) {
                Ok(()) => n += 1,
                Err(e) => log::warn!("replay {e}"),
            }
        }
        n
    }

    /// EXPORT: a copy in `replays/export`; its path.
    pub fn export(&self, key: &str) -> Result<String, String> {
        let b = self.read(key)?;
        let dir = self.dir.join("export");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let p = dir.join(key);
        std::fs::write(&p, b).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(p.display().to_string())
    }

    /// IMPORT: the folder the files are taken from (made when missing, so the player finds it).
    pub fn import_dir(&self) -> std::path::PathBuf {
        let d = self.dir.join("import");
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// IMPORT: the `.okbr` / `.br2r` files of `replays/import`, at most `MAX_IMPORT` (read one at a time by the caller).
    pub fn import_paths(&self) -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(self.import_dir()) else { return Vec::new() };
        let mut v: Vec<std::path::PathBuf> =
            entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file() && p.extension().is_some_and(is_replay)).collect();
        v.sort();
        v.truncate(MAX_IMPORT);
        v
    }

    /// IMPORT: a file taken (added, or the store had it) moved to `replays/import/done`, so the next IMPORT does
    /// not take it again.
    pub fn import_done(&self, p: &std::path::Path) -> Result<(), String> {
        let done = self.import_dir().join("done");
        std::fs::create_dir_all(&done).map_err(|e| format!("{}: {e}", done.display()))?;
        let to = done.join(p.file_name().ok_or("no file name")?);
        // a file of that name taken before: replaced (rename does not on every system)
        let _ = std::fs::remove_file(&to);
        std::fs::rename(p, &to).map_err(|e| format!("{}: {e}", p.display()))
    }
}

/// Files taken by one IMPORT.
pub const MAX_IMPORT: usize = 50;
/// Bytes the browser's IMPORT reads at once (the files beyond are refused, to be picked on their own).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const MAX_IMPORT_BYTES: usize = 16 << 20;

// ------------------------------------------------------------------ browser: localStorage

#[cfg(target_arch = "wasm32")]
pub struct Store {
    /// the page's localStorage (None: not available, the replays live in `mem` only)
    web: Option<web_sys::Storage>,
    /// every replay of this session's store (a copy of localStorage's; the ones it refused too)
    mem: std::collections::BTreeMap<String, Vec<u8>>,
    /// the replays in localStorage: their text's length (`WEB_BUDGET`)
    sizes: std::collections::BTreeMap<String, usize>,
}

/// The replay files' extension; `OLD_EXT` the one earlier builds wrote, still read (any case).
pub const EXT: &str = "okbr";
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub const OLD_EXT: &str = "br2r";

/// Whether a file extension is a replay's (`.okbr`, or an earlier build's `.br2r`; any case).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn is_replay(x: &std::ffi::OsStr) -> bool {
    x.eq_ignore_ascii_case(EXT) || x.eq_ignore_ascii_case(OLD_EXT)
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// Replays `make_room` deleted from localStorage behind the app's `Store` (which drops them at its next write
    /// and no longer lists them).
    static TAKEN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(target_arch = "wasm32")]
const PREFIX: &str = "br2r/";
/// Characters of base64 text the replays may take in localStorage, KEEP-marked ones included (the origin's ~5 M is
/// shared with the memory card and the preferences): a write beyond it takes out the oldest unmarked ones, as the
/// rotation, or (none left) stays in memory only.
#[cfg(target_arch = "wasm32")]
const WEB_BUDGET: usize = 1_500_000;

#[cfg(target_arch = "wasm32")]
impl Store {
    pub fn new() -> Self {
        let web = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
        let (mut mem, mut sizes) = (std::collections::BTreeMap::new(), std::collections::BTreeMap::new());
        if let Some(st) = &web {
            let n = st.length().unwrap_or(0);
            let keys: Vec<String> = (0..n).filter_map(|i| st.key(i).ok().flatten()).filter(|k| k.starts_with(PREFIX)).collect();
            for k in keys {
                let text = st.get_item(&k).ok().flatten();
                match text.as_deref().and_then(b64_decode) {
                    Some(b) => {
                        let key = k[PREFIX.len()..].to_string();
                        sizes.insert(key.clone(), text.map_or(0, |t| t.len()));
                        mem.insert(key, b);
                    }
                    None => log::warn!("replay storage: {k} unreadable"),
                }
            }
        } else {
            log::warn!("replay storage: no localStorage, replays kept in memory only");
        }
        Self { web, mem, sizes }
    }

    pub fn list(&self) -> Vec<Item> {
        let mut v: Vec<Item> = self
            .mem
            .iter()
            .filter(|(k, _)| !TAKEN.with(|t| t.borrow().contains(k)))
            .filter_map(|(k, b)| {
                let (_, _, header) = Replay::decode_header(b).ok()?;
                Some(Item { key: k.clone(), header, written: 0 })
            })
            .collect();
        sort_newest(&mut v);
        v
    }

    pub fn read(&self, key: &str) -> Result<Vec<u8>, String> {
        self.mem.get(key).cloned().ok_or_else(|| format!("replay {key} not found"))
    }

    /// The oldest unmarked replay in localStorage, `key` and `skip` aside.
    fn oldest_stored(&self, key: &str, skip: &[String]) -> Option<String> {
        let items: Vec<Item> = self.list().into_iter().filter(|i| i.key != key && self.sizes.contains_key(&i.key) && !skip.contains(&i.key)).collect();
        rotated_out(&items, 0).into_iter().next()
    }

    /// The replays `make_room` took out of localStorage dropped from `mem` and `sizes`.
    fn sync(&mut self) {
        for key in TAKEN.with(|t| std::mem::take(&mut *t.borrow_mut())) {
            self.mem.remove(&key);
            self.sizes.remove(&key);
        }
    }

    /// Kept in memory in any case; `Kept` Some(why) when localStorage did not take it: none, or the replays
    /// stored (KEEP-marked ones count, and never go) over `WEB_BUDGET` with it after the oldest unmarked ones, or
    /// the origin full with no unmarked replay left to make room. The unmarked ones taken out go for good only
    /// once it is stored, else they are put back (a write left in memory deletes nothing); an edited replay's old
    /// text stays in localStorage until then.
    pub fn write(&mut self, key: &str, b: &[u8]) -> Result<Kept, String> {
        self.sync();
        self.mem.insert(key.to_string(), b.to_vec());
        let Some(st) = self.web.clone() else { return Ok(Some("no browser storage".into())) };
        let text = b64_encode(b);
        // within the budget: the oldest unmarked ones taken out until it fits
        let others: usize = self.sizes.iter().filter(|(k, _)| *k != key).map(|(_, n)| n).sum();
        let mut over = (others + text.len()).saturating_sub(WEB_BUDGET);
        let mut out = Vec::new();
        while over > 0 {
            let Some(old) = self.oldest_stored(key, &out) else {
                return Ok(Some(if text.len() > WEB_BUDGET { "too long for the browser storage" } else { "browser storage taken by KEEP replays" }.into()));
            };
            over = over.saturating_sub(self.sizes.get(&old).copied().unwrap_or(0));
            out.push(old);
        }
        for old in &out {
            let _ = st.remove_item(&format!("{PREFIX}{old}"));
        }
        // the origin full (the memory card, other pages' data): more of them taken out until it is stored
        let kept = loop {
            if st.set_item(&format!("{PREFIX}{key}"), &text).is_ok() {
                break None;
            }
            let Some(old) = self.oldest_stored(key, &out) else { break Some("browser storage full".to_string()) };
            let _ = st.remove_item(&format!("{PREFIX}{old}"));
            out.push(old);
        };
        let mut lost = 0;
        for old in &out {
            if kept.is_none() {
                self.mem.remove(old);
                self.sizes.remove(old);
            } else if self.mem.get(old).is_none_or(|ob| st.set_item(&format!("{PREFIX}{old}"), &b64_encode(ob)).is_err()) {
                // not put back (the room it left taken meanwhile): listed until the page closes
                self.sizes.remove(old);
                lost += 1;
            }
        }
        if kept.is_none() {
            self.sizes.insert(key.to_string(), text.len());
            if !out.is_empty() {
                log::warn!("replay storage: {} oldest unmarked replays deleted to make room", out.len());
            }
        } else if lost > 0 {
            log::warn!("replay storage: {lost} replays not put back, kept until the page closes");
        }
        Ok(kept)
    }

    pub fn remove(&mut self, key: &str) -> Result<(), String> {
        self.sync();
        self.mem.remove(key);
        self.sizes.remove(key);
        if let Some(st) = &self.web {
            st.remove_item(&format!("{PREFIX}{key}")).map_err(|_| "browser storage refused the delete".to_string())?;
        }
        Ok(())
    }

    /// A new replay under a name of its own, then the rotation (once it is stored: a write left in memory deletes
    /// nothing); its name and `Kept`.
    pub fn add(&mut self, r: &Replay) -> Result<(String, Kept), String> {
        let base = base_name(r);
        let mut key = format!("{base}.{EXT}");
        let mut k = 2;
        while self.mem.contains_key(&key) {
            key = format!("{base}_{k}.{EXT}");
            k += 1;
        }
        let kept = self.write(&key, &r.encode())?;
        if let Some(why) = &kept {
            log::warn!("replay {key}: {why}: kept until the page closes");
        } else {
            for old in rotated_out(&self.list(), KEEP_LAST) {
                let _ = self.remove(&old);
            }
        }
        Ok((key, kept))
    }
}

/// Room in localStorage for another write (the memory card, the preferences; `platform::storage::set_make_room`):
/// the oldest unmarked replay stored taken out (and left to the app's `Store` in `TAKEN`); false when there is none.
#[cfg(target_arch = "wasm32")]
pub fn make_room() -> bool {
    let Some(st) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else { return false };
    let n = st.length().unwrap_or(0);
    let items: Vec<Item> = (0..n)
        .filter_map(|i| st.key(i).ok().flatten())
        .filter(|k| k.starts_with(PREFIX))
        .filter_map(|k| {
            let b = st.get_item(&k).ok().flatten().and_then(|t| b64_decode(&t))?;
            let (_, _, header) = Replay::decode_header(&b).ok()?;
            Some(Item { key: k, header, written: 0 })
        })
        .collect();
    let Some(old) = rotated_out(&items, 0).into_iter().next() else { return false };
    log::warn!("browser storage full: replay {old} deleted to make room");
    let ok = st.remove_item(&old).is_ok();
    if ok {
        TAKEN.with(|t| t.borrow_mut().push(old[PREFIX.len()..].to_string()));
    }
    ok
}

/// Standard base64 with padding (localStorage holds text).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn b64_encode(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            s.push(if k <= c.len() { A[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    s
}

/// `b64_encode`'s text back; None for anything else.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let v = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let b = s.as_bytes();
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    for (i, c) in b.chunks(4).enumerate() {
        let last = i + 1 == b.len() / 4;
        let pad = c.iter().rev().take_while(|&&x| x == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for &x in &c[..4 - pad] {
            n = n << 6 | v(x)? as u32;
        }
        n <<= 6 * pad as u32;
        out.extend_from_slice(&n.to_be_bytes()[1..4 - pad]);
    }
    Some(out)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::frontend::options::Prefs;
    use crate::session::replay::Recorder;
    use crate::session::{GameState, SessionStart};

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(utc(1_700_000_000), (2023, 11, 14, 22, 13, 20));
        assert_eq!(utc(951_782_400), (2000, 2, 29, 0, 0, 0));
        assert_eq!(utc(1_793_491_199), (2026, 10, 31, 23, 59, 59));
    }

    fn replay(time: u64) -> Replay {
        Recorder::new(&SessionStart::default(), Prefs::default(), [0, 1]).replay(&GameState::default(), time)
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("okb_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The rotation keeps the newest 20 and every KEEP-marked one; other files are left alone.
    #[test]
    fn rotation_keeps_last_and_marked() {
        let dir = temp("replays_rot");
        let mut st = Store::at(dir.clone());
        for k in 0..30u64 {
            let mut r = replay(1_000 + k);
            r.header.keep = k == 2 || k == 25;
            st.add(&r).expect("saved");
        }
        std::fs::write(dir.join("notes.br2r"), b"not a replay").unwrap();
        // an earlier build's replay (`.br2r`) is listed and rotated as the others
        let r = st.list().iter().find(|i| i.header.time == 1_028).map(|i| i.key.clone()).unwrap();
        std::fs::rename(dir.join(&r), dir.join(r.replace(".okbr", ".br2r"))).unwrap();
        let mut left: Vec<(u64, bool)> = st.list().iter().map(|i| (i.header.time - 1_000, i.header.keep)).collect();
        left.sort();
        let want: Vec<(u64, bool)> = std::iter::once((2, true)).chain((9..30).map(|k| (k, k == 25))).collect();
        assert_eq!(left, want);
        assert!(dir.join("notes.br2r").exists());
        // the same second twice: another name
        assert!(st.add(&replay(1_029)).unwrap().0.ends_with("_2.okbr"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A replay of an earlier build (a `.br2r` fixture) put in the replays folder is listed.
    #[test]
    fn an_earlier_replay_file_is_listed() {
        let dir = temp("replays_old_ext");
        let st = Store::at(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("menu_v1.br2r"), include_bytes!("../session/fixtures/menu_v1.br2r")).unwrap();
        assert_eq!(st.list().iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["menu_v1.br2r"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The list is newest first; the rotation drops the oldest unmarked beyond `keep`, whatever the order given.
    #[test]
    fn list_order_and_rotation() {
        let item = |key: &str, time: u64, written: u64, keep: bool| {
            let mut header = Header { time, keep, ..Default::default() };
            header.frames = 0;
            Item { key: key.into(), header, written }
        };
        let mut v = vec![item("b", 10, 5, false), item("a", 30, 0, true), item("c", 10, 7, false), item("d", 20, 0, false), item("e", 5, 0, false)];
        sort_newest(&mut v);
        assert_eq!(v.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["a", "d", "c", "b", "e"]);
        assert_eq!(rotated_out(&v, 2), ["e", "b"]);
        assert_eq!(rotated_out(&v, 0), ["e", "b", "c", "d"]);
        assert!(rotated_out(&v, 4).is_empty());
    }

    /// KEEP and RENAME written back to the file; a name too long or a damaged file is refused.
    #[test]
    fn edit_header() {
        let b = replay(5).encode();
        let k = edit(&b, |h| h.keep = true).unwrap();
        let (_, _, h) = Replay::decode_header(&k).unwrap();
        assert!(h.keep && h.name.is_none());
        let n = edit(&k, |h| h.name = Some("  MY BEST KO  ".into())).unwrap();
        let (_, _, h) = Replay::decode_header(&n).unwrap();
        assert_eq!((h.keep, h.name.as_deref()), (true, Some("MY BEST KO")));
        let n = edit(&n, |h| h.name = Some("   ".into())).unwrap();
        assert!(Replay::decode_header(&n).unwrap().2.name.is_none());
        let max = crate::session::replay::MAX_NAME;
        assert!(edit(&b, |h| h.name = Some("A".repeat(max))).is_ok());
        assert!(edit(&b, |h| h.name = Some("A".repeat(max + 1))).is_err());
        let mut bad = b.clone();
        bad[20] ^= 1;
        assert!(edit(&bad, |h| h.keep = true).is_err());
    }

    /// IMPORT takes only valid files this build plays (marked KEEP); damaged, foreign and newer files are refused
    /// with their reason.
    #[test]
    fn import_validates() {
        let r = replay(7);
        let ok = import(&r.encode()).expect("valid");
        assert!(ok.header.keep);
        assert!(import(b"").is_err() && import(b"PK\x03\x04 a zip").is_err_and(|e| e.contains("not a replay")));
        let mut b = r.encode();
        let n = b.len();
        b[n - 1] ^= 0xFF;
        assert!(import(&b).is_err_and(|e| e.contains("damaged")));
        let mut newer = r.clone();
        newer.sim_version = crate::session::SIM_VERSION + 1;
        assert_eq!(import(&newer.encode()).unwrap_err(), format!("recorded with a newer version (v{}), update the game", crate::session::SIM_VERSION + 1));
        let mut truncated = r.encode();
        truncated.truncate(truncated.len() / 2);
        assert!(import(&truncated).is_err());
        // through the folder: two good ones (one of an earlier build's `.br2r`) and a bad one, then the list holds the good ones; the files taken are
        // moved to import/done, the bad one stays
        let dir = temp("replays_imp");
        let mut st = Store::at(dir.clone());
        let imp = st.import_dir();
        std::fs::write(imp.join("a.br2r"), r.encode()).unwrap();
        std::fs::write(imp.join("b.OKBR"), replay(8).encode()).unwrap();
        std::fs::write(imp.join("c.okbr"), b"junk").unwrap();
        std::fs::write(imp.join("d.txt"), r.encode()).unwrap();
        let mut good = 0;
        for p in st.import_paths() {
            if let Ok(Imported::Added(..)) = crate::platform::storage::read_capped(&p, crate::session::replay::MAX_BYTES).and_then(|b| st.import(&b)) {
                st.import_done(&p).unwrap();
                good += 1;
            }
        }
        assert_eq!((good, st.list().len()), (2, 2));
        assert!(st.list().iter().all(|i| i.header.keep));
        let left: Vec<String> = st.import_paths().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["c.okbr"]);
        assert!(imp.join("done/a.br2r").exists() && imp.join("done/b.OKBR").exists());
        // the same recording again (renamed, its mark changed): already there, nothing added; another one is
        let again = edit(&r.encode(), |h| h.name = Some("COPY".into())).unwrap();
        assert!(matches!(st.import(&again), Ok(Imported::There(_))));
        assert!(matches!(st.import(&replay(9).encode()), Ok(Imported::Added(..))));
        assert_eq!(st.list().len(), 3);
        assert!(st.read("../x").is_err() && st.read("").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_round_trip() {
        for n in 0..70usize {
            let b: Vec<u8> = (0..n).map(|k| (k * 37 + 11) as u8).collect();
            let s = b64_encode(&b);
            assert_eq!(b64_decode(&s).as_deref(), Some(&b[..]), "{n}");
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_encode(b"Ma"), "TWE=");
        for bad in ["TWF", "TW=u", "T===", "TWE=TWE=", "TW!u"] {
            assert!(b64_decode(bad).is_none(), "{bad}");
        }
    }
}
