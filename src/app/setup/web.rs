//! The setup screen's picker and copy in the browser (engine addition).
//!
//! The browser's side of `job.rs` and `pickers.rs`, with the same names (no scan), so `setup/mod.rs` runs it the same way:
//! BROWSE opens a hidden `<input type=file webkitdirectory>` (from a user gesture; a gamepad's press leaves it to the
//! next Enter, Space or click, as the replay IMPORT does with any key); the chosen folder's files are Blobs, read
//! lazily. The picked folder is checked at once (its top, or a folder in it holding the data; the quick check over the
//! listed sizes; the storage quota), kept as the current pick, and named to the screen as a path (`Browse::poll`); `inspect` then
//! gives what that check found. `Job::copy` writes the files the engine reads into a new OPFS folder `data-<n>`
//! (`platform::game_data::web`), each one's CRC-32 taken on the way, reads every file
//! back to compare, quick-checks the folder and switches the pointer to it; the old folder goes at the next start.
//! It runs as a task on the page's thread (async reads and writes; the page stays responsive between files).
//! The imported game's disc's data (`Kind::Br1`) goes the same way into its own folder `br1-<n>` (`web::BR1`): a picked
//! folder whose (or whose `br1` folder's) manifest.json is of the second disc, or a disc image the worker converts.
//! So do Quake III Arena's maps (`Kind::Q3`, engine addition) into `q3-<n>` (`web::Q3`): a picked folder of maps this
//! game converted (its or its `q3` folder's manifest.json of Quake III Arena), or for CONVERT ITS .PK3 FILES a picked
//! folder's `.pk3` files (those under a `baseq3` folder when it has one), which the worker converts. A Quake III
//! install a session may still read is not deleted when replaced: the one it replaced before goes then, the rest at
//! the next start (`web::forget_old`).

use super::plan::{self, Failure, Item, Kind};
use super::screen::{Found, Phase, Progress};
use crate::assets::extras;
use crate::assets::manifest::{self, Br1Verdict, FileEntry, Record};
use crate::base::hash::{Crc32, crc32};
use crate::platform::game_data::web;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

pub use super::plan::Hooks;

/// The install folder a copy writes (the next `data-<n>`).
#[derive(Clone)]
pub struct Dirs {
    pub root: PathBuf,
}

impl Dirs {
    pub fn app() -> Self {
        Self { root: PathBuf::from(web::next_dir()) }
    }

    /// The next folder of the install `kind` (`data-<n>`, `br1-<n>`, `q3-<n>`), as the desktop's `Dirs::sub`.
    pub fn of(kind: Kind) -> Self {
        Self { root: PathBuf::from(slot(kind).next_dir()) }
    }
}

/// The OPFS slot an install of `kind` goes into.
pub(super) fn slot(kind: Kind) -> web::Slot {
    match kind {
        Kind::Br2 => web::BR2,
        Kind::Br1 => web::BR1,
        Kind::Q3 => web::Q3,
    }
}

/// The folder chosen and what its check found (kept after a copy: TRY AGAIN copies it again).
#[derive(Clone)]
struct Picked {
    /// the files under the game data's top, by path
    files: BTreeMap<String, web_sys::File>,
    found: Result<Found, String>,
    items: Vec<Item>,
    /// a disc image's files (CONVERT DISC IMAGE): converted by the worker instead of copied
    #[cfg(feature = "converter")]
    disc: Vec<web_sys::File>,
    kind: Kind,
}

thread_local! {
    static PICK: RefCell<Option<Picked>> = const { RefCell::new(None) };
}

pub fn can_browse() -> bool {
    true
}

