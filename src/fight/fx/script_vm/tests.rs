//! The VM's unit tests.

use super::*;

fn script(code: &[u32], obj_size: u32) -> Vec<u8> {
    let mut b = Vec::new();
    for w in [0x1C, obj_size, 0, 0, 0, 0, 0] {
        b.extend_from_slice(&w.to_le_bytes());
    }
    for w in code {
        b.extend_from_slice(&w.to_le_bytes());
    }
    b
}

/// op00.04 / op00.01 / op00.0D / JLT / YIELD / DESTROY: a counter that counts to 3 over 3 frames, then ends.
#[test]
fn counter_script() {
    let f = 0x20u32; // counter field
    let code = [
        0x0000_0400 | f << 16,
        0, // 0x1C: A.u16 = 0
        0x0000_0100 | f << 16,
        1, // 0x24: L: A.u16 += 1
        0x0000_0D00 | f << 16,
        3,                        // 0x2C: COND = A - 3
        0x0000_0707 | 0x3C << 16, // 0x34: JLT 0x3C
        0x0000_0C07,              // 0x38: DESTROY
        0x0000_0407 | 0x24 << 16, // 0x3C: YIELD L
    ];
    let mut vm = Vm::new();
    vm.set_group(0, &[Some(script(&code, 0x40))]);
    let h = vm.spawn(0, 0, None, &[]);
    let o = vm.obj_of(h);
    assert_eq!(vm.mem.u32(o), o);
    for frame in 1..=3 {
        vm.tick();
        assert_eq!(vm.mem.u16(o + f), frame as u16);
    }
    assert!(!vm.alive(h)); // DESTROY on the third frame
    vm.tick();
    assert_eq!(vm.live(), 0);
}
