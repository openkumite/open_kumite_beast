//! The debug F-keys (`debug_key`).
//!
//! Engine addition.
//!
//! The debug F-keys (`debug_key`): the overlay, the state's cheats, screenshots, the support report, pause and
//! speed, SCREEN MODE, the performance overlay.

use super::{App, perf};
use crate::frontend::options;
use crate::platform::support;
use crate::session::Cheat;
use winit::keyboard::KeyCode;

impl App {
    /// F-keys of the debug tools; true when the key was used.
    pub(super) fn debug_key(&mut self, k: KeyCode) -> bool {
        let toast = |s: &mut Self, t: String| {
            log::info!("debug: {t}");
            s.toast = Some((t, 180));
        };
        // netplay: the cheats, the app's pause and the speed would change one side only
        if self.net.is_some() && matches!(k, KeyCode::F2 | KeyCode::F3 | KeyCode::F4 | KeyCode::F6 | KeyCode::F9 | KeyCode::F10 | KeyCode::F11) {
            toast(self, "debug keys are off in netplay".into());
            return true;
        }
        match k {
            KeyCode::F1 => self.overlay = !self.overlay,
            // the state's cheats go into the next step
            KeyCode::F2 | KeyCode::F3 => {
                let i = if k == KeyCode::F2 { 1 } else { 0 };
                if self.state.fight.is_some() {
                    self.cheats.push(Cheat::OneHp(i));
                    toast(self, format!("P{} at 1 HP: the next hit is a KO", i + 1));
                }
            }
            KeyCode::F4 => {
                if self.state.fight.is_some() {
                    self.cheats.push(Cheat::Timer);
                    toast(self, "round timer set to 1 s".into());
                }
            }
            KeyCode::F6 => {
                if self.state.fight.is_some() {
                    self.cheats.push(Cheat::Gauges);
                    toast(self, "beast gauges full".into());
                }
            }
            KeyCode::F7 => {
                self.shot_req = true;
            }
            KeyCode::F8 => {
                let at = support::save_report(&self.state_text());
                toast(self, format!("report saved to {at}"));
            }
            KeyCode::F9 => self.toggle_pause(),
            KeyCode::F10 => {
                self.fast = !self.fast;
                let t = format!("speed x{}", if self.fast { 4 } else { 1 });
                toast(self, t);
            }
            // GRAPHICS → SCREEN MODE
            KeyCode::F5 => {
                use options::Aspect;
                let aspect = if self.prefs.aspect == Aspect::Wide { Aspect::Four3 } else { Aspect::Wide };
                self.apply_prefs(options::Prefs { aspect, ..self.prefs });
                toast(self, format!("aspect {}", if aspect == Aspect::Four3 { "4:3 (original)" } else { "WIDE" }));
            }
            // the performance overlay (GRAPHICS → PERF OVERLAY), its measurements restarted
            KeyCode::Backquote => {
                let on = !self.perf.on;
                if on != self.prefs.perf {
                    self.apply_prefs(options::Prefs { perf: on, ..self.prefs });
                } else {
                    // off again after OKB_PERF=1 (the saved preference already off)
                    self.perf = perf::Perf::default();
                    self.perf.on = on;
                }
            }
            KeyCode::F11 => {
                if self.state.front.is_some() {
                    self.cheats.push(Cheat::Unlock);
                    toast(self, "everything unlocked (movies and pictures too)".into());
                }
            }
            _ => return false,
        }
        true
    }
}
