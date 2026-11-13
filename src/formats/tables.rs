//! The game's static tables (`engine/tables.okt`, written by the converter): named, typed tables, each with the
//! address its first byte has in the address space its own links use (`origin`).
//!
//! State: static data (not serialised)
//!
//! ```text
//! "BR2TABLE", "BR1TABLE" or "OKBTABLE" | u32 version = 1 | u32 count |
//! count × { u8 name_len, name (UTF-8), u8 elem, u16 rec_size, u32 count, u32 origin, u32 byte_len, bytes, pad to 4 }
//! ```
//!
//! All numbers little-endian; `elem` 0 blob, 1 u8, 2 i16, 3 u16, 4 u32 (e.g. count 1: a scalar), 5 records of `rec_size`
//! bytes; padding to a multiple of 4 from the file's start. A table that holds addresses of other data (a linked
//! structure: effect tracks, script lists) is read through `Tables::deref` / `Tables::abs`, which find the table
//! whose bytes hold an address. As with the data's own readers, a read past a table's end gives 0. Only RAM addresses
//! (from `RAM` on) are links: a table with an origin below it (a file offset, `image_len`) is read by name only, and
//! an address below `RAM` (a null pointer) reads 0, as below the program image's base.

use std::collections::BTreeMap;

/// The magic the main game's converter writes, the version this build reads, and every magic accepted: the imported
/// game's converter writes `BR1TABLE`, older conversions of both `OKBTABLE`; the layout after the magic is the same.
pub const MAGIC: &[u8; 8] = b"BR2TABLE";
pub const MAGICS: [&[u8; 8]; 3] = [MAGIC, b"BR1TABLE", b"OKBTABLE"];
pub const VERSION: u32 = 1;

/// The tables both games' files have and this module's own helpers read (`Tables::rsqrt_fn` / `Tables::sqrt_fn`): a
/// file without them is refused at load (`Tables::require`).
pub const REQUIRED: &[&str] = &["math.rsqrt", "math.sqrt"];

/// Every table a main-game match and front end read, with its element type and count (ok_br2_converter
/// `gamedata::TABLES`): a `tables.okt` that lacks one or has it in another shape is not used
/// (`Tables::require_layout`, `Assets::br2_tables`).
pub const BR2: [(&str, Elem, u32); 54] = [
    ("math.random_bytes", Elem::U8, 0x100),
    ("select.neighbours", Elem::Rec(4), 11),
    ("hud.data", Elem::Blob, 0x3B8),
    ("hud.popup", Elem::U16, 20),
    ("hud.frame", Elem::Rec(4), 8),
    ("stage.shadow_floor", Elem::Rec(8), 10),
    ("select.faces", Elem::Rec(4), 11),
    ("select.plates", Elem::Rec(2), 11),
    ("select.story_beast_x", Elem::U8, 11),
    ("select.story_title_x", Elem::U8, 11),
    ("select.story_name_x", Elem::U8, 11),
    ("select.stage_half", Elem::U8, 9),
    ("select.normal_order", Elem::U8, 11),
    ("vs.announcer_delays", Elem::Rec(2), 11),
    ("movie.records", Elem::Rec(28), 4),
    ("progress.rank", Elem::Rec(16), 8),
    ("menu.cc_values", Elem::U16, 10),
    ("menu.mode_half", Elem::U8, 7),
    ("screen.title_widths", Elem::U32, 24),
    ("story.fight_count", Elem::U8, 11),
    ("story.stages", Elem::Rec(8), 11),
    ("story.opponents", Elem::Rec(8), 11),
    ("story.script", Elem::Rec(25), 11),
    ("fx.colour_flash", Elem::Blob, 0xD0),
    ("fx.lens_flare", Elem::Blob, 0x64),
    ("font.metrics", Elem::Blob, 0x224),
    ("camera.presets", Elem::Rec(8), 8),
    ("camera.random", Elem::Rec(8), 8),
    ("camera.heights", Elem::I16, 4),
    ("camera.throw", Elem::Rec(4), 8),
    ("camera.track", Elem::I16, 4),
    ("ai.recover_pads", Elem::U16, 8),
    ("input.default_buttons", Elem::U16, 8),
    ("stage.light", Elem::Rec(0x44), 13),
    ("stage.walls", Elem::Rec(16), 13),
    ("stage.backdrop", Elem::Rec(26), 13),
    ("math.atan", Elem::I16, 0x401),
    ("math.tan", Elem::I16, 0x3AF),
    ("math.sin", Elem::I16, 0x401),
    ("stage.grid_ring", Elem::Rec(8), 13),
    ("stage.ring", Elem::Rec(8), 13),
    ("stage.ring_outer", Elem::Rec(8), 13),
    ("stage.drive_grid", Elem::U32, 13),
    ("fx.hit_sprites", Elem::Blob, 0x414),
    ("fxvm.static_block", Elem::Blob, 0x2C),
    ("fxvm.timeline_records", Elem::Blob, 0x50),
    ("fxvm.timelines", Elem::U32, 4),
    ("fxvm.emitter_timelines", Elem::Blob, 0x28),
    ("fx.move_effects", Elem::Blob, 0x46C),
    ("sound.menu_volumes", Elem::Rec(6), 6),
    ("stage.object_slots", Elem::Rec(23), 13),
    ("ai.cpu", Elem::U8, 0xA0),
    ("math.sqrt", Elem::I16, 192),
    ("math.rsqrt", Elem::I16, 192),
];

