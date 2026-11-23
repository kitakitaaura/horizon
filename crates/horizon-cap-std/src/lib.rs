//! cap-std 3 itself natively. In the browser build, cap-std's API as Horizon uses it, over
//! `std::fs`: a [`fs::Dir`] is the path it was opened at. The browser's filesystem is the page's
//! own, with no other process and no symlinks to race, so the capability checks cap-std makes
//! natively have nothing to guard against there.

#[cfg(not(target_arch = "wasm32"))]
pub use cap_std_native::*;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;
