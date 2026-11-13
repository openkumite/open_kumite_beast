//! The list menus: OPTIONS, SOUND OPTIONS, CUSTOM and their sub-screens.
//!
//! State: none of its own: updates the serialised `Front`

use super::super::Tim;
use super::super::movie::After;
use super::super::{Event, Front, Mode, Screen};
use crate::frontend::menus::{self, Menu};
use crate::output::sound::Queue;

impl Front {
    /// The next P00 beast picture for the menu emblem (loaded to (960,256)).
    pub(in crate::frontend) fn next_emblem(&mut self) {
        if let Some(Some(t)) = self.data.p00.get(self.emblem) {
            self.gfx.vram.load(Tim::P00(self.emblem as u8), t, None, Some((960, 256)));
            self.gfx.vram_gen = self.gfx.vram_gen.wrapping_add(1);
        }
        self.emblem = (self.emblem + 1) % 11;
    }

    /// Open a list menu: the next emblem, XA stopped (each list menu sends 0x203 once its frame is up, and again
    /// when it is left).
    pub(in crate::frontend) fn open_menu(&mut self, kind: menus::Kind, audio: &mut Queue) {
        self.next_emblem();
        audio.stop_music();
        let menu = if kind == menus::Kind::Options { self.options_menu() } else { Menu::new(kind, self.save.settings) };
        self.screen = Screen::Menu { menu, parent: None };
    }

    /// OPTIONS with the saved settings: from simulation v9 with the CHANGE GAME DATA row (engine addition).
    fn options_menu(&self) -> Menu {
        Menu::options(self.save.settings, self.gates().data_row)
    }