/// Every table of the imported game's file an imported fighter or stage reads, with its element type and count
/// (ok_br1_converter `runtime::TABLES` and `image_len`); `anim.common_clips` (a blob of any length, origin a
/// file offset) and its ranges too (`Tables::require_br1`, `Assets::br1_tables`).
pub const BR1: [(&str, Elem, u32); 23] = [
    ("input.default_buttons", Elem::U16, 8),
    ("math.random_bytes", Elem::U8, 0x100),
    ("stage.light", Elem::Rec(0x44), 11),
    ("stage.walls", Elem::Rec(18), 11),
    ("stage.colours", Elem::Rec(44), 11),
    ("fx.data_a", Elem::Blob, 0x2BE),
    ("math.atan", Elem::I16, 0x401),
    ("math.sin", Elem::I16, 0x401),
    ("stage.ring_edge", Elem::Rec(8), 15),
    ("stage.ring_hips", Elem::Rec(8), 15),
    ("stage.shadow_floor", Elem::Rec(8), 15),
    ("fx.data_b", Elem::Blob, 0x1E68),
    ("fighter.parts", Elem::Blob, 0x300),
    ("sound.volumes", Elem::Blob, 0x7CC),
    ("stage.flap", Elem::I16, 4),
    ("ai.wait", Elem::U8, 0x40),
    ("ai.react", Elem::U8, 0x10),
    ("ai.guard", Elem::U8, 0x10),
    ("ai.recover", Elem::U8, 0x10),
    ("math.sqrt", Elem::I16, 192),
    ("math.rsqrt", Elem::I16, 192),
    ("math.atan_b", Elem::I16, 0x401),
    ("image_len", Elem::U32, 1),
];

/// The console's RAM (KSEG0) from here on: the address space of the tables' links (`Tables::deref`).
pub const RAM: u32 = 0x8000_0000;

/// The imported game's clips inside its program image (ok_br1_converter `clip_bank`): a blob whose origin is the file
/// offset of its first byte (zeros where no clip reads); `Br1Res` places it at that offset.
pub const BR1_CLIPS: &str = "anim.common_clips";
/// The [start, end) file offset ranges of `BR1_CLIPS` its clips read (u32 pairs, ok_br1_converter data format 3): the
/// bytes outside them are zeros, which no clip reads (the clip tests check every read against them).
pub const BR1_CLIP_RANGES: &str = "anim.common_clips.ranges";

/// A table's element type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    Blob,
    U8,
    I16,
    U16,
    U32,
    /// records of this many bytes
    Rec(u16),
}

impl Elem {
    /// The size of one element in bytes.
    pub fn size(self) -> usize {
        match self {
            Elem::Blob | Elem::U8 => 1,
            Elem::I16 | Elem::U16 => 2,
            Elem::U32 => 4,
            Elem::Rec(n) => n as usize,
        }
    }
}

