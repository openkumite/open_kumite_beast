//! Raw game tables: `.ACT` animation key lists and `.ANI` move-script entries (files in the converted data's `engine/`).
//!
//! State: static data (not serialised; re-attached by `rebind`)

// past the end of the data both read 0 (a truncated or corrupt file must not crash the game)
fn u16le(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32le(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One animation of an ACT file: the duration of every key in ticks (the last key has 0 = hold).
#[derive(Clone, Debug)]
pub struct ActAnim {
    pub head: u16,
    pub keys: Vec<u16>,
}

impl ActAnim {
    /// Ticks from the start of the animation to the start of key `k` (0-based).
    pub fn key_start(&self, k: usize) -> u32 {
        self.keys.iter().take(k).map(|&d| d as u32).sum()
    }
}

#[derive(Clone)]
pub struct Act {
    pub anims: Vec<Option<ActAnim>>,
    pub offsets: Vec<usize>,
    pub bytes: Vec<u8>,
}

impl Act {
    pub fn parse(b: &[u8]) -> Self {
        // at most one table entry per 4 bytes of the file
        let count = (u32le(b, 4) as usize).min(b.len() / 4);
        let mut anims = Vec::with_capacity(count);
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            let base = u32le(b, 8 + 4 * i) as usize;
            if base == 0 || base.checked_add(8).is_none_or(|e| e > b.len()) {
                anims.push(None);
                offsets.push(0);
                continue;
            }
            offsets.push(base);
            let mut keys = Vec::new();
            let mut a = base + 4;
            while a + 4 <= b.len() && keys.len() < 256 {
                let dur = u16le(b, a + 2);
                keys.push(dur);
                a += 4;
                if dur == 0 {
                    break;
                }
            }
            anims.push(Some(ActAnim { head: u16le(b, base), keys }));
        }
        Self { anims, offsets, bytes: b.to_vec() }
    }

    /// Byte offset of animation `i` (table of `count` offsets at +8), if present.
    pub fn offset(&self, i: usize) -> Option<usize> {
        self.offsets.get(i).copied().filter(|&o| o != 0)
    }

    /// Number of animations (u16 ACT header, S.A4).
    pub fn count(&self) -> usize {
        self.offsets.len()
    }

    pub fn get(&self, i: usize) -> Option<&ActAnim> {
        self.anims.get(i).and_then(|a| a.as_ref())
    }
}

/// One body / hit sphere of the ANI collision tables (header word 8): node and radius.
#[derive(Clone, Copy, Debug)]
pub struct Sphere {
    pub group: u8,
    pub node: u8,
    pub radius: i16,
}

/// Move scripts of a `<CODE><n>P.ANI`: the whole file (scripts address it directly) and the entry table.
pub struct Ani {
    pub bytes: Vec<u8>,
    pub entries: Vec<u16>,
}

impl Ani {
    pub fn parse(bytes: Vec<u8>) -> Self {
        let s_entries = u32le(&bytes, 4) as usize;
        let s_moves = u32le(&bytes, 12) as usize;
        let entries = (s_entries..s_moves.min(bytes.len())).step_by(2).map(|o| u16le(&bytes, o)).collect();
        Self { bytes, entries }
    }
    /// Per-opponent table of the ANI's group section (header word 2): 16 halfwords for opponent `chr`.
    pub fn group(&self, chr: usize) -> Vec<i16> {
        let g = u32le(&self.bytes, 8) as usize;
        let first = u32le(&self.bytes, g) as usize;
        let n = first.saturating_sub(g) / 4;
        if chr >= n {
            return vec![];
        }
        let o = u32le(&self.bytes, g + 4 * chr) as usize;
        (0..16).map(|k| self.i16(o + 2 * k)).collect()
    }
    /// Collision table `k` (0/1 = body spheres of the human / beast form, 2/3 = hit spheres).
    pub fn spheres(&self, k: usize) -> Vec<Sphere> {
        let p = u32le(&self.bytes, 32) as usize;
        let cnt = u16le(&self.bytes, p + 2 * k) as usize;
        let tab = u32le(&self.bytes, p + 8 + 4 * k) as usize;
        // only the entries that lie inside the file
        (0..cnt)
            .map_while(|i| self.bytes.get(tab + 4 * i..tab + 4 * i + 4).map(|e| Sphere { group: e[0], node: e[1], radius: i16::from_le_bytes([e[2], e[3]]) }))
            .collect()
    }
    /// Bone map (20 bytes, ANI header word 7): `form` 0/1, `mirrored` = left/right swapped.
    pub fn bone_map(&self, form: usize, mirrored: bool) -> &[u8] {
        let p = u32le(&self.bytes, 28) as usize;
        let o = u32le(&self.bytes, p + 4 * (form * 2 + mirrored as usize)) as usize;
        // a table outside the file: the straight map (node n → slot n − 1)
        const STRAIGHT: [u8; 20] = [0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18];
        self.bytes.get(o..o.saturating_add(20)).unwrap_or(&STRAIGHT)
    }

    /// The 12 parameters after the header (0x24).
    pub fn param(&self, i: usize) -> i16 {
        self.i16(0x24 + 2 * i)
    }

    pub fn u8(&self, o: usize) -> u8 {
        self.bytes.get(o).copied().unwrap_or(0)
    }
    pub fn u16(&self, o: usize) -> u16 {
        u16le(&self.bytes, o)
    }
    pub fn i16(&self, o: usize) -> i16 {
        self.u16(o) as i16
    }
}
