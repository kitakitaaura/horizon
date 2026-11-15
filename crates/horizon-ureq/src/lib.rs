//! ureq 2 itself natively. In the browser build, ureq 2's API as Horizon uses it: each request is a synchronous
//! `XMLHttpRequest`. Synchronous requests block the thread they run on, so callers keep them off
//! the main thread as they already do natively (a worker thread is a Web Worker in the browser).
//! The browser decides what it sends: `User-Agent` can't be set, and every response needs CORS
//! from its server.

#[cfg(not(target_arch = "wasm32"))]
pub use ureq_native::*;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;