/// The folder input: its answer (Some(None): cancelled), kept with its handlers while it may call them.
pub struct Browse {
    /// the chooser's answer and what its check found (this chooser's own: a slower check of an earlier one
    /// cannot replace a newer pick, `poll` makes it the current one)
    answer: Rc<RefCell<Option<Option<(PathBuf, Picked)>>>>,
    /// the chooser was opened, and it has answered (a pick page left meanwhile keeps the input until then)
    opened: Rc<Cell<bool>>,
    answered: Rc<Cell<bool>>,
    input: web_sys::HtmlInputElement,
    _handlers: Vec<Closure<dyn FnMut()>>,
    listeners: Vec<(&'static str, Closure<dyn FnMut(wasm_bindgen::JsValue)>)>,
}

impl Drop for Browse {
    fn drop(&mut self) {
        if let Some(w) = web_sys::window() {
            for (ev, f) in &self.listeners {
                let _ = w.remove_event_listener_with_callback(ev, f.as_ref().unchecked_ref());
            }
        }
        if self.opened.get() && !self.answered.get() {
            // its chooser still open: the input and its handlers stay until it answers (they remove the input then;
            // the answer goes to nobody)
            for h in std::mem::take(&mut self._handlers) {
                h.forget();
            }
        } else {
            self.input.remove();
        }
    }
}

impl Browse {
    /// The chooser, opened now in a user gesture, else at the next Enter, Space or click: a disc image (`file`,
    /// CONVERT DISC IMAGE: one or more files, a `.cue` with its `.bin`) or a folder (Quake III Arena's maps: always a
    /// folder; with `file` its `.pk3` files are converted).
    pub fn open(file: bool, kind: Kind) -> Option<Browse> {
        let win = web_sys::window()?;
        let doc = win.document()?;
        let input: web_sys::HtmlInputElement = doc.create_element("input").ok()?.dyn_into().ok()?;
        input.set_type("file");
        if file && kind != Kind::Q3 {
            input.set_accept(".cue,.bin,.chd,.iso");
        } else {
            input.set_webkitdirectory(true);
        }
        input.set_multiple(true);
        input.style().set_property("display", "none").ok();
        doc.body()?.append_child(&input).ok()?;
        let answer = Rc::new(RefCell::new(None));
        let (opened, answered) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
        let (inp, a, done) = (input.clone(), answer.clone(), answered.clone());
        let on_change = Closure::<dyn FnMut()>::new(move || {
            done.set(true);
            let files = inp.files();
            // the input's work is done (a Browse dropped while its chooser was open left it for this)
            inp.remove();
            let a = a.clone();
            wasm_bindgen_futures::spawn_local(async move {
                #[cfg(feature = "converter")]
                let (name, picked) = if file { check_disc(files, kind).await } else { check_pick(files, kind).await };
                #[cfg(not(feature = "converter"))]
                let (name, picked) = check_pick(files, kind).await;
                *a.borrow_mut() = Some(Some((PathBuf::from(name), picked)));
            });
        });
        input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
        let (inp, a, done) = (input.clone(), answer.clone(), answered.clone());
        let on_cancel = Closure::<dyn FnMut()>::new(move || {
            done.set(true);
            inp.remove();
            *a.borrow_mut() = Some(None);
        });
        input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref()).ok();
        let mut listeners = Vec::new();
        if super::super::web_files::activation() {
            opened.set(true);
            input.click();
        } else {
            // a gamepad's press: the next Enter / Space or click on the page opens it, once (the pick page left
            // first: `Run` drops this, its listeners go)
            for ev in ["keydown", "pointerdown"] {
                let (inp, opened) = (input.clone(), opened.clone());
                let f = Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(move |e: wasm_bindgen::JsValue| {
                    let key = js_sys::Reflect::get(&e, &"key".into()).ok().and_then(|k| k.as_string());
                    if key.as_ref().is_some_and(|k| k != "Enter" && k != " ") {
                        return;
                    }
                    // once, and never an input already gone
                    if inp.is_connected() && !opened.replace(true) {
                        inp.click();
                    }
                });
                if win.add_event_listener_with_callback(ev, f.as_ref().unchecked_ref()).is_ok() {
                    listeners.push((ev, f));
                }
            }
        }
        Some(Browse { answer, opened, answered, input, _handlers: vec![on_change, on_cancel], listeners })
    }

    /// The chooser's answer once checked: the folder's name (`inspect` has what was found), or None (cancelled).
    pub fn poll(&self) -> Option<Option<PathBuf>> {
        let a = self.answer.borrow_mut().take()?;
        Some(a.map(|(name, picked)| {
            PICK.with(|p| *p.borrow_mut() = Some(picked));
            name
        }))
    }
}

/// The chosen files checked: the folder's name and what was found (`Browse::poll` makes it the current pick).
async fn check_pick(list: Option<web_sys::FileList>, kind: Kind) -> (String, Picked) {
    let mut all: Vec<(String, web_sys::File)> = Vec::new();
    if let Some(list) = list {
        for k in 0..list.length() {
            let Some(f) = list.get(k) else { continue };
            let rel = js_sys::Reflect::get(&f, &"webkitRelativePath".into()).ok().and_then(|v| v.as_string()).unwrap_or_else(|| f.name());
            all.push((rel, f));
        }
    }
    let name = all.first().map(|(r, _)| r.split('/').next().unwrap_or("").to_string()).unwrap_or_default();
    let picked = pick_of(all, &name, kind).await;
    (name, picked)
}

