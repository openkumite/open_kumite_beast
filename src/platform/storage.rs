//! Where the engine keeps its files: the memory card file, the preferences, save slots, replays.
//!
//! Engine addition.
//!
//! Where the engine keeps its files (engine addition): the memory card file (`save.json`), the preferences
//! (`prefs.json`), the NETWORK menu's settings (`net.json`), the user's gamepad mappings (`gamecontrollerdb.txt`, read only), the debug tools' save-state slots and the replays; in the config directory on desktop (on Android
//! the app's private files, with the touch controls' `touch.json`), in the page's localStorage on the web. Bytes and text only: `settings` turns them into its `Card` / `Prefs`, the app
//! moves them between the two.

const KEY: &str = "okb_save";

/// The config directory's name; `OLD_DIR` the one earlier builds used (renamed to it once, `migrated`).
#[cfg(not(target_arch = "wasm32"))]
const DIR: &str = "openkumitebeast";
#[cfg(not(target_arch = "wasm32"))]
const OLD_DIR: &str = "br2_engine";

/// The engine's config directory (desktop): `OKB_SAVE_DIR`, `APPDATA`, `XDG_CONFIG_HOME` or `~/.config`, + `openkumitebeast`
/// (an earlier build's `br2_engine` beside it renamed to it once); on Android the app's private files folder +
/// `openkumitebeast`. Decided at the first call and kept for the process, so a later rename never moves the folder
/// under a running game.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn dir() -> std::path::PathBuf {
    static DIR_ONCE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR_ONCE
        .get_or_init(|| {
            #[cfg(target_os = "android")]
            if let Some(d) = crate::platform::android::files_dir() {
                return d.join(DIR);
            }
            let base = crate::base::env::var_os("SAVE_DIR")
                .or_else(|| std::env::var_os("APPDATA"))
                .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
                .map(std::path::PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            migrated(&base, |old, new| std::fs::rename(old, new))
        })
        .clone()
}

/// The config directory under `base`: `base/openkumitebeast`; an earlier build's `base/br2_engine` is renamed to it
/// (`rename`) when it alone exists (the same parent: a rename, no copy). A failed rename is logged; the new directory
/// is used when it exists by then (another instance renamed it), else the old one stays in use.
#[cfg(not(target_arch = "wasm32"))]
fn migrated(base: &std::path::Path, rename: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>) -> std::path::PathBuf {
    let (new, old) = (base.join(DIR), base.join(OLD_DIR));
    if new.exists() || !old.is_dir() {
        return new;
    }
    match rename(&old, &new) {
        Ok(()) => {
            log::info!("config directory {} renamed to {}", old.display(), new.display());
            new
        }
        Err(e) if new.exists() => {
            log::info!("config directory {} not renamed ({e}): {} exists now", old.display(), new.display());
            new
        }
        Err(e) => {
            log::warn!("config directory {} not renamed to {} ({e}): kept", old.display(), new.display());
            old
        }
    }
}

/// A file of the debug tools beside the save (`<config dir>/<name>`; desktop): the save-state slots.
#[cfg(not(target_arch = "wasm32"))]
pub fn debug_path(name: &str) -> std::path::PathBuf {
    dir().join(name)
}

/// The folder of the recorded matches (`<config dir>/replays`; desktop; `session::replay`).
#[cfg(not(target_arch = "wasm32"))]
pub fn replay_dir() -> std::path::PathBuf {
    dir().join("replays")
}

/// A replay file's bytes, read at most `max` of them (`session::replay::MAX_BYTES`; desktop: the app's --replay, the
/// replays folder, the headless check; also `touch.json` and `net.json`).
#[cfg(not(target_arch = "wasm32"))]
pub fn read_capped(path: &std::path::Path, max: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut b = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(max as u64 + 1).read_to_end(&mut b)).map_err(|e| e.to_string())?;
    if b.len() > max {
        return Err(format!("larger than {max} bytes"));
    }
    Ok(b)
}

/// `bytes` to `path` through a temporary file beside it and a rename, so a process ending mid-write (Android ends it
/// in `onDestroy`, a crash on desktop) leaves the old file or the new one, never a cut one.
#[cfg(not(target_arch = "wasm32"))]
fn write_whole(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The user's SDL gamepad mappings (`gamecontrollerdb.txt` beside the save; desktop; `app::gamepads`).
#[cfg(not(target_arch = "wasm32"))]
pub fn pad_mappings_path() -> std::path::PathBuf {
    dir().join("gamecontrollerdb.txt")
}

/// The user's SDL gamepad mappings' text (read at most 4 MiB): None when missing, Err when it cannot be read.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_pad_mappings() -> Option<Result<String, String>> {
    let p = pad_mappings_path();
    if !p.exists() {
        return None;
    }
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 4 << 20) {
        return Some(Err("larger than 4 MiB".into()));
    }
    Some(std::fs::read_to_string(&p).map_err(|e| e.to_string()))
}

