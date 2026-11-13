//! CONTROLLER CONFIG.: each port edits its own player's buttons; the screen it draws.
//!
//! State: none of its own: updates the serialised `Menu`

use super::{Action, BACK, HL, Menu, cc_values};
use crate::assets::text;
use crate::fight::input::pad::{DOWN, LEFT, RIGHT, UP};
use crate::output::font::Fonts;
use crate::output::prim::Prim;

impl Menu {
    /// CONTROLLER CONFIG.: each port edits its own player: Up/Down = button, Left/Right = action (wrap),
    /// L1+R1 = that player's defaults, START (or △) = back to OPTIONS.
    pub(super) fn controller(&mut self, press: u16, port_press: [u16; 2], port_rep: [u16; 2], held: [u16; 2], sounds: &mut Vec<u16>) -> Action {
        if press & (0x800 | BACK) != 0 {
            sounds.push(if press & BACK != 0 { 0x0D } else { 0x0C });
            return Action::Leave;
        }
        for p in 0..2 {
            let r = port_rep[p];
            if r & DOWN != 0 {
                self.cc_row[p] = (self.cc_row[p] + 1) % 8;
                sounds.push(0x0A);
            } else if r & UP != 0 {
                self.cc_row[p] = (self.cc_row[p] + 7) % 8;
                sounds.push(0x0A);
            }
            let cur = &mut self.s.buttons[p][self.cc_row[p]];
            let mut k = cc_values().iter().position(|v| v == cur).unwrap_or(9) as u8;
            if Self::wrap(&mut k, 9, r & (LEFT | RIGHT), sounds) {
                *cur = cc_values()[k as usize];
            }
            if held[p] & 0xC == 0xC && port_press[p] & 0xC != 0 {
                self.s.buttons[p] = crate::assets::game_tables::default_buttons();
                sounds.push(0x0C);
            }
        }
        Action::None
    }

    pub(super) fn draw_controller(&self, f: &Fonts, out: &mut Vec<Prim>) {
        for p in 0..2 {
            let x = 48 + 256 * p as i32;
            f.menu_text(out, &text::fill(text::get("player_n"), &[&(p + 1)]), x, 96, HL);
            for row in 0..8 {
                let icon = text::item("cc_icons", row);
                let y = 128 + 24 * row as i32;
                f.menu_icons(out, icon, x, y, 8);
                let v = self.s.buttons[p][row];
                let name = text::item("cc_names", cc_values().iter().position(|c| *c == v).unwrap_or(9));
                f.menu_text(out, name, x + 64, y, HL);
                if row == self.cc_row[p] {
                    // the cursor brackets ("<          >") around the action
                    f.menu_text(out, "<", x + 48, y, HL);
                    f.menu_text(out, ">", x + 64 + f.menu_width(name) + 2, y, HL);
                }
            }
        }
    }
}