async fn pick_of(all: Vec<(String, web_sys::File)>, name: &str, kind: Kind) -> Picked {
    let fail = |e: String| Picked {
        files: BTreeMap::new(),
        found: Err(e),
        items: Vec::new(),
        #[cfg(feature = "converter")]
        disc: Vec::new(),
        kind,
    };
    if !web::can_write() {
        return fail(web::NO_WRITE.into());
    }
    // the top: the shortest folder with manifest.json and engine/ (the imported game: the shortest whose manifest is of
    // the second disc; Quake III Arena's maps, without engine/: the shortest whose manifest is of Quake III Arena)
    let mut tops: Vec<&str> = all
        .iter()
        .filter_map(|(r, _)| r.strip_suffix("manifest.json"))
        .filter(|pre| pre.is_empty() || pre.ends_with('/'))
        .filter(|pre| kind == Kind::Q3 || all.iter().any(|(r, _)| r.starts_with(&format!("{pre}engine/"))))
        .collect();
    tops.sort_by_key(|pre| pre.len());
    let mut top = None;
    // a manifest of other data (okconv-q3's own output): said so, rather than "no data"
    let mut other = false;
    for pre in tops {
        if kind == Kind::Br2 {
            top = Some(pre.to_string());
            break;
        }
        let m = all.iter().find(|(r, _)| *r == format!("{pre}manifest.json")).map(|(_, f)| f.clone());
        let ours = |b: &[u8]| if kind == Kind::Q3 { extras::is_q3_manifest(b) } else { manifest::is_br1_manifest(b) };
        if let Some(m) = m.filter(|m| m.size() <= manifest::MAX_JSON as f64)
            && let Ok(b) = web::read_blob(&m).await
        {
            if ours(&b) {
                top = Some(pre.to_string());
                break;
            }
            other = true;
        }
    }
    let Some(top) = top else {
        return fail(match kind {
            Kind::Br1 => format!("No converted Bloody Roar 1 data in {name} (it needs its manifest.json and engine/)."),
            Kind::Q3 if other => plan::other_q3(name),
            Kind::Q3 => format!("No converted Quake III Arena maps in {name}: to convert Quake III Arena's .pk3 files, choose CONVERT ITS .PK3 FILES."),
            Kind::Br2 => format!("No converted game data in {name} (it needs manifest.json and engine/)."),
        });
    };
    let files: BTreeMap<String, web_sys::File> = all
        .into_iter()
        .filter_map(|(r, f)| Some((r.strip_prefix(top.as_str())?.to_string(), f)))
        .filter(|(r, _)| manifest::safe_path(r) && !r.starts_with("play/") && !r.split('/').any(|c| c.starts_with('.')))
        .collect();
    // the manifest read whole: at most `MAX_JSON` bytes, as on the desktop
    let Some(m) = files.get("manifest.json").cloned() else { return fail("manifest.json is missing".into()) };
    if m.size() > manifest::MAX_JSON as f64 {
        return fail("manifest.json is too large".into());
    }
    let Ok(text) = web::read_blob(&m).await else { return fail("manifest.json cannot be read".into()) };
    let (items, described) = if kind == Kind::Q3 {
        let v = extras::quick_check_q3(Some(&text), &|p| files.get(p).map(|f| f.size() as u64));
        let described = plan::describe_q3(&v, Some(&text), name);
        if let Err(e) = &described {
            return fail(e.clone());
        }
        match plan::items_q3(&text) {
            Ok(items) => (items, described),
            Err(e) => return fail(e),
        }
    } else if kind == Kind::Br1 {
        let v: Br1Verdict = manifest::quick_check_br1(Some(&text), &|p| files.get(p).map(|f| f.size() as u64));
        let own = Record::parse(&text).and_then(Result::ok);
        let described = plan::describe_br1(&v, own.as_ref(), name);
        if let Err(e) = &described {
            return fail(e.clone());
        }
        match plan::items_extra(&text, manifest::required_br1()) {
            Ok(items) => (items, described),
            Err(e) => return fail(e),
        }
    } else {
        let v = manifest::quick_check(Some(&text), &|p| files.get(p).map(|f| f.size() as u64));
        if !v.usable() {
            return fail(plan::describe(&v, None, name).err().unwrap_or_default());
        }
        let (items, rec) = match plan::items(&text) {
            Ok(x) => x,
            Err(e) => return fail(e),
        };
        let described = plan::describe(&v, Some(&rec), name);
        (items, described)
    };
    let bytes: u64 = items.iter().map(|i| i.size).sum();
    let found = described.map(|(what, warn)| Found { path: PathBuf::from(name), what, bytes, warn });
    // the browser's quota for this site, with a tenth to spare
    let found = match (found, web::estimate().await) {
        (Ok(f), Some((usage, quota))) if (f.bytes as f64) * 1.1 > quota - usage => Err(format!(
            "Not enough storage for this site: the game data needs {} MB, the browser allows {} MB more.",
            f.bytes >> 20,
            ((quota - usage).max(0.0) as u64) >> 20
        )),
        (f, _) => f,
    };
    Picked {
        files,
        found,
        items,
        #[cfg(feature = "converter")]
        disc: Vec::new(),
        kind,
    }
}

