//! Android services: the activity, the app's storage, the input the Java activity hands over, its calls.
//!
//! Engine addition.
//!
//! Android (engine addition, `docs/android.md`): the `AndroidApp` of `android_main` (`set_app`), the app's private
//! files folder (`files_dir`; the game data folder in it is `platform::game_data`'s), the input the Java activity
//! (`android/java/com/openkumitebeast/game/OkbActivity.java`) forwards through `nativeInput` (`take_input`), and the calls into
//! it: the system's folder picker (`pick_folder`, its answer through `nativePicked`, `take_picked`), a picked tree's
//! files (`list_tree`, `open_document`), the free space, `restart`, `quit`, `show_fatal`.
#![allow(unsafe_code)] // the JNI boundary: the exported callbacks and the raw VM/activity handles from android-activity

use std::path::PathBuf;
use std::sync::Mutex;
use winit::platform::android::activity::AndroidApp;

static APP: Mutex<Option<AndroidApp>> = Mutex::new(None);

/// `android_main`'s app (an activity created again in the same process replaces it).
pub fn set_app(app: AndroidApp) {
    *APP.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
}

pub fn app() -> Option<AndroidApp> {
    APP.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The app's private files folder (`Context.getFilesDir()`).
pub fn files_dir() -> Option<PathBuf> {
    app().and_then(|a| a.internal_data_path())
}

/// What the activity forwards (`nativeInput(kind, a, b, x, y, z, w, u, v)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// kind 0: a finger (`phase` 0 down, 1 move, 2 up, 3 cancel; `id` the pointer id), window pixels
    Touch { phase: u8, id: i32, x: f32, y: f32 },
    /// kind 1: a key (`KeyEvent.getKeyCode()`); `pad`: from a gamepad or joystick (else a keyboard); `dev` the
    /// `InputDevice` id
    Key { code: i32, down: bool, pad: bool, dev: i32 },
    /// kind 2: a joystick's axes: left stick x / y, hat x / y, left / right trigger; `dev` the `InputDevice` id
    Axes { dev: i32, axes: [f32; 6] },
    /// kind 3: the safe insets (left, top, right, bottom), window pixels
    Insets([f32; 4]),
    /// kind 4: an input device went away
    Removed(i32),
}

static INPUT: Mutex<Vec<Input>> = Mutex::new(Vec::new());

/// The input forwarded since the last call, oldest first.
pub fn take_input() -> Vec<Input> {
    std::mem::take(&mut *INPUT.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `OkbActivity.nativeInput`: called on the UI thread for every input event the activity forwards.
#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_openkumitebeast_game_OkbActivity_nativeInput(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
    kind: jni::sys::jint,
    a: jni::sys::jint,
    b: jni::sys::jint,
    dev: jni::sys::jint,
    x: jni::sys::jfloat,
    y: jni::sys::jfloat,
    z: jni::sys::jfloat,
    w: jni::sys::jfloat,
    u: jni::sys::jfloat,
    v: jni::sys::jfloat,
) {
    let ev = match kind {
        0 => Input::Touch { phase: a as u8, id: b, x, y },
        1 => Input::Key { code: a, down: b & 1 != 0, pad: b & 2 != 0, dev },
        2 => Input::Axes { dev, axes: [x, y, z, w, u, v] },
        3 => Input::Insets([x, y, z, w]),
        4 => Input::Removed(dev),
        _ => return,
    };
    push_bounded(&mut INPUT.lock().unwrap_or_else(|e| e.into_inner()), ev);
}

/// The input queue's length past which an event makes room for itself.
const MAX_QUEUE: usize = 4096;

/// `ev` onto the queue. Nothing drains it while the native side waits (a long load): when it is full the oldest
/// touch, key or axes event goes (the newest input matters); a device gone (`Removed`) and the insets are never
/// dropped, as a lost one would leave a gamepad's player taken or the layout wrong for good.
fn push_bounded(q: &mut Vec<Input>, ev: Input) {
    if q.len() >= MAX_QUEUE {
        let kept = |e: &Input| matches!(e, Input::Removed(_) | Input::Insets(_));
        match q.iter().position(|e| !kept(e)) {
            Some(k) => {
                q.remove(k);
            }
            // all of them kept ones (never in practice): a new one of those still goes in, another is dropped
            None if !kept(&ev) => return,
            None => {}
        }
    }
    q.push(ev);
}

/// The launch intent's debug hooks (`OkbActivity.nativeEnv`: its `okb_env` extra (an earlier build's `br2_env` too), `KEY=value` pairs split by `;`).
static ENV: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// `OkbActivity.nativeEnv`: called in `onCreate` before the native side starts.
#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_openkumitebeast_game_OkbActivity_nativeEnv(mut env: jni::JNIEnv, _class: jni::objects::JClass, text: jni::objects::JString) {
    let Ok(text) = env.get_string(&text).map(String::from) else { return };
    let vars = text.split(';').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.to_string())).collect();
    *ENV.lock().unwrap_or_else(|e| e.into_inner()) = vars;
}

/// A debug hook given by the launch intent (`app::debug_env`; Android has no environment).
pub fn debug_var(k: &str) -> Option<String> {
    ENV.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
}

/// The folder picker's answer (`OkbActivity.nativePicked`): Some(None) cancelled, Some(Some(uri)) a tree.
static PICKED: Mutex<Option<Option<String>>> = Mutex::new(None);

/// `OkbActivity.nativePicked`: the folder picker closed with a tree's URI, or null (cancelled).
#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_openkumitebeast_game_OkbActivity_nativePicked(mut env: jni::JNIEnv, _class: jni::objects::JClass, uri: jni::objects::JString) {
    let picked = if uri.is_null() { None } else { env.get_string(&uri).ok().map(String::from) };
    *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(picked);
}

