//! The game's start in the browser.

use wasm_bindgen::prelude::*;

/// Starts the game with `args` (its command line): opens WebGPU, which only resolves through
/// the browser's event loop, then runs the game's own start. winit's loop then runs from the
/// page's callbacks, so this returns once the window is up.
#[wasm_bindgen(js_name = startGame)]
pub async fn start_game(args: Vec<String>) -> Result<String, JsError> {
    let adapter = horizon_client_render::gfx::preopen_web()
        .await
        .map_err(|e| JsError::new(&e))?;
    for arg in &args {
        crate::host::push_arg(arg);
    }
    horizon_client::entry::main();
    Ok(adapter)
}