/// One table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    elem: Elem,
    count: u32,
    origin: u32,
    bytes: Vec<u8>,
}

impl Table {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn elem(&self) -> Elem {
        self.elem
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn origin(&self) -> u32 {
        self.origin
    }

    /// `n` bytes at byte offset `off` (zeros past the end).
    fn at(&self, off: usize, n: usize) -> [u8; 4] {
        let mut b = [0; 4];
        if let Some(s) = off.checked_add(n).and_then(|e| self.bytes.get(off..e)) {
            b[..n].copy_from_slice(s);
        }
        b
    }

    pub fn u8(&self, off: usize) -> u8 {
        self.at(off, 1)[0]
    }

    pub fn i16(&self, off: usize) -> i16 {
        let b = self.at(off, 2);
        i16::from_le_bytes([b[0], b[1]])
    }

    pub fn u16(&self, off: usize) -> u16 {
        self.i16(off) as u16
    }

    pub fn u32(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.at(off, 4))
    }

    /// Element `k` of an i16 table (0 past the end).
    pub fn i16_at(&self, k: usize) -> i16 {
        self.i16(2 * k)
    }

    /// Record `k`'s bytes (empty past the end).
    pub fn rec(&self, k: usize) -> &[u8] {
        let n = self.elem.size();
        k.checked_mul(n).and_then(|o| self.bytes.get(o..o.checked_add(n)?)).unwrap_or(&[])
    }

    /// The byte offset of the address `addr` in this table, if its bytes hold it.
    pub fn offset_of(&self, addr: u32) -> Option<usize> {
        let o = addr.wrapping_sub(self.origin) as usize;
        (addr >= self.origin && o < self.bytes.len()).then_some(o)
    }
}

/// The tables of one `tables.okt`, by name.
#[derive(Clone, Debug, Default)]
pub struct Tables {
    by_name: BTreeMap<String, Table>,
    /// (origin, name) of every table with a RAM origin (`RAM` on), sorted by origin (`deref`)
    by_origin: Vec<(u32, String)>,
}

impl Tables {
    /// Reads a `tables.okt` (its magic and version, every table's bounds, unique names, count × size = length).
    pub fn parse(b: &[u8]) -> Result<Tables, String> {
        let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or("tables.okt: cut short".to_string());
        if !MAGICS.iter().any(|m| b.get(..8) == Some(&m[..])) {
            return Err("tables.okt: not a table file".into());
        }
        let v = u32_at(8)?;
        if v != VERSION {
            return Err(format!("tables.okt: version {v} (this build reads {VERSION})"));
        }
        let n = u32_at(12)? as usize;
        let mut t = Tables::default();
        let mut o = 16usize;
        for _ in 0..n {
            let len = *b.get(o).ok_or("tables.okt: cut short")? as usize;
            let name =
                std::str::from_utf8(b.get(o + 1..o + 1 + len).ok_or("tables.okt: cut short")?).map_err(|_| "tables.okt: a name is not UTF-8")?.to_string();
            o += 1 + len;
            let code = *b.get(o).ok_or("tables.okt: cut short")?;
            let rec = b.get(o + 1..o + 3).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or("tables.okt: cut short")?;
            let elem = match code {
                0 => Elem::Blob,
                1 => Elem::U8,
                2 => Elem::I16,
                3 => Elem::U16,
                4 => Elem::U32,
                5 if rec > 0 => Elem::Rec(rec),
                c => return Err(format!("tables.okt: {name}: element type {c}")),
            };
            let (count, origin, byte_len) = (u32_at(o + 3)?, u32_at(o + 7)?, u32_at(o + 11)? as usize);
            o += 15;
            if (count as usize).checked_mul(elem.size()) != Some(byte_len) {
                return Err(format!("tables.okt: {name}: {byte_len} bytes for {count} × {}", elem.size()));
            }
            let bytes = o.checked_add(byte_len).and_then(|e| b.get(o..e)).ok_or_else(|| format!("tables.okt: {name} cut short"))?.to_vec();
            o += byte_len;
            o = o.div_ceil(4) * 4;
            t.insert(name, Table { elem, count, origin, bytes })?;
        }
        t.by_origin.sort();
        Ok(t)
    }

