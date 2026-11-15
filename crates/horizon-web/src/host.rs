//! The hooks the patched std declares (`std/src/sys/web_host.rs`), and the worker pool's
//! entry points for `web/horizon.js` and `web/worker.js`.

use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

#[wasm_bindgen]
extern "C" {
    /// `Performance`, in a window or a worker.
    type Performance;
    #[wasm_bindgen(method, js_name = now)]
    fn now(this: &Performance) -> f64;
    #[wasm_bindgen(method, getter, js_name = timeOrigin)]
    fn time_origin(this: &Performance) -> f64;
}

thread_local! {
    /// This thread's `performance` and its time origin: a worker's origin is when it started,
    /// so each thread adds its own to share one clock.
    static PERFORMANCE: (Performance, f64) = {
        let performance: Performance = Reflect::get(&js_sys::global(), &JsValue::from_str("performance"))
            .expect("no performance object")
            .unchecked_into();
        let origin = performance.time_origin();
        (performance, origin)
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn __horizon_web_monotonic_ms() -> f64 {
    PERFORMANCE.with(|(performance, origin)| origin + performance.now())
}

#[unsafe(no_mangle)]
pub extern "C" fn __horizon_web_wall_ms() -> f64 {
    js_sys::Date::now()
}

#[unsafe(no_mangle)]
pub extern "C" fn __horizon_web_hardware_concurrency() -> u32 {
    Reflect::get(&js_sys::global(), &JsValue::from_str("navigator"))
        .and_then(|navigator| Reflect::get(&navigator, &JsValue::from_str("hardwareConcurrency")))
        .ok()
        .and_then(|n| n.as_f64())
        .map_or(4, |n| n as u32)
}

/// std's random bytes (`std::random`, `sys::random::fill_bytes`): `crypto.getRandomValues`, into
/// a buffer of its own first, since it takes no view of shared memory; 64 KiB a call at most.
#[unsafe(no_mangle)]
pub extern "C" fn __horizon_web_random_fill(ptr: *mut u8, len: usize) {
    // SAFETY: std passes a slice it owns for the call.
    let out = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
    let crypto = Reflect::get(&js_sys::global(), &JsValue::from_str("crypto")).expect("no crypto");
    let fill: js_sys::Function = Reflect::get(&crypto, &JsValue::from_str("getRandomValues"))
        .expect("no crypto.getRandomValues")
        .unchecked_into();
    for chunk in out.chunks_mut(65536) {
        let buffer = js_sys::Uint8Array::new_with_length(chunk.len() as u32);
        fill.call1(&crypto, &buffer).expect("crypto.getRandomValues failed");
        buffer.copy_to(chunk);
    }
}

thread_local! {
    /// This thread's console output not yet ended by a newline.
    static CONSOLE_LINE: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// stdout and stderr: whole lines to the console, panics as errors.
#[unsafe(no_mangle)]
pub extern "C" fn __horizon_web_write_console(ptr: *const u8, len: usize) {
    // SAFETY: std passes a slice it owns for the call.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let _ = CONSOLE_LINE.try_with(|line| {
        let mut line = line.borrow_mut();
        for &b in bytes {
            if b == b'\n' {
                let text = String::from_utf8_lossy(&line).into_owned();
                if text.contains("panicked") || text.starts_with("error") {
                    web_console_error(&text);
                } else {
                    web_console_log(&text);
                }
                line.clear();
            } else {
                line.push(b);
            }
        }
    });
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn web_console_log(text: &str);
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn web_console_error(text: &str);
}

unsafe extern "C" {
    fn __horizon_web_args_push(ptr: *const u8, len: usize);
    fn __horizon_web_worker_main();
    fn __horizon_web_pool_wanted() -> u32;
    fn __horizon_web_pool_wanted_addr() -> usize;
    fn __horizon_web_pool_idle() -> u32;
    fn __horizon_web_pool_set_reserve(count: u32);
}

/// A pool worker's life, once wasm-bindgen has set it up: wait for a thread, run it, return.
#[wasm_bindgen(js_name = workerMain)]
pub fn worker_main() {
    // SAFETY: defined by the patched std; takes and returns nothing.
    unsafe { __horizon_web_worker_main() }
}

/// How many more workers the page should start now.
#[wasm_bindgen(js_name = poolWanted)]
pub fn pool_wanted() -> u32 {
    // SAFETY: as above.
    unsafe { __horizon_web_pool_wanted() }
}

/// The byte address of the pool's "workers wanted" counter (a u32), for the spawner's
/// `Atomics.waitAsync`.
#[wasm_bindgen(js_name = poolWantedAddr)]
pub fn pool_wanted_addr() -> usize {
    // SAFETY: as above.
    unsafe { __horizon_web_pool_wanted_addr() }
}

/// The size of the module's (shared) memory in bytes.
#[wasm_bindgen(js_name = wasmMemoryBytes)]
pub fn wasm_memory_bytes() -> f64 {
    (core::arch::wasm32::memory_size(0) * 65536) as f64
}

/// Workers waiting for a thread.
#[wasm_bindgen(js_name = poolIdle)]
pub fn pool_idle() -> u32 {
    // SAFETY: as above.
    unsafe { __horizon_web_pool_idle() }
}

/// How many idle workers the page keeps ready.
#[wasm_bindgen(js_name = poolSetReserve)]
pub fn pool_set_reserve(count: u32) {
    // SAFETY: as above.
    unsafe { __horizon_web_pool_set_reserve(count) }
}

/// Appends a command-line argument for the game (`std::env::args`), before it starts.
#[wasm_bindgen(js_name = pushArg)]
pub fn push_arg(arg: &str) {
    // SAFETY: defined by the patched std; it copies the bytes.
    unsafe { __horizon_web_args_push(arg.as_ptr(), arg.len()) }
}

/// Sets an environment variable before the program starts (the page's options).
#[wasm_bindgen(js_name = setEnv)]
pub fn set_env(key: &str, value: &str) {
    // SAFETY: the patched std keeps variables in a locked map; no C code reads them.
    unsafe { std::env::set_var(key, value) }
}

/// Writes a file into the filesystem, making its directories (the uploaded jar, the page's
/// copies of saves).
#[wasm_bindgen(js_name = writeFile)]
pub fn write_file(path: &str, bytes: &[u8]) -> Result<(), JsError> {
    let path = std::path::Path::new(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Every file under `root` but those under `skip`, one a line: its path from `root`, its size
/// and its modified time in milliseconds since the Unix epoch, separated by tabs. For the page's
/// copy of the game's saves in the browser's storage (web/saves.js).
#[wasm_bindgen(js_name = listFiles)]
pub fn list_files(root: &str, skip: Vec<String>) -> String {
    use std::path::Path;
    fn walk(dir: &Path, root: &Path, skip: &[String], out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let rel = rel.to_string_lossy();
            if skip.iter().any(|s| rel == s.as_str() || rel.starts_with(&format!("{s}/"))) {
                continue;
            }
            if meta.is_dir() {
                walk(&path, root, skip, out);
            } else {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_millis());
                out.push_str(&format!("{rel}\t{}\t{modified}\n", meta.len()));
            }
        }
    }
    let mut out = String::new();
    walk(Path::new(root), Path::new(root), &skip, &mut out);
    out
}

/// A file's bytes, or undefined.
#[wasm_bindgen(js_name = readFile)]
pub fn read_file(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// Removes a file; false when there was none.
#[wasm_bindgen(js_name = removeFile)]
pub fn remove_file(path: &str) -> bool {
    std::fs::remove_file(path).is_ok()
}
