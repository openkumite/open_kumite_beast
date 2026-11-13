//! `Front`'s serialised form: its groups' fields flat, in the order of the state's format.
//!
//! Engine addition.
//! State: serialised: this file fixes `Front`'s field order (a change is a `STATE_VERSION` change)
//!
//! `Front`'s serialised form (engine addition): the fields of its `gfx`, `save` and `run` groups flat among the
//! others, in the order `Front` declared them before it had the groups. So the state's postcard bytes (runtime
//! snapshots, the state hash: `session::STATE_VERSION`) and its CBOR (`.okbs` files: one map of named fields) are
//! those of a `Front` without the groups. The list in `flat!` is that layout: a change to it is a change of the
//! state's format (`STATE_VERSION`; a renamed or retyped field also breaks older `.okbs` files).

use super::movie::MoviePlay;
use super::story_flow::StoryScreen;
use super::{Event, Front, Gfx, Mode, Pads, Resident, Run, Save, Screen};
use crate::frontend::options::Settings;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `Ref` (serialising, borrowed) and `Owned` (deserialising) from one field list, so the two cannot differ.
/// Attributes go to `Owned` only.
macro_rules! flat {
    ($($(#[$a:meta])* $f:ident: $t:ty,)*) => {
        #[derive(Serialize)]
        #[serde(rename = "Front")]
        struct Ref<'a> {
            $($f: &'a $t,)*
        }

        #[derive(Deserialize)]
        #[serde(rename = "Front")]
        struct Owned {
            $($(#[$a])* $f: $t,)*
        }
    };
}

flat! {
    vram: Resident,
    vram_gen: u32,
    screen: Screen,
    pads: Pads,
    frame: u32,
    menu_cursor: usize,
    mode: Mode,
    humans: [bool; 2],
    streak: [u16; 2],
    best: [u16; 2],
    champ: [bool; 2],
    dropped: [bool; 2],
    settings: Settings,
    dirty: bool,
    card: Option<crate::frontend::memcard::CardDialog>,
    memory_card: crate::frontend::options::Card,
    unlock_override: bool,
    /// a state file without it reads false (the game's own progression, as it ran)
    #[serde(default)]
    all_open: bool,
    emblem: usize,
    last: [u8; 2],
    stage: u8,
    stage_random: bool,
    ladder_o: [u8; 11],
    ladder_s: [u8; 11],
    a2: i32,
    continues: u32,
    rank: u16,
    shoulders: bool,
    human_costume: [u8; 2],
    rng: u32,
    title_visits: u8,
    play_time: u32,
    pending_entry: Option<crate::frontend::ranking::Entry>,
    rank_cam: crate::fight::camera::Camera,
    movie: Option<MoviePlay>,
    story: Option<crate::frontend::story_screens::Run>,
    story_screen: Option<StoryScreen>,
    pending_event: Option<Event>,
    file_cache: std::collections::BTreeSet<String>,
    ending_movie: usize,
    vram_saved: Option<Resident>,
    /// engine addition (the import S10): a state file without it reads 0
    #[serde(default)]
    br1_boss: u8,
    /// engine addition (the team battle, T4): a state file without them reads the defaults
    #[serde(default)]
    ext: Pads,
    #[serde(default)]
    team_port: u8,
    #[serde(default)]
    team_last: [u8; 4],
    #[serde(default)]
    team_stage: u8,
    #[serde(default)]
    team_result_row: u8,
    /// engine addition (TEAM ARCADE, simulation v26): a state file without it reads None
    #[serde(default)]
    team_ladder: Option<super::team_arcade::TeamLadder>,
}

impl Serialize for Front {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // every field named: a field added to `Front` or a group does not compile until it is placed here
        let Front {
            data: _,
            gfx: Gfx { vram, vram_gen, vram_saved },
            save: Save { settings, dirty, card, memory_card, unlock_override, all_open },
            run:
                Run {
                    mode,
                    humans,
                    streak,
                    best,
                    champ,
                    dropped,
                    ladder_o,
                    ladder_s,
                    a2,
                    continues,
                    rank,
                    shoulders,
                    human_costume,
                    play_time,
                    pending_entry,
                    ending_movie,
                    br1_boss,
                    team_ladder,
                },
            screen,
            pads,
            frame,
            menu_cursor,
            emblem,
            last,
            stage,
            stage_random,
            rng,
            title_visits,
            rank_cam,
            movie,
            story,
            story_screen,
            pending_event,
            file_cache,
            prefs: _,
            sim_version: _,
            browser: _,
            network: _,
            extras: _,
            android: _,
            br1: _,
            ext,
            team_port,
            team_last,
            team_stage,
            team_result_row,
            netplay: _,
        } = self;
        Ref {
            vram,
            vram_gen,
            screen,
            pads,
            frame,
            menu_cursor,
            mode,
            humans,
            streak,
            best,
            champ,
            dropped,
            settings,
            dirty,
            card,
            memory_card,
            unlock_override,
            all_open,
            emblem,
            last,
            stage,
            stage_random,
            ladder_o,
            ladder_s,
            a2,
            continues,
            rank,
            shoulders,
            human_costume,
            rng,
            title_visits,
            play_time,
            pending_entry,
            rank_cam,
            movie,
            story,
            story_screen,
            pending_event,
            file_cache,
            ending_movie,
            vram_saved,
            br1_boss,
            ext,
            team_port,
            team_last,
            team_stage,
            team_result_row,
            team_ladder,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Front {
    /// The static data, the preferences, the simulation version and the browser, network and Android flags are not state:
    /// `rebind` and `session::rebind` set them after a load.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let Owned {
            vram,
            vram_gen,
            screen,
            pads,
            frame,
            menu_cursor,
            mode,
            humans,
            streak,
            best,
            champ,
            dropped,
            settings,
            dirty,
            card,
            memory_card,
            unlock_override,
            all_open,
            emblem,
            last,
            stage,
            stage_random,
            ladder_o,
            ladder_s,
            a2,
            continues,
            rank,
            shoulders,
            human_costume,
            rng,
            title_visits,
            play_time,
            pending_entry,
            rank_cam,
            movie,
            story,
            story_screen,
            pending_event,
            file_cache,
            ending_movie,
            vram_saved,
            br1_boss,
            ext,
            team_port,
            team_last,
            team_stage,
            team_result_row,
            team_ladder,
        } = Owned::deserialize(d)?;
        Ok(Front {
            data: Default::default(),
            gfx: Gfx { vram, vram_gen, vram_saved },
            save: Save { settings, dirty, card, memory_card, unlock_override, all_open },
            run: Run {
                mode,
                humans,
                streak,
                best,
                champ,
                dropped,
                ladder_o,
                ladder_s,
                a2,
                continues,
                rank,
                shoulders,
                human_costume,
                play_time,
                pending_entry,
                ending_movie,
                br1_boss,
                team_ladder,
            },
            screen,
            pads,
            frame,
            menu_cursor,
            emblem,
            last,
            stage,
            stage_random,
            rng,
            title_visits,
            rank_cam,
            movie,
            story,
            story_screen,
            pending_event,
            file_cache,
            prefs: Default::default(),
            sim_version: crate::session::sim_version_now(),
            browser: crate::session::browser_build(),
            network: false,
            extras: 0,
            android: false,
            br1: false,
            ext,
            team_port,
            team_last,
            team_stage,
            team_result_row,
            netplay: false,
        })
    }
}
