//! Where the game data lives and how a new install replaces it (engine addition).
//!
//! The data folder is `game_data/` in the engine's config directory (`storage`: `OKB_SAVE_DIR`, `APPDATA`,
//! `XDG_CONFIG_HOME` or `~/.config`, + `openkumitebeast`; on Android the app's files). A copy or a conversion writes
//! `game_data.part/`, the full check reads it back, then `install` swaps it in: `game_data` → `game_data.old`,
//! `.part` → `game_data`, `.old` deleted. `recover` at start puts a `.old` back when the swap was cut between its two
//! renames, and clears what an interrupted install left, unless another instance's copy holds the install lock
//! (`lock`: `game_data.lock`, an OS file lock, gone with its process however it ends).
//!
//! The optional installs (`Dirs::sub`) are folders of `game_data/`: the imported game's disc's `br1` (`BR1_DIR`) and
//! Quake III Arena's maps' `q3` (`Q3_DIR`, the extra stages), each installed the same way with its own `<name>.part`
//! / `<name>.old` inside `game_data/` and the same lock. A new install of the main game carries them over from the
//! replaced folder, with the extra stages' older `extras/` (`CARRIED`; `install`, and `recover` after a cut swap);
//! `remove` renames one to `<name>.del` and deletes that (`recover` finishes a cut delete, never puts it back).
//!
//! The browser keeps it in the origin private file system (OPFS, `navigator.storage.getDirectory()`): a folder
//! `data-<n>` per install, the current one named by localStorage `okb_data_dir` (`pointer`), so switching to a new
//! install is one `setItem`; the replaced one is named by `okb_data_old` and deleted at the next start, with the next
//! `data-<n>` a closed tab's copy left half written (`web::forget_old`). The imported game's install has its own
//! folders `br1-<n>` and pointers `okb_br1_dir` / `okb_br1_old` (`web::BR1`), Quake III Arena's maps theirs (`q3-<n>`,
//! `web::Q3`), untouched by a main-game install; a replaced one is deleted right after a switch
//! (`forget_replaced`), as one is replaced or removed at start without a reload (over the game the page reloads).
//! `web` has the async file operations the asset loader and the setup screen use.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// The imported game's install's folder in the data folder (`assets::manifest` checks it).
pub const BR1_DIR: &str = "br1";
/// Quake III Arena's maps' install folder in the data folder (engine addition, the extra stages; `assets::extras`
/// checks it).
pub const Q3_DIR: &str = "q3";
/// What a new install of the main game carries over from the data folder it replaces: the optional installs and the
/// extra stages' older folder (`extras/<map>/`, still read when there is no `q3`).
pub const CARRIED: [&str; 3] = [BR1_DIR, Q3_DIR, "extras"];

