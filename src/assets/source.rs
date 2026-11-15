//! Asset access: files of the converted export, read from disk on desktop and Android, and in the browser from the
//! game data installed in its private storage (OPFS, `platform::game_data::web`) or fetched over HTTP (`?assets=URL`).

/// Engine addition (the import S10): the prefix of a fetched file in the imported game's install (`Assets::route`): its
/// ending movies and pictures.
pub const BR1_PREFIX: &str = "br1/";

/// The static tables' file in a converted export (`Assets::tables`; the imported game's install has its own).
pub const TABLES: &str = "engine/tables.okt";

/// The continue screens' pictures as the logic asks for them (the disc's CON folder) and where a converter from
/// version 0.1.1 writes them: "con" is a reserved device name on Windows (Windows 10 refuses a folder of that name,
/// os error 267). `Assets::load` reads the new folder first and the old one for data converted before.
const CONTINUE_ASKED: &str = "engine/con/";
const CONTINUE_DIR: &str = "engine/continue/";

#[derive(Clone)]
pub struct Assets {
    #[cfg(not(target_arch = "wasm32"))]
    root: std::path::PathBuf,
    #[cfg(target_arch = "wasm32")]
    base: String,
    /// the browser's installed game data: its OPFS folder (`data-<n>`); None: over HTTP from `base`
    #[cfg(target_arch = "wasm32")]
    opfs: Option<String>,
    /// the browser's install of the imported game beside the installed game data: its OPFS folder (`br1-<n>`) and content id
    /// (`set_br1_opfs`; engine addition, the import S9)
    #[cfg(target_arch = "wasm32")]
    br1_opfs: Option<(String, u32)>,
    /// the browser's Quake III install beside the installed game data: its OPFS folder (`q3-<n>`) and the extra stages
    /// it has (`set_q3_opfs`; engine addition, `assets::extras`)
    #[cfg(target_arch = "wasm32")]
    q3_opfs: Option<(String, u8)>,
}

impl Assets {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new(base: impl Into<String>) -> Self {
        Self { base: base.into(), opfs: None, br1_opfs: None, q3_opfs: None }
    }

    /// The folder read from.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The browser's installed game data in its OPFS folder `dir`.
    #[cfg(target_arch = "wasm32")]
    pub fn opfs(dir: impl Into<String>) -> Self {
        Self { base: String::new(), opfs: Some(dir.into()), br1_opfs: None, q3_opfs: None }
    }

    /// The OPFS folder read from (None: HTTP).
    #[cfg(target_arch = "wasm32")]
    pub fn opfs_dir(&self) -> Option<&str> {
        self.opfs.as_deref()
    }

    /// The browser's install of the imported game found at the start (`assets::manifest::opfs_br1_install`: its OPFS folder and
    /// content id), for `br1` and `assets::manifest::br1_install_id` (engine addition, the import S9).
    #[cfg(target_arch = "wasm32")]
    pub fn set_br1_opfs(&mut self, br1: Option<(String, u32)>) {
        self.br1_opfs = br1;
    }

    /// The content id of `set_br1_opfs`'s install (None without one, and for data over HTTP).
    #[cfg(target_arch = "wasm32")]
    pub fn br1_opfs_id(&self) -> Option<u32> {
        self.opfs.as_ref().and(self.br1_opfs.as_ref()).map(|(_, id)| *id)
    }

