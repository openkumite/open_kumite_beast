//! Debug (OKB_OLDHUD, desktop only): the engine's first stand-in HUD.
//!
//! Engine addition.
//! State: presentation (reads the state, never writes it)
//!
//! Debug (OKB_OLDHUD, desktop only): the engine's first stand-in HUD (engine addition), health and gauge bars, timer,
//! round wins and messages as plain quads in a 5×7 font.

use crate::fight::sim::Match;

/// HUD quads: health (yellow, recoverable part red), beast gauge, timer, round wins, messages.
pub(super) fn old_hud(m: &Match, w: f32, h: f32) -> Vec<([f32; 4], [f32; 4])> {
    let mut q = Vec::new();
    let u = (w / 640.0).min(h / 360.0);
    if m.round.hud_hidden {
        // match +0xB0: bars and timer hidden (intro, results)
        if !m.round.message.is_none() {
            q.push(([0.0, h * 0.42, w, h * 0.58], [0.0, 0.0, 0.0, 0.7]));
            text_quads(&mut q, &m.round.message.text(), w / 2.0, h / 2.0, 6.0 * u, [1.0, 0.85, 0.2, 1.0]);
        }
        return q;
    }
    let bar_w = 240.0 * u;
    let bar_h = 14.0 * u;
    let top = 18.0 * u;
    for i in 0..2 {
        let f = &m.fighters[i];
        let max = f.o_b0.max(1) as f32;
        let life = ((max - f.o_ac.max(0) as f32) / max).clamp(0.0, 1.0);
        let red = ((max - f.o_ae.max(0) as f32) / max).clamp(0.0, 1.0);
        // P1 bar empties toward the centre from the left edge, P2 mirrored
        let (x0, x1) = if i == 0 { (w / 2.0 - 34.0 * u - bar_w, w / 2.0 - 34.0 * u) } else { (w / 2.0 + 34.0 * u, w / 2.0 + 34.0 * u + bar_w) };
        q.push(([x0 - 2.0, top - 2.0, x1 + 2.0, top + bar_h + 2.0], [0.0, 0.0, 0.0, 0.7]));
        let seg = |frac: f32| if i == 0 { [x1 - bar_w * frac, top, x1, top + bar_h] } else { [x0, top, x0 + bar_w * frac, top + bar_h] };
        q.push((seg(red.max(life)), [0.85, 0.1, 0.05, 1.0]));
        q.push((seg(life), [1.0, 0.85, 0.1, 1.0]));
        // beast gauge (0x1E00 = transform possible, 0x3BFF full)
        let g = (f.gauge.max(0) as f32 / 0x3BFF as f32).clamp(0.0, 1.0);
        let gy = h - 26.0 * u;
        let gw = 160.0 * u;
        let (g0, g1) = if i == 0 { (20.0 * u, 20.0 * u + gw) } else { (w - 20.0 * u - gw, w - 20.0 * u) };
        q.push(([g0 - 2.0, gy - 2.0, g1 + 2.0, gy + 10.0 * u + 2.0], [0.0, 0.0, 0.0, 0.7]));
        let full = f.gauge >= 0x1E00;
        let col = if full { [0.2, 0.9, 1.0, 1.0] } else { [0.15, 0.45, 0.95, 1.0] };
        let seg = if i == 0 { [g0, gy, g0 + gw * g, gy + 10.0 * u] } else { [g1 - gw * g, gy, g1, gy + 10.0 * u] };
        q.push((seg, col));
        // round wins
        for k in 0..m.round.lost[1 - i].min(m.round.need) {
            let cx = if i == 0 { x1 - 10.0 * u - k as f32 * 14.0 * u } else { x0 + 10.0 * u + k as f32 * 14.0 * u };
            q.push(([cx - 5.0 * u, top + bar_h + 5.0 * u, cx + 5.0 * u, top + bar_h + 15.0 * u], [1.0, 0.9, 0.3, 1.0]));
        }
    }
    // timer (seconds), seven-segment digits
    let secs = if m.round.seconds() < 0 { 99 } else { m.round.seconds() };
    let dw = 12.0 * u;
    let dh = 22.0 * u;
    for (k, d) in [secs / 10 % 10, secs % 10].iter().enumerate() {
        let x = w / 2.0 - dw - 3.0 * u + k as f32 * (dw + 6.0 * u);
        seven_segment(&mut q, x, top - 4.0 * u, dw, dh, *d as u8, [1.0, 1.0, 1.0, 1.0]);
    }
    // big message bar
    if !m.round.message.is_none() {
        q.push(([0.0, h * 0.42, w, h * 0.58], [0.0, 0.0, 0.0, 0.7]));
        text_quads(&mut q, &m.round.message.text(), w / 2.0, h / 2.0, 6.0 * u, [1.0, 0.85, 0.2, 1.0]);
    }
    q
}

