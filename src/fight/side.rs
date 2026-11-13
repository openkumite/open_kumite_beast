//! Fighters, slots and banks (engine addition): a fighter's index (`Fid`), the
//! game's player number it plays as (`Slot`), which copy of the per-player resources it uses (`Bank`), and the
//! per-player arrays of the game indexed by slot only (`PerSlot`).
//!
//! State: `PerSlot` is serialised as the `[T; 2]` it holds (`#[serde(transparent)]`: the same postcard and CBOR
//! bytes); the others are not serialised
//!
//! Every per-player resource of the game exists twice, for P1 and P2. Fighter `k` (0, 1 in a match; 0..3 in the
//! team battle) plays as slot `k & 1` with bank `k >> 1`: P3 is "P1 in bank 1", P4 "P2 in bank 1". The per-fighter code
//! reads the slot (`Fighter::player`); the team battle keeps the real index and the bank. `Bank` holds what bank 1
//! changes: the CLUT word's bit 15 (VRAM bank 1), the character sound slots, the AI's random index, the
//! effect VM's groups and the `<CHR>EFT.DAT` entries in EffTab.

use serde::{Deserialize, Serialize};

/// A fighter's index: 0 / 1 in a match, 0..3 in the team battle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fid(pub u8);

impl Fid {
    /// Fighter `k`.
    pub const fn of(k: usize) -> Fid {
        Fid(k as u8)
    }

    /// The game's player number it plays as (`k & 1`).
    pub const fn slot(self) -> Slot {
        Slot::of_player(self.0)
    }

    /// Its index into the shared AI random table (`ai::Ai::new_at`): its slot's (slot·128, as the
    /// game's `player.wrapping_mul(128)`) plus its bank's offset: 0 / 128 / 64 / 192.
    pub const fn ai_index(self) -> u8 {
        (self.slot().index() as u8).wrapping_mul(128).wrapping_add(self.bank().ai_offset())
    }

    /// The copy of the per-player resources it uses (`k >> 1`): 0 for fighters 0 / 1, 1 for the team battle's 2 / 3.
    pub const fn bank(self) -> Bank {
        if self.0 >> 1 == 0 { Bank::B0 } else { Bank::B1 }
    }
}

/// The game's player number (P1 / P2), the index of its per-player resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    P1,
    P2,
}

impl Slot {
    /// The slot of player number `p` (`Fighter::player`; its bit 0, as the per-fighter code reads `player & 1`).
    pub const fn of_player(p: u8) -> Slot {
        if p & 1 == 0 { Slot::P1 } else { Slot::P2 }
    }

    /// 0 for P1, 1 for P2.
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Which copy of the per-player resources a fighter uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bank {
    /// the game's own: a match's fighters, the team battle's 0 / 1
    B0,
    /// the engine's copy for the team battle's fighters 2 / 3 (P3 / P4)
    B1,
}

impl Bank {
    /// The CLUT word's bit 15 for an effect primitive drawn from this bank's VRAM (`render/shaders/psx.wgsl` samples
    /// VRAM bank 1 when it is set; the PS1 GPU never reads it).
    pub const fn clut_bit(self) -> u16 {
        match self {
            Bank::B0 => 0,
            Bank::B1 => 0x8000,
        }
    }

    /// A main-game character sound `id` (slots 3 / 4, ids 0xC0..0x13F) in this bank: bank 1 plays it from slots
    /// 7 / 8 (+0x100; slots 5 / 6 stay the game-over voices).
    pub const fn br2_sound(self, id: i16) -> i16 {
        match self {
            Bank::B1 if matches!(id >> 6, 3 | 4) => id + 0x100,
            _ => id,
        }
    }

    /// An imported character sound `id` (from 0xC0: slots 3 / 4 after the P2 remap) in this bank: bank 1 plays it
    /// from slots 5 / 6 (+0x80).
    pub const fn br1_sound(self, id: i16) -> i16 {
        match self {
            Bank::B1 if id >= 0xC0 => id.wrapping_add(0x80),
            _ => id,
        }
    }

    /// What this bank adds to a slot's index into the shared AI random table (slot·128): bank 1 64, so
    /// the four fighters read 0 / 128 / 64 / 192.
    pub const fn ai_offset(self) -> u8 {
        match self {
            Bank::B0 => 0,
            Bank::B1 => 64,
        }
    }