/// The data folder, its two siblings of an install, the one of a removal and the install lock's file.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct Dirs {
    pub root: PathBuf,
    pub part: PathBuf,
    pub old: PathBuf,
    /// a removal's folder (`remove`; `recover` deletes it)
    pub del: PathBuf,
    pub lock: PathBuf,
    /// The main game's data folder: an install carries the optional installs over (`carry`)
    holds_extras: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Dirs {
    /// The folders `game_data*` in `base`.
    pub fn at(base: &Path) -> Self {
        Self {
            root: base.join("game_data"),
            part: base.join("game_data.part"),
            old: base.join("game_data.old"),
            del: base.join("game_data.del"),
            lock: base.join("game_data.lock"),
            holds_extras: true,
        }
    }

    /// The app's data folders (beside the save).
    pub fn app() -> Self {
        Self::at(&super::storage::dir())
    }

    /// An optional install's folders (`BR1_DIR`, `Q3_DIR`): `name` in the data folder, its `.part`, `.old` and `.del`
    /// beside it, the same lock.
    pub fn sub(&self, name: &str) -> Self {
        // a name, not "" (that would make `game_data/.part`, the data folder's own siblings' look-alikes)
        debug_assert!(!name.is_empty() && !name.contains(['/', '\\', '.']), "{name:?}");
        Self {
            root: self.root.join(name),
            part: self.root.join(format!("{name}.part")),
            old: self.root.join(format!("{name}.old")),
            del: self.root.join(format!("{name}.del")),
            lock: self.lock.clone(),
            holds_extras: false,
        }
    }

    /// The optional installs in the replaced data folder `old` (`CARRIED`) moved into the new one (a main-game
    /// install keeps them; nothing for one the new folder has itself, nothing for an optional install's own `Dirs`).
    /// False: a move failed, `old` must stay for the next `recover` to try again.
    fn carry(&self) -> bool {
        if !self.holds_extras {
            return true;
        }
        let mut all = true;
        for name in CARRIED {
            let (from, to) = (self.old.join(name), self.root.join(name));
            if from.is_dir() && self.root.is_dir() && !to.exists() {
                match rename_retried(&from, &to) {
                    Ok(()) => log::info!("game data: {name} kept in {}", to.display()),
                    Err(e) => {
                        log::warn!("game data: {name} could not be kept yet ({e}): {} left for the next start", self.old.display());
                        all = false;
                    }
                }
            }
        }
        all
    }

    /// The install removed (renamed to `.del` first, so a cut or failed delete is finished by the next `recover`; a
    /// `.old` is a cut swap's, put back).
    pub fn remove(&self) -> std::io::Result<()> {
        if !self.root.exists() {
            return Ok(());
        }
        if self.del.exists() {
            std::fs::remove_dir_all(&self.del)?;
        }
        rename_retried(&self.root, &self.del)?;
        std::fs::remove_dir_all(&self.del)
    }

    /// The install lock, held (the file) while a copy or a conversion runs; None when another process holds it. A
    /// system without file locks takes it always.
    pub fn lock(&self) -> std::io::Result<Option<std::fs::File>> {
        if let Some(d) = self.lock.parent() {
            std::fs::create_dir_all(d)?;
        }
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&self.lock)?;
        match f.try_lock() {
            Ok(()) => Ok(Some(f)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) if e.kind() == std::io::ErrorKind::Unsupported => Ok(Some(f)),
            Err(std::fs::TryLockError::Error(e)) => Err(e),
        }
    }

    /// At start: a swap cut after its first rename (no `game_data`, a `.old`) is undone; a left `.part` (a copy cut
    /// short), a left `.old` (a swap cut before its delete) and a left `.del` (a removal cut or failed) are removed.
    /// Nothing while another instance installs.
    pub fn recover(&self) {
        let _held = match self.lock() {
            Ok(None) => {
                log::warn!("game data: another instance is installing into {}: left as it is", self.root.display());
                return;
            }
            Ok(Some(f)) => Some(f),
            Err(_) => None,
        };
        if !self.root.exists() && self.old.exists() {
            match std::fs::rename(&self.old, &self.root) {
                Ok(()) => log::warn!("game data: an interrupted install undone ({} put back)", self.root.display()),
                Err(e) => log::warn!("game data: could not put {} back: {e}", self.old.display()),
            }
        }
        // a swap cut before its optional installs were carried over (or a carry that failed): `.old` kept while it
        // still holds one
        let carried = self.carry();
        for d in [&self.part, &self.old, &self.del] {
            if d.exists() && (carried || d != &self.old) {
                match std::fs::remove_dir_all(d) {
                    Ok(()) => log::info!("game data: removed the leftover {}", d.display()),
                    Err(e) => log::warn!("game data: could not remove {}: {e}", d.display()),
                }
            }
        }
    }

    /// A fresh, empty `.part` folder for a new install.
    pub fn fresh_part(&self) -> std::io::Result<()> {
        if self.part.exists() {
            std::fs::remove_dir_all(&self.part)?;
        }
        std::fs::create_dir_all(&self.part)
    }

    /// The checked `.part` becomes the data folder; the old data (if any) goes.
    pub fn install(&self) -> std::io::Result<()> {
        if self.old.exists() {
            // a carry that failed before: tried again, the install refused while `.old` still holds the data
            if !self.carry() {
                return Err(std::io::Error::other("the Bloody Roar 1 or Quake III data of the replaced game data could not be moved: try again"));
            }
            std::fs::remove_dir_all(&self.old)?;
        }
        let had = self.root.exists();
        if had {
            rename_retried(&self.root, &self.old)?;
        }
        if let Err(e) = rename_retried(&self.part, &self.root) {
            if had {
                let _ = rename_retried(&self.old, &self.root);
            }
            return Err(e);
        }
        // the old data deleted once its optional installs are out of it (else `recover` carries them at the next start)
        if had
            && self.carry()
            && let Err(e) = std::fs::remove_dir_all(&self.old)
        {
            log::warn!("game data: could not remove {}: {e}", self.old.display());
        }
        Ok(())
    }

    /// The `.part` of a cancelled or failed install removed.
    pub fn discard_part(&self) {
        if self.part.exists()
            && let Err(e) = std::fs::remove_dir_all(&self.part)
        {
            log::warn!("game data: could not remove {}: {e}", self.part.display());
        }
    }
}