    /// Tables from their parts (name, element type, count, origin, bytes), as `parse` checks them.
    pub fn build(parts: impl IntoIterator<Item = (String, Elem, u32, u32, Vec<u8>)>) -> Result<Tables, String> {
        let mut t = Tables::default();
        for (name, elem, count, origin, bytes) in parts {
            if elem == Elem::Rec(0) || (count as usize).checked_mul(elem.size()) != Some(bytes.len()) {
                return Err(format!("tables: {name}: {} bytes for {count} × {}", bytes.len(), elem.size()));
            }
            t.insert(name, Table { elem, count, origin, bytes })?;
        }
        t.by_origin.sort();
        Ok(t)
    }

    /// (Tests) The file of these tables, as the converter writes it (`parse` reads it back).
    #[cfg(test)]
    pub(crate) fn to_okt(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(VERSION.to_le_bytes());
        out.extend((self.by_name.len() as u32).to_le_bytes());
        for (name, t) in &self.by_name {
            let (code, rec) = match t.elem {
                Elem::Blob => (0, 0),
                Elem::U8 => (1, 0),
                Elem::I16 => (2, 0),
                Elem::U16 => (3, 0),
                Elem::U32 => (4, 0),
                Elem::Rec(r) => (5, r),
            };
            out.push(name.len() as u8);
            out.extend(name.as_bytes());
            out.push(code);
            out.extend(rec.to_le_bytes());
            for w in [t.count, t.origin, t.bytes.len() as u32] {
                out.extend(w.to_le_bytes());
            }
            out.extend(&t.bytes);
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
        out
    }

    fn insert(&mut self, name: String, table: Table) -> Result<(), String> {
        if self.by_name.contains_key(&name) {
            return Err(format!("tables: {name} twice"));
        }
        if table.origin >= RAM {
            self.by_origin.push((table.origin, name.clone()));
        }
        self.by_name.insert(name, table);
        Ok(())
    }

    /// The table `name`, or an empty one when there is none (a read of it gives 0 then, as past a table's end). For
    /// the readers of a table the loader checked (`Tables::require` with `REQUIRED`, `Tables::require_layout` with
    /// `BR2`): there is always one then.
    pub fn table(&self, name: &str) -> &Table {
        static EMPTY: Table = Table { elem: Elem::Blob, count: 0, origin: 0, bytes: Vec::new() };
        self.by_name.get(name).unwrap_or(&EMPTY)
    }

    /// The table `name` (Err naming it when the file has none: data of an older converter).
    pub fn get(&self, name: &str) -> Result<&Table, String> {
        self.by_name.get(name).ok_or_else(|| format!("tables.okt: no table {name} (convert the game data again)"))
    }

    /// Err naming the first of `names` the file has none of (data of an older or partial converter).
    pub fn require(&self, names: &[&str]) -> Result<(), String> {
        names.iter().try_for_each(|n| self.get(n).map(|_| ()))
    }

    /// Err naming the first table of `layout` (name, element type, count) the file has none of or has with another
    /// element type or count (data of an older, partial or other game's converter).
    pub fn require_layout(&self, layout: &[(&str, Elem, u32)]) -> Result<(), String> {
        layout.iter().try_for_each(|&(name, elem, count)| {
            let t = self.get(name)?;
            if (t.elem, t.count) == (elem, count) {
                Ok(())
            } else {
                Err(format!("tables.okt: {name} is {} × {:?}, not {count} × {elem:?} (convert the game data again)", t.count, t.elem))
            }
        })
    }

    /// Err unless the file has the imported game's tables: every table of `BR1` in its shape, the clips (`BR1_CLIPS`, a
    /// blob) and their ranges (`BR1_CLIP_RANGES`, u32 pairs, data format 3).
    pub fn require_br1(&self) -> Result<(), String> {
        self.require_layout(&BR1)?;
        let again = "(convert the game data again)";
        if self.get(BR1_CLIPS)?.elem != Elem::Blob {
            return Err(format!("tables.okt: {BR1_CLIPS} is not a blob {again}"));
        }
        let r = self.get(BR1_CLIP_RANGES)?;
        if r.elem != Elem::U32 || r.count % 2 != 0 {
            return Err(format!("tables.okt: {BR1_CLIP_RANGES} is not u32 pairs {again}"));
        }
        Ok(())
    }

    /// The [start, end) file offset ranges `BR1_CLIP_RANGES` lists (none without the table).
    pub fn clip_ranges(&self) -> Vec<(usize, usize)> {
        let r = self.table(BR1_CLIP_RANGES);
        (0..r.count as usize / 2).map(|k| (r.u32(8 * k) as usize, r.u32(8 * k + 4) as usize)).collect()
    }

    /// Every table's name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// The table whose bytes hold the RAM address `addr` and its byte offset there (the last table with a RAM origin
    /// starting at or before it; None when no table holds it, and for every address below `RAM`).
    pub fn deref(&self, addr: u32) -> Option<(&Table, usize)> {
        let k = self.by_origin.partition_point(|(o, _)| *o <= addr);
        self.by_origin[..k].iter().rev().find_map(|(_, n)| {
            let t = &self.by_name[n];
            t.offset_of(addr).map(|o| (t, o))
        })
    }

    /// The bytes at the address `addr` of whichever table holds them, as a reader of the linked data needs them
    /// (`deref`; zeros when no table holds the whole read, so for a null or other address below `RAM`).
    fn abs(&self, addr: u32, n: usize) -> [u8; 4] {
        match self.deref(addr) {
            Some((t, o)) => t.at(o, n),
            None => [0; 4],
        }
    }

    pub fn abs_u8(&self, addr: u32) -> u8 {
        self.abs(addr, 1)[0]
    }

    pub fn abs_i16(&self, addr: u32) -> i16 {
        let b = self.abs(addr, 2);
        i16::from_le_bytes([b[0], b[1]])
    }

    pub fn abs_u16(&self, addr: u32) -> u16 {
        self.abs_i16(addr) as u16
    }

    pub fn abs_u32(&self, addr: u32) -> u32 {
        u32::from_le_bytes(self.abs(addr, 4))
    }

    /// Entry `k` of the i16 table `name`, `k` clamped to 0..=191, as a function that looks the table up once (the
    /// square-root tables' readers).
    fn clamped_fn(&self, name: &str) -> impl Fn(i32) -> i32 + '_ {
        let t = self.table(name);
        move |k| t.i16_at(k.clamp(0, 191) as usize) as i32
    }

