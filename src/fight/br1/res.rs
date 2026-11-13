//! An imported fighter's static data: its ANM, CMN.ACT, the game's static tables and both forms' skeletons.
//!
//! State: static data (not serialised; stage S7 re-attaches it by `rebind`)
//!
//! Read from the imported game's install (`game_data/br1`, `assets::manifest::BR1_DIR`): `engine/XX/XX31.DAT` (the
//! file the game loads; `XX.ANM` differs from it in 2 bytes), `engine/CMN/CMN.ACT` and the static tables
//! (`Assets::br1_tables`: `engine/tables.okt`, the program image's bytes as the converter cut them out; without it, an
//! error asking for a new conversion). The session's content id covers the files all of them come from (the netplay
//! rule).

use super::pose::{self, Bank, Banks, Skeleton};
use crate::assets::Assets;
use crate::fight::physics::Trig;
use crate::formats::br1_anm::{Anm, Clips};
use crate::formats::tables::{BR1_CLIPS, Tables};
use std::sync::Arc;

/// The imported game's static tables as the fighters of a match share them (loaded once, `Br1Tables::load`): the tables,
/// the program image's clips (`Src::Image`) placed at their file offsets and the program image's length.
#[derive(Clone)]
pub struct Br1Tables {
    pub tables: Arc<Tables>,
    /// `tables::BR1_CLIPS` placed at its origin, zeros before it
    pub image_clips: Arc<[u8]>,
    /// the program image's length (`image_len`): an image clip's offset is below it
    pub image_len: usize,
}

impl Br1Tables {
    /// Err when the clips or `image_len` lie outside the program image or past the console's 2 MB of RAM.
    pub fn new(tables: Tables) -> Result<Self, String> {
        let (clips, image_len) = (tables.table(BR1_CLIPS), tables.table("image_len").u32(0) as usize);
        // (no wrap of a 32-bit usize)
        let end = (clips.origin() as usize).checked_add(clips.bytes().len());
        if image_len > 0x20_0000 || end.is_none_or(|e| e > image_len) {
            return Err(format!("tables.okt: {BR1_CLIPS} or image_len out of range (convert the game data again)"));
        }
        let mut image_clips = vec![0; clips.origin() as usize];
        image_clips.extend_from_slice(clips.bytes());
        Ok(Self { tables: Arc::new(tables), image_clips: image_clips.into(), image_len })
    }

    /// The tables of the imported game's install `br1` (`Assets::br1_tables`).
    pub async fn load(br1: &Assets) -> Result<Self, String> {
        Self::new(br1.br1_tables().await?)
    }
}

pub struct Br1Res {
    /// the fighter's code (`assets::manifest::BR1_FIGHTERS`)
    pub code: String,
    pub anm: Anm,
    /// CMN.ACT's bytes and its clip block
    pub cmn: Vec<u8>,
    pub cmn_clips: Clips,
    /// The imported game's static tables, the program image's clips and its length (`Br1Tables`, shared)
    pub tables: Arc<Tables>,
    pub image_clips: Arc<[u8]>,
    pub image_len: usize,
    pub trig: Trig,
    /// the human and the beast form's skeleton
    pub skels: [Skeleton; 2],
    /// per form, the nodes the world walk and the model shift reach (bit n; `linked_nodes`)
    pub linked: [u32; 2],
}

impl Br1Res {
    /// From the files' bytes and the tables; Err when a file does not parse.
    pub fn new(code: &str, anm: Vec<u8>, cmn: Vec<u8>, shared: &Br1Tables) -> Result<Self, String> {
        let anm = Anm::parse(anm).map_err(|e| format!("{code}31.DAT: {e}"))?;
        let cmn_clips = Clips::parse(&cmn, 0).map_err(|e| format!("CMN.ACT: {e}"))?;
        let trig = Trig::from_tables(&shared.tables);
        let skels = [Skeleton::new(&anm, 0, &trig), Skeleton::new(&anm, 1, &trig)];
        let linked = [0, 1].map(|form| linked_nodes(code, form, skels[form].nodes()));
        let (tables, image_clips, image_len) = (shared.tables.clone(), shared.image_clips.clone(), shared.image_len);
        Ok(Self { code: code.into(), anm, cmn, cmn_clips, tables, image_clips, image_len, trig, skels, linked })
    }

    /// Fighter `code` from the imported game's install `br1` (an `Assets` on its folder) with its tables `shared`.
    pub async fn load(br1: &Assets, code: &str, shared: &Br1Tables) -> Result<Self, String> {
        let anm = br1.load(&format!("engine/{code}/{code}31.DAT")).await?;
        let cmn = br1.load("engine/CMN/CMN.ACT").await?;
        Self::new(code, anm, cmn, shared)
    }

    /// The clip blocks of this fighter: its own, CMN.ACT, the opponent's (an imported opponent's ANM) and the static tables' image.
    pub fn banks<'a>(&'a self, opp: Option<&'a Anm>) -> Banks<'a> {
        Banks {
            own: Bank::of(&self.anm),
            cmn: Bank { bytes: &self.cmn, clips: &self.cmn_clips },
            opp: opp.map(Bank::of),
            image: &self.image_clips,
            maps: &self.anm.bone_maps,
        }
    }
}

/// The nodes of form `form` (of `nodes`) that the MD child (+0x44) / sibling (+0x48) links reach from node 0 in a
/// fight: the form setup links every node to its parent, the spring chains' setup unlinks
/// their nodes again (the chains place them): MIT beast 19..22, FOX 19..21 / 19..22, GRE 19, URI human 19..25.
/// URI's beast's extra nodes 19..23 stay linked. The four fighters' sets as the
/// recorded runs' links show them; `reference_frame::linked` reads those links and the frame runs check this table
/// against them on every record. Only MIT, FOX, GRE and URI are verified; the other codes (ZIN, EVE,
/// VAK, GAD, RON) take the default nodes 0..18 untested until the frame oracle covers them.
pub fn linked_nodes(code: &str, form: usize, nodes: usize) -> u32 {
    let walked = if code == "URI" && form == 1 { 24 } else { pose::BASE_NODES };
    let n = walked.min(nodes).min(32);
    if n == 32 { u32::MAX } else { (1u32 << n) - 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::tables::Elem;

    /// The program image's clips go to their file offset; clips past `image_len` (also by a wrapping offset, a 32-bit
    /// `usize`'s) or an `image_len` past 2 MB are refused.
    #[test]
    fn image_clips_at_their_offset() {
        let new = |origin: u32, len: usize, image_len: u32| {
            let tables = Tables::build([
                (BR1_CLIPS.into(), Elem::Blob, len as u32, origin, vec![5; len]),
                ("image_len".into(), Elem::U32, 1, 0, image_len.to_le_bytes().to_vec()),
            ])
            .unwrap();
            let shared = Br1Tables::new(tables)?;
            Br1Res::new("MIT", crate::formats::br1_anm::tests::synthetic(), [12u8, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0].to_vec(), &shared)
        };
        let r = new(0x800, 8, 0x1000).unwrap();
        assert_eq!((r.image_clips.len(), r.image_clips[0x7FF], r.image_clips[0x800], r.image_len), (0x808, 0, 5, 0x1000));
        assert!(new(0xFF8, 8, 0x1000).is_ok() && new(0xFF9, 8, 0x1000).is_err());
        assert!(new(0xFFFF_FFF0, 0x20, 0x1000).is_err());
        assert!(new(0, 8, 0x20_0001).is_err());
    }
}
