//! The setup's native pickers on desktop: the scan for converted data and the BROWSE helper (engine addition).
//!
//! Without a dialog crate: `scan` looks in a few likely folders (the executable's, the current one, `~/Downloads`,
//! `~/Desktop`; debug OKB_SETUP_SCAN replaces them) two levels deep for folders holding `manifest.json` and `engine/`.
//! Not `~/Downloads` and `~/Desktop` on macOS, deliberately: reading them there makes the system ask the player for
//! the permission before they have asked for anything (BROWSE or a drop names a folder there instead).
//! `Browse` runs the system's folder dialog as a helper process on a thread (macOS `osascript`, Windows PowerShell,
//! Linux `zenity` or `kdialog`); drag and drop and the typed path are the window's (`setup/mod.rs`). On Android
//! there is no scan, and BROWSE is the system's folder picker (`platform::android::pick_folder`), whose answer is a
//! tree's `content://` URI (`job::Source::Tree`).

use super::plan::Kind;
use std::path::{Path, PathBuf};

/// At most this many entries read per folder, and this many found.
const MAX_ENTRIES: usize = 400;
const MAX_FOUND: usize = 6;

/// A folder of converted game data.
fn is_export(d: &Path) -> bool {
    d.join("manifest.json").is_file() && d.join("engine").is_dir()
}

/// The folders searched: OKB_SETUP_SCAN (`:`-separated, `;` on Windows) or the likely ones (module header).
fn roots(over: Option<&str>) -> Vec<PathBuf> {
    if let Some(v) = over {
        return std::env::split_paths(v).filter(|p| !p.as_os_str().is_empty()).collect();
    }
    let mut v = Vec::new();
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        v.push(d);
    }
    v.extend(std::env::current_dir().ok());
    // not on macOS: its privacy prompts (module header)
    if !cfg!(target_os = "macos")
        && let Some(h) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
    {
        v.push(h.join("Downloads"));
        v.push(h.join("Desktop"));
    }
    v
}

/// A disc image the converter may take: a `.cue`, `.chd` or `.iso`, or a `.bin` without a `.cue` of its name beside
/// it (the `.cue` stands for it).
fn is_disc(p: &Path) -> bool {
    let ext = p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match ext.as_str() {
        "cue" | "chd" | "iso" => true,
        "bin" => !["cue", "CUE"].iter().any(|c| p.with_extension(c).is_file()),
        _ => false,
    }
}

/// What the scan finds in `roots` (or OKB_SETUP_SCAN's), up to two levels below each: folders of converted game
/// data (`skip`, the data folder, left out) and disc images (only with the converter).
pub fn scan(over: Option<&str>, skip: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut dirs, mut discs): (Vec<PathBuf>, Vec<PathBuf>) = (Vec::new(), Vec::new());
    if cfg!(target_os = "android") && over.is_none() {
        return (dirs, discs);
    }
    let skip = plain(skip.canonicalize().unwrap_or_else(|_| skip.to_path_buf()));
    let mut add = |d: &Path| {
        // only folders and files named as disc images are resolved (most entries are other files)
        let ext = d.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !(cfg!(feature = "converter") && ["cue", "bin", "chd", "iso"].contains(&ext.as_str())) && !d.is_dir() {
            return;
        }
        let c = plain(d.canonicalize().unwrap_or_else(|_| d.to_path_buf()));
        if c.is_dir() {
            if dirs.len() < MAX_FOUND && !c.starts_with(&skip) && !dirs.contains(&c) && is_export(&c) {
                dirs.push(c);
            }
        } else if cfg!(feature = "converter") && discs.len() < MAX_FOUND && !discs.contains(&c) && is_disc(&c) {
            discs.push(c);
        }
    };
    let entries = |d: &Path| -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(d) else { return Vec::new() };
        let mut v: Vec<PathBuf> = rd.take(MAX_ENTRIES).flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).map(|e| e.path()).collect();
        v.sort();
        v
    };
    for r in roots(over) {
        add(&r);
        for d in entries(&r) {
            add(&d);
            if d.is_dir() {
                for d2 in entries(&d) {
                    add(&d2);
                }
            }
        }
    }
    (dirs, discs)
}

/// The system's folder dialog (or with `file`, its file dialog: a disc image) as a command, if this system has one;
/// `kind` what it asks for.
fn helper(file: bool, kind: Kind) -> Option<std::process::Command> {
    let prompt = match (kind, file) {
        (Kind::Q3, _) => "Choose the baseq3 folder of Quake III Arena (its .pk3 files), or its converted maps",
        (Kind::Br1, true) => "Choose the Bloody Roar (USA) disc image",
        (Kind::Br1, false) => "Choose the folder of converted Bloody Roar 1 data",
        (Kind::Br2, true) => "Choose the Bloody Roar II (USA) disc image",
        (Kind::Br2, false) => "Choose the folder of converted game data",
    };
    if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("/usr/bin/osascript");
        let what = if file { "file" } else { "folder" };
        c.args(["-e", &format!("POSIX path of (choose {what} with prompt \"{prompt}\")")]);
        return Path::new("/usr/bin/osascript").exists().then_some(c);
    }
    if cfg!(windows) {
        // the system's Windows PowerShell by its full path (a bare name is searched for in the executable's folder
        // first), single-threaded apartment for the dialog, its answer written as UTF-8 (redirected, it would be the
        // console's code page)
        let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        let exe = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let mut c = std::process::Command::new(&exe);
        c.args([
            "-NoProfile",
            "-STA",
            "-Command",
            &format!(
                "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false; \
                 Add-Type -AssemblyName System.Windows.Forms; {dialog}; \
                 if ($d.ShowDialog() -eq 'OK') {{ {answer} }}",
                dialog = if file {
                    format!("$d = New-Object System.Windows.Forms.OpenFileDialog; $d.Title = '{prompt}'; $d.Filter = 'Disc images|*.cue;*.bin;*.chd;*.iso|All files|*.*'")
                } else {
                    format!("$d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '{prompt}'")
                },
                answer = if file { "$d.FileName" } else { "$d.SelectedPath" },
            ),
        ]);
        return exe.is_file().then_some(c);
    }
    let on_path = |n: &str| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(n).is_file()));
    if on_path("zenity") {
        let mut c = std::process::Command::new("zenity");
        c.args(["--file-selection", &format!("--title={prompt}")]);
        if !file {
            c.arg("--directory");
        }
        return Some(c);
    }
    if on_path("kdialog") {
        let mut c = std::process::Command::new("kdialog");
        c.args([if file { "--getopenfilename" } else { "--getexistingdirectory" }, ".", "--title", prompt]);
        return Some(c);
    }
    None
}