    pub(in crate::frontend) fn tick_menu(&mut self, mut menu: Menu, parent: Option<Menu>, audio: &mut Queue) -> Option<Event> {
        let mut sounds = Vec::new();
        if let Some(mut card) = self.save.card {
            // MEMORY CARD with a dialog up: the menu takes no input, its banner stops in the busy states
            menu.scroll(if card.busy() { 0 } else { 4 });
            let out =
                card.tick(self.pads.any_press(), self.pads.any_rep(), &mut self.save.settings, &mut self.save.dirty, &mut self.save.memory_card, &mut sounds);
            self.save.card = (out == crate::frontend::memcard::Out::Stay).then_some(card);
            if out == crate::frontend::memcard::Out::Loaded {
                self.apply_sound(audio);
            }
            // opt.22 is re-read every frame
            menu.s = self.save.settings;
            self.screen = Screen::Menu { menu, parent };
            for id in sounds {
                self.play(audio, id, 0);
            }
            return None;
        }
        let act = menu.tick(self.pads.any_press(), self.pads.any_rep(), self.pads.press, self.pads.rep, self.pads.held, &mut sounds);
        self.screen = Screen::Menu { menu, parent };
        for id in sounds {
            self.play(audio, id, 0);
        }
        match act {
            menus::Action::None => None,
            menus::Action::ApplySound | menus::Action::SoundDefault => {
                if act == menus::Action::SoundDefault {
                    audio.stop_music();
                }
                let s = menu.s;
                audio.set_options(s.master, s.bgm_volume, s.output != 0);
                None
            }
            menus::Action::PlayTrack(t) => Some(Event::Music(t.map(|t| (t, false)))), // once
            menus::Action::PlaySe(b, n) => {
                let (name, _, vols) = &self.data.banks[[0, 2, 1, 3][b]];
                audio.play_tone(&format!("fe:{name}"), n, vols.get(n).copied().unwrap_or(0x1FC0), 2);
                None
            }
            menus::Action::OpenMemCard => {
                // the OPTIONS values are committed first
                self.save.dirty |= self.save.settings != menu.s;
                self.save.settings = menu.s;
                self.next_emblem();
                audio.stop_music();
                self.screen = Screen::Menu { menu: Menu::new(menus::Kind::MemCard, self.save.settings), parent: Some(menu) };
                None
            }
            menus::Action::Card(item) => {
                self.save.settings.auto_save = menu.s.auto_save;
                self.save.card = Some(crate::frontend::memcard::CardDialog::menu(item));
                None
            }
            menus::Action::OpenController => {
                audio.stop_music();
                self.screen = Screen::Menu { menu: Menu::new(menus::Kind::Controller, menu.s), parent: Some(menu) };
                None
            }
            menus::Action::OpenPlayData => {
                audio.stop_music();
                self.screen = Screen::Menu { menu: Menu::new(menus::Kind::PlayData, menu.s), parent: Some(menu) };
                None
            }
            menus::Action::OpenMovies => {
                audio.stop_music();
                let mut m = Menu::new(menus::Kind::Movies, menu.s);
                if self.save.unlock_override {
                    m.s.movies = 0xF;
                    m.s.pictures = [!0, !0, 0x7FFF_FFFF];
                }
                self.screen = Screen::Menu { menu: m, parent: Some(menu) };
                None
            }
            menus::Action::PlayMovie(n) => Some(self.play_movie(n, After::Menu { menu, parent }, audio)),
            menus::Action::ShowPicture(n) => self.show_picture(n, menu, parent),
            // engine addition: OPTIONS → GRAPHICS, the app's preferences edited and applied at once
            menus::Action::OpenGraphics => {
                audio.stop_music();
                let mut m = Menu::new(menus::Kind::Graphics, menu.s);
                (m.g, m.browser, m.android) = (self.prefs, self.browser, self.android);
                self.screen = Screen::Menu { menu: m, parent: Some(menu) };
                None
            }
            // engine addition (v9): OPTIONS → CHANGE GAME DATA, the app's setup screen over the menu
            menus::Action::ChangeData => Some(Event::ChangeData),
            menus::Action::ApplyGraphics => {
                self.prefs = menu.g;
                Some(Event::Prefs(menu.g))
            }
            menus::Action::OpenSound => {
                audio.stop_music();
                self.screen = Screen::Menu { menu: Menu::new(menus::Kind::Sound, menu.s), parent: Some(menu) };
                None
            }
            menus::Action::Leave => {
                match menu.kind {
                    menus::Kind::Sound => {
                        // back to OPTIONS with the sound values, the cursor on SOUND OPTIONS
                        let mut p = parent.unwrap_or_else(|| self.options_menu());
                        (p.s.output, p.s.bgm_select, p.s.master, p.s.bgm_volume) = (menu.s.output, menu.s.bgm_select, menu.s.master, menu.s.bgm_volume);
                        p.cursor = 9;
                        audio.stop_music();
                        self.screen = Screen::Menu { menu: p, parent: None };
                        return None;
                    }
                    menus::Kind::Controller => {
                        let mut p = parent.unwrap_or_else(|| self.options_menu());
                        p.s.buttons = menu.s.buttons;
                        p.cursor = 2;
                        self.screen = Screen::Menu { menu: p, parent: None };
                        return None;
                    }
                    menus::Kind::MemCard => {
                        // back to OPTIONS on MEMORY CARD with its values re-read (a LOAD applies at once)
                        self.save.settings.auto_save = menu.s.auto_save;
                        let mut p = parent.unwrap_or_else(|| self.options_menu());
                        p.s = self.save.settings;
                        p.cursor = 1;
                        audio.stop_music();
                        self.apply_sound(audio);
                        self.screen = Screen::Menu { menu: p, parent: None };
                        return None;
                    }
                    menus::Kind::Movies | menus::Kind::PlayData | menus::Kind::Graphics => {
                        let mut p = parent.unwrap_or_else(|| self.options_menu());
                        p.cursor = p.options_row(match menu.kind {
                            menus::Kind::Graphics => 10,
                            menus::Kind::Movies => 11,
                            _ => 12,
                        });
                        audio.stop_music();
                        self.screen = Screen::Menu { menu: p, parent: None };
                        return None;
                    }
                    menus::Kind::Options => {
                        self.save.dirty |= self.save.settings != menu.s;
                        self.save.settings = menu.s;
                    }
                    menus::Kind::Custom => {
                        self.save.dirty |= self.save.settings.custom != menu.s.custom;
                        self.save.settings.custom = menu.s.custom;
                    }
                }
                self.save.settings.save(&mut self.save.memory_card);
                self.apply_sound(audio);
                // back to the title's own menu: not a mode 5 entry, no auto-save
                self.enter_title(audio);
                None
            }
            menus::Action::GameStart(port) => {
                self.save.dirty |= self.save.settings.custom != menu.s.custom;
                self.save.settings.custom = menu.s.custom;
                self.save.settings.save(&mut self.save.memory_card);
                self.start_mode(Mode::Custom(menu.s.custom.game.min(3)), port, audio);
                None
            }
        }
    }
}
