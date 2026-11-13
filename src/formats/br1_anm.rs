//! The imported game's fighter file `XX.ANM` (= `XX31.DAT`, the one the game loads) and its clip blocks (`CMN.ACT`).
//!
//! State: static data (not serialised)
//!
//! One file per fighter: the header (11 file offsets, then the per-fighter parameters at +0x2C), the collision
//! spheres, the bone maps, the two skeleton forms, the state scripts and their table, the per-opponent move table,
//! the command table and its sequences, the chain programs, the AI section and the clip block, in that order. Most
//! record formats are the main game's (`act_ani.rs`); the differences are the header's word order, the parameters,
//! the clips living in the ANM (fixed 120-byte keys, no stretch records) and 9 opponents.
//!
//! The game relocates the clip table, the CMN.ACT table and 5 AI offsets to pointers at load; the engine keeps the
//! raw bytes and adds the base on use. `Anm::parse` checks the whole structure and returns an error on
//! anything out of range (a damaged or foreign file), so the readers after it index without further checks.

use super::act_ani::Sphere;

/// A key's size: yaw, root offset, node 1 / node 4 translations, 16 direction slots, 2 hip twists.
pub const KEY: usize = 120;
/// State table entries.
pub const STATES: usize = 466;
/// Opponents of the per-opponent table (`chr` 0..8 = ZIN GRE EVE VAK MIT GAD FOX RON URI).
pub const OPPONENTS: usize = 9;
/// The roster order of `chr` (the game's name table).
pub const CHARACTERS: [&str; OPPONENTS] = ["ZIN", "GRE", "EVE", "VAK", "MIT", "GAD", "FOX", "RON", "URI"];

// the parser's bounds: far above the 9 files' (≤ 26 nodes, 15 spheres, 28 commands, 32 chains, 27 AI records,
// 858 clips, 40 keys per clip), so a damaged count cannot make it allocate much
const MAX_NODES: usize = 64;
const MAX_SPHERES: usize = 64;
const MAX_COMMANDS: usize = 64;
const MAX_STEPS: usize = 64;
const MAX_CHAINS: usize = 64;
const MAX_CHAIN_WORDS: usize = 512;
const MAX_AI_ACTIONS: usize = 64;
const MAX_CLIPS: usize = 4096;
const MAX_KEYS: usize = 1024;

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o.checked_add(2)?).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32le(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The 10-bit signed field of a packed `x | y << 10 | z << 20` word.
pub fn s10(w: u32) -> i32 {
    (((w & 0x3FF) << 22) as i32) >> 22
}

/// The three 10-bit fields of a packed word.
pub fn s10x3(w: u32) -> [i32; 3] {
    [s10(w), s10(w >> 10), s10(w >> 20)]
}

/// A reader over the file's bytes that turns every out-of-range read into an error naming what was read.
struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn u8(&self, o: usize, what: &str) -> Result<u8, String> {
        self.0.get(o).copied().ok_or_else(|| format!("{what} at {o:#x} is past the end"))
    }
    fn u16(&self, o: usize, what: &str) -> Result<u16, String> {
        u16le(self.0, o).ok_or_else(|| format!("{what} at {o:#x} is past the end"))
    }
    fn i16(&self, o: usize, what: &str) -> Result<i16, String> {
        self.u16(o, what).map(|v| v as i16)
    }
    fn u32(&self, o: usize, what: &str) -> Result<u32, String> {
        u32le(self.0, o).ok_or_else(|| format!("{what} at {o:#x} is past the end"))
    }
    /// A file offset read at `o`, which must point inside the file.
    fn offset(&self, o: usize, what: &str) -> Result<usize, String> {
        let v = self.u32(o, what)? as usize;
        if v >= self.0.len() {
            return Err(format!("{what} ({v:#x}) points past the end"));
        }
        Ok(v)
    }
}

/// The per-fighter parameters after the header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Params {
    /// node count per form (+0x2C / +0x2E: F.24 +0x01 / +0x1D)
    pub nodes: [u8; 2],
    /// op 0x14 sub 0 timer factor per form (+0x30 / +0x32: F.24 +0x1A / +0x36)
    pub timer_factor: [u16; 2],
    /// weight per form (+0x34 / +0x36: F.24 +0x18 / +0x34)
    pub weight: [u16; 2],
    /// red-health regeneration interval in frames (+0x38: F.D1)
    pub regen: u8,
    /// max HP before the vitality factor (+0x3A: F.A8)
    pub max_hp: u16,
    /// node 1 positive-y scale of CMN.ACT and opponent clips, /128 (+0x3C: S.C4)
    pub root_scale: u16,
}