    /// The reciprocal square root table (`math.rsqrt`) as a function of `k` (clamped to 0..=191; the skeleton's
    /// normalisations call it per node).
    pub fn rsqrt_fn(&self) -> impl Fn(i32) -> i32 + '_ {
        self.clamped_fn("math.rsqrt")
    }

    /// The square root table (`math.sqrt`) as a function of `k` (clamped to 0..=191).
    pub fn sqrt_fn(&self) -> impl Fn(i32) -> i32 + '_ {
        self.clamped_fn("math.sqrt")
    }

    /// The square root table (`math.sqrt`), entry `k` clamped to 0..=191.
    pub fn sqrt(&self, k: i32) -> i32 {
        self.sqrt_fn()(k)
    }
}

/// (Tests) Tables of `layout`'s shapes, zeros, at RAM origins one after the other, and an empty text table (which
/// installs no text: `text::install`).
#[cfg(test)]
pub(crate) fn of_layout(layout: &[(&str, Elem, u32)]) -> Tables {
    let mut origin = RAM;
    let text = (crate::assets::text::TABLE.to_string(), Elem::Blob, 0, 0, Vec::new());
    Tables::build(
        layout
            .iter()
            .map(|&(n, elem, count)| {
                let len = count as usize * elem.size();
                origin += len as u32;
                (n.to_string(), elem, count, origin - len as u32, vec![0; len])
            })
            .chain([text]),
    )
    .unwrap()
}

