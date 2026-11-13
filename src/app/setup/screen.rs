//! The setup screen's pages and their input: a pure state machine, tested without a window (engine addition).
//!
//! `Setup` holds the page (`Page`), the cursor, the found sources, the typed path and the latest progress; `rows`
//! lists what can be chosen on the page; `pad`, `key`, `text`, `click` and `dropped` turn input into a `Cmd` for the
//! app (`setup/mod.rs`), which runs it (a check, a copy, the BROWSE helper, quit) and reports back through `found`,
//! `failed`, `progress` and `picked`.
//!
//! The optional installs (`Kind::EXTRAS`: the imported game's disc, Quake III Arena's maps): with usable data of the main game
//! (`br1` / `q3` set: over the game, or right after an install at start, `Page::Done`) the intro offers ADD (or
//! REPLACE and REMOVE) BLOODY ROAR 1 DISC and QUAKE III ARENA MAPS; ADD (`Act::Add`) switches `kind` to it, whose
//! intro, pick, confirmation and job pages are the same pages asking for its data, and whose end (`br1_done`,
//! `q3_done`) comes back to where it was opened from.

pub use super::plan::Kind;
use crate::assets::extras::Q3Verdict;
use crate::assets::manifest::Br1Verdict;
use crate::fight::input::pad::{CIRCLE, CROSS, DOWN, START, TRIANGLE, UP};
use std::path::PathBuf;

/// A job's phase (`job`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Reading,
    Copying,
    /// the converter's steps (`done` / `total` count steps, not bytes)
    Converting,
    Verifying,
    Installing,
}

/// A job's progress: the phase, bytes done of the total, the file at hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
    pub file: String,
}

impl Default for Progress {
    fn default() -> Self {
        Self { phase: Phase::Reading, done: 0, total: 0, file: String::new() }
    }
}

/// A source the check found usable: what it is, its size, and a warning to confirm (none yet for a folder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    /// what was found, one line ("converted data, format 1, from a BIN/CUE disc image" …)
    pub what: String,
    /// the bytes the copy takes
    pub bytes: u64,
    pub warn: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    /// why the screen is up, and the choices
    Intro,
    /// a folder (or later a disc image) asked for: the found list, BROWSE, the typed path
    Pick,
    /// the source being checked (`job::inspect` on a thread): no rows
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Checking(PathBuf),
    /// a usable source: INSTALL or CHOOSE ANOTHER
    Confirm(Found),
    /// the copy under way (`Setup::progress`)
    Working,
    /// something went wrong: the message; TRY AGAIN runs `retry` again
    Error { msg: String, retry: Option<PathBuf> },
    /// The main game's data installed at start: START THE GAME, or add the imported game's disc first
    Done,
    /// REMOVE (an optional install) asked: confirm it
    Remove(Kind),
}

/// What a row does when chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    UseFolder,
    /// CONVERT DISC IMAGE (feature `converter`)
    UseDisc,
    Quit,
    /// back to the game the screen was opened from (OPTIONS → CHANGE GAME DATA)
    Close,
    /// a found source (index into `Setup::found`)
    Found(usize),
    Browse,
    /// the typed path's row: Enter checks it
    Typed,
    Back,
    Install,
    ChooseAnother,
    TryAgain,
    Cancel,
    /// ADD / REPLACE an optional install (BLOODY ROAR 1 DISC): its pages
    Add(Kind),
    /// REMOVE it (asks first), and its confirmation
    Remove(Kind),
    ConfirmRemove,
    /// START THE GAME (`Page::Done`)
    Start,
}

/// What the app must do after an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    None,
    Quit,
    /// close the screen, back to the game under it
    Close,
    /// the native folder picker (`pickers::browse`); its answer comes back through `picked`
    Browse,
    /// check this source (a folder or a file): `checking`, then `found` or `failed` with the answer
    Check(PathBuf),
    /// copy the confirmed source into the data folder: `progress`, then `failed` or the app goes on
    Install(PathBuf),
    /// stop the job (its end comes back through `failed`)
    Cancel,
    /// start the game with the data just installed (`Page::Done`)
    Start,
    /// remove an optional install's data (`removed` then)
    Remove(Kind),
}

/// At most this many characters of a reason or an error are shown (a few lines; a broken file's text could be
/// megabytes, one quad per glyph pixel run).
const MAX_TEXT: usize = 400;

pub struct Setup {
    /// why the screen is up (the quick check's verdict, or a start that failed)
    pub reason: String,
    pub page: Page,
    pub cursor: usize,
    /// the sources the scan found (`pickers::scan`): folders of converted data, disc images
    pub found: Vec<PathBuf>,
    pub discs: Vec<PathBuf>,
    /// CONVERT DISC IMAGE offered (the build has the converter)
    pub convert: bool,
    /// the pick page asks for a disc image (CONVERT DISC IMAGE), not a folder
    pub disc: bool,
    /// the typed path
    pub typed: String,
    /// a BROWSE helper exists (`pickers::can_browse`; Android: the system's folder picker)
    pub browse: bool,
    /// the typed path's row (not on Android, where a folder comes from the system's picker)
    pub path_row: bool,
    /// how the screen is left without data: QUIT (desktop, Android at start), BACK TO THE GAME (opened from the game's
    /// OPTIONS), or not at all (the browser at start)
    pub leave: Option<Act>,
    pub progress: Progress,
    /// a CANCEL was asked for: the job is stopping
    pub cancelling: bool,
    /// what the pages ask for: the main game's data, or (ADD BLOODY ROAR 1 DISC) the second disc's
    pub kind: Kind,
    /// The imported game's data's state, when it can be added (the main game's data is usable); None: not offered
    pub br1: Option<Br1Verdict>,
    /// Quake III Arena's maps' state, the same way (engine addition, the extra stages)
    pub q3: Option<Q3Verdict>,
    /// The main game's data was installed here at start (`Page::Done` is where the imported pages go back to)
    pub installed: bool,
    /// a REMOVE under way (the browser: a task): its page shows REMOVING... and takes no input
    pub removing: bool,
    /// the removal failed (why): its page offers only BACK
    pub remove_error: Option<String>,
}