/// What the current pick's check found.
pub fn inspect(_p: &Path, _data: &Path) -> Result<Found, String> {
    PICK.with(|p| p.borrow().as_ref().map(|p| p.found.clone())).unwrap_or_else(|| Err("Choose the folder again.".into()))
}

/// A copy running as a task on the page's thread.
pub struct Job {
    progress: Rc<RefCell<Progress>>,
    cancel: Rc<Cell<bool>>,
    end: Rc<RefCell<Option<Result<(), Failure>>>>,
    /// a conversion's worker, its message handler and its folder (set once it has the install lock)
    #[cfg(feature = "converter")]
    worker: Rc<RefCell<Option<(web_sys::Worker, Closure<dyn FnMut(web_sys::MessageEvent)>, String)>>>,
    /// the worker has answered (its files written, or a failure): a cancel is then left to the page's own end
    /// (`finish` checks it before the switch and removes the folder)
    #[cfg(feature = "converter")]
    answered: Rc<Cell<bool>>,
    /// the install lock (`web::install_lock`), held until the end is taken
    lock: Rc<RefCell<Option<web::InstallLock>>>,
}

/// What a second tab is told while one installs.
const LOCKED: &str = "Another tab of the game is installing its data: try again when it is done.";
/// What a tab is told when another one installed meanwhile (`dirs.root`, named when the screen opened, is now that
/// tab's install or its next one: never written here).
const CHANGED: &str = "Another tab of the game has installed its data meanwhile: reload the page.";

impl Job {
    /// The current pick copied into `dirs.root` (an OPFS folder name; `kind`: the slot it is installed into).
    pub fn copy(_src: PathBuf, dirs: Dirs, _hooks: Hooks, kind: Kind) -> Job {
        let job = Job {
            progress: Rc::default(),
            cancel: Rc::default(),
            end: Rc::default(),
            #[cfg(feature = "converter")]
            worker: Rc::default(),
            #[cfg(feature = "converter")]
            answered: Rc::default(),
            lock: Rc::default(),
        };
        let (p, c, e, l) = (job.progress.clone(), job.cancel.clone(), job.end.clone(), job.lock.clone());
        // a pick made for the other kind (its pages left meanwhile) is not installed here
        let picked = PICK.with(|p| p.borrow().clone()).filter(|pk| pk.kind == kind);
        let dir = dirs.root.to_string_lossy().to_string();
        #[cfg(feature = "converter")]
        if let Some(pk) = picked.as_ref().filter(|pk| !pk.disc.is_empty()) {
            let (files, w, a) = (pk.disc.clone(), job.worker.clone(), job.answered.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let Some(lock) = web::install_lock().await else {
                    *e.borrow_mut() = Some(Err(Failure::Io(LOCKED.into())));
                    return;
                };
                *l.borrow_mut() = Some(lock);
                if c.get() {
                    *e.borrow_mut() = Some(Err(Failure::Cancelled));
                    return;
                }
                if slot(kind).next_dir() != dir {
                    *e.borrow_mut() = Some(Err(Failure::Io(CHANGED.into())));
                    return;
                }
                match convert(&files, &dir, &p, &e, &c, &a, kind) {
                    Ok((worker, on)) => *w.borrow_mut() = Some((worker, on, dir)),
                    Err(x) => *e.borrow_mut() = Some(Err(Failure::Io(x))),
                }
            });
            return job;
        }
        wasm_bindgen_futures::spawn_local(async move {
            let r = match (picked, web::install_lock().await) {
                (_, None) => Err(Failure::Io(LOCKED.into())),
                (Some(pk), Some(lock)) => {
                    *l.borrow_mut() = Some(lock);
                    if slot(kind).next_dir() != dir { Err(Failure::Io(CHANGED.into())) } else { copy_into(pk, &dir, &p, &c).await }
                }
                (None, Some(_)) => Err(Failure::Io("choose the folder again".into())),
            };
            *e.borrow_mut() = Some(r);
        });
        job
    }

    pub fn cancel(&self) {
        self.cancel.set(true);
        // a conversion still under way: its worker ended, its folder removed (once it has answered, the page's end
        // sees the flag)
        #[cfg(feature = "converter")]
        if let Some((w, _, dir)) = &*self.worker.borrow()
            && self.end.borrow().is_none()
            && !self.answered.get()
        {
            self.answered.set(true);
            stop_worker(w);
            let (dir, end) = (dir.clone(), self.end.clone());
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(root) = web::root().await {
                    web::remove(&root, &dir).await;
                }
                *end.borrow_mut() = Some(Err(Failure::Cancelled));
            });
        }
    }

    /// The page goes: the copy is told to stop (a folder it leaves half written is cleared at the next start).
    pub fn stop(self) {
        self.cancel();
    }

    pub fn now(&self) -> Progress {
        self.progress.borrow().clone()
    }

    pub fn poll(&mut self) -> Option<Result<(), Failure>> {
        let end = self.end.borrow_mut().take()?;
        // the install over: the lock let go for another tab
        self.lock.borrow_mut().take();
        Some(end)
    }
}

