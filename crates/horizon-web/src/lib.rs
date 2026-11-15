//! Horizon in the browser. Built for `wasm32-unknown-unknown` with the `horizon-web` toolchain,
//! whose std is patched (`web/std.patch`) to give the program a clock, threads on Web Workers,
//! a filesystem in shared memory and environment variables. This crate defines the host hooks
//! that std calls, with js-sys, and the entry points the page and its workers call.

#[cfg(target_arch = "wasm32")]
mod game;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod smoke;

#[cfg(target_arch = "wasm32")]
pub use host::*;
