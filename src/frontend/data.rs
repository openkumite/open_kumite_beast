//! CMN files and sound banks of the front end, loaded at boot.
//!
//! State: static data (`FrontData`: not serialised; re-attached by `rebind`)

use crate::assets::Assets;
use crate::formats::music::Music;
use std::sync::Arc;

/// CMN files and sound banks of the front end.
#[derive(Default)]
pub struct FrontData {
    pub(super) init: Vec<Option<Vec<u8>>>,
    pub(super) title: Vec<Option<Vec<u8>>>,
    pub(super) se00: Vec<u8>,
    pub(super) p00: Vec<Option<Vec<u8>>>,
    /// STORY / SURVIVAL select pictures (file 0x15): beasts 0..10, humans 11..21, 22 the Stun human used
    pub(super) p01: Vec<Option<Vec<u8>>>,
    /// VS screen pages (file 0x81): entries 0x30..0x35
    pub(super) sel00: Vec<Option<Vec<u8>>>,
    /// NORMAL select portraits (file 0x82): entries 0x10 + c, 8-bit 192×256 with their CLUT
    pub(super) sel01: Vec<Option<Vec<u8>>>,
    pub(super) sb: Vec<Vec<u8>>,
    /// (bank name, bytes, volumes): COMMON_2, CHASEL_1, TITLE, CHASEL_2, ENDING
    pub(crate) banks: Vec<(String, Vec<u8>, Vec<u16>)>,
    pub(super) select_music: Option<Arc<Music>>,
    /// track 1: the select music of BGM SELECT = ARCADE
    pub(super) select_music_arcade: Option<Arc<Music>>,
    /// boot logos: entry 0 of /CMN/SCEA.DAT (file 0x1D) and /CMN/HUD.DAT (file 0x1F, the HUDSON logo)
    pub(super) scea: Option<Vec<u8>>,
    pub(super) hudson: Option<Vec<u8>>,
    /// engine addition (the import S9): the imported game's portraits for the select's imported row and the VS
    /// screen (`assets::br1_art`; blank without the imported game's install)
    pub(super) br1: crate::assets::br1_art::Br1Art,
}

impl FrontData {
    /// XA track `track` when it is one of the front end's own (the select music: 0x1F, and 1 for BGM SELECT =
    /// ARCADE), for the debug tools' music re-sync.
    pub fn music(&self, track: u8) -> Option<Arc<Music>> {
        match track {
            0x1F => self.select_music.clone(),
            1 => self.select_music_arcade.clone(),
            _ => None,
        }
    }

    /// A VRAM with only the small font and its palettes (INIT.DAT entries 0 and 2, where `Front::boot_vram` puts
    /// them): the app's netplay connect screen, drawn before the session (and its front end) exists.
    pub fn font_vram(&self) -> crate::formats::vram::Vram {
        let mut v = crate::formats::vram::Vram::default();
        for (k, at) in [(0, (960, 0)), (2, (496, 480))] {
            if let Some(Some(t)) = self.init.get(k) {
                v.load_tim(t, None, Some(at));
            }
        }
        v
    }
}

pub async fn load(assets: &Assets) -> Result<FrontData, String> {
    let cmn = |n: &str| format!("engine/cmn/{n}");
    let init = crate::formats::container::read(&assets.load(&cmn("INIT.DAT")).await?);
    let title = crate::formats::container::read(&assets.load(&cmn("USATITLE.DAT")).await?);
    let se00 = assets.load(&cmn("SE00.DAT")).await?;
    let p00 = crate::formats::container::read(&assets.load(&cmn("P00.DAT")).await?);
    let p01 = crate::formats::container::read(&assets.load(&cmn("P01.DAT")).await.unwrap_or_default());
    let sel00 = crate::formats::container::read(&assets.load(&cmn("SEL00.DAT")).await.unwrap_or_default());
    let sel01 = crate::formats::container::read(&assets.load(&cmn("SEL01.DAT")).await.unwrap_or_default());
    let mut sb = Vec::new();
    for k in 0..12 {
        sb.push(assets.load(&cmn(&format!("SB{k:02X}.DAT"))).await.unwrap_or_default());
    }
    // the tables (and with them the text, the fonts and the front end's number tables): none, the setup screen
    let tables = assets.br2_tables().await?;
    let index = assets.load_json("engine/index.json").await?;
    let mut banks = Vec::new();
    for name in ["COMMON_2.SND", "CHASEL_1.SND", "TITLE.SND", "CHASEL_2.SND", "ENDING.SND"] {
        let bytes = assets.load(&format!("engine/snd/{name}")).await.unwrap_or_default();
        let mut vols: Vec<u16> =
            index["sound_defs"][name].as_array().map(|a| a.iter().map(|d| d[3].as_u64().unwrap_or(0x1FC0) as u16).collect()).unwrap_or_default();
        if vols.is_empty() && name == "ENDING.SND" {
            // sound-definition table 0x12 is not in the index: `sound.menu_volumes` (6 records of 6 bytes, volume at +4)
            let v = tables.table("sound.menu_volumes");
            vols = (0..6).map(|k| v.u16(6 * k + 4)).collect();
        }
        banks.push((name.to_string(), bytes, vols));
    }
    // character select music: track 0x1F (BGM SELECT = CONSOLE, opt.D = 0) → XA MC04 channel 7
    let music = async |track| assets.load_music(&crate::formats::music::track_file(track)).await.ok().and_then(crate::formats::music::decode).map(Arc::new);
    let select_music = music(0x1F).await;
    let select_music_arcade = music(1).await;
    let logo = |b: Vec<u8>| crate::formats::container::read(&b).into_iter().next().flatten();
    let scea = logo(assets.load(&cmn("SCEA.DAT")).await.unwrap_or_default());
    let hudson = logo(assets.load(&cmn("HUD.DAT")).await.unwrap_or_default());
    // the imported select art (engine addition, the import S9); the browser's only from the install its start
    // found usable (data over HTTP has none: `manifest::br1_install_id`)
    #[cfg(target_arch = "wasm32")]
    let br1_dir = assets.br1().filter(|_| assets.br1_opfs_id().is_some());
    #[cfg(not(target_arch = "wasm32"))]
    let br1_dir = assets.br1();
    let br1 = crate::assets::br1_art::load(br1_dir.as_ref()).await;
    // its text (its names on the select), when it has tables (none: no install of the imported game)
    if let Some(t) = match &br1_dir {
        Some(d) => d.tables().await,
        None => None,
    } {
        crate::assets::text::install(crate::assets::text::Game::Br1, &t).inspect_err(|e| log::warn!("{e}")).ok();
    }
    Ok(FrontData { init, title, se00, p00, p01, sel00, sel01, sb, banks, select_music, select_music_arcade, scea, hudson, br1 })
}