/// This system has a folder dialog (BROWSE is shown).
pub fn can_browse() -> bool {
    cfg!(target_os = "android") || helper(false, Kind::Br2).is_some()
}

/// The folder dialog's helper process (kept, so that closing the setup screen or quitting closes the dialog too);
/// its answer once it has ended. On Android the system's picker (`platform::android`).
pub struct Browse {
    child: Option<std::process::Child>,
}

impl Browse {
    /// Opens the dialog for a folder, or with `file` a disc image (`kind`: what it asks for; Quake III Arena's maps:
    /// always a folder); None when this system has none, or the helper does not start.
    pub fn open(file: bool, kind: Kind) -> Option<Browse> {
        let file = file && kind != Kind::Q3;
        #[cfg(target_os = "android")]
        {
            crate::platform::android::pick(file);
            return Some(Browse { child: None });
        }
        #[allow(unreachable_code)]
        let mut c = helper(file, kind)?;
        let child = c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn();
        match child {
            Ok(ch) => Some(Browse { child: Some(ch) }),
            Err(e) => {
                log::warn!("setup: the folder dialog did not start: {e}");
                None
            }
        }
    }

    /// The dialog's answer once closed: the folder, or None (cancelled); None while it is open.
    pub fn poll(&mut self) -> Option<Option<PathBuf>> {
        #[cfg(target_os = "android")]
        return crate::platform::android::take_picked().map(|p| p.map(PathBuf::from));
        #[allow(unreachable_code)]
        let ch = self.child.as_mut()?;
        let status = match ch.try_wait() {
            Ok(None) => return None,
            Ok(Some(s)) => s,
            Err(_) => return Some(None),
        };
        let mut out = String::new();
        if let Some(mut so) = ch.stdout.take() {
            use std::io::Read;
            let _ = so.read_to_string(&mut out);
        }
        self.child = None;
        let s = out.trim_start_matches('\u{feff}').trim().to_string();
        Some((status.success() && !s.is_empty()).then(|| PathBuf::from(s)))
    }
}

impl Drop for Browse {
    /// A dialog still open is closed with the screen (its process ended).
    fn drop(&mut self) {
        if let Some(mut ch) = self.child.take()
            && matches!(ch.try_wait(), Ok(None))
        {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}

/// A path as shown and used from the scan: without Windows' verbatim `\\?\` prefix that `canonicalize` adds.
fn plain(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scan_finds_exports_two_levels_down() {
        let base = std::env::temp_dir().join(format!("okb_setup_scan_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for d in ["a/export/engine", "b/c/conv/engine", "b/c/d/deep/engine", "data/engine", "plain"] {
            std::fs::create_dir_all(base.join(d)).unwrap();
        }
        for d in ["a/export", "b/c/conv", "b/c/d/deep", "data"] {
            std::fs::write(base.join(d).join("manifest.json"), "{}").unwrap();
        }
        // the scan returns paths without Windows' verbatim prefix, so the expected ones must not have it either
        let base = super::plain(base.canonicalize().unwrap());
        let over = base.to_string_lossy().to_string();
        for f in ["a/disc.cue", "a/disc.bin", "a/other.bin", "b/c/game.chd", "a/notes.txt"] {
            std::fs::write(base.join(f), "x").unwrap();
        }
        let (found, discs) = super::scan(Some(&over), &base.join("data"));
        // `deep` is three levels down, `data` is the data folder
        assert_eq!(found, [base.join("a/export")]);
        // a .bin with its .cue is listed as the .cue; images only with the converter
        let want = if cfg!(feature = "converter") { vec![base.join("a/disc.cue"), base.join("a/other.bin")] } else { vec![] };
        assert_eq!(discs, want);
        let (found, discs) = super::scan(Some(&base.join("b").to_string_lossy()), &base.join("data"));
        assert_eq!(found, [base.join("b/c/conv")]);
        assert_eq!(discs.len(), cfg!(feature = "converter") as usize);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn verbatim_prefix_dropped() {
        use super::plain;
        assert_eq!(plain(r"\\?\C:\Games\br2_export".into()), std::path::PathBuf::from(r"C:\Games\br2_export"));
        assert_eq!(plain(r"\\?\UNC\srv\share\x".into()), std::path::PathBuf::from(r"\\srv\share\x"));
        assert_eq!(plain("/a/b".into()), std::path::PathBuf::from("/a/b"));
    }
}
