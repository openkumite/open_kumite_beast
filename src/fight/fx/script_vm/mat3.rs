//! The effect VM's 3×3 4.12 fixed-point matrices.
//!
//! State: none
//!
//! The 3×3 4.12 fixed-point matrices of the VM: read / write in VM memory, the product, the Euler and axis
//! rotations.

use super::{Vm, rcos, rsin};

pub type M3 = [[i32; 3]; 3];

impl Vm {
    // ------------------------------------------------------------------ matrices

    pub(crate) fn read_m3(&self, a: u32) -> M3 {
        let mut m = [[0; 3]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = self.mem.s16(a + 2 * (3 * r + c) as u32) as i32;
            }
        }
        m
    }

    pub(crate) fn write_m3(&mut self, a: u32, m: &M3) {
        for r in 0..3 {
            for c in 0..3 {
                self.mem.w16(a + 2 * (3 * r + c) as u32, m[r][c] as u16);
            }
        }
    }

    pub(crate) fn mul(a: &M3, b: &M3) -> M3 {
        let mut o = [[0; 3]; 3];
        for r in 0..3 {
            for c in 0..3 {
                o[r][c] = (a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c]) >> 12;
            }
        }
        o
    }

    /// Euler matrix of op04.00 / op03.18.
    pub(crate) fn euler(x: i32, y: i32, z: i32) -> M3 {
        let (sx, cx, sy, cy, sz, cz) = (rsin(x), rcos(x), rsin(y), rcos(y), rsin(z), rcos(z));
        let syx = (sy * sx) >> 12;
        let cyx = (cy * sx) >> 12;
        [
            [((cy * cz) >> 12) - ((syx * sz) >> 12), -((cy * sz) >> 12) - ((syx * cz) >> 12), -((sy * cx) >> 12)],
            [(cx * sz) >> 12, (cx * cz) >> 12, -sx],
            [((sy * cz) >> 12) + ((cyx * sz) >> 12), -((sy * sz) >> 12) + ((cyx * cz) >> 12), (cy * cx) >> 12],
        ]
    }

    pub(crate) fn rot_x(a: i32) -> M3 {
        let (s, c) = (rsin(a), rcos(a));
        [[4096, 0, 0], [0, c, -s], [0, s, c]]
    }
    pub(crate) fn rot_y(a: i32) -> M3 {
        let (s, c) = (rsin(a), rcos(a));
        [[c, 0, s], [0, 4096, 0], [-s, 0, c]]
    }
    pub(crate) fn rot_z(a: i32) -> M3 {
        let (s, c) = (rsin(a), rcos(a));
        [[c, -s, 0], [s, c, 0], [0, 0, 4096]]
    }
    /// Ry(b)·Rx(a)
    pub(crate) fn rot2(a: i32, b: i32) -> M3 {
        let (sa, ca, sb, cb) = (rsin(a), rcos(a), rsin(b), rcos(b));
        [[cb, (sb * sa) >> 12, (sb * ca) >> 12], [0, ca, -sa], [-sb, (cb * sa) >> 12, (cb * ca) >> 12]]
    }
}