impl Setup {
    pub fn new(reason: String, browse: bool, leave: Option<Act>) -> Self {
        Self {
            reason: crate::assets::manifest::clip(&reason, MAX_TEXT),
            page: Page::Intro,
            cursor: 0,
            found: Vec::new(),
            discs: Vec::new(),
            convert: false,
            disc: false,
            typed: String::new(),
            browse,
            path_row: !cfg!(any(target_os = "android", target_arch = "wasm32")),
            leave,
            progress: Progress::default(),
            cancelling: false,
            kind: Kind::Br2,
            br1: None,
            q3: None,
            installed: false,
            removing: false,
            remove_error: None,
        }
    }

    /// The page's rows: label and action.
    pub fn rows(&self) -> Vec<(String, Act)> {
        let mut v = Vec::new();
        match &self.page {
            Page::Intro if self.kind != Kind::Br2 => {
                if self.convert {
                    let label =
                        // at most 42 characters (test `intro_rows_fit_a_row`): a longer row is cut to its tail (`draw::fit`)
                        match self.kind {
                            Kind::Q3 => "CONVERT ITS .PK3 FILES (THE BASEQ3 FOLDER)",
                            // Android opens only the chosen file: never a .cue's .bin files (`convert::cue_on_android`)
                            _ if cfg!(target_os = "android") => "CONVERT ITS DISC (.BIN .CHD .ISO)",
                            _ => "CONVERT ITS DISC (.CUE .BIN .CHD .ISO)",
                        };
                    v.push((label.into(), Act::UseDisc));
                }
                v.push(("USE ITS CONVERTED DATA (A FOLDER)".into(), Act::UseFolder));
                v.push(("BACK".into(), Act::Back));
            }
            Page::Intro => {
                if self.convert {
                    let label = if cfg!(target_os = "android") { "CONVERT DISC IMAGE (.BIN .CHD .ISO)" } else { "CONVERT DISC IMAGE (.CUE .BIN .CHD .ISO)" };
                    v.push((label.into(), Act::UseDisc));
                }
                v.push(("USE CONVERTED GAME DATA (A FOLDER)".into(), Act::UseFolder));
                for k in Kind::EXTRAS {
                    v.extend(self.extra_rows(k));
                }
                v.extend(self.leave_row());
            }
            Page::Done => {
                v.push(("START THE GAME".into(), Act::Start));
                for k in Kind::EXTRAS {
                    v.extend(self.extra_rows(k));
                }
            }
            Page::Remove(_) if self.removing => {}
            Page::Remove(_) if self.remove_error.is_some() => v.push(("BACK".into(), Act::Back)),
            Page::Remove(k) => {
                v.push((format!("REMOVE THE {}", k.data().to_uppercase()), Act::ConfirmRemove));
                v.push(("BACK".into(), Act::Back));
            }
            Page::Pick => {
                for (k, p) in self.listed().iter().enumerate() {
                    v.push((p.display().to_string(), Act::Found(k)));
                }
                if self.browse {
                    // Quake III's paks are always picked as their folder (the browser's webkitdirectory input,
                    // Android's tree picker), though they are converted like a disc
                    let file = self.disc && self.kind != Kind::Q3;
                    let label = match (cfg!(any(target_os = "android", target_arch = "wasm32")), file) {
                        (true, true) => "CHOOSE FILE...",
                        (true, false) => "CHOOSE FOLDER...",
                        (false, _) => "BROWSE...",
                    };
                    v.push((label.into(), Act::Browse));
                }
                if self.path_row {
                    v.push((format!("PATH: {}", self.typed), Act::Typed));
                }
                v.push(("BACK".into(), Act::Back));
            }
            Page::Confirm(_) => {
                v.push(("INSTALL".into(), Act::Install));
                v.push(("CHOOSE ANOTHER".into(), Act::ChooseAnother));
            }
            Page::Checking(_) => {}
            Page::Working => v.push((if self.cancelling { "CANCELLING..." } else { "CANCEL" }.into(), Act::Cancel)),
            Page::Error { retry, .. } => {
                if retry.is_some() {
                    v.push(("TRY AGAIN".into(), Act::TryAgain));
                }
                v.push(("CHOOSE ANOTHER".into(), Act::ChooseAnother));
                v.extend(self.leave_row());
            }
        }
        v
    }

    /// The found sources the pick page lists: disc images or folders (the scan's are the imported game's: none for Quake III's
    /// maps).
    pub fn listed(&self) -> &[PathBuf] {
        match (self.kind, self.disc) {
            (Kind::Q3, _) => &[],
            (_, true) => &self.discs,
            (_, false) => &self.found,
        }
    }

    /// Whether the optional install `kind` is there (None: it cannot be added now).
    fn has(&self, kind: Kind) -> Option<bool> {
        match kind {
            Kind::Br2 => None,
            Kind::Br1 => self.br1.as_ref().map(|v| *v != Br1Verdict::Absent),
            Kind::Q3 => self.q3.as_ref().map(|v| *v != Q3Verdict::Absent),
        }
    }