    /// The imported game's install beside this game data (engine addition, the import: its folder
    /// `platform::game_data::BR1_DIR`); in the browser's installed data the folder of its own slot
    /// (`platform::game_data::web::BR1`) that the start found usable (`set_br1_opfs`), else None.
    /// Debug (desktop): OKB_BR1=dir names the install instead (e.g. `../okb_export/br1` beside `--assets ../okb_export`).
    pub fn br1(&self) -> Option<Assets> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(d) = crate::base::env::var_os("BR1").filter(|d| !d.is_empty()) {
            return Some(Self { root: d.into() });
        }
        #[cfg(not(target_arch = "wasm32"))]
        return Some(Self { root: self.root.join(crate::platform::game_data::BR1_DIR) });
        #[cfg(target_arch = "wasm32")]
        return match &self.opfs {
            None => Some(Self::new(format!("{}{}/", self.base, crate::platform::game_data::BR1_DIR))),
            Some(_) => self.br1_opfs.as_ref().map(|(d, _)| Self::opfs(d.clone())),
        };
    }

    /// The Quake III install beside this game data (engine addition, the extra stages: its folder
    /// `platform::game_data::Q3_DIR`, checked by `assets::extras`); in the browser's installed data the folder of its
    /// own slot (`platform::game_data::web::Q3`) that the start found usable (`set_q3_opfs`), else None (also over
    /// HTTP). Debug (desktop): OKB_Q3=dir names the install instead.
    pub fn q3(&self) -> Option<Assets> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(d) = crate::base::env::var_os("Q3").filter(|d| !d.is_empty()) {
            return Some(Self { root: d.into() });
        }
        #[cfg(not(target_arch = "wasm32"))]
        return Some(Self { root: self.root.join(crate::platform::game_data::Q3_DIR) });
        #[cfg(target_arch = "wasm32")]
        return self.opfs.as_ref().and(self.q3_opfs.as_ref()).map(|(d, _)| Self::opfs(d.clone()));
    }

    /// The browser's Quake III install found at the start (`assets::extras::opfs_q3_install`: its OPFS folder and the
    /// extra stages it has), for `q3` and `assets::extras::present` (engine addition).
    #[cfg(target_arch = "wasm32")]
    pub fn set_q3_opfs(&mut self, q3: Option<(String, u8)>) {
        self.q3_opfs = q3;
    }

    /// The extra stages of `set_q3_opfs`'s install (bit k = `assets::extras::STAGES[k]`; 0 without one, and for data
    /// over HTTP).
    #[cfg(target_arch = "wasm32")]
    pub fn q3_mask(&self) -> u8 {
        self.opfs.as_ref().and(self.q3_opfs.as_ref()).map_or(0, |(_, m)| *m)
    }

    /// Data over HTTP: the folder `dir` under it (an extra stage's older `extras/<map>/`); None for installed data.
    #[cfg(target_arch = "wasm32")]
    pub fn legacy_http(&self, dir: &str) -> Option<Assets> {
        self.opfs.is_none().then(|| Self::new(format!("{}{dir}/", self.base)))
    }

    /// A file of the data by its name in the export; a continue picture from `CONTINUE_DIR`, else (data converted
    /// before converter 0.1.1) from the folder the logic names.
    pub async fn load(&self, rel: &str) -> Result<Vec<u8>, String> {
        let Some(name) = rel.strip_prefix(CONTINUE_ASKED) else { return self.load_file(rel).await };
        match self.load_file(&format!("{CONTINUE_DIR}{name}")).await {
            Ok(b) => Ok(b),
            // neither: the new folder's error first (data from converter 0.1.1 on has no old folder)
            Err(new) => self.load_file(rel).await.map_err(|old| format!("{new}; {old}")),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn load_file(&self, rel: &str) -> Result<Vec<u8>, String> {
        std::fs::read(self.root.join(rel)).map_err(|e| format!("{}: {e}", self.root.join(rel).display()))
    }

    #[cfg(target_arch = "wasm32")]
    async fn load_file(&self, rel: &str) -> Result<Vec<u8>, String> {
        use wasm_bindgen::JsCast;
        if let Some(dir) = &self.opfs {
            return crate::platform::game_data::web::read(dir, rel).await;
        }
        use wasm_bindgen_futures::JsFuture;
        let url = format!("{}{}", self.base, rel);
        let window = web_sys::window().ok_or("no window")?;
        let resp = JsFuture::from(window.fetch_with_str(&url)).await.map_err(|e| format!("{url}: {e:?}"))?;
        let resp: web_sys::Response = resp.dyn_into().map_err(|_| "not a Response")?;
        if !resp.ok() {
            return Err(format!("{url}: HTTP {}", resp.status()));
        }
        let buf = JsFuture::from(resp.array_buffer().map_err(|e| format!("{e:?}"))?).await.map_err(|e| format!("{e:?}"))?;
        Ok(js_sys::Uint8Array::new(&buf).to_vec())
    }

    /// A music file by its MP3 name (`formats::music::track_file`, a movie's `video/<NAME>.mp3`): the CD-XA `.xa`
    /// beside it first (exports that keep the disc's audio; `formats::music::decode` tells them apart), else the MP3
    /// of older exports. A `.xa` whose header or length is wrong (a damaged file, a web host's HTML page for a
    /// missing one) counts as missing. Other names as `load`.
    /// Engine addition (the import S10): the data a fetched file `rel` is read from and its name there: this data, or
    /// for a name with `BR1_PREFIX` the imported game's install (`br1`) without the prefix; None without that install.
    pub fn route<'a>(&self, rel: &'a str) -> Option<(Assets, &'a str)> {
        match rel.strip_prefix(BR1_PREFIX) {
            Some(r) => self.br1().map(|a| (a, r)),
            None => Some((self.clone(), rel)),
        }
    }

    pub async fn load_music(&self, rel: &str) -> Result<Vec<u8>, String> {
        if let Some(stem) = rel.strip_suffix(".mp3") {
            let xa = format!("{stem}.xa");
            if let Ok(b) = self.load(&xa).await {
                match crate::formats::xa::parse(&b) {
                    Ok(_) => return Ok(b),
                    Err(e) => log::warn!("{xa}: {e}: the MP3 instead"),
                }
            }
        }
        self.load(rel).await
    }

    /// A fetched file (`session::Request::Fetch`'s name): `load_music` from the data `route` gives (engine addition, the
    /// the import S10).
    pub async fn fetch(&self, rel: &str) -> Result<Vec<u8>, String> {
        let (a, r) = self.route(rel).ok_or_else(|| format!("{rel}: no data of the imported game"))?;
        a.load_music(r).await
    }

    /// The game's static tables (`engine/tables.okt`, `formats::tables`): None when the data has none (converted before
    /// they were written; also a file without the table magic, as a web host's page for a missing file is) or the file
    /// does not parse or lacks a table every game has (`tables::REQUIRED`; logged).
    pub async fn tables(&self) -> Option<crate::formats::tables::Tables> {
        use crate::formats::tables::{MAGICS, REQUIRED, Tables};
        let b = self.load(TABLES).await.ok()?;
        if !MAGICS.iter().any(|m| b.starts_with(&m[..])) {
            return None;
        }
        Tables::parse(&b).and_then(|t| t.require(REQUIRED).map(|_| t)).inspect_err(|e| log::warn!("{e}")).ok()
    }

    /// The main game's static tables: `engine/tables.okt` when it holds every table of `tables::BR2` in its shape and
    /// the game's text, which it installs with the fonts and the number tables (`text::install`, `output::font::install`,
    /// `game_tables::install`); Err asking for a new conversion otherwise (data converted
    /// before it was written, or a partial file).
    pub async fn br2_tables(&self) -> Result<crate::formats::tables::Tables, String> {
        let t = self.tables().await.ok_or("no engine/tables.okt (convert the game data again)")?;
        t.require_layout(&crate::formats::tables::BR2)?;
        super::text::install(super::text::Game::Br2, &t)?;
        crate::output::font::install(t.get(crate::output::font::METRICS)?.bytes());
        super::game_tables::install(&t);
        Ok(t)
    }

    /// The imported game's static tables (this `Assets` on its install): `engine/tables.okt` when it holds the imported game's
    /// tables (`Tables::require_br1`: every table of `tables::BR1`, the clips and their ranges) and its text, which it
    /// installs; Err asking for a new conversion otherwise.
    pub async fn br1_tables(&self) -> Result<crate::formats::tables::Tables, String> {
        let t = self.tables().await.ok_or("no engine/tables.okt (convert the game data again)")?;
        t.require_br1()?;
        super::text::install(super::text::Game::Br1, &t)?;
        Ok(t)
    }

    pub async fn load_json(&self, rel: &str) -> Result<serde_json::Value, String> {
        let b = self.load(rel).await?;
        serde_json::from_slice(&b).map_err(|e| format!("{rel}: {e}"))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::Assets;

    /// `load_music`: the `.xa` beside the `.mp3` name first, else the MP3; other names as `load`.
    #[test]
    fn music_xa_first_then_mp3() {
        let dir = std::env::temp_dir().join(format!("okb_load_music_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("m")).unwrap();
        let a = Assets::new(&dir);
        let load = |rel: &str| pollster::block_on(a.load_music(rel));
        std::fs::write(dir.join("m/t.mp3"), b"mp3").unwrap();
        assert_eq!(load("m/t.mp3").unwrap(), b"mp3");
        // not a `.xa` file (an HTML page, a damaged one): the MP3
        std::fs::write(dir.join("m/t.xa"), b"<html>").unwrap();
        assert_eq!(load("m/t.mp3").unwrap(), b"mp3");
        let mut xa = crate::formats::xa::MAGIC.to_vec();
        xa.extend([1, 0x01, 1, 0, 0, 0, 0, 0, 0, 0]);
        xa.resize(16 + 2304, 0);
        std::fs::write(dir.join("m/t.xa"), &xa[..100]).unwrap();
        assert_eq!(load("m/t.mp3").unwrap(), b"mp3");
        std::fs::write(dir.join("m/t.xa"), &xa).unwrap();
        assert_eq!(load("m/t.mp3").unwrap(), xa);
        std::fs::remove_file(dir.join("m/t.mp3")).unwrap();
        assert_eq!(load("m/t.mp3").unwrap(), xa);
        std::fs::write(dir.join("m/t.xa"), b"<html>").unwrap();
        assert!(load("m/t.mp3").is_err());
        assert!(load("m/u.mp3").is_err());
        // not a music name: no `.xa` tried
        std::fs::write(dir.join("m/t.bsf"), b"bsf").unwrap();
        assert_eq!(load("m/t.bsf").unwrap(), b"bsf");
        assert!(load("m/t").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `Assets::tables`: a missing file, or another file in its place (a web host's page for a missing one), is no
    /// tables; a table file is read, with either magic; a damaged one is none too (logged).
    #[test]
    fn tables_file() {
        let dir = std::env::temp_dir().join(format!("okb_tables_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("engine")).unwrap();
        let a = Assets::new(&dir);
        let get = || pollster::block_on(a.tables());
        assert!(get().is_none());
        std::fs::write(dir.join(super::TABLES), b"<!doctype html><html></html>").unwrap();
        assert!(get().is_none());
        // the two tables every file has, empty
        let mut okt = crate::formats::tables::MAGIC.to_vec();
        okt.extend(crate::formats::tables::VERSION.to_le_bytes());
        okt.extend(2u32.to_le_bytes());
        for name in ["math.rsqrt", "math.sqrt"] {
            okt.push(name.len() as u8);
            okt.extend(name.as_bytes());
            okt.extend([2, 0, 0]);
            okt.extend([0; 12]);
            while !okt.len().is_multiple_of(4) {
                okt.push(0);
            }
        }
        std::fs::write(dir.join(super::TABLES), &okt).unwrap();
        assert!(get().is_some_and(|t| t.names().count() == 2));
        // the other accepted magics (the imported game's converter, older conversions)
        for magic in [b"BR1TABLE", b"OKBTABLE"] {
            let mut other = okt.clone();
            other[..8].copy_from_slice(magic);
            std::fs::write(dir.join(super::TABLES), &other).unwrap();
            assert!(get().is_some_and(|t| t.names().count() == 2));
        }
        // `br2_tables`: a file without the main game's other tables is not used; one with all of them is
        let br2 = || pollster::block_on(a.br2_tables());
        assert!(br2().unwrap_err().contains("convert the game data again"));
        std::fs::write(dir.join(super::TABLES), crate::formats::tables::of_layout(&crate::formats::tables::BR2).to_okt()).unwrap();
        assert!(br2().unwrap().require_layout(&crate::formats::tables::BR2).is_ok());
        // cut inside the last table's header
        std::fs::write(dir.join(super::TABLES), &okt[..okt.len() - 10]).unwrap();
        assert!(get().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `br1_tables`: a `tables.okt` with the imported game's tables, the clips as a blob and their ranges is used; Err asking
    /// for a new conversion without the file, for a partial one, one without the ranges (format 2) or clips that are
    /// not a blob.
    #[test]
    fn br1_tables_file() {
        use crate::formats::tables::{BR1, BR1_CLIP_RANGES, BR1_CLIPS, Elem, Tables, of_layout};
        let dir = std::env::temp_dir().join(format!("okb_br1_tables_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("engine")).unwrap();
        let a = Assets::new(&dir);
        let br1 = || pollster::block_on(a.br1_tables());
        assert!(br1().unwrap_err().contains("convert the game data again"));
        // the layout without `skip`, the clips (8 bytes at file offset 0x800, of `clips`) and, with `ranges`, their range
        let write = |skip: &str, clips: Elem, ranges: bool| {
            let base = of_layout(&BR1);
            let mut parts: Vec<(String, Elem, u32, u32, Vec<u8>)> = base
                .names()
                .filter(|n| *n != skip)
                .map(|n| {
                    let t = base.get(n).unwrap();
                    (n.to_string(), t.elem(), t.count(), t.origin(), t.bytes().to_vec())
                })
                .collect();
            parts.push((BR1_CLIPS.into(), clips, 8, 0x800, vec![5; 8]));
            if ranges {
                parts.push((BR1_CLIP_RANGES.into(), Elem::U32, 2, 0, [0x800u32, 0x808].iter().flat_map(|v| v.to_le_bytes()).collect()));
            }
            std::fs::write(dir.join(super::TABLES), Tables::build(parts).unwrap().to_okt()).unwrap();
        };
        write("", Elem::Blob, true);
        let t = br1().unwrap();
        assert_eq!((t.get(BR1_CLIPS).unwrap().origin(), t.clip_ranges()), (0x800, vec![(0x800, 0x808)]));
        write("", Elem::Blob, false);
        assert!(br1().unwrap_err().contains(BR1_CLIP_RANGES), "format 2: no ranges");
        write("ai.wait", Elem::Blob, true);
        assert!(br1().unwrap_err().contains("ai.wait"));
        write("", Elem::U8, true);
        assert!(br1().unwrap_err().contains("blob"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod continue_tests {
    use super::Assets;

    /// A continue picture asked for by its disc folder (`engine/con/`) is read from `engine/continue/` (converter 0.1.1
    /// on: "con" is reserved on Windows), else from `engine/con/` (data converted before); other names as they are.
    #[test]
    fn continue_pictures_from_the_new_folder_or_the_old() {
        let root = std::env::temp_dir().join(format!("okb_assets_continue_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, name, body) in [("continue", "YUGCON.DAT", "new"), ("con", "ALICON.DAT", "old"), ("con", "YUGCON.DAT", "stale")] {
            std::fs::create_dir_all(root.join("engine").join(dir)).unwrap();
            std::fs::write(root.join("engine").join(dir).join(name), body).unwrap();
        }
        let a = Assets::new(&root);
        let load = |rel: &str| pollster::block_on(a.load(rel));
        assert_eq!(load("engine/con/YUGCON.DAT").unwrap(), b"new");
        assert_eq!(load("engine/con/ALICON.DAT").unwrap(), b"old");
        let e = load("engine/con/LONCON.DAT").unwrap_err();
        assert!(e.find("continue").is_some_and(|i| e.find("con/").is_some_and(|j| i < j)), "the new folder named first: {e}");
        assert_eq!(load("engine/continue/YUGCON.DAT").unwrap(), b"new");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