/// One skeleton form (header +0x20, 0x14 bytes per form: 5 file offsets).
#[derive(Clone, Debug, Default)]
pub struct Form {
    /// the 5 table offsets: parents, object map, bind offsets, object offsets, base angles
    pub tables: [u32; 5],
    /// parent per node (−1 root, −2 = node 0, below −2 = the opponent's node −p−3)
    pub parents: Vec<i16>,
    /// bind offset (s10 ×3, the node's static local translation) of nodes 1..; `bind[n - 1]` is node n's
    pub bind: Vec<[i32; 3]>,
    /// base angles (s10 ×3, raw: × 4 = 4096-unit Euler angles) of nodes 1..; `base_angles[n - 1]` is node n's
    pub base_angles: Vec<[i32; 3]>,
}

impl Form {
    pub fn nodes(&self) -> usize {
        self.parents.len()
    }
}

/// A step of a command sequence (10 bytes, the main game `input.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub hold_frames: i16,
    pub miss_limit: i16,
    /// 0 ends the sequence; 1..7 held / pressed / recent / long / double / held dir + pressed buttons / special
    pub source: i16,
    pub mask: u16,
    pub value: u16,
}

/// A command record (16 bytes) and its sequence.
#[derive(Clone, Debug)]
pub struct Command {
    pub flags: u32,
    pub mv: i16,
    pub life: i16,
    pub window: i16,
    pub param: i16,
    /// file offset of the sequence
    pub seq: u32,
    /// the sequence's steps up to and with the one of source 0
    pub steps: Vec<Step>,
}

/// A chain program: its file offset and its u16 words up to and with the ending 0.
#[derive(Clone, Debug)]
pub struct Chain {
    pub offset: u32,
    pub words: Vec<u16>,
}

/// An AI action record (12 bytes): the opponent-state mask and a script per distance band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiAction {
    pub mask: u32,
    /// file offsets of the scripts (0 = none)
    pub scripts: [u16; 4],
}

/// The AI section (header +0x18).
#[derive(Clone, Debug, Default)]
pub struct Ai {
    /// its 10 words: w1 distance bands, w2 action records, w5..w8 the scripts of the AI init modes 2, 9, 10, 11
    pub words: [u32; 10],
    /// distance bands, ascending, with the ending 0xFFFF
    pub bands: Vec<u16>,
    /// actions 0..5 (fixed situations), then the opponent-state records until mask 0
    pub actions: Vec<AiAction>,
}

/// One clip: `u16 frames, u16 0, (u16 key offset, u16 duration)…` with duration 0 last.
#[derive(Clone, Debug)]
pub struct Clip {
    /// file offset of the clip (the game's S.B8 = file base + this)
    pub offset: usize,
    /// total frames (= Σ durations + 1)
    pub frames: u16,
    /// (key offset from the clip, an s16 as the game reads it; duration in frames) per key, the last with duration 0
    pub keys: Vec<(u16, u16)>,
}

impl Clip {
    /// File offset of key `k`'s 120 bytes.
    pub fn key(&self, k: usize) -> Option<usize> {
        self.keys.get(k).and_then(|&(o, _)| self.offset.checked_add_signed(o as i16 as isize))
    }
}

/// A clip block: `u32 size, u16 count, u16 0, s32 offset[count] (from the block; ≤ 0 = empty), −1`. The ANM's at
/// header word 0 (S.B4 = block + 8, S.AC = count), CMN.ACT's at file offset 0 (S.B0).
#[derive(Clone, Debug, Default)]
pub struct Clips {
    /// file offset of the block
    pub base: usize,
    /// the raw table entries
    pub entries: Vec<i32>,
    pub clips: Vec<Option<Clip>>,
}