    /// ADD (BLOODY ROAR 1 DISC …), or REPLACE and REMOVE once there is one (none while it cannot be added).
    fn extra_rows(&self, kind: Kind) -> Vec<(String, Act)> {
        let l = kind.label();
        match self.has(kind) {
            None => Vec::new(),
            Some(false) => vec![(if self.page == Page::Done { format!("ADD {l} (OPTIONAL)") } else { format!("ADD {l}") }, Act::Add(kind))],
            Some(true) => vec![(format!("REPLACE {l}"), Act::Add(kind)), (format!("REMOVE {l}"), Act::Remove(kind))],
        }
    }

    /// Where an optional install's pages go back to: the intro, or the page after an install at start.
    fn home(&mut self) {
        self.kind = Kind::Br2;
        self.go(if self.installed { Page::Done } else { Page::Intro });
    }

    fn leave_row(&self) -> Option<(String, Act)> {
        match self.leave {
            Some(Act::Quit) => Some(("QUIT".into(), Act::Quit)),
            Some(Act::Close) => Some(("BACK TO THE GAME".into(), Act::Close)),
            _ => None,
        }
    }

    /// The cursor is on the typed path's row: the keyboard types into it.
    pub fn typing(&self) -> bool {
        self.page == Page::Pick && self.rows().get(self.cursor).is_some_and(|r| r.1 == Act::Typed)
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        self.cursor = 0;
    }

    /// The cursor onto row `k` (the last when past it), without choosing it: a tap on a row marked as having more
    /// rows above or below it (`draw`'s `^` / `v`) brings the next one into view this way. Nothing while a job, a
    /// check or a removal runs.
    pub fn reveal(&mut self, k: usize) {
        let n = self.rows().len();
        if n > 0 && !self.removing && !matches!(self.page, Page::Checking(_) | Page::Working) {
            self.cursor = k.min(n - 1);
        }
    }

    /// The cursor `rows` rows down (up when negative), stopping at the first and the last (a finger dragged over the
    /// rows, `Drag`).
    pub fn scroll(&mut self, rows: i32) {
        self.reveal((self.cursor as i64 + rows as i64).max(0) as usize);
    }

    /// The pick page, the cursor on its first row (the first found source, else BROWSE or the path).
    pub fn open_pick(&mut self) {
        self.go(Page::Pick);
    }

    /// The row `k` chosen.
    pub fn choose(&mut self, k: usize) -> Cmd {
        let Some((_, act)) = self.rows().into_iter().nth(k) else { return Cmd::None };
        self.cursor = k;
        match act {
            Act::UseFolder | Act::UseDisc => {
                self.disc = act == Act::UseDisc;
                self.open_pick();
                Cmd::None
            }
            Act::ChooseAnother => {
                self.open_pick();
                Cmd::None
            }
            Act::Quit => Cmd::Quit,
            Act::Close => Cmd::Close,
            Act::Found(i) => Cmd::Check(self.listed()[i].clone()),
            Act::Browse => Cmd::Browse,
            Act::Typed if self.typed.trim().is_empty() => Cmd::None,
            Act::Typed => Cmd::Check(PathBuf::from(expand_home(self.typed.trim()))),
            // BACK from an optional install's intro or the removal question: where they were opened from
            Act::Back if matches!(self.page, Page::Remove(_)) || (self.page == Page::Intro && self.kind != Kind::Br2) => {
                self.home();
                Cmd::None
            }
            Act::Back => {
                self.pick_back();
                Cmd::None
            }
            Act::Add(k) => {
                self.kind = k;
                self.go(Page::Intro);
                Cmd::None
            }
            Act::Remove(k) => {
                self.remove_error = None;
                self.go(Page::Remove(k));
                Cmd::None
            }
            Act::ConfirmRemove => match self.page {
                Page::Remove(k) => {
                    self.removing = true;
                    Cmd::Remove(k)
                }
                _ => Cmd::None,
            },
            Act::Start => Cmd::Start,
            Act::Install => match &self.page {
                Page::Confirm(f) => {
                    let p = f.path.clone();
                    self.start_work();
                    Cmd::Install(p)
                }
                _ => Cmd::None,
            },
            Act::TryAgain => match &self.page {
                Page::Error { retry: Some(p), .. } => {
                    let p = p.clone();
                    Cmd::Check(p)
                }
                _ => Cmd::None,
            },
            Act::Cancel if self.cancelling => Cmd::None,
            Act::Cancel => {
                self.cancelling = true;
                Cmd::Cancel
            }
        }
    }

    /// Back from the pick page: its intro (the main game's after an install at start: the done page).
    fn pick_back(&mut self) {
        if self.kind == Kind::Br2 { self.home() } else { self.go(Page::Intro) }
    }

    fn start_work(&mut self) {
        self.cancelling = false;
        self.progress = Progress::default();
        self.go(Page::Working);
    }