/// The folder picker's answer once it has closed (taken once): the tree's URI, or None (cancelled).
pub fn take_picked() -> Option<Option<String>> {
    PICKED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// `f` with the activity and a JNI env of this thread (any native thread: the game's, the setup's copy;
/// `OkbActivity` runs its UI work on its own thread). A Java exception it leaves (no such method …) is cleared: it
/// must not stay pending on this thread.
fn with_activity<R>(f: impl FnOnce(&mut jni::JNIEnv, &jni::objects::JObject) -> jni::errors::Result<R>) -> Result<R, String> {
    let app = app().ok_or("no activity")?;
    // SAFETY: android-activity keeps the process's JavaVM and a global reference to the activity (not the
    // application context, which is what ndk-context holds) for as long as `android_main` runs
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
    // SAFETY: as above; the global reference is not deleted here (JObject has no Drop)
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let r = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    r.map_err(|e| e.to_string())
}

/// A method of the activity taking strings (`(Ljava/lang/String;…)` + `ret`), its result given to `then` in the
/// call's JNI frame.
fn call<R>(
    method: &str,
    args: &[&str],
    ret: &str,
    then: impl FnOnce(&mut jni::JNIEnv, jni::objects::JValueOwned) -> jni::errors::Result<R>,
) -> Result<R, String> {
    let sig = format!("({}){ret}", "Ljava/lang/String;".repeat(args.len()));
    with_activity(|env, activity| {
        let mut strings = Vec::new();
        for a in args {
            strings.push(env.new_string(a)?);
        }
        let values: Vec<jni::objects::JValue> = strings.iter().map(|s| jni::objects::JValue::Object(s)).collect();
        let v = env.call_method(activity, method, &sig, &values)?;
        then(env, v)
    })
}

fn call_void(method: &str, args: &[&str]) {
    if let Err(e) = call(method, args, "V", |_, _| Ok(())) {
        log::warn!("{method}: {e}");
    }
}

/// The system's picker: a folder (`ACTION_OPEN_DOCUMENT_TREE`) or with `file` a document (`ACTION_OPEN_DOCUMENT`,
/// a disc image), starting in Download; its answer comes through `take_picked`.
pub fn pick(file: bool) {
    *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    call_void(if file { "pickFile" } else { "pickFolder" }, &[]);
}

/// A picked document (its URI) opened for reading as a file descriptor the process owns.
pub fn open_uri(uri: &str) -> Result<std::fs::File, String> {
    let fd = call("openUri", &[uri], "I", |_, v| v.i())?;
    if fd < 0 {
        return Err("the file cannot be opened".into());
    }
    use std::os::fd::FromRawFd;
    // SAFETY: as in `open_document`: `detachFd` hands over a descriptor this process now owns alone
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// Every file of a picked tree (its URI): (path from the tree's top, document id, size). The activity walks it with
/// limits (depth, entries) and refuses names that could leave the copy's folder. A size the provider does not report
/// is the opened document's (0 when that is not known either: the copy then stops at the first byte past it).
pub fn list_tree(uri: &str) -> Result<Vec<(String, String, u64)>, String> {
    let text = call("listTree", &[uri], "Ljava/lang/String;", |env, v| {
        let js = jni::objects::JString::from(v.l()?);
        if js.is_null() {
            return Ok(None);
        }
        Ok(Some(String::from(env.get_string(&js)?)))
    })?
    .ok_or("the folder cannot be read")?;
    if let Some(e) = text.strip_prefix('!') {
        return Err(e.to_string());
    }
    let mut out = Vec::new();
    for l in text.lines().filter(|l| !l.is_empty()) {
        let mut f = l.split('\t');
        let (Some(rel), Some(doc), Some(size)) = (f.next(), f.next(), f.next()) else { return Err(format!("a bad listing line: {l}")) };
        let size = match size.parse::<i64>().ok().and_then(|n| u64::try_from(n).ok()) {
            Some(n) => n,
            None => open_document(uri, doc).ok().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len()),
        };
        out.push((rel.to_string(), doc.to_string(), size));
    }
    Ok(out)
}

/// A document of a picked tree opened for reading: the activity's `openFileDescriptor(…, "r").detachFd()`.
pub fn open_document(uri: &str, doc: &str) -> Result<std::fs::File, String> {
    let fd = call("openDocument", &[uri, doc], "I", |_, v| v.i())?;
    if fd < 0 {
        return Err(format!("{doc} cannot be opened"));
    }
    use std::os::fd::FromRawFd;
    // SAFETY: `detachFd` hands over a file descriptor this process now owns alone (the ParcelFileDescriptor gave up
    // its own), so the File is its only owner and closes it once
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// The free space of the app's storage (`getUsableSpace`), if known.
pub fn free_space() -> Option<u64> {
    call("freeSpace", &[], "J", |_, v| v.j()).ok().and_then(|n| u64::try_from(n).ok())
}

/// The app started again (a new game data installed while a session ran on the old).
pub fn restart() {
    call_void("restartApp", &[]);
}

/// The app ends through the activity (`finish`; QUIT on the setup screen).
pub fn quit() {
    call_void("quit", &[]);
}

/// The game cannot start (no GPU: nothing can be drawn): the activity's dialog with `msg`, which ends the app
/// through the activity. The native side stays idle meanwhile (ending the event loop from here would race the
/// activity's UI thread).
pub fn show_fatal(msg: &str) {
    call_void("showFatal", &[msg]);
}