/// The tables of the test data (`OKB_EXPORT` or `../okb_export`): its `engine/tables.okt`; None without one. A
/// `tables.okt` that does not parse or lacks a table of `BR2` fails the test (a converter regression).
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn export_tables() -> Option<Tables> {
    let root = crate::base::env::var("EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export").into());
    let b = std::fs::read(std::path::Path::new(&root).join("engine/tables.okt")).ok()?;
    let t = Tables::parse(&b).expect("tables.okt");
    t.require_layout(&BR2).expect("tables.okt");
    crate::assets::text::install(crate::assets::text::Game::Br2, &t).expect("tables.okt");
    crate::output::font::install(t.table(crate::output::font::METRICS).bytes());
    crate::assets::game_tables::install(&t);
    Some(t)
}

/// The imported game's tables of the test install (`BR1_EXPORT` or `../okb_export/br1`): its `engine/tables.okt`; None
/// without one. A `tables.okt` that does not parse or lacks a table of `BR1`, the clips or their ranges fails the test.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn br1_export_tables() -> Option<Tables> {
    let root = std::env::var("BR1_EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export/br1").into());
    let b = std::fs::read(std::path::Path::new(&root).join("engine/tables.okt")).ok()?;
    let t = Tables::parse(&b).expect("tables.okt");
    t.require_br1().expect("tables.okt");
    crate::assets::text::install(crate::assets::text::Game::Br1, &t).expect("tables.okt");
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file of `tables` (name, elem code, rec size, count, origin, bytes), as the converter writes it.
    fn file(tables: &[(&str, u8, u16, u32, u32, &[u8])]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(VERSION.to_le_bytes());
        out.extend((tables.len() as u32).to_le_bytes());
        for (name, elem, rec, count, origin, bytes) in tables {
            out.push(name.len() as u8);
            out.extend(name.as_bytes());
            out.push(*elem);
            out.extend(rec.to_le_bytes());
            out.extend(count.to_le_bytes());
            out.extend(origin.to_le_bytes());
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(*bytes);
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
        out
    }

    #[test]
    fn parse_read_and_deref() {
        let rs: Vec<u8> = (0..192i16).flat_map(|k| (k * 3).to_le_bytes()).collect();
        let b = file(&[
            ("math.rsqrt", 2, 0, 192, 0x8000_1000, &rs),
            ("a.rec", 5, 3, 2, 0x8000_2000, &[1, 2, 3, 4, 5, 6]),
            ("a.link", 0, 0, 8, 0x8000_2006, &[0x00, 0x20, 0, 0x80, 0xFF, 0xFF, 0x7F, 0x80]),
            ("n", 4, 0, 1, 0, &[1, 0, 0, 0]),
            ("off", 0, 0, 4, 0x10, &[9, 9, 9, 9]),
        ]);
        let t = Tables::parse(&b).unwrap();
        assert_eq!(t.names().collect::<Vec<_>>(), ["a.link", "a.rec", "math.rsqrt", "n", "off"]);
        let r = t.get("a.rec").unwrap();
        assert_eq!((r.elem(), r.count(), r.origin(), r.rec(1), r.rec(2)), (Elem::Rec(3), 2, 0x8000_2000, &[4u8, 5, 6][..], &[][..]));
        assert_eq!((r.u8(5), r.u8(6), r.u16(0), r.u32(4)), (6, 0, 0x0201, 0), "past the end: 0");
        assert!(t.get("nope").unwrap_err().contains("nope"));
        assert!(t.require(&["math.rsqrt", "n"]).is_ok() && t.require(&["n", "math.sqrt"]).unwrap_err().contains("math.sqrt"));
        let rs = t.rsqrt_fn();
        assert_eq!((rs(5), rs(-3), rs(999), t.sqrt(1), t.sqrt_fn()(1)), (15, 0, 191 * 3, 0, 0));
        // addresses: the table holding them, adjacent tables, none
        assert_eq!(t.deref(0x8000_2005).map(|(t, o)| (t.origin(), o)), Some((0x8000_2000, 5)));
        assert_eq!(t.deref(0x8000_2006).map(|(t, o)| (t.origin(), o)), Some((0x8000_2006, 0)));
        assert!(t.deref(0x8000_200E).is_none() && t.deref(0x8000_0FFF).is_none());
        let abs = (t.abs_u32(0x8000_2006), t.abs_i16(0x8000_200A), t.abs_u16(0x8000_200C), t.abs_u8(0x8000_2001));
        assert_eq!(abs, (0x8000_2000, -1, 0x807F, 2));
        // a read running past the table holding its first byte: 0, as past the program image's end
        assert_eq!((t.abs_u32(0x8000_200C), t.abs_u16(0x8000_200D)), (0, 0));
        // tables with an origin below RAM (a file offset, a scalar) are no link's target: null and small addresses read 0
        assert!(t.deref(0).is_none() && t.deref(0x10).is_none());
        assert_eq!((t.abs_u32(0), t.abs_u8(0x10), t.get("off").unwrap().u8(0)), (0, 0, 9));
        assert_eq!(t.get("n").unwrap().u32(0), 1);
    }

    #[test]
    fn refuses_broken_files() {
        let ok = file(&[("x", 2, 0, 2, 0, &[1, 2, 3, 4])]);
        assert!(Tables::parse(&ok).is_ok());
        // the imported game's converter and older conversions write the other accepted magics
        for magic in [b"BR1TABLE", b"OKBTABLE"] {
            let mut other = ok.clone();
            other[..8].copy_from_slice(magic);
            assert!(Tables::parse(&other).is_ok());
        }
        assert!(Tables::parse(&ok[..ok.len() - 4]).is_err());
        assert!(Tables::parse(b"OKBTABLX").is_err());
        let mut v2 = ok.clone();
        v2[8] = 2;
        assert!(Tables::parse(&v2).unwrap_err().contains("version 2"));
        assert!(Tables::parse(&file(&[("x", 2, 0, 3, 0, &[1, 2, 3, 4])])).unwrap_err().contains("x"));
        assert!(Tables::parse(&file(&[("x", 1, 0, 1, 0, &[1]), ("x", 1, 0, 1, 0, &[1])])).unwrap_err().contains("twice"));
        assert!(Tables::parse(&file(&[("x", 9, 0, 1, 0, &[1])])).is_err());
        assert!(Tables::parse(&file(&[("x", 5, 0, 0, 0, &[])])).is_err(), "records of 0 bytes");
    }

    /// `require_layout`: a file with only the tables every game has (`REQUIRED`) or with a `BR2` table of another
    /// count is refused for a main-game match; one of the layout's shapes passes.
    #[test]
    fn br2_layout() {
        let rs = [0u8; 384];
        let only = Tables::parse(&file(&[("math.rsqrt", 2, 0, 192, 0x1000, &rs), ("math.sqrt", 2, 0, 192, 0x2000, &rs)])).unwrap();
        assert!(only.require(REQUIRED).is_ok());
        assert!(only.require_layout(&BR2).unwrap_err().contains("math.random_bytes"));
        let full = of_layout(&BR2);
        assert!(full.require_layout(&BR2).is_ok());
        let short = Tables::build(full.names().map(|n| {
            let t = full.get(n).unwrap();
            let (count, bytes) = if n == "math.random_bytes" { (16, vec![0; 16]) } else { (t.count(), t.bytes().to_vec()) };
            (n.to_string(), t.elem(), count, t.origin(), bytes)
        }))
        .unwrap();
        assert!(short.require_layout(&BR2).unwrap_err().contains("math.random_bytes is 16"));
    }

    /// `require_br1`: the imported game's layout, the clips as a blob and their ranges as u32 pairs (format 3); without the
    /// ranges (an install of format 2), or with them in another shape, refused; `clip_ranges` reads the pairs.
    #[test]
    fn br1_layout() {
        let with = |extra: Vec<(String, Elem, u32, u32, Vec<u8>)>| {
            let base = of_layout(&BR1);
            Tables::build(
                base.names()
                    .map(|n| {
                        let t = base.get(n).unwrap();
                        (n.to_string(), t.elem(), t.count(), t.origin(), t.bytes().to_vec())
                    })
                    .chain(extra),
            )
            .unwrap()
        };
        let clips = || (BR1_CLIPS.to_string(), Elem::Blob, 4, 0x100, vec![1; 4]);
        let ranges = |elem: Elem, pairs: &[u32]| {
            let bytes: Vec<u8> = pairs.iter().flat_map(|v| v.to_le_bytes()).collect();
            (BR1_CLIP_RANGES.to_string(), elem, (bytes.len() / elem.size()) as u32, 0, bytes)
        };
        let t = with(vec![clips(), ranges(Elem::U32, &[0x100, 0x102, 0x103, 0x104])]);
        assert!(t.require_br1().is_ok());
        assert_eq!(t.clip_ranges(), [(0x100, 0x102), (0x103, 0x104)]);
        assert!(with(vec![clips()]).require_br1().unwrap_err().contains(BR1_CLIP_RANGES));
        assert!(with(vec![clips(), ranges(Elem::U32, &[0x100])]).require_br1().unwrap_err().contains("u32 pairs"));
        assert!(with(vec![clips(), ranges(Elem::I16, &[0x100, 0x102])]).require_br1().is_err());
        assert!(with(vec![(BR1_CLIPS.to_string(), Elem::U8, 4, 0x100, vec![1; 4]), ranges(Elem::U32, &[])]).require_br1().unwrap_err().contains("blob"));
        assert!(of_layout(&BR1).clip_ranges().is_empty());
    }

    /// The imported game's sine, arctangent and square root tables are the main game's (data-gated: both
    /// installs).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn imported_maths_are_the_main_games() {
        let (Some(br1), Some(br2)) = (br1_export_tables(), export_tables()) else {
            eprintln!("skipped: no okb_export / okb_export/br1");
            return;
        };
        for n in ["math.sin", "math.atan", "math.rsqrt", "math.sqrt"] {
            assert_eq!(br1.get(n).unwrap().bytes(), br2.get(n).unwrap().bytes(), "{n}");
        }
    }

    /// The imported game's install (data-gated): its tables apart from the clips' are at RAM origins and resolve to
    /// themselves; the clips (by file offset), their ranges and `image_len` are below `RAM`, inside the image, and no
    /// address resolves into them; the ranges lie in order inside the clips, which hold zeros outside them.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn br1_file_offsets_stay_apart() {
        let Some(t) = br1_export_tables() else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let len = t.get("image_len").unwrap().u32(0) as usize;
        let clips = t.get(BR1_CLIPS).unwrap();
        let (o, n) = (clips.origin() as usize, clips.bytes().len());
        // the bounds first, then the bytes
        assert!(n > 0 && o + n <= len && len < RAM as usize);
        let low = |name: &str| t.get(name).unwrap().origin() < RAM;
        assert!(t.names().filter(|n| low(n)).all(|n| [BR1_CLIPS, BR1_CLIP_RANGES, "image_len", crate::assets::text::TABLE].contains(&n)));
        let last = clips.origin() + n as u32 - 1;
        assert!(t.deref(clips.origin()).is_none() && t.deref(last).is_none() && t.deref(0).is_none());
        assert_eq!((t.abs_u32(0), t.abs_u32(clips.origin()), t.abs_u8(last)), (0, 0, 0));
        for name in t.names().filter(|n| !low(n)) {
            let tab = t.get(name).unwrap();
            assert_eq!(t.deref(tab.origin()).map(|(d, _)| d.origin()), Some(tab.origin()), "{name}");
        }
        let ranges = t.clip_ranges();
        assert!(!ranges.is_empty() && ranges.windows(2).all(|w| w[0].1 < w[1].0));
        assert!(ranges.iter().all(|&(a, b)| o <= a && a < b && b <= o + n));
        assert_eq!((ranges[0].0, ranges[ranges.len() - 1].1), (o, o + n));
        let inside = |k: usize| ranges.iter().any(|&(a, b)| (a..b).contains(&(o + k)));
        assert!(clips.bytes().iter().enumerate().all(|(k, &b)| b == 0 || inside(k)));
    }
}
