//! FNV-1a 64: the state hashes (`session::hash`), the replay checksums and sync hashes, netplay's checksum. CRC-32
//! (IEEE): the game data's file checksums (`assets::manifest`; the converter `ok_br2_converter` writes the same ones).

/// FNV-1a 64's basis.
pub(crate) const FNV_BASIS: u64 = 0xCBF2_9CE4_8422_2325;

/// FNV-1a 64 of `b` on from `h`.
pub(crate) fn fnv1a(mut h: u64, b: &[u8]) -> u64 {
    for &x in b {
        h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// CRC-32 (IEEE 802.3, reflected polynomial 0xEDB88320) table, one entry per byte value.
const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// A running CRC-32 (IEEE): `update` with the bytes in order, then `value`.
#[derive(Clone, Copy, Debug)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self(0xFFFF_FFFF)
    }
}

impl Crc32 {
    pub fn update(&mut self, b: &[u8]) {
        let mut c = self.0;
        for &x in b {
            c = CRC_TABLE[((c ^ x as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }

    pub fn value(self) -> u32 {
        !self.0
    }
}

/// CRC-32 (IEEE) of `b`.
pub fn crc32(b: &[u8]) -> u32 {
    let mut c = Crc32::default();
    c.update(b);
    c.value()
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc32_check_value() {
        // the shared test vector (the converter checks the same one)
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(super::crc32(b""), 0);
        let mut c = super::Crc32::default();
        c.update(b"1234");
        c.update(b"56789");
        assert_eq!(c.value(), 0xCBF4_3926);
    }
}