fn io(path: &str, e: String) -> Failure {
    if e.contains("Quota") || e.contains("quota") { Failure::NoSpace(path.to_string()) } else { Failure::Io(format!("{path}: {e}")) }
}

/// The full check of a written folder (every file read back, its CRC-32 compared with `entries`), the quick check,
/// and the switch to it (a copy's and a conversion's end).
async fn finish(
    d: &web_sys::FileSystemDirectoryHandle,
    dir: &str,
    entries: &[FileEntry],
    total: u64,
    progress: &RefCell<Progress>,
    cancel: &Cell<bool>,
    kind: Kind,
) -> Result<(), Failure> {
    let set = |phase: Phase, done: u64, file: &str| *progress.borrow_mut() = Progress { phase, done, total, file: file.to_string() };
    // the full check: every file read back
    let mut back = 0u64;
    for e in entries {
        if cancel.get() {
            return Err(Failure::Cancelled);
        }
        set(Phase::Verifying, back, &e.path);
        let f = web::get(d, &e.path).await.map_err(|x| io(&e.path, x))?;
        let bytes = web::read_blob(&f).await.map_err(|x| io(&e.path, x))?;
        let mut c = Crc32::default();
        c.update(&bytes);
        if bytes.len() as u64 != e.size || c.value() != e.crc32 {
            return Err(Failure::Verify(e.path.clone()));
        }
        back += e.size;
    }
    if kind == Kind::Q3 {
        // the full check (the start's is a light one)
        let v = extras::check_opfs_q3(Some(dir), false).await.0;
        if !v.usable() {
            return Err(Failure::Io(v.note()));
        }
    } else if kind == Kind::Br1 {
        let v = manifest::check_opfs_br1(Some(dir)).await;
        if !v.usable() {
            return Err(Failure::Io(v.note()));
        }
    } else {
        let v = manifest::check_opfs(Some(dir)).await.map_err(Failure::Io)?;
        if !v.usable() {
            return Err(Failure::Io(v.reason().unwrap_or_default()));
        }
    }
    set(Phase::Installing, total, "");
    // the last moment a cancel keeps the old data (no await from here to the switch)
    if cancel.get() {
        return Err(Failure::Cancelled);
    }
    // Quake III Arena's maps: the folder they replaced before goes now (nothing reads it), the current one at the next
    // start (`web::forget_old`; the page reloads after an install over the game, `setup_done`)
    if kind == Kind::Q3
        && let Ok(root) = web::root().await
    {
        if cancel.get() {
            return Err(Failure::Cancelled);
        }
        web::Q3.forget_replaced(&root).await;
    }
    slot(kind).switch_to(Some(dir)).map_err(Failure::Io)?;
    // the replaced folder of the imported game goes now (another can replace this one before a reload)
    if kind == Kind::Br1
        && let Ok(root) = web::root().await
    {
        web::BR1.forget_replaced(&root).await;
    }
    if !web::persist().await {
        log::warn!("setup: the browser may delete the game data when space runs low (storage not persisted)");
    }
    Ok(())
}