    /// The effect VM's script / emitter group of slot `s`'s `<CHR>EFT` entries: bank 0 1 / 2, bank 1 5 / 6.
    pub const fn vm_group(self, s: Slot) -> usize {
        match self {
            Bank::B0 => 1 + s.index(),
            Bank::B1 => 5 + s.index(),
        }
    }

    /// The bank of the effect VM's group `g` (5 and over: bank 1's).
    pub const fn of_vm_group(g: usize) -> Bank {
        if g >= Bank::B1.vm_group(Slot::P1) { Bank::B1 } else { Bank::B0 }
    }

    /// Where slot `s`'s `<CHR>EFT.DAT` entries 1..20 start in EffTab: bank 0 40 / 60, bank 1 100 / 120 (its scripts
    /// from the 11th, its emitters from the 1st).
    pub const fn eft_base(self, s: Slot) -> usize {
        match self {
            Bank::B0 => 40 + 20 * s.index(),
            Bank::B1 => 100 + 20 * s.index(),
        }
    }
}

/// The CLUT bank bit of fighter `side`'s effect primitives (`Bank::clut_bit` of its bank).
pub const fn bank_bit(side: usize) -> u16 {
    Fid::of(side).bank().clut_bit()
}

/// One value per slot of the game (P1, P2), indexed by `Slot` only: the per-player arrays whose index must be a
/// slot, never a fighter (in the team battle two fighters share a slot). Serialised as `[T; 2]`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PerSlot<T>(pub [T; 2]);

impl<T> PerSlot<T> {
    /// The values of P1 and P2.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.0.iter()
    }

    /// The values of P1 and P2.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.0.iter_mut()
    }
}

impl<T> std::ops::Index<Slot> for PerSlot<T> {
    type Output = T;
    fn index(&self, s: Slot) -> &T {
        &self.0[s.index()]
    }
}

impl<T> std::ops::IndexMut<Slot> for PerSlot<T> {
    fn index_mut(&mut self, s: Slot) -> &mut T {
        &mut self.0[s.index()]
    }
}

/// As the array it holds (the debug tools print it).
impl<T: std::fmt::Debug> std::fmt::Debug for PerSlot<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot and bank of each fighter and bank 1's resources.
    #[test]
    fn the_bank_table() {
        let f: [Fid; 4] = std::array::from_fn(Fid::of);
        assert_eq!(f.map(Fid::slot), [Slot::P1, Slot::P2, Slot::P1, Slot::P2]);
        assert_eq!(f.map(Fid::bank), [Bank::B0, Bank::B0, Bank::B1, Bank::B1]);
        assert_eq!([0, 1, 2, 3].map(bank_bit), [0, 0, 0x8000, 0x8000]);
        assert_eq!(f.map(|f| f.bank().vm_group(f.slot())), [1, 2, 5, 6]);
        assert_eq!(f.map(|f| f.bank().eft_base(f.slot())), [40, 60, 100, 120]);
        assert_eq!(f.map(Fid::ai_index), [0, 128, 64, 192]);
        assert_eq!([0, 4, 5, 6].map(Bank::of_vm_group), [Bank::B0, Bank::B0, Bank::B1, Bank::B1]);
        assert_eq!([0xBF, 0xC0, 0x13F, 0x140].map(|id| Bank::B1.br2_sound(id)), [0xBF, 0x1C0, 0x23F, 0x140]);
        assert_eq!([0xC0, 0x13F].map(|id| Bank::B0.br2_sound(id)), [0xC0, 0x13F]);
        assert_eq!([0xBF, 0xC0].map(|id| (Bank::B0.br1_sound(id), Bank::B1.br1_sound(id))), [(0xBF, 0xBF), (0xC0, 0x140)]);
    }

    /// `PerSlot` serialises as the array it holds (postcard: snapshots and traces; CBOR: save states).
    #[test]
    fn per_slot_is_its_array_on_the_wire() {
        let a = [[1i16, -2, 3, 4], [5, 6, -7, 8]];
        assert_eq!(postcard::to_allocvec(&PerSlot(a)).unwrap(), postcard::to_allocvec(&a).unwrap());
        assert_eq!(postcard::from_bytes::<PerSlot<[i16; 4]>>(&postcard::to_allocvec(&a).unwrap()).unwrap(), PerSlot(a));
        let (mut x, mut y) = (Vec::new(), Vec::new());
        ciborium::into_writer(&PerSlot(a), &mut x).unwrap();
        ciborium::into_writer(&a, &mut y).unwrap();
        assert_eq!(x, y, "CBOR (save states)");
        assert_eq!(format!("{:?}", PerSlot(a)), format!("{a:?}"));
    }
}
