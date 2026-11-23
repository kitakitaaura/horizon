//! cap-fs-ext 3 itself natively. In the browser build, the extension traits Horizon uses on
//! [`cap_std`]'s stand-in (horizon-cap-std).

#[cfg(not(target_arch = "wasm32"))]
pub use cap_fs_ext_native::*;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;