/// 5×7 pixel font for HUD messages (until the game's own HUD font is drawn).
const FONT: &[(char, [u8; 7])] = &[
    ('A', [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('B', [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110]),
    ('C', [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110]),
    ('D', [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110]),
    ('E', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111]),
    ('F', [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('G', [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111]),
    ('H', [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001]),
    ('I', [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('J', [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100]),
    ('K', [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001]),
    ('L', [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111]),
    ('M', [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001]),
    ('N', [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001]),
    ('O', [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('P', [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000]),
    ('Q', [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101]),
    ('R', [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001]),
    ('S', [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('T', [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('U', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110]),
    ('V', [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
    ('W', [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010]),
    ('X', [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001]),
    ('Y', [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100]),
    ('Z', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111]),
    ('0', [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110]),
    ('1', [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110]),
    ('2', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111]),
    ('3', [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110]),
    ('4', [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010]),
    ('5', [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110]),
    ('6', [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110]),
    ('7', [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000]),
    ('8', [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110]),
    ('9', [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100]),
    ('!', [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00000, 0b00100]),
    ('.', [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100]),
    ('?', [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b00000, 0b00100]),
];

/// Text as HUD quads, centred on (cx, cy), pixel size `px`.
fn text_quads(q: &mut Vec<([f32; 4], [f32; 4])>, s: &str, cx: f32, cy: f32, px: f32, col: [f32; 4]) {
    let n = s.chars().count() as f32;
    let x0 = cx - n * 6.0 * px / 2.0;
    let y0 = cy - 3.5 * px;
    for (k, ch) in s.chars().enumerate() {
        let Some((_, g)) = FONT.iter().find(|(c, _)| *c == ch) else { continue };
        for (r, bits) in g.iter().enumerate() {
            for c in 0..5 {
                if bits & (0x10 >> c) != 0 {
                    let x = x0 + (k as f32 * 6.0 + c as f32) * px;
                    let y = y0 + r as f32 * px;
                    q.push(([x, y, x + px, y + px], col));
                }
            }
        }
    }
}

/// Seven-segment digit made of quads.
fn seven_segment(q: &mut Vec<([f32; 4], [f32; 4])>, x: f32, y: f32, w: f32, h: f32, d: u8, c: [f32; 4]) {
    const SEG: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
    let t = (w * 0.2).max(2.0);
    let hh = h / 2.0;
    let bits = SEG[d as usize % 10];
    let rects = [
        [x, y, x + w, y + t],                           // a
        [x + w - t, y, x + w, y + hh],                  // b
        [x + w - t, y + hh, x + w, y + h],              // c
        [x, y + h - t, x + w, y + h],                   // d
        [x, y + hh, x + t, y + h],                      // e
        [x, y, x + t, y + hh],                          // f
        [x, y + hh - t / 2.0, x + w, y + hh + t / 2.0], // g
    ];
    for (k, r) in rects.iter().enumerate() {
        if bits >> k & 1 != 0 {
            q.push((*r, c));
        }
    }
}
