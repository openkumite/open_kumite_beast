//! The browser's conversion worker (`app::setup::worker`, engine addition): CONVERT DISC IMAGE off the page's thread.
//! Built by Trunk as a Web Worker (`index.html`); on other targets it does nothing.

#[cfg(target_arch = "wasm32")]
fn main() {
    open_kumite_beast::convert_worker_main();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("convert_worker: the browser build's Web Worker (built by Trunk); nothing to do here");
}