    /// Back one page (triangle, Escape): from the pick page to the intro, from a confirmation or an error to the
    /// pick page; the intro and a running job stay. On Escape (`esc`): quit from the intro and from the done page
    /// (the data stays installed), CANCEL a running job.
    pub fn back(&mut self, esc: bool) -> Cmd {
        match self.page {
            Page::Remove(_) if self.removing => Cmd::None,
            Page::Remove(_) => {
                self.home();
                Cmd::None
            }
            Page::Intro if self.kind != Kind::Br2 => {
                self.home();
                Cmd::None
            }
            Page::Intro | Page::Done if esc && self.leave == Some(Act::Quit) => Cmd::Quit,
            Page::Intro if self.leave == Some(Act::Close) => Cmd::Close,
            Page::Done => Cmd::None,
            Page::Working if esc => self.choose(0),
            Page::Intro | Page::Checking(_) | Page::Working => Cmd::None,
            Page::Pick => {
                self.pick_back();
                Cmd::None
            }
            Page::Confirm(_) | Page::Error { .. } => {
                self.open_pick();
                Cmd::None
            }
        }
    }

    /// The pad buttons pressed (or repeated) this frame: up / down move, × ○ START choose, △ goes back.
    pub fn pad(&mut self, press: u16) -> Cmd {
        let n = self.rows().len();
        if press & UP != 0 && n > 0 {
            self.cursor = (self.cursor + n - 1) % n;
        }
        if press & DOWN != 0 && n > 0 {
            self.cursor = (self.cursor + 1) % n;
        }
        if press & (CROSS | CIRCLE | START) != 0 {
            return self.choose(self.cursor);
        }
        if press & TRIANGLE != 0 {
            return self.back(false);
        }
        Cmd::None
    }

    /// Typed text into the path field (only while `typing`).
    pub fn text(&mut self, s: &str) {
        if self.typing() {
            self.typed.extend(s.chars().filter(|c| !c.is_control()));
        }
    }

    /// Backspace in the path field.
    pub fn erase(&mut self) {
        if self.typing() {
            self.typed.pop();
        }
    }

    /// A file or folder dropped onto the window (or OKB_SETUP_FROM): checked, unless a check or a job runs.
    pub fn dropped(&mut self, p: PathBuf) -> Cmd {
        if matches!(self.page, Page::Checking(_) | Page::Working) {
            return Cmd::None;
        }
        Cmd::Check(p)
    }

    /// The BROWSE helper's answer: a path to check, or nothing (cancelled).
    pub fn picked(&mut self, p: Option<PathBuf>) -> Cmd {
        match p {
            Some(p) => self.dropped(p),
            None => Cmd::None,
        }
    }

    /// The scan's sources: on the pick page the cursor goes to the first of them, unless the path field is being
    /// typed into (it stays there: the next keys are still text).
    pub fn set_found(&mut self, (found, discs): (Vec<PathBuf>, Vec<PathBuf>)) {
        let typing = self.typing();
        (self.found, self.discs) = (found, discs);
        if typing {
            self.cursor = self.rows().iter().position(|r| r.1 == Act::Typed).unwrap_or(0);
        } else if self.page == Page::Pick {
            self.cursor = 0;
        }
    }

    /// The check of `p` started (`Cmd::Check`): its answer comes through `found` or `failed`.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn checking(&mut self, p: PathBuf) {
        self.go(Page::Checking(p));
    }

    /// A checked source is usable.
    pub fn found(&mut self, f: Found) {
        self.go(Page::Confirm(f));
    }

    /// A check or a job failed (or a job was cancelled): `retry` is what TRY AGAIN checks again.
    pub fn failed(&mut self, msg: String, retry: Option<PathBuf>) {
        self.cancelling = false;
        self.go(Page::Error { msg: crate::assets::manifest::clip(&msg, MAX_TEXT), retry });
    }

    /// The main game's data installed at start: `Page::Done`, with the imported game's data's state (`br1`).
    pub fn br2_done(&mut self, br1: Br1Verdict) {
        (self.installed, self.br1) = (true, Some(br1));
        self.home();
    }

    /// The imported game's install finished (its quick check `v`): back where its pages were opened from.
    pub fn br1_done(&mut self, v: Br1Verdict) {
        self.br1 = Some(v);
        self.home();
    }

    /// Quake III Arena's maps installed (their quick check `v`): back where their pages were opened from.
    pub fn q3_done(&mut self, v: Q3Verdict) {
        self.q3 = Some(v);
        self.home();
    }

    /// The optional install `kind`'s data removed (or why not).
    pub fn removed(&mut self, kind: Kind, r: Result<(), String>) {
        self.removing = false;
        let here = self.page == Page::Remove(kind);
        match r {
            Ok(()) => {
                match kind {
                    Kind::Br2 => {}
                    Kind::Br1 => self.br1 = Some(Br1Verdict::Absent),
                    Kind::Q3 => self.q3 = Some(Q3Verdict::Absent),
                }
                if here {
                    self.home();
                }
            }
            // only BACK (to where it was opened from), not the main game's pick page
            Err(e) if here => {
                self.remove_error = Some(format!("The {} could not be removed: {e}", kind.data()));
                self.cursor = 0;
            }
            Err(_) => {}
        }
    }
}

/// What a finger did on the setup screen (`Drag::event`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// it came down and went up in place: the row under this point is chosen
    Tap([f32; 2]),
    /// it was dragged by this many rows (up: positive, the rows below come into view)
    Rows(i32),
}

/// One finger on the setup screen (touch screens; a second finger is ignored while it is down): a tap where it came
/// down when it goes up without having moved half a row, else a drag that scrolls a row per row's height moved.
#[derive(Default)]
pub struct Drag {
    /// the finger's id, where it came down and where the last whole row was counted
    down: Option<(i32, [f32; 2], f32)>,
    dragged: bool,
}