/// A folder renamed, tried a few times over a second: on Windows a virus scanner or the indexer often still holds
/// one of the files just written ("Access is denied").
#[cfg(not(target_arch = "wasm32"))]
fn rename_retried(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if tries < 5 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                tries += 1;
                log::warn!("game data: renaming {}: {e}; again", from.display());
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            r => return r,
        }
    }
}

/// A browser slot's next install folder (`<prefix><n>`): numbered past both its current one and the one it replaced
/// (`old`, kept until the next start: a REMOVE then ADD in one tab must neither overwrite it nor be taken for it).
#[cfg(any(target_arch = "wasm32", test))]
fn next_slot_dir(prefix: &str, pointer: Option<&str>, old: Option<&str>) -> String {
    let num = |d: Option<&str>| d.and_then(|d| d.strip_prefix(prefix)).and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    format!("{prefix}{}", num(pointer).max(num(old)) + 1)
}

/// The browser's game data in OPFS (async; every call checked, an API missing gives an Err, never a panic).
#[cfg(target_arch = "wasm32")]
pub mod web {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{FileSystemDirectoryHandle, FileSystemFileHandle};

    /// An install's place: its pointer and replaced-folder keys in localStorage, its folders' prefix.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Slot {
        pointer: &'static str,
        old: &'static str,
        prefix: &'static str,
    }

    /// The main game's data.
    pub const BR2: Slot = Slot { pointer: "okb_data_dir", old: "okb_data_old", prefix: "data-" };
    /// The optional imported data.
    pub const BR1: Slot = Slot { pointer: "okb_br1_dir", old: "okb_br1_old", prefix: "br1-" };
    /// Quake III Arena's maps (engine addition, the extra stages).
    pub const Q3: Slot = Slot { pointer: "okb_q3_dir", old: "okb_q3_old", prefix: "q3-" };
    /// What a browser without OPFS (Firefox private windows, old Safari) is told.
    pub const NO_STORAGE: &str = "This browser window can't store the game data: use a normal (not private) window of a current browser.";
    /// What a browser with OPFS but no writable streams (Safari before 26) is told.
    pub const NO_WRITE: &str = "This browser cannot store the game data: use a current Chrome, Edge or Firefox, or update Safari.";

    /// Files in OPFS can be written (`FileSystemFileHandle.prototype.createWritable` exists; the web-sys binding
    /// cannot catch the TypeError calling a missing one throws, which would end the copy's task without an answer).
    pub fn can_write() -> bool {
        let proto = js_sys::Reflect::get(&js_sys::global(), &"FileSystemFileHandle".into())
            .and_then(|c| if c.is_undefined() { Err(c) } else { js_sys::Reflect::get(&c, &"prototype".into()) });
        proto.is_ok_and(|p| p.is_object() && js_sys::Reflect::get(&p, &"createWritable".into()).is_ok_and(|f| f.is_function()))
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window().and_then(|w| w.local_storage().ok().flatten())
    }

    impl Slot {
        /// The current install's folder (`data-<n>`, `br1-<n>`, `q3-<n>`), if any.
        pub fn pointer(self) -> Option<String> {
            crate::platform::storage::web_get(self.pointer).filter(|d| d.starts_with(self.prefix))
        }

        /// The next install's folder name (past the replaced one too, `super::next_slot_dir`).
        pub fn next_dir(self) -> String {
            let old = crate::platform::storage::web_get(self.old);
            super::next_slot_dir(self.prefix, self.pointer().as_deref(), old.as_deref())
        }

        /// `dir` becomes the current install (None: none, the slot's data removed); the one before is deleted at the
        /// next start (`forget_old`), or right after by `forget_replaced` (the optional installs: at start they are replaced
        /// and removed without a reload, so their replaced folders must not pile up).
        pub fn switch_to(self, dir: Option<&str>) -> Result<(), String> {
            let s = storage().ok_or("no localStorage")?;
            if let Some(old) = self.pointer() {
                let _ = s.set_item(self.old, &old);
            }
            match dir {
                Some(d) => s.set_item(self.pointer, d).map_err(|_| "the game data's pointer could not be saved".to_string()),
                None => s.remove_item(self.pointer).map_err(|_| "the game data's pointer could not be removed".to_string()),
            }
        }

        /// The replaced install's folder (`switch_to`'s record) deleted and its record cleared (under the install
        /// lock); the current one never.
        pub async fn forget_replaced(self, root: &FileSystemDirectoryHandle) {
            let Some(s) = storage() else { return };
            if let Some(old) = crate::platform::storage::web_get(self.old) {
                if Some(&old) != self.pointer().as_ref() {
                    remove(root, &old).await;
                }
                let _ = s.remove_item(self.old);
            }
        }
    }

    /// The folder of the current install of the main game (`data-<n>`), if any.
    pub fn pointer() -> Option<String> {
        BR2.pointer()
    }

    /// The folder name of the next install of the main game.
    pub fn next_dir() -> String {
        BR2.next_dir()
    }

    /// `dir` becomes the current install of the main game; the one before is deleted at the next start.
    pub fn switch_to(dir: &str) -> Result<(), String> {
        BR2.switch_to(Some(dir))
    }

    /// The installs replaced last time, and the next ones' folders a copy left half written (the tab closed during
    /// it), removed (at start, before anything reads the data).
    pub async fn forget_old() {
        let Ok(r) = root().await else { return };
        // another tab installing (its `next_dir` under way): left alone
        let Some(_lock) = install_lock().await else { return };
        for slot in [BR2, BR1, Q3] {
            // the folder a copy cut short wrote, named while the replaced one was still recorded (`next_slot_dir`
            // numbers past it): taken before `forget_replaced` clears that record
            let next = slot.next_dir();
            slot.forget_replaced(&r).await;
            remove(&r, &next).await;
        }
    }

    /// The install lock (Web Locks `okb_install`), held while the value lives: one tab installs at a time, and no tab
    /// removes a folder another is writing. None: another tab holds it. A browser without Web Locks: an empty lock.
    pub struct InstallLock {
        release: Option<js_sys::Function>,
    }

    impl Drop for InstallLock {
        fn drop(&mut self) {
            if let Some(f) = self.release.take() {
                let _ = f.call0(&wasm_bindgen::JsValue::NULL);
            }
        }
    }

    pub async fn install_lock() -> Option<InstallLock> {
        let get = |o: &wasm_bindgen::JsValue, k: &str| js_sys::Reflect::get(o, &k.into()).ok().filter(|v| !v.is_undefined() && !v.is_null());
        let Some(locks) = get(&js_sys::global(), "navigator").and_then(|n| get(&n, "locks")) else { return Some(InstallLock { release: None }) };
        let Some(request) = get(&locks, "request").and_then(|f| f.dyn_into::<js_sys::Function>().ok()) else {
            return Some(InstallLock { release: None });
        };
        // `held` settles when the lock is let go (its resolve kept in the guard); `got` says whether it was had
        let mut release = None;
        let held = js_sys::Promise::new(&mut |res, _| release = Some(res));
        let mut got_res = None;
        let got = js_sys::Promise::new(&mut |res, _| got_res = Some(res));
        let cb = wasm_bindgen::closure::Closure::once_into_js(move |lock: wasm_bindgen::JsValue| -> wasm_bindgen::JsValue {
            let had = !lock.is_null() && !lock.is_undefined();
            if let Some(r) = &got_res {
                let _ = r.call1(&wasm_bindgen::JsValue::NULL, &had.into());
            }
            if had { held.into() } else { wasm_bindgen::JsValue::UNDEFINED }
        });
        let opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&opts, &"ifAvailable".into(), &true.into());
        if request.call3(&locks, &"okb_install".into(), &opts, &cb).is_err() {
            return Some(InstallLock { release: None });
        }
        match JsFuture::from(got).await.ok().and_then(|v| v.as_bool()) {
            Some(true) => Some(InstallLock { release }),
            _ => None,
        }
    }

    async fn wait(p: js_sys::Promise) -> Result<wasm_bindgen::JsValue, String> {
        JsFuture::from(p)
            .await
            .map_err(|e| e.as_string().or_else(|| js_sys::Reflect::get(&e, &"message".into()).ok()?.as_string()).unwrap_or_else(|| format!("{e:?}")))
    }

    /// The origin's private folder (from the page or from a worker: the global scope's `navigator.storage`).
    pub async fn root() -> Result<FileSystemDirectoryHandle, String> {
        let get = |o: &wasm_bindgen::JsValue, k: &str| js_sys::Reflect::get(o, &k.into()).ok().filter(|v| !v.is_undefined() && !v.is_null());
        let st: web_sys::StorageManager =
            get(&js_sys::global(), "navigator").and_then(|n| get(&n, "storage")).and_then(|s| s.dyn_into().ok()).ok_or(NO_STORAGE)?;
        if get(&st, "getDirectory").is_none() {
            return Err(NO_STORAGE.into());
        }
        wait(st.get_directory()).await.map_err(|_| NO_STORAGE.to_string())?.dyn_into().map_err(|_| NO_STORAGE.into())
    }

    pub async fn subdir(d: &FileSystemDirectoryHandle, name: &str, create: bool) -> Result<FileSystemDirectoryHandle, String> {
        let o = web_sys::FileSystemGetDirectoryOptions::new();
        o.set_create(create);
        wait(d.get_directory_handle_with_options(name, &o)).await?.dyn_into().map_err(|_| "not a folder".into())
    }

    /// The file at `path` ('/'-separated) under `d`, its folders made when `create`.
    pub async fn file(d: &FileSystemDirectoryHandle, path: &str, create: bool) -> Result<FileSystemFileHandle, String> {
        let mut dir = d.clone();
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().ok_or("an empty path")?;
        for p in parts {
            dir = subdir(&dir, p, create).await?;
        }
        let o = web_sys::FileSystemGetFileOptions::new();
        o.set_create(create);
        wait(dir.get_file_handle_with_options(name, &o)).await?.dyn_into().map_err(|_| "not a file".into())
    }

    /// A file's `File` (its size, its bytes through `read_blob`).
    pub async fn get(d: &FileSystemDirectoryHandle, path: &str) -> Result<web_sys::File, String> {
        let h = file(d, path, false).await?;
        wait(h.get_file()).await?.dyn_into().map_err(|_| "not a file".into())
    }

    /// A Blob's (a File's) bytes.
    pub async fn read_blob(b: &web_sys::Blob) -> Result<Vec<u8>, String> {
        Ok(js_sys::Uint8Array::new(&wait(b.array_buffer()).await?).to_vec())
    }

    /// The bytes of `path` in the install folder `dir`.
    pub async fn read(dir: &str, path: &str) -> Result<Vec<u8>, String> {
        let d = subdir(&root().await?, dir, false).await.map_err(|e| format!("{dir}: {e}"))?;
        let f = get(&d, path).await.map_err(|e| format!("{path}: {e}"))?;
        read_blob(&f).await.map_err(|e| format!("{path}: {e}"))
    }

    /// `bytes` written as `path` under `d` (folders made, an old file replaced).
    pub async fn write(d: &FileSystemDirectoryHandle, path: &str, bytes: &[u8]) -> Result<(), String> {
        if !can_write() {
            return Err(NO_WRITE.into());
        }
        let h = file(d, path, true).await?;
        let w: web_sys::FileSystemWritableFileStream = wait(h.create_writable()).await?.dyn_into().map_err(|_| "no writable stream")?;
        let arr = js_sys::Uint8Array::from(bytes);
        let written = match w.write_with_js_u8_array(&arr) {
            Ok(p) => wait(p).await.map(|_| ()),
            Err(e) => Err(format!("{e:?}")),
        };
        let closed = wait(w.close()).await.map(|_| ());
        written.and(closed)
    }

    /// The entry `name` of `d` and everything in it removed (nothing when missing).
    pub async fn remove(d: &FileSystemDirectoryHandle, name: &str) {
        let o = web_sys::FileSystemRemoveOptions::new();
        o.set_recursive(true);
        let _ = wait(d.remove_entry_with_options(name, &o)).await;
    }

    /// The origin's storage (usage, quota) in bytes, if the browser says.
    pub async fn estimate() -> Option<(f64, f64)> {
        let st = web_sys::window()?.navigator().storage();
        let e: web_sys::StorageEstimate = wait(st.estimate().ok()?).await.ok()?.dyn_into().ok()?;
        Some((e.get_usage()?, e.get_quota()?))
    }

    /// The data kept when space runs low (`navigator.storage.persist()`); whether the browser agreed.
    pub async fn persist() -> bool {
        let Some(st) = web_sys::window().map(|w| w.navigator().storage()) else { return false };
        match st.persist() {
            Ok(p) => wait(p).await.ok().and_then(|v| v.as_bool()).unwrap_or(false),
            Err(_) => false,
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{Dirs, next_slot_dir};

    #[test]
    fn next_slot_dir_skips_the_kept_replaced_folder() {
        assert_eq!(next_slot_dir("q3-", None, None), "q3-1");
        assert_eq!(next_slot_dir("q3-", Some("q3-1"), None), "q3-2");
        // REMOVE then ADD in one tab: the removed q3-1 is kept until the next start
        assert_eq!(next_slot_dir("q3-", None, Some("q3-1")), "q3-2");
        assert_eq!(next_slot_dir("q3-", Some("q3-2"), Some("q3-1")), "q3-3");
        assert_eq!(next_slot_dir("q3-", Some("q3-2"), Some("q3-5")), "q3-6");
        // another slot's or a foreign name counts as none
        assert_eq!(next_slot_dir("q3-", Some("data-4"), Some("q3-x")), "q3-1");
        // REMOVE, then an ADD cut short (the tab closed) wrote q3-2: the next start's `forget_old` names it before it
        // clears the record of the removed q3-1 (after it, the name would be q3-1 and q3-2 would stay)
        let next = next_slot_dir("q3-", None, Some("q3-1"));
        assert_eq!((next.as_str(), next_slot_dir("q3-", None, None).as_str()), ("q3-2", "q3-1"));
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("okb_game_data_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn install_swaps_and_recover_undoes_a_cut_swap() {
        let base = temp("swap");
        let d = Dirs::at(&base);
        // the first install: no old data
        d.fresh_part().unwrap();
        std::fs::write(d.part.join("a"), b"1").unwrap();
        d.install().unwrap();
        assert_eq!(std::fs::read(d.root.join("a")).unwrap(), b"1");
        assert!(!d.part.exists() && !d.old.exists());
        // a second one replaces it
        d.fresh_part().unwrap();
        std::fs::write(d.part.join("a"), b"2").unwrap();
        d.install().unwrap();
        assert_eq!(std::fs::read(d.root.join("a")).unwrap(), b"2");
        assert!(!d.part.exists() && !d.old.exists());
        // a swap cut after its first rename: the old data comes back, the part goes
        std::fs::rename(&d.root, &d.old).unwrap();
        std::fs::create_dir_all(&d.part).unwrap();
        d.recover();
        assert_eq!(std::fs::read(d.root.join("a")).unwrap(), b"2");
        assert!(!d.part.exists() && !d.old.exists());
        // a swap cut before its delete: the old copy goes, the data stays
        std::fs::create_dir_all(&d.old).unwrap();
        d.recover();
        assert!(d.root.join("a").exists() && !d.old.exists());
        // another copy under way (the lock held): its .part stays
        let held = d.lock().unwrap();
        assert!(held.is_some() && d.lock().unwrap().is_none());
        std::fs::create_dir_all(&d.part).unwrap();
        d.recover();
        assert!(d.part.exists());
        drop(held);
        d.recover();
        assert!(!d.part.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_br1_install_lives_in_the_data_folder_and_survives_a_new_one() {
        let base = temp("br1");
        let d = Dirs::at(&base);
        d.fresh_part().unwrap();
        d.install().unwrap();
        let b = d.sub(super::BR1_DIR);
        assert_eq!((b.root.parent(), b.lock.clone()), (Some(d.root.as_path()), d.lock.clone()));
        b.fresh_part().unwrap();
        std::fs::write(b.part.join("m"), b"1").unwrap();
        b.install().unwrap();
        assert_eq!(std::fs::read(d.root.join("br1/m")).unwrap(), b"1");
        // a new install of the main game keeps it
        d.fresh_part().unwrap();
        std::fs::write(d.part.join("a"), b"2").unwrap();
        d.install().unwrap();
        assert!(d.root.join("a").exists() && d.root.join("br1/m").exists() && !d.old.exists());
        // a swap cut after its renames, before the carry: recover carries it
        std::fs::rename(&b.root, base.join("keep")).unwrap();
        std::fs::create_dir_all(&d.old).unwrap();
        std::fs::rename(base.join("keep"), d.old.join("br1")).unwrap();
        d.recover();
        assert!(d.root.join("br1/m").exists() && !d.old.exists());
        // removed: nothing left of it, the rest stays
        b.remove().unwrap();
        assert!(!b.root.exists() && !b.old.exists() && !b.del.exists() && d.root.join("a").exists());
        b.remove().unwrap();
        // a removal cut after its rename (its delete failed, or the app was killed): recover finishes it
        b.fresh_part().unwrap();
        std::fs::write(b.part.join("m"), b"1").unwrap();
        b.install().unwrap();
        std::fs::rename(&b.root, &b.del).unwrap();
        d.recover();
        b.recover();
        assert!(!b.root.exists() && !b.del.exists() && d.root.join("a").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Quake III Arena's maps (`q3`) and the extra stages' older `extras/` survive a new install of the main game with
    /// the imported game's install; a folder the new data has itself is not replaced.
    #[test]
    fn the_q3_install_and_extras_survive_a_new_one() {
        let base = temp("q3");
        let d = Dirs::at(&base);
        d.fresh_part().unwrap();
        d.install().unwrap();
        let q = d.sub(super::Q3_DIR);
        assert_eq!((q.root.clone(), q.part.clone()), (d.root.join("q3"), d.root.join("q3.part")));
        q.fresh_part().unwrap();
        std::fs::write(q.part.join("manifest.json"), b"q").unwrap();
        q.install().unwrap();
        std::fs::create_dir_all(d.root.join("extras/q3dm17")).unwrap();
        std::fs::write(d.root.join("extras/q3dm17/shaders.json"), b"e").unwrap();
        std::fs::create_dir_all(d.root.join("br1")).unwrap();
        // the new data brings an `extras` of its own: kept as it is
        d.fresh_part().unwrap();
        std::fs::create_dir_all(d.part.join("extras")).unwrap();
        d.install().unwrap();
        assert!(d.root.join("q3/manifest.json").exists() && d.root.join("br1").is_dir() && !d.old.exists());
        assert!(!d.root.join("extras/q3dm17").exists());
        // without one: carried
        std::fs::create_dir_all(d.root.join("extras/q3dm6")).unwrap();
        d.fresh_part().unwrap();
        d.install().unwrap();
        assert!(d.root.join("q3/manifest.json").exists() && d.root.join("extras/q3dm6").is_dir() && d.root.join("br1").is_dir());
        q.remove().unwrap();
        assert!(!q.root.exists() && d.root.join("br1").is_dir());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_carry_keeps_the_br1_install_in_the_old_folder() {
        use std::os::unix::fs::PermissionsExt;
        let base = temp("carry");
        let d = Dirs::at(&base);
        let b = d.sub(super::BR1_DIR);
        d.fresh_part().unwrap();
        d.install().unwrap();
        b.fresh_part().unwrap();
        std::fs::write(b.part.join("m"), b"1").unwrap();
        b.install().unwrap();
        // the new data folder cannot take it (read-only): `.old` keeps it, recover keeps it too
        d.fresh_part().unwrap();
        std::fs::set_permissions(&d.part, std::fs::Permissions::from_mode(0o555)).unwrap();
        d.install().unwrap();
        assert!(d.old.join("br1/m").exists() && !b.root.exists());
        d.recover();
        assert!(d.old.join("br1/m").exists());
        // writable again: the next start carries it and clears `.old`
        std::fs::set_permissions(&d.root, std::fs::Permissions::from_mode(0o755)).unwrap();
        d.recover();
        assert!(b.root.join("m").exists() && !d.old.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
