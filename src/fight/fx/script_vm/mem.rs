//! The effect VM's address space: segments at fixed virtual addresses.
//!
//! State: serialised: part of `GameState` (field changes are `STATE_VERSION` changes)
//!
//! The VM's address space: flat little-endian memory made of segments at fixed virtual addresses;
//! the script blobs and EXE tables are mapped as static data, copied on the first write.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A segment's bytes: static data (the script blobs, the EXE tables; not serialised, put back by `Vm::rebind`)
/// until the first write makes them owned (copy-on-write).
#[derive(Clone, Serialize, Deserialize)]
enum Bytes {
    Static(#[serde(skip)] Arc<[u8]>),
    Owned(Vec<u8>),
}

impl Bytes {
    fn get(&self) -> &[u8] {
        match self {
            Bytes::Static(b) => b,
            Bytes::Owned(b) => b,
        }
    }

    fn get_mut(&mut self) -> &mut [u8] {
        if let Bytes::Static(b) = self {
            *self = Bytes::Owned(b.to_vec());
        }
        match self {
            Bytes::Owned(b) => b,
            Bytes::Static(_) => unreachable!(),
        }
    }
}

/// One memory segment.
#[derive(Clone, Serialize, Deserialize)]
struct Seg {
    base: u32,
    data: Bytes,
}

/// Flat little-endian memory made of segments (unmapped reads give 0, writes are dropped and logged once).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Mem {
    segs: Vec<Seg>,
    /// debug: the unmapped write was logged
    #[serde(skip)]
    warned: bool,
}

impl Mem {
    fn find(&self, a: u32, n: u32) -> Option<(usize, usize)> {
        let i = self.segs.partition_point(|s| s.base <= a).checked_sub(1)?;
        let s = &self.segs[i];
        let off = (a - s.base) as usize;
        off.checked_add(n as usize).is_some_and(|e| e <= s.data.get().len()).then_some((i, off))
    }

    /// Map (or replace) a segment.
    pub fn map(&mut self, base: u32, data: Vec<u8>) {
        self.insert(base, Bytes::Owned(data));
    }

    /// Map (or replace) a segment of static data, shared until written.
    pub fn map_static(&mut self, base: u32, data: Arc<[u8]>) {
        self.insert(base, Bytes::Static(data));
    }

    fn insert(&mut self, base: u32, data: Bytes) {
        self.unmap(base);
        let i = self.segs.partition_point(|s| s.base < base);
        self.segs.insert(i, Seg { base, data });
    }

    pub fn unmap(&mut self, base: u32) {
        if let Ok(i) = self.segs.binary_search_by_key(&base, |s| s.base) {
            self.segs.remove(i);
        }
    }

    /// After deserialising: the static segments' bytes from `fresh` (the same segments as mapped for the fight).
    /// Err when the segments are not in address order (`find` relies on it) or a static one has no static
    /// counterpart in `fresh` (other match data).
    pub(super) fn rebind(&mut self, fresh: &Mem) -> Result<(), String> {
        if !self.segs.windows(2).all(|w| w[0].base < w[1].base) {
            return Err("fxvm: segments out of order".into());
        }
        for s in self.segs.iter_mut() {
            if let Bytes::Static(b) = &mut s.data {
                match fresh.segs.binary_search_by_key(&s.base, |f| f.base).map(|i| &fresh.segs[i].data) {
                    Ok(Bytes::Static(f)) => *b = f.clone(),
                    _ => return Err(format!("fxvm: static segment {:#010x} not in the match data", s.base)),
                }
            }
        }
        Ok(())
    }

    /// Copy bytes into mapped memory (engine-shared blocks are refreshed this way every frame).
    pub fn write(&mut self, a: u32, bytes: &[u8]) {
        if let Some((i, off)) = self.find(a, bytes.len() as u32) {
            self.segs[i].data.get_mut()[off..off + bytes.len()].copy_from_slice(bytes);
        }
    }

    fn rd<const N: usize>(&self, a: u32) -> [u8; N] {
        match self.find(a, N as u32) {
            Some((i, off)) => self.segs[i].data.get()[off..off + N].try_into().unwrap(),
            None => [0; N],
        }
    }

    fn wr(&mut self, a: u32, b: &[u8]) {
        match self.find(a, b.len() as u32) {
            Some((i, off)) => self.segs[i].data.get_mut()[off..off + b.len()].copy_from_slice(b),
            None => {
                if !self.warned {
                    log::warn!("fxvm: write to unmapped {a:#010x}");
                    self.warned = true;
                }
            }
        }
    }

    pub fn u8(&self, a: u32) -> u8 {
        self.rd::<1>(a)[0]
    }
    pub fn u16(&self, a: u32) -> u16 {
        u16::from_le_bytes(self.rd(a))
    }
    pub fn s16(&self, a: u32) -> i16 {
        self.u16(a) as i16
    }
    pub fn u32(&self, a: u32) -> u32 {
        u32::from_le_bytes(self.rd(a))
    }
    pub fn s32(&self, a: u32) -> i32 {
        self.u32(a) as i32
    }
    pub fn w8(&mut self, a: u32, v: u8) {
        self.wr(a, &[v]);
    }
    pub fn w16(&mut self, a: u32, v: u16) {
        self.wr(a, &v.to_le_bytes());
    }
    pub fn w32(&mut self, a: u32, v: u32) {
        self.wr(a, &v.to_le_bytes());
    }
}