async fn copy_into(pk: Picked, dir: &str, progress: &RefCell<Progress>, cancel: &Cell<bool>) -> Result<(), Failure> {
    let set = |phase: Phase, done: u64, total: u64, file: &str| *progress.borrow_mut() = Progress { phase, done, total, file: file.to_string() };
    let root = web::root().await.map_err(Failure::Io)?;
    web::remove(&root, dir).await;
    let d = web::subdir(&root, dir, true).await.map_err(|e| io(dir, e))?;
    let total: u64 = pk.items.iter().map(|i| i.size).sum();
    let r = async {
        let mut done = 0u64;
        let mut entries = Vec::new();
        for it in &pk.items {
            if cancel.get() {
                return Err(Failure::Cancelled);
            }
            set(Phase::Copying, done, total, &it.path);
            let f = pk.files.get(&it.path).ok_or_else(|| Failure::Io(format!("{} is missing", it.path)))?;
            // a size not the manifest's: not read (a Blob is read whole)
            if f.size() as u64 != it.size {
                return Err(Failure::Io(format!("{}: {} bytes, the manifest says {}", it.path, f.size() as u64, it.size)));
            }
            let bytes = web::read_blob(f).await.map_err(|e| io(&it.path, e))?;
            let crc = crc32(&bytes);
            if bytes.len() as u64 != it.size {
                return Err(Failure::Io(format!("{}: {} bytes, the manifest says {}", it.path, bytes.len(), it.size)));
            }
            if it.crc32.is_some_and(|c| c != crc) {
                return Err(Failure::Io(format!("{} in the chosen folder is damaged (its checksum differs from the manifest's)", it.path)));
            }
            web::write(&d, &it.path, &bytes).await.map_err(|e| io(&it.path, e))?;
            done += it.size;
            entries.push(FileEntry { path: it.path.clone(), size: it.size, crc32: crc });
        }
        finish(&d, dir, &entries, total, progress, cancel, pk.kind).await
    }
    .await;
    if r.is_err() {
        web::remove(&root, dir).await;
    }
    r
}

/// The worker's script (Trunk's loader of the `convert_worker` bin, beside the page).
#[cfg(feature = "converter")]
const WORKER_URL: &str = "./convert_worker_loader.js";
/// How long the worker may take to say it is ready (its wasm fetched and compiled; a failed fetch inside the
/// worker fires no event on the page).
#[cfg(feature = "converter")]
const READY_MS: i32 = 60_000;

/// A conversion worker (`worker.rs`) with the handler `on`, posted `msg` once it says it is ready (its wasm loads
/// after the script: a message before that would find no handler and be lost). `on` gets every later message, an
/// error event, a ready of another build (`worker::PROTOCOL`), or an empty message when it is not ready in
/// `READY_MS`; after anything but progress the worker is stopped.
#[cfg(feature = "converter")]
fn start_worker(
    msg: js_sys::Object,
    mut on: impl FnMut(web_sys::MessageEvent) + 'static,
) -> Result<(web_sys::Worker, Closure<dyn FnMut(web_sys::MessageEvent)>), String> {
    let w = web_sys::Worker::new(WORKER_URL).map_err(|e| format!("the conversion worker did not start: {e:?}"))?;
    let (w2, ready) = (w.clone(), Rc::new(Cell::new(false)));
    let r = ready.clone();
    let handler = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
        let data = ev.data();
        let k = kind(&data);
        if k == "ready" && !r.replace(true) && field(&data, "v").as_f64() == Some(super::worker::PROTOCOL as f64) {
            if let Err(e) = w2.post_message(&msg) {
                log::warn!("setup: the conversion worker cannot take the files: {e:?}");
            }
            return;
        }
        r.set(true);
        on(ev);
        if k != "p" {
            stop_worker(&w2);
        }
    });
    w.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    w.set_onerror(Some(handler.as_ref().unchecked_ref()));
    w.set_onmessageerror(Some(handler.as_ref().unchecked_ref()));
    // not ready in time: an empty message to the handler (none once the worker is stopped: its handler is unset)
    let w3 = w.clone();
    let late = Closure::once_into_js(move || {
        if !ready.get()
            && let Ok(ev) = web_sys::MessageEvent::new("message")
        {
            let _ = w3.dispatch_event(&ev);
        }
    });
    if let Some(win) = web_sys::window() {
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(late.unchecked_ref(), READY_MS);
    }
    Ok((w, handler))
}

/// A worker ended, its handlers unset (a late timer or event finds none).
#[cfg(feature = "converter")]
fn stop_worker(w: &web_sys::Worker) {
    w.terminate();
    w.set_onmessage(None);
    w.set_onerror(None);
    w.set_onmessageerror(None);
}