impl Clips {
    /// The block at `base` of `b`; every used entry's clip and keys must lie inside `b`.
    pub fn parse(b: &[u8], base: usize) -> Result<Self, String> {
        let r = Rd(b);
        let size = r.u32(base, "clip block size")? as usize;
        if base.checked_add(size).is_none_or(|e| e > b.len()) {
            return Err(format!("clip block at {base:#x}: size {size:#x} runs past the end"));
        }
        let count = r.u16(base + 4, "clip count")? as usize;
        if count > MAX_CLIPS {
            return Err(format!("clip block at {base:#x}: {count} clips"));
        }
        let mut entries = Vec::with_capacity(count);
        let mut clips = Vec::with_capacity(count);
        for i in 0..count {
            let e = r.u32(base + 8 + 4 * i, "clip table")? as i32;
            entries.push(e);
            if e <= 0 {
                clips.push(None);
                continue;
            }
            let at = base.checked_add(e as usize).filter(|&a| a < b.len()).ok_or_else(|| format!("clip {i} at {e:#x} is past the end"))?;
            let frames = r.u16(at, "clip frames")?;
            let mut keys = Vec::new();
            loop {
                let p = at + 4 + 4 * keys.len();
                let (off, dur) = (r.u16(p, "clip key")?, r.u16(p + 2, "clip key")?);
                // the game reads the offset as an s16 (S.C0 = S.B8 + lh)
                if at.checked_add_signed(off as i16 as isize).and_then(|k| k.checked_add(KEY)).is_none_or(|end| end > b.len()) {
                    return Err(format!("clip {i}: key {} at {off:#x} runs past the end", keys.len()));
                }
                keys.push((off, dur));
                if dur == 0 {
                    break;
                }
                if keys.len() >= MAX_KEYS {
                    return Err(format!("clip {i}: more than {MAX_KEYS} keys"));
                }
            }
            clips.push(Some(Clip { offset: at, frames, keys }));
        }
        Ok(Self { base, entries, clips })
    }

    pub fn count(&self) -> usize {
        self.clips.len()
    }

    pub fn get(&self, i: usize) -> Option<&Clip> {
        self.clips.get(i).and_then(|c| c.as_ref())
    }
}

/// A parsed `XX.ANM`: the raw bytes (scripts and keys are read from them by offset) and every table decoded.
#[derive(Clone, Debug)]
pub struct Anm {
    pub bytes: Vec<u8>,
    /// the 11 header words (file offsets): 0 clips, 1 states, 2 per-opponent table, 3 commands, 4 chain programs,
    /// 5 (0), 6 AI, 7 (0), 8 skeleton forms, 9 bone maps, 10 spheres
    pub header: [u32; 11],
    pub params: Params,
    /// collision tables: 0 / 1 body spheres of the human / beast form, 2 / 3 hit spheres
    pub spheres: [Vec<Sphere>; 4],
    /// bone maps: form 0 straight, form 0 mirrored, form 1 straight, form 1 mirrored (S+0x110..+0x11C)
    pub bone_maps: [[u8; 20]; 4],
    pub forms: [Form; 2],
    /// state n's script offset (pc = ANM + table[n]); entries 464 / 465 are 0xFFFF, 0 in all but ZIN's
    pub states: Vec<u16>,
    /// per opponent `chr` the 16 throw-victim moves (state = move − 0x80)
    pub opponents: [[i16; 16]; OPPONENTS],
    pub commands: Vec<Command>,
    pub chains: Vec<Chain>,
    pub ai: Ai,
    pub clips: Clips,
}