/// The touch controls' preferences' text (`touch.json` beside the save, read at most 16 KiB; Android's
/// `app::touch`), None when missing.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_touch_prefs() -> Option<String> {
    read_capped(&dir().join("touch.json"), 16 << 10).ok().and_then(|b| String::from_utf8(b).ok())
}

/// The touch controls' preferences to `touch.json` (logged when that fails).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_touch_prefs(text: &str) {
    let d = dir();
    if std::fs::create_dir_all(&d).and_then(|_| write_whole(&d.join("touch.json"), text.as_bytes())).is_err() {
        log::warn!("could not save the touch controls to {}", d.display());
    }
}

#[cfg(target_arch = "wasm32")]
const NET_KEY: &str = "okb_net";

/// The NETWORK menu's settings' text (`net.json` beside the save, read at most 16 KiB; localStorage `okb_net` in the
/// browser; `app::net_menu::NetPrefs`), None when missing.
pub fn read_net_prefs() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    return read_capped(&dir().join("net.json"), 16 << 10).ok().and_then(|b| String::from_utf8(b).ok());
    #[cfg(target_arch = "wasm32")]
    return web_get(NET_KEY);
}

/// The NETWORK menu's settings to `net.json` / localStorage (logged when that fails).
pub fn write_net_prefs(text: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let d = dir();
        if std::fs::create_dir_all(&d).and_then(|_| write_whole(&d.join("net.json"), text.as_bytes())).is_err() {
            log::warn!("could not save the network settings to {}", d.display());
        }
    }
    #[cfg(target_arch = "wasm32")]
    if !web_put(NET_KEY, text) {
        log::warn!("could not save the network settings to localStorage");
    }
}

const PREFS_KEY: &str = "okb_prefs";

/// The stored preferences' text (`Prefs::from_stored` reads it; None when missing).
pub fn read_prefs() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    return std::fs::read_to_string(dir().join("prefs.json")).ok();
    #[cfg(target_arch = "wasm32")]
    return web_get(PREFS_KEY);
}

/// The preferences' text (`Prefs::to_stored`; None: it could not be encoded, nothing is written) to `prefs.json` /
/// localStorage.
pub fn write_prefs(text: Option<String>) {
    let Some(text) = text else { return };
    let _ = PREFS_KEY;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let d = dir();
        if std::fs::create_dir_all(&d).and_then(|_| write_whole(&d.join("prefs.json"), text.as_bytes())).is_err() {
            log::warn!("could not save the preferences to {}", d.display());
        }
    }
    #[cfg(target_arch = "wasm32")]
    if !web_put(PREFS_KEY, &text) {
        log::warn!("could not save the preferences to localStorage");
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    /// What makes room in a full localStorage (`set_make_room`: the app's `replay_store::make_room`, which deletes
    /// the oldest unmarked replay); none set: a full storage refuses the write.
    static MAKE_ROOM: std::cell::Cell<Option<fn() -> bool>> = const { std::cell::Cell::new(None) };
}

/// The app's room maker for `web_put` (set once at start, before anything is stored).
#[cfg(target_arch = "wasm32")]
pub fn set_make_room(f: fn() -> bool) {
    MAKE_ROOM.with(|m| m.set(Some(f)));
}

/// localStorage key `key` (`okb_…`): its value; when it has none, the value of the key an earlier build used
/// (`br2_…`), copied to `key` and, once the copy is stored, removed.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) fn migrated_item(key: &str, get: impl Fn(&str) -> Option<String>, set: impl Fn(&str, &str) -> bool, remove: impl Fn(&str)) -> Option<String> {
    if let Some(v) = get(key) {
        return Some(v);
    }
    let old = format!("br2_{}", key.strip_prefix("okb_")?);
    let v = get(&old)?;
    if set(key, &v) {
        remove(&old);
    }
    Some(v)
}

/// localStorage key `key`, an earlier build's key moved to it first (`migrated_item`).
#[cfg(target_arch = "wasm32")]
pub(crate) fn web_get(key: &str) -> Option<String> {
    let st = web_sys::window().and_then(|w| w.local_storage().ok().flatten())?;
    migrated_item(
        key,
        |k| st.get_item(k).ok().flatten(),
        |k, v| st.set_item(k, v).is_ok(),
        |k| {
            let _ = st.remove_item(k);
        },
    )
}

/// `text` into localStorage under `key`; a full storage makes room by deleting the oldest unmarked replays
/// (`replay_store::make_room`, through `set_make_room`) rather than lose the memory card or the preferences.
#[cfg(target_arch = "wasm32")]
fn web_put(key: &str, text: &str) -> bool {
    let Some(st) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else { return false };
    loop {
        if st.set_item(key, text).is_ok() {
            return true;
        }
        if !MAKE_ROOM.with(|m| m.get()).is_some_and(|make_room| make_room()) {
            return false;
        }
    }
}

