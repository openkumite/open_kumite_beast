//! The browser's conversion worker: CONVERT DISC IMAGE off the page's thread (engine addition, feature `converter`).
//!
//! The bin `convert_worker` (Trunk `data-type="worker"`) runs `worker_main` in a Web Worker. The page posts it the
//! picked files (structured clones of the `File`s: nothing is read whole) with a mode and a game (`br2`, or `br1`:
//! the optional disc of the imported game, through `imports::br1`; its `.cue` brings its CD audio track's `.bin`
//! when that was picked too; or `q3`, engine addition: Quake III Arena's `.pk3` files, through `imports::q3`, whose
//! `check` reads only their zip directories and answers `{t: "ok", source: "pk3", paks, maps}`, the maps' names as
//! JSON): `check` opens the image (a `.cue` with its `.bin` among the picked files, else a `.bin`, `.chd` or `.iso` by
//! contents) and reads its volume; `convert` also runs the converter (`ok_br2_converter`, or the import's; one thread)
//! step by step, reading the image at random places with `FileReaderSync` over `Blob.slice`, and writes each step's
//! files into the OPFS folder named by the page through sync access handles. It first posts `{t: "ready", v: PROTOCOL}`
//! (the page waits for it; a worker of another build, cached, is refused), then plain objects: `{t: "p", done, total,
//! step}` progress, `{t: "ok", source, xa, cdda, files}` (the files as JSON `[[path, size, crc32], …]`, manifest.json
//! not in it), or `{t: "err", msg}`. The page (`web.rs`) checks every file back, switches to the folder, or ends the
//! worker (`terminate`) to cancel.

use super::imports::{br1, q3};
use super::plan::{br1_disc_error, disc_error, q3_error};
use crate::base::hash::crc32;
use crate::platform::game_data::web;
use ok_br2_converter::{Converter, Disc, Options, ReadAt, SectorReader, Sink};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::JsValue;

/// A picked file read at random places (`FileReaderSync`, a worker's synchronous reader).
struct BlobAt {
    blob: web_sys::Blob,
    len: u64,
    reader: web_sys::FileReaderSync,
}

impl ReadAt for BlobAt {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
        let end = offset + buf.len() as u64;
        if end > self.len {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let err = |e: JsValue| std::io::Error::other(format!("{e:?}"));
        let part = self.blob.slice_with_f64_and_f64(offset as f64, end as f64).map_err(err)?;
        let ab = self.reader.read_as_array_buffer(&part).map_err(err)?;
        js_sys::Uint8Array::new(&ab).copy_to(buf);
        Ok(())
    }
    fn len(&self) -> u64 {
        self.len
    }
}

fn blob_at(f: &web_sys::File) -> Result<BlobAt, String> {
    let reader = web_sys::FileReaderSync::new().map_err(|_| "no FileReaderSync in this worker".to_string())?;
    Ok(BlobAt { blob: f.clone().into(), len: f.size() as u64, reader })
}

/// The page's and the worker's messages (module header): raised with any change to them. The worker's files are not
/// hashed by Trunk, so a browser may pair a new page with a cached worker of another build.
pub const PROTOCOL: u32 = 3;

/// The disc in the picked files (module header).
fn open(files: &[web_sys::File]) -> Result<Disc, String> {
    Disc::open(image(files, false)?).map_err(disc_error)
}

/// The imported game's disc in the picked files (module header).
fn open_br1(files: &[web_sys::File]) -> Result<br1::Disc, String> {
    br1::open(image(files, true)?).map_err(br1_disc_error)
}

/// Quake III Arena's paks in the picked files (the page sends only `.pk3` files: `web::pk3_files`).
fn paks(files: &[web_sys::File]) -> Result<q3::Paks, String> {
    let mut v: Vec<(String, Box<dyn ReadAt>)> = Vec::new();
    for f in files.iter().filter(|f| f.name().to_ascii_lowercase().ends_with(".pk3")) {
        v.push((f.name(), Box::new(blob_at(f)?)));
    }
    if v.is_empty() {
        return Err("No .pk3 files were chosen.".into());
    }
    q3::open(v).map_err(q3_error)
}