impl Anm {
    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        let r = Rd(&bytes);
        if bytes.len() < 0x40 {
            return Err(format!("{} bytes: too short for an ANM", bytes.len()));
        }
        let mut header = [0u32; 11];
        for (k, h) in header.iter_mut().enumerate() {
            *h = r.u32(4 * k, "header")?;
            if *h as usize >= bytes.len() {
                return Err(format!("header word {k} ({:#x}) points past the end", *h));
            }
        }
        let params = Params {
            nodes: [r.u8(0x2C, "params")?, r.u8(0x2E, "params")?],
            timer_factor: [r.u16(0x30, "params")?, r.u16(0x32, "params")?],
            weight: [r.u16(0x34, "params")?, r.u16(0x36, "params")?],
            regen: r.u8(0x38, "params")?,
            max_hp: r.u16(0x3A, "params")?,
            root_scale: r.u16(0x3C, "params")?,
        };
        // the base skeleton is 19 nodes (0 root .. 18 hips) in every form; the sampler and the matrices index them
        for (k, &n) in params.nodes.iter().enumerate() {
            if !(19..=MAX_NODES as u8).contains(&n) {
                return Err(format!("form {k}: {n} nodes"));
            }
        }
        // 2.1 spheres: u16 count[4], u32 table[4]
        let sp = header[10] as usize;
        let mut spheres: [Vec<Sphere>; 4] = Default::default();
        for (k, s) in spheres.iter_mut().enumerate() {
            let n = r.u16(sp + 2 * k, "sphere count")? as usize;
            if n > MAX_SPHERES {
                return Err(format!("sphere table {k}: {n} spheres"));
            }
            let t = r.offset(sp + 8 + 4 * k, "sphere table")?;
            for i in 0..n {
                let e = t + 4 * i;
                s.push(Sphere { group: r.u8(e, "sphere")?, node: r.u8(e + 1, "sphere")?, radius: r.i16(e + 2, "sphere")? });
            }
        }
        // 2.2 bone maps: 4 offsets to 20-byte maps; every slot index within the 19 data slots
        let bm = header[9] as usize;
        let mut bone_maps = [[0u8; 20]; 4];
        for (k, m) in bone_maps.iter_mut().enumerate() {
            let o = r.offset(bm + 4 * k, "bone map")?;
            let src = bytes.get(o..o + 20).ok_or_else(|| format!("bone map {k} runs past the end"))?;
            if src.iter().any(|&v| v > 18) {
                return Err(format!("bone map {k}: a slot past 18"));
            }
            m.copy_from_slice(src);
        }
        // 2.3 skeleton forms: 5 offsets per form
        let sk = header[8] as usize;
        let mut forms: [Form; 2] = Default::default();
        for (k, f) in forms.iter_mut().enumerate() {
            for (i, t) in f.tables.iter_mut().enumerate() {
                *t = r.offset(sk + 0x14 * k + 4 * i, "skeleton table")? as u32;
            }
            let nn = params.nodes[k] as usize;
            for n in 0..nn {
                let p = r.i16(f.tables[0] as usize + 2 * n, "parents")?;
                // node 0 the root, the others a node of the form or −2 (node 0): no opponent links in any file
                if (n == 0) != (p == -1) || p < -2 || p >= nn as i16 {
                    return Err(format!("form {k}: node {n} has parent {p}"));
                }
                f.parents.push(p);
            }
            // a tree: every node reaches the root (the world walk recurses over the links)
            for n in 0..nn {
                let (mut p, mut steps) = (n as i16, 0);
                while p >= 0 {
                    p = f.parents[p as usize];
                    steps += 1;
                    if steps > nn {
                        return Err(format!("form {k}: node {n} is in a parent cycle"));
                    }
                }
            }
            for n in 1..nn {
                f.bind.push(s10x3(r.u32(f.tables[2] as usize + 4 * (n - 1), "bind offsets")?));
                f.base_angles.push(s10x3(r.u32(f.tables[4] as usize + 4 * (n - 1), "base angles")?));
            }
        }
        // 2.5 state table: 466 entries from header word 1; it runs to word 3 except in ZIN's file, which has scripts
        // between its end and word 3
        let (s0, s1) = (header[1] as usize, header[3] as usize);
        if s1 < s0 || (s1 - s0) / 2 < STATES {
            return Err(format!("state table {s0:#x}..{s1:#x}: fewer than {STATES} entries"));
        }
        let states: Vec<u16> = (0..STATES).map(|i| r.u16(s0 + 2 * i, "state table")).collect::<Result<_, _>>()?;
        if let Some(i) = states.iter().position(|&s| s != 0xFFFF && s as usize >= bytes.len()) {
            return Err(format!("state {i} at {:#x} is past the end", states[i]));
        }
        // 2.6 per-opponent table: 9 offsets to 16 × s16
        let mut opponents = [[0i16; 16]; OPPONENTS];
        for (c, row) in opponents.iter_mut().enumerate() {
            let o = r.offset(header[2] as usize + 4 * c, "per-opponent table")?;
            for (k, v) in row.iter_mut().enumerate() {
                *v = r.i16(o + 2 * k, "per-opponent row")?;
            }
        }
        // 2.7 commands: 16-byte records until flags == 0, each with its 10-byte steps until source 0
        let mut commands = Vec::new();
        let c0 = header[3] as usize;
        loop {
            let o = c0 + 16 * commands.len();
            let flags = r.u32(o, "command")?;
            if flags == 0 {
                break;
            }
            if commands.len() >= MAX_COMMANDS {
                return Err(format!("more than {MAX_COMMANDS} commands"));
            }
            let seq = r.offset(o + 12, "command sequence")?;
            let mut steps = Vec::new();
            loop {
                let e = seq + 10 * steps.len();
                let s = Step {
                    hold_frames: r.i16(e, "sequence")?,
                    miss_limit: r.i16(e + 2, "sequence")?,
                    source: r.i16(e + 4, "sequence")?,
                    mask: r.u16(e + 6, "sequence")?,
                    value: r.u16(e + 8, "sequence")?,
                };
                steps.push(s);
                if s.source == 0 {
                    break;
                }
                if steps.len() >= MAX_STEPS {
                    return Err(format!("command {}: more than {MAX_STEPS} steps", commands.len()));
                }
            }
            commands.push(Command {
                flags,
                mv: r.i16(o + 4, "command")?,
                life: r.i16(o + 6, "command")?,
                window: r.i16(o + 8, "command")?,
                param: r.i16(o + 10, "command")?,
                seq: seq as u32,
                steps,
            });
        }
        // 2.8 chain programs: u32 offsets until 0
        let mut chains = Vec::new();
        loop {
            let o = r.u32(header[4] as usize + 4 * chains.len(), "chain list")?;
            if o == 0 {
                break;
            }
            if chains.len() >= MAX_CHAINS {
                return Err(format!("more than {MAX_CHAINS} chain programs"));
            }
            chains.push(Chain { offset: o, words: chain_words(&r, o as usize).map_err(|e| format!("chain {}: {e}", chains.len()))? });
        }
        // 2.10 AI section
        let ai = ai_section(&r, header[6] as usize)?;
        // 2.9 clips
        let clips = Clips::parse(&bytes, header[0] as usize)?;
        Ok(Self { header, params, spheres, bone_maps, forms, states, opponents, commands, chains, ai, clips, bytes })
    }

    /// The bone map (S+0x120 / +0x124: the current form's) of `form` 0 / 1, straight or mirrored.
    pub fn bone_map(&self, form: usize, mirrored: bool) -> &[u8; 20] {
        &self.bone_maps[(form & 1) * 2 + mirrored as usize]
    }

    /// State `n`'s script offset, None for an unused entry.
    pub fn state(&self, n: usize) -> Option<usize> {
        self.states.get(n).filter(|&&s| s != 0xFFFF).map(|&s| s as usize)
    }

    /// Object `k`'s node in `form` (desc[1]; −1 = none). The object count comes from the model (F.6), not the ANM:
    /// past the end of the file it reads −1.
    pub fn object_node(&self, form: usize, k: usize) -> i16 {
        u16le(&self.bytes, self.forms[form & 1].tables[1] as usize + 2 * k).map_or(-1, |v| v as i16)
    }

    /// Object `k`'s offset in its node's frame (desc[3], s10 ×3); 0 past the end of the file.
    pub fn object_offset(&self, form: usize, k: usize) -> [i32; 3] {
        s10x3(u32le(&self.bytes, self.forms[form & 1].tables[3] as usize + 4 * k).unwrap_or(0))
    }

    pub fn u8(&self, o: usize) -> u8 {
        self.bytes.get(o).copied().unwrap_or(0)
    }
    pub fn u16(&self, o: usize) -> u16 {
        u16le(&self.bytes, o).unwrap_or(0)
    }
    pub fn i16(&self, o: usize) -> i16 {
        self.u16(o) as i16
    }
    pub fn u32(&self, o: usize) -> u32 {
        u32le(&self.bytes, o).unwrap_or(0)
    }
}