impl Drag {
    /// A touch event (`phase` 0 down, 1 move, 2 up, 3 cancel; `row_h` a row's height in the same pixels).
    pub fn event(&mut self, phase: u8, id: i32, p: [f32; 2], row_h: f32) -> Option<Gesture> {
        let row_h = row_h.max(1.0);
        match phase {
            // a new finger, or the same id again: its up / cancel was lost (a dropped ACTION_UP; Android reuses ids),
            // so a stale finger does not block touch there; opening a native picker also resets the drag
            // (`setup_cmd`, for the web's ever-new ids); a second finger is ignored
            0 if self.down.is_none_or(|d| d.0 == id) => {
                (self.down, self.dragged) = (Some((id, p, p[1])), false);
                None
            }
            1 => {
                let (fid, at, last) = self.down.as_mut()?;
                if *fid != id {
                    return None;
                }
                self.dragged |= (p[1] - at[1]).abs() >= row_h / 2.0;
                let n = ((*last - p[1]) / row_h) as i32;
                if n == 0 {
                    return None;
                }
                *last -= n as f32 * row_h;
                Some(Gesture::Rows(n))
            }
            2 if self.down.is_some_and(|d| d.0 == id) => {
                let (_, at, _) = self.down.take()?;
                (!self.dragged).then_some(Gesture::Tap(at))
            }
            3 if self.down.is_some_and(|d| d.0 == id) => {
                self.down = None;
                None
            }
            _ => None,
        }
    }
}