/// Where the save lives (support report).
pub fn save_location() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    return save_path().display().to_string();
    #[cfg(target_arch = "wasm32")]
    return format!("localStorage[\"{KEY}\"]");
}

#[cfg(not(target_arch = "wasm32"))]
fn save_path() -> std::path::PathBuf {
    dir().join("save.json")
}

/// App side: the stored save (`save.json` in the config directory, `localStorage["okb_save"]` on the web), for
/// `options::Card::new`. A file that exists but is not a save (bad bytes) is kept, so LOAD says DATA ERROR; one
/// over 1 MiB, or one that cannot be read, counts as no file.
pub fn read_card() -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    return std::fs::metadata(save_path()).ok().filter(|m| m.len() <= 1 << 20).and_then(|_| std::fs::read(save_path()).ok());
    #[cfg(target_arch = "wasm32")]
    return web_get(KEY).map(String::into_bytes);
}

/// App side, after a step that wrote the card: `file` (`options::Card::file`) to the disk / localStorage; false
/// when that failed (logged).
pub fn store_card(file: &[u8]) -> bool {
    let _ = KEY;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let p = save_path();
        let ok = p.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && write_whole(&p, file).is_ok();
        if ok {
            log::info!("settings saved to {}", p.display());
        } else {
            log::warn!("could not save the settings to {}", p.display());
        }
        ok
    }
    #[cfg(target_arch = "wasm32")]
    {
        let ok = std::str::from_utf8(file).is_ok_and(|text| web_put(KEY, text));
        if ok {
            log::info!("settings saved to localStorage");
        } else {
            log::warn!("could not save the settings to localStorage");
        }
        ok
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn write_whole_replaces_and_leaves_no_temporary() {
        let d = std::env::temp_dir().join(format!("okb_write_whole_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("save.json");
        super::write_whole(&p, b"old").unwrap();
        super::write_whole(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert!(!d.join("save.json.tmp").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_earlier_config_dir_is_renamed_once() {
        let rename = |old: &std::path::Path, new: &std::path::Path| std::fs::rename(old, new);
        let base = std::env::temp_dir().join(format!("okb_migrate_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // neither: the new name
        assert_eq!(super::migrated(&base, rename), base.join("openkumitebeast"));
        // the old one alone: renamed, its files kept
        std::fs::create_dir_all(base.join("br2_engine")).unwrap();
        std::fs::write(base.join("br2_engine/save.json"), b"card").unwrap();
        assert_eq!(super::migrated(&base, rename), base.join("openkumitebeast"));
        assert!(!base.join("br2_engine").exists());
        assert_eq!(std::fs::read(base.join("openkumitebeast/save.json")).unwrap(), b"card");
        // both: the new one, the old one left alone
        std::fs::create_dir_all(base.join("br2_engine")).unwrap();
        assert_eq!(super::migrated(&base, rename), base.join("openkumitebeast"));
        assert!(base.join("br2_engine").exists());
        // the rename fails: the old one kept
        std::fs::remove_dir_all(base.join("openkumitebeast")).unwrap();
        let denied = |_: &std::path::Path, _: &std::path::Path| Err(std::io::Error::other("in use"));
        assert_eq!(super::migrated(&base, denied), base.join("br2_engine"));
        // the rename fails because another instance renamed it first: the new one
        let raced = |old: &std::path::Path, new: &std::path::Path| {
            std::fs::rename(old, new)?;
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        };
        assert_eq!(super::migrated(&base, raced), base.join("openkumitebeast"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn an_earlier_storage_key_moves_once() {
        use std::cell::RefCell;
        use std::collections::BTreeMap;
        let st = RefCell::new(BTreeMap::from([("br2_save".to_string(), "card".to_string())]));
        let get = |k: &str| st.borrow().get(k).cloned();
        let set = |k: &str, v: &str| st.borrow_mut().insert(k.into(), v.into()).is_none() || true;
        let remove = |k: &str| {
            st.borrow_mut().remove(k);
        };
        // the old key's value, moved to the new key
        assert_eq!(super::migrated_item("okb_save", get, set, remove).as_deref(), Some("card"));
        assert_eq!(st.borrow().keys().collect::<Vec<_>>(), ["okb_save"]);
        // the new key wins over an old one written later
        st.borrow_mut().insert("br2_save".into(), "older".into());
        assert_eq!(super::migrated_item("okb_save", get, set, remove).as_deref(), Some("card"));
        // a failed copy keeps the old key
        let st2 = RefCell::new(BTreeMap::from([("br2_net".to_string(), "net".to_string())]));
        let get2 = |k: &str| st2.borrow().get(k).cloned();
        let remove2 = |k: &str| {
            st2.borrow_mut().remove(k);
        };
        assert_eq!(super::migrated_item("okb_net", get2, |_, _| false, remove2).as_deref(), Some("net"));
        assert!(st2.borrow().contains_key("br2_net"));
        assert_eq!(super::migrated_item("okb_prefs", |_| None, |_, _| true, |_| {}), None);
    }
}
