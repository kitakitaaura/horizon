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
    // The engine's platform socket, which horizon-mod-host fills natively from the mod API. Here
    // the mod API would be a wasm mod's guest side, and the browser loads no code mods: there are
    // no packages or parts to install and no profiler. A second start finds it filled already.
    let _ = horizon_engine_slots::install_platform(horizon_engine_slots::Platform {
        install_packages: |_, _| {},
        install_registration_data: |_| {},
        install_parts: |_, _, _| Ok(()),
        profiler_is_enabled: || false,
        profiler_start: || false,
        profiler_stop: || {},
        profiler_summary: String::new,
        profiler_firefox_profile: || Err("The browser has no profiler".into()),
    });
    // The engine with no code mods: the browser loads neither native nor wasm mods.
    let startup = horizon_client::entry::prepare();
    horizon_client::entry::start(
        Default::default(),
        |_, _| (Vec::new(), ()),
        horizon_client::entry::CodeRuntime {
            initialize: |_| {},
            refusal: |_| None,
            startup_refused: |_| false,
        },
        startup,
    );
    Ok(adapter)
}