/// `~/…` → the home folder's path (a typed path).
fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))) {
        (Some(rest), Ok(h)) => format!("{h}/{rest}"),
        _ => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(p: &str) -> Found {
        Found { path: p.into(), what: "x".into(), bytes: 1, warn: None }
    }

    #[test]
    fn intro_open_pick_and_back() {
        let mut s = Setup::new("No game data yet.".into(), true, Some(Act::Quit));
        assert_eq!(s.rows().len(), 2);
        assert_eq!(s.pad(DOWN), Cmd::None);
        assert_eq!(s.pad(CROSS), Cmd::Quit);
        assert_eq!(s.pad(UP), Cmd::None);
        assert_eq!(s.pad(CROSS), Cmd::None);
        assert_eq!(s.page, Page::Pick);
        // no source found: BROWSE, the path, BACK
        assert_eq!(s.rows().iter().map(|r| r.1.clone()).collect::<Vec<_>>(), [Act::Browse, Act::Typed, Act::Back]);
        assert_eq!(s.pad(CROSS), Cmd::Browse);
        assert_eq!(s.picked(None), Cmd::None);
        assert_eq!(s.pad(TRIANGLE), Cmd::None);
        assert_eq!(s.page, Page::Intro);
        assert_eq!(s.back(true), Cmd::Quit);
    }

    #[test]
    fn typed_path_and_found_list() {
        let mut s = Setup::new(String::new(), false, None);
        s.found = vec!["/a/export".into()];
        s.open_pick();
        assert_eq!(s.pad(CROSS), Cmd::Check("/a/export".into()));
        s.text("x");
        assert_eq!(s.typed, "", "not on the path row");
        s.pad(DOWN);
        assert!(s.typing());
        // an empty path does nothing
        assert_eq!(s.pad(START), Cmd::None);
        s.text("/b/expory");
        s.erase();
        s.text("t\n");
        assert_eq!(s.typed, "/b/export");
        assert_eq!(s.choose(s.cursor), Cmd::Check("/b/export".into()));
        // no QUIT in the browser
        s.back(false);
        s.back(false);
        assert_eq!(s.page, Page::Intro);
        assert_eq!(s.rows().len(), 1);
        assert_eq!(s.back(true), Cmd::None);
    }

    #[test]
    fn confirm_work_cancel_error_retry() {
        let mut s = Setup::new(String::new(), false, Some(Act::Quit));
        s.open_pick();
        assert_eq!(s.dropped("/c".into()), Cmd::Check("/c".into()));
        // while it is checked: no rows, a drop or a back does nothing
        s.checking("/c".into());
        assert!(s.rows().is_empty());
        assert_eq!(s.dropped("/d".into()), Cmd::None);
        assert_eq!((s.pad(CROSS), s.back(true)), (Cmd::None, Cmd::None));
        s.found(found("/c"));
        assert_eq!(s.pad(CROSS), Cmd::Install("/c".into()));
        assert_eq!(s.page, Page::Working);
        // a drop or a back while working does nothing; CANCEL once
        assert_eq!(s.dropped("/d".into()), Cmd::None);
        assert_eq!(s.pad(TRIANGLE), Cmd::None);
        assert_eq!(s.pad(CROSS), Cmd::Cancel);
        assert_eq!(s.pad(CROSS), Cmd::None);
        assert_eq!(s.back(true), Cmd::None);
        assert_eq!(s.rows()[0].0, "CANCELLING...");
        s.failed("Cancelled.".into(), Some("/c".into()));
        assert!(!s.cancelling);
        assert_eq!(s.pad(CROSS), Cmd::Check("/c".into()));
        // CHOOSE ANOTHER, then QUIT
        s.pad(DOWN);
        assert_eq!(s.pad(CROSS), Cmd::None);
        assert_eq!(s.page, Page::Pick);
        s.failed("no".into(), None);
        assert_eq!(s.rows().iter().map(|r| r.1.clone()).collect::<Vec<_>>(), [Act::ChooseAnother, Act::Quit]);
        // a confirmation goes back to the pick page
        s.found(found("/e"));
        s.pad(DOWN);
        assert_eq!(s.pad(CROSS), Cmd::None);
        assert_eq!(s.page, Page::Pick);
    }

    #[test]
    fn opened_from_the_game() {
        let mut s = Setup::new(String::new(), false, Some(Act::Close));
        assert_eq!(s.rows()[1], ("BACK TO THE GAME".to_string(), Act::Close));
        // △ and Escape close it from the intro, a pick page goes back first
        assert_eq!(s.pad(TRIANGLE), Cmd::Close);
        assert_eq!(s.back(true), Cmd::Close);
        s.open_pick();
        assert_eq!(s.back(true), Cmd::None);
        s.failed("x".into(), None);
        assert_eq!(s.rows().last().unwrap().1, Act::Close);
    }

    #[test]
    fn scan_results_keep_the_path_field() {
        let mut s = Setup::new(String::new(), true, Some(Act::Quit));
        s.open_pick();
        s.pad(DOWN);
        assert!(s.typing());
        s.set_found((vec!["/a".into(), "/b".into()], Vec::new()));
        assert!(s.typing());
        s.text("s");
        assert_eq!(s.typed, "s");
        // on another row: the first found source
        s.pad(DOWN);
        s.set_found((vec!["/a".into()], Vec::new()));
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn escape_cancels_a_job() {
        let mut s = Setup::new(String::new(), false, Some(Act::Quit));
        s.found(found("/c"));
        assert_eq!(s.pad(CROSS), Cmd::Install("/c".into()));
        assert_eq!(s.back(false), Cmd::None);
        assert_eq!(s.back(true), Cmd::Cancel);
        assert!(s.cancelling && s.page == Page::Working);
        assert_eq!(s.back(true), Cmd::None);
    }

    #[test]
    fn convert_disc_image_lists_the_found_images() {
        let mut s = Setup::new(String::new(), true, Some(Act::Quit));
        assert_eq!(s.rows()[0].1, Act::UseFolder);
        s.convert = true;
        s.set_found((vec!["/folder".into()], vec!["/disc.cue".into(), "/disc.chd".into()]));
        assert_eq!(s.rows()[0].1, Act::UseDisc);
        assert_eq!(s.pad(CROSS), Cmd::None);
        assert!(s.disc && s.page == Page::Pick);
        assert_eq!(s.rows().len(), 5);
        s.pad(DOWN);
        assert_eq!(s.pad(CROSS), Cmd::Check("/disc.chd".into()));
        // CHOOSE ANOTHER keeps asking for an image; BACK and the folder row ask for a folder
        s.failed("wrong disc".into(), None);
        s.pad(CROSS);
        assert!(s.disc && s.rows()[0].0 == "/disc.cue");
        s.back(false);
        s.pad(DOWN);
        s.pad(CROSS);
        assert!(!s.disc && s.rows()[0].0 == "/folder");
    }

    #[test]
    fn the_br1_disc_pages() {
        // over the game, without it: ADD; its pages ask for it and come back
        let mut s = Setup::new(String::new(), true, Some(Act::Close));
        assert!(!s.rows().iter().any(|r| r.1 == Act::Add(Kind::Br1)), "not offered without usable BR2 data");
        s.br1 = Some(Br1Verdict::Absent);
        s.convert = true;
        assert_eq!(s.rows().iter().map(|r| r.1.clone()).collect::<Vec<_>>(), [Act::UseDisc, Act::UseFolder, Act::Add(Kind::Br1), Act::Close]);
        assert_eq!(s.choose(2), Cmd::None);
        assert_eq!((s.kind, &s.page), (Kind::Br1, &Page::Intro));
        assert!(s.rows()[0].0.starts_with("CONVERT ITS") && s.rows()[2].1 == Act::Back);
        assert_eq!(s.choose(0), Cmd::None);
        assert!(s.disc && s.page == Page::Pick && s.kind == Kind::Br1);
        s.back(false);
        assert_eq!((s.kind, &s.page), (Kind::Br1, &Page::Intro));
        assert_eq!(s.back(true), Cmd::None, "Escape on its intro goes back, not out");
        assert_eq!((s.kind, &s.page), (Kind::Br2, &Page::Intro));
        // installed: REPLACE and REMOVE (asked first)
        s.choose(2);
        s.found(found("/br1.cue"));
        assert_eq!(s.pad(CROSS), Cmd::Install("/br1.cue".into()));
        s.br1_done(Br1Verdict::Ok);
        assert_eq!((s.kind, &s.page), (Kind::Br2, &Page::Intro));
        let acts: Vec<Act> = s.rows().into_iter().map(|r| r.1).collect();
        assert_eq!(acts, [Act::UseDisc, Act::UseFolder, Act::Add(Kind::Br1), Act::Remove(Kind::Br1), Act::Close]);
        assert_eq!(s.choose(3), Cmd::None);
        assert_eq!(s.page, Page::Remove(Kind::Br1));
        assert_eq!(s.back(false), Cmd::None);
        assert_eq!(s.page, Page::Intro);
        s.choose(3);
        assert_eq!(s.choose(0), Cmd::Remove(Kind::Br1));
        // under way (the browser's task): no rows, no input, △ stays
        assert!(s.removing && s.rows().is_empty());
        assert_eq!((s.pad(CROSS), s.back(true)), (Cmd::None, Cmd::None));
        assert_eq!(s.page, Page::Remove(Kind::Br1));
        s.removed(Kind::Br1, Ok(()));
        assert_eq!((s.br1.clone(), &s.page, s.removing), (Some(Br1Verdict::Absent), &Page::Intro, false));
        // an answer arriving off its page changes no page
        s.removed(Kind::Br1, Err("busy".into()));
        assert_eq!(s.page, Page::Intro);
        // a failed removal: its message and only BACK, to where it was opened from
        s.br1 = Some(Br1Verdict::Ok);
        s.choose(3);
        s.choose(0);
        s.removed(Kind::Br1, Err("busy".into()));
        assert!(s.remove_error.as_ref().is_some_and(|m| m.contains("busy")) && !s.removing);
        assert_eq!(s.rows().iter().map(|r| r.1.clone()).collect::<Vec<_>>(), [Act::Back]);
        assert_eq!(s.choose(0), Cmd::None);
        assert_eq!((s.kind, &s.page, s.br1.clone()), (Kind::Br2, &Page::Intro, Some(Br1Verdict::Ok)));
        // asked again: the question, not the old error
        s.choose(3);
        assert!(s.remove_error.is_none() && s.rows()[0].1 == Act::ConfirmRemove);
    }

    #[test]
    fn done_at_start_offers_the_br1_disc() {
        let mut s = Setup::new("No game data yet.".into(), true, Some(Act::Quit));
        s.br2_done(Br1Verdict::Absent);
        assert_eq!(s.page, Page::Done);
        assert_eq!(s.rows().iter().map(|r| r.1.clone()).collect::<Vec<_>>(), [Act::Start, Act::Add(Kind::Br1)]);
        assert!(s.rows()[1].0.contains("OPTIONAL"));
        // △ stays; Escape quits as from the intro (the data stays installed)
        assert_eq!((s.back(false), s.pad(TRIANGLE), s.back(true)), (Cmd::None, Cmd::None, Cmd::Quit));
        assert_eq!(s.choose(1), Cmd::None);
        assert_eq!(s.kind, Kind::Br1);
        // BACK from its intro: the done page again; installed: START
        s.choose(1);
        s.back(false);
        s.back(false);
        assert_eq!(s.page, Page::Done);
        s.choose(1);
        s.br1_done(Br1Verdict::NoMusic);
        assert_eq!((s.kind, &s.page), (Kind::Br2, &Page::Done));
        // a removal that failed: BACK (the row or △) leads to the done page again
        let remove = s.rows().iter().position(|r| r.1 == Act::Remove(Kind::Br1)).unwrap();
        s.choose(remove);
        s.choose(0);
        s.removed(Kind::Br1, Err("busy".into()));
        assert_eq!(s.page, Page::Remove(Kind::Br1));
        s.choose(0);
        assert_eq!((s.kind, &s.page), (Kind::Br2, &Page::Done));
        s.choose(remove);
        s.choose(0);
        s.removed(Kind::Br1, Err("busy".into()));
        s.back(false);
        assert_eq!(s.page, Page::Done);
        assert_eq!(s.choose(0), Cmd::Start);
    }

    /// Every intro row of every kind fits a row, so none is cut to its tail.
    #[test]
    fn intro_rows_fit_a_row() {
        // the widest row shown whole in the browser at 1512×782 (draw::fit cuts a longer one to its tail)
        const ROW_MAX: usize = 42;
        for kind in [Kind::Br2, Kind::Br1, Kind::Q3] {
            let mut s = Setup::new(String::new(), true, Some(Act::Close));
            (s.convert, s.kind) = (true, kind);
            (s.br1, s.q3) = (Some(Br1Verdict::Ok), Some(Q3Verdict::Ok));
            for (label, _) in s.rows() {
                assert!(label.chars().count() <= ROW_MAX, "{kind:?}: {label} is longer than {ROW_MAX}");
            }
        }
    }

    /// (Engine addition) Quake III Arena's maps: offered beside the imported game's disc, its own intro (CONVERT ITS .PK3
    /// FILES), no scanned sources, REPLACE and REMOVE once installed; a removal or an install of one kind leaves the
    /// other's state alone.
    #[test]
    fn the_q3_maps_pages() {
        let mut s = Setup::new(String::new(), true, Some(Act::Close));
        s.convert = true;
        s.set_found((vec!["/export".into()], vec!["/disc.cue".into()]));
        (s.br1, s.q3) = (Some(Br1Verdict::Ok), Some(Q3Verdict::Absent));
        let acts: Vec<Act> = s.rows().into_iter().map(|r| r.1).collect();
        assert_eq!(acts, [Act::UseDisc, Act::UseFolder, Act::Add(Kind::Br1), Act::Remove(Kind::Br1), Act::Add(Kind::Q3), Act::Close]);
        assert_eq!(s.rows()[4].0, "ADD QUAKE III ARENA MAPS");
        assert_eq!(s.choose(4), Cmd::None);
        assert_eq!((s.kind, &s.page), (Kind::Q3, &Page::Intro));
        let rows = s.rows();
        assert_eq!(rows.iter().map(|r| r.1.clone()).collect::<Vec<_>>(), [Act::UseDisc, Act::UseFolder, Act::Back]);
        assert!(rows[0].0.contains(".PK3") && rows[1].0.starts_with("USE ITS CONVERTED"));
        // the pick page lists none of the scan's (the imported game's) sources, for the paks or a folder
        s.choose(0);
        assert!(s.disc && s.page == Page::Pick && s.listed().is_empty());
        assert_eq!(s.rows()[0].1, Act::Browse);
        // a folder is picked, never a file
        assert!(!s.rows()[0].0.contains("FILE"), "{}", s.rows()[0].0);
        s.found(found("/q3/baseq3"));
        assert_eq!(s.pad(CROSS), Cmd::Install("/q3/baseq3".into()));
        s.q3_done(Q3Verdict::Ok);
        assert_eq!((s.kind, &s.page), (Kind::Br2, &Page::Intro));
        let acts: Vec<Act> = s.rows().into_iter().map(|r| r.1).collect();
        assert_eq!(acts[4..], [Act::Add(Kind::Q3), Act::Remove(Kind::Q3), Act::Close]);
        assert_eq!(s.rows()[4].0, "REPLACE QUAKE III ARENA MAPS");
        // REMOVE asks, names the maps; removed: ADD again, the imported game's disc stays
        assert_eq!(s.choose(5), Cmd::None);
        assert_eq!(s.page, Page::Remove(Kind::Q3));
        assert_eq!(s.rows()[0].0, "REMOVE THE QUAKE III ARENA MAPS");
        assert_eq!(s.choose(0), Cmd::Remove(Kind::Q3));
        s.removed(Kind::Q3, Err("busy".into()));
        assert!(s.remove_error.as_ref().is_some_and(|m| m.starts_with("The Quake III Arena maps could not be removed")));
        s.choose(0);
        s.choose(5);
        s.choose(0);
        s.removed(Kind::Q3, Ok(()));
        assert_eq!((s.q3.clone(), s.br1.clone(), &s.page), (Some(Q3Verdict::Absent), Some(Br1Verdict::Ok), &Page::Intro));
        // after an install at start: both offered, marked optional
        let mut s = Setup::new(String::new(), true, Some(Act::Quit));
        s.br2_done(Br1Verdict::Absent);
        s.q3 = Some(Q3Verdict::Absent);
        let labels: Vec<String> = s.rows().into_iter().map(|r| r.0).collect();
        assert_eq!(labels, ["START THE GAME", "ADD BLOODY ROAR 1 DISC (OPTIONAL)", "ADD QUAKE III ARENA MAPS (OPTIONAL)"]);
    }

    /// (Engine addition) A tap on a row marked `v` / `^` reveals the next row instead of choosing it; a drag moves the
    /// cursor a row per row's height and does not tap; a short wobble is a tap where the finger came down.
    #[test]
    fn reveal_scroll_and_drag() {
        let mut s = Setup::new(String::new(), true, Some(Act::Close));
        (s.convert, s.br1, s.q3) = (true, Some(Br1Verdict::Ok), Some(Q3Verdict::Absent));
        let n = s.rows().len();
        s.reveal(4);
        assert_eq!((s.cursor, &s.page), (4, &Page::Intro));
        s.reveal(99);
        assert_eq!(s.cursor, n - 1);
        s.scroll(-2);
        assert_eq!(s.cursor, n - 3);
        s.scroll(-99);
        assert_eq!(s.cursor, 0);
        s.scroll(2);
        assert_eq!(s.cursor, 2);
        // not while a job runs
        s.found(found("/x"));
        s.pad(CROSS);
        s.reveal(1);
        assert_eq!((s.cursor, &s.page), (0, &Page::Working));
        let mut d = Drag::default();
        assert_eq!(d.event(0, 7, [100.0, 300.0], 40.0), None);
        assert_eq!(d.event(1, 7, [100.0, 310.0], 40.0), None);
        assert_eq!(d.event(2, 7, [100.0, 310.0], 40.0), Some(Gesture::Tap([100.0, 300.0])));
        d.event(0, 7, [100.0, 300.0], 40.0);
        // a second finger meanwhile: ignored
        assert_eq!(d.event(0, 8, [10.0, 10.0], 40.0), None);
        assert_eq!(d.event(1, 8, [10.0, 200.0], 40.0), None);
        assert_eq!(d.event(1, 7, [100.0, 250.0], 40.0), Some(Gesture::Rows(1)));
        assert_eq!(d.event(1, 7, [100.0, 170.0], 40.0), Some(Gesture::Rows(2)));
        assert_eq!(d.event(1, 7, [100.0, 260.0], 40.0), Some(Gesture::Rows(-2)));
        assert_eq!(d.event(2, 7, [100.0, 260.0], 40.0), None, "a drag is no tap");
        d.event(0, 7, [1.0, 1.0], 40.0);
        assert_eq!(d.event(3, 7, [1.0, 1.0], 40.0), None);
        assert_eq!(d.event(2, 7, [1.0, 1.0], 40.0), None);
        // a lost up: the next down of the same id starts a new gesture (a stale finger never blocks touch), while a
        // second finger's down is still ignored
        d.event(0, 0, [50.0, 50.0], 40.0);
        assert_eq!(d.event(0, 1, [9.0, 9.0], 40.0), None);
        d.event(0, 0, [60.0, 70.0], 40.0);
        assert_eq!(d.event(2, 0, [60.0, 70.0], 40.0), Some(Gesture::Tap([60.0, 70.0])));
    }

    #[test]
    fn long_texts_are_cut() {
        let mut s = Setup::new("x".repeat(5000), false, None);
        assert!(s.reason.len() <= MAX_TEXT + 3);
        s.failed("y".repeat(1 << 20), None);
        assert!(matches!(&s.page, Page::Error { msg, .. } if msg.len() <= MAX_TEXT + 3));
    }

    #[test]
    fn home_expansion() {
        if let Ok(h) = std::env::var("HOME") {
            assert_eq!(expand_home("~/x"), format!("{h}/x"));
        }
        assert_eq!(expand_home("/x"), "/x");
    }
}
