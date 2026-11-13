//! The browser's replay export (download) and import (file chooser).
//!
//! Engine addition.
//!
//! The browser's replay files (engine addition, stage 6b): EXPORT as a download (a blob URL on a hidden anchor,
//! both dropped a while after the click: a download may start after it), IMPORT through a hidden
//! `<input type=file>` whose files come back to the app as `UserEvent::Imported`. The file chooser opens only in a
//! user gesture: an IMPORT without one (a gamepad's button) leaves it to the next key press or click on the page
//! (for `ARMED_MS`, and only while the replay list is open: `unarm`).
//! Every call is checked: a page without the APIs gets a message, never a panic.

use super::UserEvent;
use super::replay_store::{MAX_IMPORT, MAX_IMPORT_BYTES};
use crate::session::replay::MAX_BYTES;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use winit::event_loop::EventLoopProxy;

/// Milliseconds a download's blob URL and anchor are kept after the click.
const REVOKE_MS: i32 = 10_000;
/// Milliseconds an IMPORT without a gesture waits for the key press or click that opens the chooser.
const ARMED_MS: i32 = 10_000;

fn document() -> Result<web_sys::Document, String> {
    web_sys::window().and_then(|w| w.document()).ok_or_else(|| "no document".to_string())
}

/// `bytes` saved by the browser as `name`.
pub fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    let win = web_sys::window().ok_or("no window")?;
    let doc = document()?;
    let arr = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&arr);
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(|_| "no Blob")?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| "no object URL")?;
    let a: web_sys::HtmlAnchorElement = doc.create_element("a").map_err(|_| "no anchor")?.dyn_into().map_err(|_| "no anchor")?;
    a.set_href(&url);
    a.set_download(name);
    a.style().set_property("display", "none").ok();
    let body = doc.body().ok_or("no body")?;
    body.append_child(&a).map_err(|_| "no anchor")?;
    a.click();
    // Firefox and Safari read the URL after this task: revoked later (FileSaver.js does the same)
    let later = Closure::once_into_js(move || {
        a.remove();
        web_sys::Url::revoke_object_url(&url).ok();
    });
    if win.set_timeout_with_callback_and_timeout_and_arguments_0(later.unchecked_ref(), REVOKE_MS).is_err() {
        log::warn!("download: no timer, the blob URL stays until the page closes");
    }
    Ok(())
}

/// An IMPORT's file input, its handlers and the page's key / pointer listeners that open it from a gesture.
struct Pick {
    input: web_sys::HtmlInputElement,
    /// the chooser was opened (by `pick` or a gesture since)
    opened: Rc<Cell<bool>>,
    /// the input's change / cancel handlers, kept while it may call them
    _handlers: Vec<Closure<dyn FnMut()>>,
    /// (event, listener) on the window
    listeners: Vec<(&'static str, Closure<dyn FnMut()>)>,
}

impl Pick {
    /// Its listeners off the page and its input gone (its handlers are dropped with it, none can run after).
    fn end(self) {
        if let Some(w) = web_sys::window() {
            for (ev, f) in &self.listeners {
                let _ = w.remove_event_listener_with_callback(ev, f.as_ref().unchecked_ref());
            }
        }
        self.input.remove();
    }
}

thread_local! {
    /// The IMPORT under way: kept until the next one (a handler is never dropped while it runs).
    static PICK: RefCell<Option<Pick>> = const { RefCell::new(None) };
}

/// The page has transient user activation (a key press or click just now): the chooser may open. True when the
/// browser does not say (`navigator.userActivation` missing).
pub(super) fn activation() -> bool {
    let Some(nav) = web_sys::window().map(|w| w.navigator()) else { return true };
    let ua = js_sys::Reflect::get(&nav, &"userActivation".into()).unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    if ua.is_undefined() || ua.is_null() {
        return true;
    }
    js_sys::Reflect::get(&ua, &"isActive".into()).ok().and_then(|v| v.as_bool()).unwrap_or(true)
}

/// The file picker (`.okbr`, several files); the chosen files' bytes (or why not) go to the app. True when the
/// chooser opened; false when the page has no user gesture now (the next key press or click opens it).
pub fn pick(proxy: EventLoopProxy<UserEvent>) -> Result<bool, String> {
    PICK.with(|cell| {
        let mut cur = cell.borrow_mut();
        // the last IMPORT's input still there (its chooser open, or never answered): it serves this one too
        if cur.as_ref().is_none_or(|p| !p.input.is_connected()) {
            if let Some(old) = cur.take() {
                old.end();
            }
            *cur = Some(new_pick(proxy)?);
        }
        let p = cur.as_ref().ok_or("no input")?;
        if activation() {
            p.opened.set(true);
            p.input.click();
            Ok(true)
        } else {
            p.opened.set(false);
            // disarmed after a while: a key press much later (in a fight) does not open it
            let opened = p.opened.clone();
            let later = Closure::once_into_js(move || opened.set(true));
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(later.unchecked_ref(), ARMED_MS);
            }
            Ok(false)
        }
    })
}