/// A chain program's words with the main game's op lengths (0 end; 1 wait: 1 word, or `n | 0x4000` and n more; 2 input: 4;
/// 3 follow-up: 1; 4 window: 2), up to and with the ending 0.
fn chain_words(r: &Rd, o: usize) -> Result<Vec<u16>, String> {
    let mut w = Vec::new();
    loop {
        if w.len() >= MAX_CHAIN_WORDS {
            return Err(format!("longer than {MAX_CHAIN_WORDS} words"));
        }
        let op = r.u16(o + 2 * w.len(), "chain program")?;
        let args = match op {
            0 => {
                w.push(0);
                return Ok(w);
            }
            1 => {
                let a = r.u16(o + 2 * w.len() + 2, "chain program")?;
                if a & 0x4000 != 0 { (a & 0xFFF) as usize + 1 } else { 1 }
            }
            2 => 4,
            3 => 1,
            4 => 2,
            _ => return Err(format!("op {op} at word {}", w.len())),
        };
        for _ in 0..=args {
            w.push(r.u16(o + 2 * w.len(), "chain program")?);
        }
    }
}

/// The AI section at `o`: 10 words, the distance bands (u16 until 0xFFFF), the action records (12 bytes; from
/// record 6 on, mask 0 ends the list).
fn ai_section(r: &Rd, o: usize) -> Result<Ai, String> {
    let mut ai = Ai::default();
    for (k, w) in ai.words.iter_mut().enumerate() {
        *w = r.u32(o + 4 * k, "AI section")?;
    }
    let b = ai.words[1] as usize;
    loop {
        let v = r.u16(b + 2 * ai.bands.len(), "AI bands")?;
        ai.bands.push(v);
        if v == 0xFFFF {
            break;
        }
        if ai.bands.len() > 8 {
            return Err("AI bands: no 0xFFFF in 9 entries".into());
        }
    }
    let a = ai.words[2] as usize;
    loop {
        let e = a + 12 * ai.actions.len();
        let mask = r.u32(e, "AI action")?;
        if ai.actions.len() >= 6 && mask == 0 {
            break;
        }
        if ai.actions.len() >= MAX_AI_ACTIONS {
            return Err(format!("more than {MAX_AI_ACTIONS} AI actions"));
        }
        let mut scripts = [0u16; 4];
        for (j, s) in scripts.iter_mut().enumerate() {
            *s = r.u16(e + 4 + 2 * j, "AI action")?;
        }
        ai.actions.push(AiAction { mask, scripts });
    }
    Ok(ai)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The converted data of the imported game (`BR1_EXPORT`, else `../okb_export/br1`), None when missing.
    pub(crate) fn br1_file(rel: &str) -> Option<Vec<u8>> {
        let root = std::env::var("BR1_EXPORT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../okb_export/br1").into());
        std::fs::read(std::path::Path::new(&root).join(rel)).ok()
    }

    /// A small well-formed ANM: 19 nodes per form, one sphere per table, a command, a chain, the AI section and a
    /// clip of two keys.
    pub(crate) fn synthetic() -> Vec<u8> {
        let mut b = vec![0u8; 0x40];
        let put16 = |b: &mut Vec<u8>, o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_le_bytes());
        let put32 = |b: &mut Vec<u8>, o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let grow = |b: &mut Vec<u8>, n: usize| -> usize {
            let at = b.len();
            b.resize(at + n, 0);
            at
        };
        b[0x2C] = 19;
        b[0x2E] = 19;
        put16(&mut b, 0x34, 820);
        put16(&mut b, 0x36, 820);
        b[0x38] = 10;
        put16(&mut b, 0x3A, 250);
        put16(&mut b, 0x3C, 112);
        // spheres
        let sp = grow(&mut b, 24);
        let st = grow(&mut b, 4);
        b[st..st + 4].copy_from_slice(&[1, 4, 89, 0]);
        for k in 0..4 {
            put16(&mut b, sp + 2 * k, 1);
            put32(&mut b, sp + 8 + 4 * k, st as u32);
        }
        put32(&mut b, 0x28, sp as u32);
        // bone maps
        let bm = grow(&mut b, 16);
        let map = grow(&mut b, 20);
        for n in 1..20 {
            b[map + n] = (n - 1) as u8;
        }
        for k in 0..4 {
            put32(&mut b, bm + 4 * k, map as u32);
        }
        put32(&mut b, 0x24, bm as u32);
        // skeleton: parents (MIT's), objects, bind offsets, object offsets, base angles
        let sk = grow(&mut b, 0x28);
        let par = grow(&mut b, 2 * 19);
        for (n, p) in [-1i16, 0, 3, 4, 1, 17, 5, 6, 18, 8, 9, 3, 11, 12, 3, 14, 15, 4, 4].into_iter().enumerate() {
            put16(&mut b, par + 2 * n, p as u16);
        }
        let obj = grow(&mut b, 2 * 19);
        let bind = grow(&mut b, 4 * 18);
        put32(&mut b, bind, 5 | (0x3F0 << 10) | (3 << 20)); // node 1: (5, −16, 3)
        let oofs = grow(&mut b, 4 * 19);
        let ang = grow(&mut b, 4 * 18);
        put32(&mut b, ang + 4, 0x100 | (0x3FF << 20)); // node 2: (0x100, 0, −1) × 4
        for k in 0..2 {
            for (i, t) in [par, obj, bind, oofs, ang].into_iter().enumerate() {
                put32(&mut b, sk + 0x14 * k + 4 * i, t as u32);
            }
        }
        put32(&mut b, 0x20, sk as u32);
        // a script, the per-opponent table, the state table
        let script = grow(&mut b, 4);
        let grp = grow(&mut b, 4 * 9 + 32);
        for c in 0..9 {
            put32(&mut b, grp + 4 * c, (grp + 36) as u32);
        }
        put16(&mut b, grp + 36, 224);
        put32(&mut b, 0x08, grp as u32);
        let states = grow(&mut b, 2 * STATES);
        for i in 0..STATES {
            put16(&mut b, states + 2 * i, if i == 464 { 0xFFFF } else { script as u16 });
        }
        put32(&mut b, 0x04, states as u32);
        // a command and its sequence
        let cmd = grow(&mut b, 32);
        let seq = grow(&mut b, 20);
        put32(&mut b, cmd, 0x80);
        put16(&mut b, cmd + 4, 0x90);
        put32(&mut b, cmd + 12, seq as u32);
        put16(&mut b, seq + 4, 2);
        put16(&mut b, seq + 6, 0xE8);
        put32(&mut b, 0x0C, cmd as u32);
        // a chain program: 1 (list of 2), 4 window, 0
        let ch = grow(&mut b, 8);
        let prog = grow(&mut b, 2 * 7);
        for (k, w) in [1u16, 0x4001, 0x91, 4, 2, 9, 0].into_iter().enumerate() {
            put16(&mut b, prog + 2 * k, w);
        }
        put32(&mut b, ch, prog as u32);
        put32(&mut b, 0x10, ch as u32);
        // AI: words, bands, 6 fixed actions + 1 + the ending mask 0
        let ai = grow(&mut b, 40);
        let bands = grow(&mut b, 8);
        for (k, v) in [0x180u16, 0x200, 0x300, 0xFFFF].into_iter().enumerate() {
            put16(&mut b, bands + 2 * k, v);
        }
        let acts = grow(&mut b, 12 * 8);
        put32(&mut b, acts + 12 * 6, 0x8);
        for k in 0..10 {
            put32(&mut b, ai + 4 * k, bands as u32);
        }
        put32(&mut b, ai + 8, acts as u32);
        put32(&mut b, 0x18, ai as u32);
        // clips: 3 entries (0 empty, 1 = two keys, 2 empty), −1
        let blk = grow(&mut b, 8 + 4 * 4);
        let clip = grow(&mut b, 4 + 8);
        let keys = grow(&mut b, 2 * KEY);
        put16(&mut b, clip, 5);
        put16(&mut b, clip + 4, (keys - clip) as u16);
        put16(&mut b, clip + 6, 4);
        put16(&mut b, clip + 8, (keys + KEY - clip) as u16);
        put16(&mut b, keys + KEY, 0x40); // key 1's yaw
        put16(&mut b, blk + 4, 3);
        put32(&mut b, blk + 12, (clip - blk) as u32);
        put32(&mut b, blk + 20, u32::MAX);
        let size = (b.len() - blk) as u32;
        put32(&mut b, blk, size);
        put32(&mut b, 0x00, blk as u32);
        b
    }

    #[test]
    fn synthetic_file() {
        let a = Anm::parse(synthetic()).unwrap();
        assert_eq!(a.params, Params { nodes: [19, 19], timer_factor: [0, 0], weight: [820, 820], regen: 10, max_hp: 250, root_scale: 112 });
        assert_eq!(a.spheres[0].len(), 1);
        assert_eq!((a.spheres[3][0].node, a.spheres[3][0].radius), (4, 89));
        assert_eq!(a.bone_map(1, true)[18], 17);
        assert_eq!(a.forms[0].parents[5], 17);
        assert_eq!(a.forms[0].bind[0], [5, -16, 3]);
        assert_eq!(a.forms[1].base_angles[1], [0x100, 0, -1]);
        assert_eq!(a.state(464), None);
        assert!(a.state(463).is_some());
        assert_eq!(a.opponents[8][0], 224);
        assert_eq!(a.commands.len(), 1);
        assert_eq!((a.commands[0].mv, a.commands[0].steps.len(), a.commands[0].steps[0].mask), (0x90, 2, 0xE8));
        assert_eq!(a.chains[0].words, vec![1, 0x4001, 0x91, 4, 2, 9, 0]);
        assert_eq!(a.ai.bands, vec![0x180, 0x200, 0x300, 0xFFFF]);
        assert_eq!(a.ai.actions.len(), 7);
        assert_eq!(a.clips.count(), 3);
        let c = a.clips.get(1).unwrap();
        assert_eq!((c.frames, c.keys.clone()), (5, vec![((c.key(0).unwrap() - c.offset) as u16, 4), ((c.key(1).unwrap() - c.offset) as u16, 0)]));
        assert_eq!(a.i16(c.key(1).unwrap()), 0x40);
        assert!(a.clips.get(0).is_none() && a.clips.get(2).is_none() && a.clips.get(3).is_none());
        assert_eq!(a.object_node(0, 1000), -1);
    }

    /// Key offsets are s16 (the key setup's `lh`): one before the clip is valid while it stays in the file.
    #[test]
    fn key_offsets_are_signed() {
        let good = synthetic();
        let c = Anm::parse(good.clone()).unwrap().clips.get(1).unwrap().offset;
        let with = |off: i16| {
            let mut b = good.clone();
            b[c + 4..c + 6].copy_from_slice(&off.to_le_bytes());
            Anm::parse(b)
        };
        assert_eq!(with(-(c as i16)).unwrap().clips.get(1).unwrap().key(0), Some(0));
        assert!(with(-(c as i16) - 1).is_err());
    }

    /// Every truncation and many single-byte changes of the synthetic file parse or fail without a panic.
    #[test]
    fn damaged_files_do_not_panic() {
        let good = synthetic();
        for n in 0..good.len() {
            let _ = Anm::parse(good[..n].to_vec());
        }
        for o in 0..good.len() {
            for v in [0u8, 0x7F, 0x80, 0xFF] {
                let mut b = good.clone();
                b[o] = v;
                if let Ok(a) = Anm::parse(b) {
                    for c in a.clips.clips.iter().flatten() {
                        assert!((0..c.keys.len()).all(|k| c.key(k).is_some_and(|o| o + KEY <= a.bytes.len())));
                    }
                }
            }
        }
        assert!(Anm::parse(vec![0; 0x3F]).is_err());
        assert!(Anm::parse(vec![0xFF; 0x400]).is_err());
    }

    /// The four imported fighters' files: counts as the research parser found them.
    #[test]
    fn real_files() {
        let Some(cmn) = br1_file("engine/CMN/CMN.ACT") else {
            eprintln!("skipped: no okb_export/br1");
            return;
        };
        let cmn = Clips::parse(&cmn, 0).unwrap();
        assert_eq!((cmn.count(), cmn.clips.iter().flatten().count()), (127, 78));
        // (code, commands, chains, clip slots, used, max HP, root scale, nodes)
        for (code, cmds, chains, slots, used, hp, scale, nodes) in [
            ("MIT", 25, 21, 850, 175, 250, 128, [19, 23]),
            ("FOX", 28, 32, 856, 159, 250, 128, [22, 23]),
            ("GRE", 26, 31, 849, 170, 250, 112, [20, 20]),
            ("URI", 25, 18, 858, 164, 200, 128, [26, 24]),
            ("ZIN", 25, 32, 901, 177, 250, 128, [22, 24]),
        ] {
            let a = Anm::parse(br1_file(&format!("engine/{code}/{code}31.DAT")).unwrap()).unwrap_or_else(|e| panic!("{code}: {e}"));
            assert_eq!((a.commands.len(), a.chains.len()), (cmds, chains), "{code}");
            assert_eq!((a.clips.count(), a.clips.clips.iter().flatten().count()), (slots, used), "{code}");
            assert_eq!((a.params.max_hp, a.params.root_scale, a.params.nodes, a.params.weight), (hp, scale, nodes, [820, 820]), "{code}");
            assert_eq!(a.spheres.iter().map(Vec::len).collect::<Vec<_>>(), [5, 5, 15, 15], "{code}");
            // the same bone maps as the main game's
            assert_eq!(a.bone_map(0, true), &[0, 0, 1, 2, 3, 7, 8, 9, 4, 5, 6, 13, 14, 15, 10, 11, 12, 17, 16, 18], "{code}");
            // the throw-victim moves per opponent
            for (c, first) in [192, 304, 288, 272, 224, 208, 256, 240, 320].into_iter().enumerate() {
                assert_eq!(a.opponents[c], std::array::from_fn(|k| first + k as i16), "{code} opponent {c}");
            }
            // clip header frames = Σ durations + 1
            for (i, c) in a.clips.clips.iter().enumerate() {
                if let Some(c) = c {
                    assert_eq!(c.frames as u32, c.keys.iter().map(|k| k.1 as u32).sum::<u32>() + 1, "{code} clip {i}");
                }
            }
            assert_eq!(*a.ai.bands.last().unwrap(), 0xFFFF);
            assert!(a.ai.actions.len() >= 26, "{code}");
            assert_eq!(a.forms[0].parents[..5], [-1, 0, 3, 4, 1], "{code}");
        }
    }
}