/// The image in the picked files: a `.cue` (`audio`: with the CD audio tracks it names among them), else a `.bin`,
/// `.chd` or `.iso`.
fn image(files: &[web_sys::File], audio: bool) -> Result<Box<dyn SectorReader>, String> {
    let ext = |f: &web_sys::File| f.name().rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    Ok(if let Some(cue) = files.iter().find(|f| ext(f) == "cue") {
        if cue.size() > (64 << 10) as f64 {
            return Err("That .cue sheet is too large.".into());
        }
        let reader = web_sys::FileReaderSync::new().map_err(|_| "no FileReaderSync in this worker".to_string())?;
        let text = reader.read_as_text(cue).map_err(|_| "the .cue cannot be read".to_string())?;
        let mut opener = |name: &str| -> std::io::Result<Box<dyn ReadAt>> {
            let f = files
                .iter()
                .find(|f| f.name().eq_ignore_ascii_case(name))
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("choose {name} together with the .cue (both files at once)")))?;
            Ok(Box::new(blob_at(f).map_err(std::io::Error::other)?))
        };
        if audio {
            Box::new(ok_br2_converter::open_cue_all(&text, &mut opener).map_err(disc_error)?)
        } else {
            ok_br2_converter::open_cue(&text, &mut opener).map_err(disc_error)?
        }
    } else {
        let f = files.iter().find(|f| ["bin", "chd", "iso"].contains(&ext(f).as_str())).ok_or("Choose a .cue, .bin, .chd or .iso file.")?;
        ok_br2_converter::open_read_at(Box::new(blob_at(f)?)).map_err(disc_error)?
    })
}

/// A step's files, kept until the page's storage has them (OPFS writes are async).
#[derive(Default)]
struct Pending {
    files: Vec<(String, Vec<u8>)>,
}

impl Sink for Pending {
    fn put(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        if !crate::assets::manifest::safe_path(path) {
            return Err(format!("{path}: not a path inside the data folder"));
        }
        self.files.push((path.to_string(), data.to_vec()));
        Ok(())
    }
}

impl br1::Sink for Pending {
    fn put(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        Sink::put(self, path, data)
    }
}

impl q3::Sink for Pending {
    fn put(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        Sink::put(self, path, data)
    }
}

/// `bytes` as `path` under `d` through a sync access handle (a longer file left there cut to them).
async fn write(d: &web_sys::FileSystemDirectoryHandle, path: &str, bytes: &[u8]) -> Result<(), String> {
    let h = web::file(d, path, true).await?;
    let p = wasm_bindgen_futures::JsFuture::from(h.create_sync_access_handle()).await.map_err(|e| format!("{path}: {e:?}"))?;
    let a: web_sys::FileSystemSyncAccessHandle = p.dyn_into().map_err(|_| "no sync access handle")?;
    let r = a
        .write_with_u8_array(bytes)
        .map_err(|e| format!("{path}: {e:?}"))
        .and_then(|n| if n as usize == bytes.len() { Ok(()) } else { Err(format!("{path}: {n} of {} bytes written (the storage is full?)", bytes.len())) });
    let r = r.and_then(|_| a.truncate_with_f64(bytes.len() as f64).map_err(|e| format!("{path}: {e:?}")));
    let r = r.and_then(|_| a.flush().map_err(|e| format!("{path}: {e:?}")));
    a.close();
    r
}

fn post(scope: &web_sys::DedicatedWorkerGlobalScope, kv: &[(&str, JsValue)]) {
    let o = js_sys::Object::new();
    for (k, v) in kv {
        let _ = js_sys::Reflect::set(&o, &(*k).into(), v);
    }
    let _ = scope.post_message(&o);
}

/// One conversion's steps, the main game's, the imported game's disc's or Quake III Arena's maps'.
enum Job {
    Br2(Converter),
    Br1(br1::Converter),
    Q3(q3::Converter),
}