/// Why a worker's answer is not a result (`err`, a ready of another build, an error event, no answer in time).
#[cfg(feature = "converter")]
fn worker_failed(data: &wasm_bindgen::JsValue) -> String {
    match kind(data).as_str() {
        "err" => field(data, "msg").as_string().unwrap_or_default(),
        "ready" => "The conversion worker is of another version of the game: reload the page.".into(),
        _ => "The conversion worker failed (its script could not be loaded?).".into(),
    }
}

#[cfg(feature = "converter")]
fn message(mode: &str, files: &[web_sys::File], dir: &str, kind: Kind) -> js_sys::Object {
    let o = js_sys::Object::new();
    let arr: js_sys::Array = files.iter().cloned().map(wasm_bindgen::JsValue::from).collect();
    let game = match kind {
        Kind::Br1 => "br1",
        Kind::Q3 => "q3",
        Kind::Br2 => "br2",
    };
    for (k, v) in [("mode", mode.into()), ("files", arr.into()), ("dir", dir.into()), ("game", game.into())] {
        let _ = js_sys::Reflect::set(&o, &wasm_bindgen::JsValue::from_str(k), &v);
    }
    o
}

/// A worker's message: its kind (`ready`, `p`, `ok`, `err`; an error event has none) and a field getter.
#[cfg(feature = "converter")]
fn kind(data: &wasm_bindgen::JsValue) -> String {
    js_sys::Reflect::get(data, &"t".into()).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

#[cfg(feature = "converter")]
fn field(data: &wasm_bindgen::JsValue, k: &str) -> wasm_bindgen::JsValue {
    js_sys::Reflect::get(data, &k.into()).unwrap_or(wasm_bindgen::JsValue::UNDEFINED)
}

/// The `.pk3` files of a picked folder (`files` with their paths in it): those under a `baseq3` folder when there
/// are any, else all of them (Quake III Arena's maps; `worker::paks` reads them).
#[cfg(feature = "converter")]
fn pk3_files(files: Vec<(String, web_sys::File)>) -> Vec<web_sys::File> {
    let pk3: Vec<(String, web_sys::File)> = files.into_iter().filter(|(r, _)| r.to_ascii_lowercase().ends_with(".pk3")).collect();
    let base = |r: &str| r.to_ascii_lowercase().split('/').rev().nth(1) == Some("baseq3");
    let any_base = pk3.iter().any(|(r, _)| base(r));
    pk3.into_iter().filter(|(r, _)| !any_base || base(r)).map(|(_, f)| f).collect()
}

/// The picked disc image checked by a worker (it opens the image and reads the volume; Quake III Arena's maps: the
/// picked folder's `.pk3` files, their zip directories): its name and what was found.
#[cfg(feature = "converter")]
async fn check_disc(list: Option<web_sys::FileList>, game: Kind) -> (String, Picked) {
    let mut files: Vec<web_sys::File> = list.map(|l| (0..l.length()).filter_map(|k| l.get(k)).collect()).unwrap_or_default();
    let mut name =
        files.iter().map(|f| f.name()).find(|n| n.to_ascii_lowercase().ends_with(".cue")).or_else(|| files.first().map(|f| f.name())).unwrap_or_default();
    if game == Kind::Q3 {
        let rel = |f: &web_sys::File| js_sys::Reflect::get(f, &"webkitRelativePath".into()).ok().and_then(|v| v.as_string()).unwrap_or_else(|| f.name());
        name = files.first().map(|f| rel(f).split('/').next().unwrap_or("").to_string()).unwrap_or_default();
        files = pk3_files(files.into_iter().map(|f| (rel(&f), f)).collect());
        if files.is_empty() {
            let found = Err(format!("No .pk3 files in {name}: choose Quake III Arena's baseq3 folder (or the game's folder holding it)."));
            return (name, Picked { files: BTreeMap::new(), found, items: Vec::new(), disc: files, kind: game });
        }
    }
    let slot: Rc<RefCell<Option<js_sys::Function>>> = Rc::default();
    let promise = js_sys::Promise::new(&mut |resolve, _| *slot.borrow_mut() = Some(resolve));
    let s = slot.clone();
    let on = move |ev: web_sys::MessageEvent| {
        if let Some(f) = s.borrow_mut().take() {
            let _ = f.call1(&wasm_bindgen::JsValue::NULL, &ev.data());
        }
    };
    let found = match start_worker(message("check", &files, "", game), on) {
        Err(e) => Err(e),
        Ok((w, handler)) => {
            let data = wasm_bindgen_futures::JsFuture::from(promise).await.unwrap_or_default();
            stop_worker(&w);
            drop(handler);
            match kind(&data).as_str() {
                "ok" => {
                    let source = field(&data, "source").as_string().unwrap_or_default();
                    let (what, warn, bytes) = match game {
                        Kind::Br2 => {
                            let (w, x, b) = plan::disc_found(&source);
                            (w.to_string(), x, b)
                        }
                        Kind::Br1 => {
                            let (w, x, b) = plan::br1_disc_found(&source, field(&data, "cdda").as_bool().unwrap_or(false));
                            (w.to_string(), x, b)
                        }
                        Kind::Q3 => {
                            let maps: Vec<String> = serde_json::from_str(&field(&data, "maps").as_string().unwrap_or_default()).unwrap_or_default();
                            plan::q3_found(field(&data, "paks").as_f64().unwrap_or(0.0) as usize, &maps)
                        }
                    };
                    match web::estimate().await {
                        Some((usage, quota)) if bytes as f64 * 1.1 > quota - usage => Err(format!(
                            "Not enough storage for this site: the game data needs about {} MB, the browser allows {} MB more.",
                            bytes >> 20,
                            ((quota - usage).max(0.0) as u64) >> 20
                        )),
                        _ => Ok(Found { path: PathBuf::from(&name), what, bytes, warn }),
                    }
                }
                _ => Err(worker_failed(&data)),
            }
        }
    };
    (name, Picked { files: BTreeMap::new(), found, items: Vec::new(), disc: files, kind: game })
}

/// The conversion of `files` into the OPFS folder `dir` by a worker: its progress into `progress`; at its end
/// (`answered` set) the full check and the switch (`finish`, seeing `cancel`), the result into `end`.
#[cfg(feature = "converter")]
fn convert(
    files: &[web_sys::File],
    dir: &str,
    progress: &Rc<RefCell<Progress>>,
    end: &Rc<RefCell<Option<Result<(), Failure>>>>,
    cancel: &Rc<Cell<bool>>,
    answered: &Rc<Cell<bool>>,
    game: Kind,
) -> Result<(web_sys::Worker, Closure<dyn FnMut(web_sys::MessageEvent)>), String> {
    let (p, e, d, c, a) = (progress.clone(), end.clone(), dir.to_string(), cancel.clone(), answered.clone());
    let on = move |ev: web_sys::MessageEvent| {
        if e.borrow().is_some() || a.get() {
            return;
        }
        let data = ev.data();
        let num = |k: &str| field(&data, k).as_f64().unwrap_or(0.0) as u64;
        match kind(&data).as_str() {
            "p" => {
                *p.borrow_mut() =
                    Progress { phase: Phase::Converting, done: num("done"), total: num("total"), file: field(&data, "step").as_string().unwrap_or_default() }
            }
            "ok" => {
                a.set(true);
                // the files as the worker wrote them (an answer not read, or no file: a failure, never an install
                // without the full check)
                let list: Option<Vec<(String, u64, u32)>> = serde_json::from_str(&field(&data, "files").as_string().unwrap_or_default()).ok();
                let entries: Option<Vec<FileEntry>> =
                    list.filter(|l| !l.is_empty()).map(|l| l.into_iter().map(|(path, size, crc32)| FileEntry { path, size, crc32 }).collect());
                let (p, e, d, c) = (p.clone(), e.clone(), d.clone(), c.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let r = async {
                        let root = web::root().await.map_err(Failure::Io)?;
                        let r = match entries {
                            None => Err(Failure::Io("the conversion worker's answer cannot be read".into())),
                            Some(entries) => {
                                let total = entries.iter().map(|f| f.size).sum();
                                match web::subdir(&root, &d, false).await {
                                    Ok(dh) => finish(&dh, &d, &entries, total, &p, &c, game).await,
                                    Err(x) => Err(Failure::Io(x)),
                                }
                            }
                        };
                        if r.is_err() {
                            web::remove(&root, &d).await;
                        }
                        r
                    }
                    .await;
                    *e.borrow_mut() = Some(r);
                });
            }
            _ => {
                a.set(true);
                let msg = worker_failed(&data);
                let failure = if msg.contains("Quota") || msg.contains("full") { Failure::NoSpace(msg) } else { Failure::Io(msg) };
                let (e, d, c) = (e.clone(), d.clone(), c.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(root) = web::root().await {
                        web::remove(&root, &d).await;
                    }
                    *e.borrow_mut() = Some(Err(if c.get() { Failure::Cancelled } else { failure }));
                });
            }
        }
    };
    start_worker(message("convert", files, dir, game), on)
}