/// An IMPORT left to the next gesture given up (the replay list closed, a playback started); a chooser already
/// open still delivers its files.
pub fn unarm() {
    PICK.with(|cell| {
        if let Some(p) = cell.borrow().as_ref() {
            p.opened.set(true);
        }
    });
}

fn new_pick(proxy: EventLoopProxy<UserEvent>) -> Result<Pick, String> {
    let win = web_sys::window().ok_or("no window")?;
    let doc = document()?;
    let input: web_sys::HtmlInputElement = doc.create_element("input").map_err(|_| "no input")?.dyn_into().map_err(|_| "no input")?;
    input.set_type("file");
    input.set_accept(".okbr,.br2r");
    input.set_multiple(true);
    input.style().set_property("display", "none").ok();
    let body = doc.body().ok_or("no body")?;
    body.append_child(&input).map_err(|_| "no input")?;
    let inp = input.clone();
    let on_change = Closure::<dyn FnMut()>::new(move || {
        let files = inp.files();
        inp.remove();
        wasm_bindgen_futures::spawn_local(read_files(files, proxy.clone()));
    });
    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    // the chooser closed without files: the input goes (browsers without the event keep it for the next IMPORT)
    let inp = input.clone();
    let on_cancel = Closure::<dyn FnMut()>::new(move || inp.remove());
    input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref()).ok();
    // a gesture after an IMPORT that had none opens the chooser, once
    let opened = Rc::new(Cell::new(true));
    let mut listeners = Vec::new();
    for ev in ["keydown", "pointerdown"] {
        let (inp, opened) = (input.clone(), opened.clone());
        let f = Closure::<dyn FnMut()>::new(move || {
            if !opened.replace(true) && inp.is_connected() {
                inp.click();
            }
        });
        if win.add_event_listener_with_callback(ev, f.as_ref().unchecked_ref()).is_ok() {
            listeners.push((ev, f));
        }
    }
    Ok(Pick { input, opened, _handlers: vec![on_change, on_cancel], listeners })
}

/// The chosen files read one after another (at most `MAX_IMPORT` files and `MAX_IMPORT_BYTES` in all), then sent.
async fn read_files(files: Option<web_sys::FileList>, proxy: EventLoopProxy<UserEvent>) {
    let mut out = Vec::new();
    let mut total = 0usize;
    if let Some(list) = files {
        for k in 0..list.length().min(MAX_IMPORT as u32) {
            let Some(file) = list.get(k) else { continue };
            let name = file.name();
            let size = file.size() as usize;
            let bytes = if size > MAX_BYTES {
                Err(format!("more than {MAX_BYTES} bytes, not a replay"))
            } else if total + size > MAX_IMPORT_BYTES {
                Err("too much at once: import it on its own".to_string())
            } else {
                total += size;
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => Ok(js_sys::Uint8Array::new(&buf).to_vec()),
                    Err(_) => Err("unreadable".to_string()),
                }
            };
            out.push((name, bytes));
        }
    }
    let _ = proxy.send_event(UserEvent::Imported(out));
}