impl Job {
    fn progress(&self) -> (usize, usize, String) {
        match self {
            Job::Br2(c) => {
                let p = c.progress();
                (p.done, p.total, p.step)
            }
            Job::Br1(c) => {
                let p = c.progress();
                (p.done, p.total, p.step)
            }
            Job::Q3(c) => {
                let p = c.progress();
                (p.done, p.total, p.step)
            }
        }
    }
    fn step(&mut self, sink: &mut Pending) -> Result<bool, String> {
        match self {
            Job::Br2(c) => c.step(sink).map_err(|e| e.to_string()),
            Job::Br1(c) => c.step(sink).map_err(|e| e.to_string()),
            Job::Q3(c) => c.step(sink).map_err(|e| e.to_string()),
        }
    }
}

/// The opened image, before its conversion (a `check` stops there).
enum Opened {
    Br2(Disc),
    Br1(br1::Disc),
    Q3(q3::Paks),
}

async fn run(scope: &web_sys::DedicatedWorkerGlobalScope, mode: &str, game: &str, files: Vec<web_sys::File>, dir: String) -> Result<(), String> {
    let disc = match game {
        "br1" => Opened::Br1(open_br1(&files)?),
        "q3" => Opened::Q3(paks(&files)?),
        _ => Opened::Br2(open(&files)?),
    };
    let (source, xa, cdda) = match &disc {
        Opened::Br2(d) => (d.source(), d.raw(), false),
        Opened::Br1(d) => (d.source(), d.raw(), !d.audio_tracks().is_empty()),
        Opened::Q3(_) => ("pk3", false, false),
    };
    if mode == "check" {
        let mut kv = vec![("t", "ok".into()), ("source", source.into()), ("xa", xa.into()), ("cdda", cdda.into()), ("files", "[]".into())];
        if let Opened::Q3(p) = &disc {
            kv.push(("paks", (p.names().len() as f64).into()));
            kv.push(("maps", serde_json::Value::from(p.maps()).to_string().into()));
        }
        post(scope, &kv);
        return Ok(());
    }
    let mut conv = match disc {
        Opened::Br2(d) => Job::Br2(Converter::new(d, Options { viewer: false, threads: 1 })),
        Opened::Br1(d) => Job::Br1(br1::Converter::new(d)),
        Opened::Q3(p) => Job::Q3(q3::Converter::new(p)),
    };
    let root = web::root().await?;
    // a folder of that name left (a removed install's, not yet deleted) is not written over
    web::remove(&root, &dir).await;
    let d = web::subdir(&root, &dir, true).await?;
    let mut written: Vec<(String, u64, u32)> = Vec::new();
    loop {
        let (done, total, step) = conv.progress();
        post(scope, &[("t", "p".into()), ("done", (done as f64).into()), ("total", (total as f64).into()), ("step", step.into())]);
        let mut sink = Pending::default();
        let end = conv.step(&mut sink)?;
        for (path, data) in sink.files {
            write(&d, &path, &data).await?;
            if path != "manifest.json" {
                written.retain(|w| w.0 != path);
                written.push((path, data.len() as u64, crc32(&data)));
            }
        }
        if end {
            break;
        }
    }
    let list: Vec<serde_json::Value> = written.iter().map(|(p, n, c)| serde_json::json!([p, n, c])).collect();
    let files = serde_json::Value::Array(list).to_string();
    post(scope, &[("t", "ok".into()), ("source", source.into()), ("xa", xa.into()), ("cdda", cdda.into()), ("files", files.into())]);
    Ok(())
}

/// The worker's entry (`bin/convert_worker.rs`): waits for the page's message (module header).
pub fn worker_main() {
    console_error_panic_hook::set_once();
    let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let s = scope.clone();
    let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
        let data = ev.data();
        let get = |k: &str| js_sys::Reflect::get(&data, &k.into()).unwrap_or(JsValue::UNDEFINED);
        let mode = get("mode").as_string().unwrap_or_default();
        let game = get("game").as_string().unwrap_or_default();
        let dir = get("dir").as_string().unwrap_or_default();
        let files: Vec<web_sys::File> = js_sys::Array::from(&get("files")).iter().filter_map(|f| f.dyn_into().ok()).collect();
        let s = s.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = run(&s, &mode, &game, files, dir).await {
                post(&s, &[("t", "err".into()), ("msg", e.into())]);
            }
        });
    });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
    // the page posts its files once this handler is there
    post(&scope, &[("t", "ready".into()), ("v", PROTOCOL.into())]);
}
