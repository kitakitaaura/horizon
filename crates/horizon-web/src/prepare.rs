//! Preparation in the browser: `horizon-prepare`'s own steps, on files the page copies into the
//! filesystem from the player's own Minecraft folder (web/minecraft.js). Only the steps that need
//! no Java run here so far: the assets copied from the launcher's asset store (sounds, panorama,
//! fonts, languages, built-in packs).

use std::path::Path;
use wasm_bindgen::prelude::*;

/// Where preparation finds Horizon's pins (`research-lock.json`) in the filesystem.
const ROOT: &str = "/";
/// Where the page puts what it copies from the picked Minecraft folder.
pub const MINECRAFT: &str = "/minecraft";

fn lock() -> Result<horizon_prepare::artifacts::Lock, String> {
    let path = Path::new(ROOT).join("research-lock.json");
    if !path.is_file() {
        std::fs::write(&path, include_str!("../../../research-lock.json"))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    horizon_prepare::artifacts::Lock::read(Path::new(ROOT))
}

/// The pinned version and what the page has to fetch or bring for it, as JSON:
/// `{"version", "manifest_sha1", "manifest_path"}`. `manifest_path` is where Mojang's own
/// version JSON (fetched by its SHA-1) goes, so a launcher's rewritten copy isn't needed.
#[wasm_bindgen(js_name = preparePins)]
pub fn prepare_pins() -> Result<String, JsError> {
    let lock = lock().map_err(|e| JsError::new(&e))?;
    let sha1 = lock.pin("manifest_sha1").map_err(|e| JsError::new(&e))?;
    let manifest = lock.directory.join("manifest.json");
    Ok(format!(
        "{{\"version\":{:?},\"manifest_sha1\":{:?},\"manifest_path\":{:?},\"minecraft\":{:?}}}",
        lock.version,
        sha1,
        manifest.to_string_lossy(),
        MINECRAFT
    ))
}

/// A whole preparation started by [`prepare_start`]: what it said so far, and how it ended.
#[derive(Default)]
struct Job {
    lines: Vec<String>,
    progress: f32,
    outcome: Option<Result<String, String>>,
}

static JOB: std::sync::Mutex<Option<Job>> = std::sync::Mutex::new(None);

fn job<T>(f: impl FnOnce(&mut Option<Job>) -> T) -> T {
    f(&mut JOB.lock().unwrap_or_else(|e| e.into_inner()))
}

/// Starts `horizon-prepare --prepare` on a thread of its own (the page's thread may not block,
/// and preparation waits on its Java runtimes): from the Minecraft files the page copied to
/// `/minecraft`, into `output`. The exporters run on the page's JVMs (web/jvm/spawner.js), from
/// the prebuilt classes at `/exporters`; the exporters' sources must be at
/// `/mods/horizon/src/test/java`. Follow it with [`prepare_poll`].
#[wasm_bindgen(js_name = prepareStart)]
pub fn prepare_start(output: &str) -> Result<(), JsError> {
    lock().map_err(|e| JsError::new(&e))?;
    if job(|j| j.as_ref().is_some_and(|j| j.outcome.is_none())) {
        return Err(JsError::new("A preparation is running already"));
    }
    job(|j| *j = Some(Job::default()));
    let options = horizon_prepare::Options {
        root: ROOT.into(),
        import: true,
        minecraft_dir: Some(MINECRAFT.into()),
        server_jar: None,
        java_home: None,
        inputs: horizon_prepare::artifacts::Inputs::default(),
        output: Some(output.into()),
    };
    std::thread::Builder::new()
        .name("prepare".into())
        .spawn(move || {
            let log = |line: &str| {
                println!("[horizon-prepare] {line}");
                job(|j| j.as_mut().map(|j| j.lines.push(line.into())));
            };
            let progress = |fraction: f32| {
                job(|j| j.as_mut().map(|j| j.progress = fraction));
            };
            let outcome = horizon_prepare::prepare_with_progress(&options, &log, &progress)
                .map(|p| p.to_string_lossy().into_owned());
            job(|j| j.as_mut().map(|j| j.outcome = Some(outcome)));
        })
        .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(())
}

/// The running preparation's news since the last call, as JSON: `{"lines": [...], "progress":
/// 0..1, "java": {step: what its JVM is doing}, "done": bool, "error": null or why it failed,
/// "output": the folder written}`.
#[wasm_bindgen(js_name = preparePoll)]
pub fn prepare_poll() -> String {
    use horizon_prepare::pyjson::{COMPACT, Value, dumps};
    job(|j| {
        let Some(j) = j.as_mut() else {
            return "null".into();
        };
        let lines = std::mem::take(&mut j.lines);
        let (done, error, output) = match &j.outcome {
            None => (false, Value::Null, Value::Null),
            Some(Ok(path)) => (true, Value::Null, Value::Str(path.clone())),
            Some(Err(e)) => (true, Value::Str(e.clone()), Value::Null),
        };
        dumps(
            &Value::Object(vec![
                (
                    "lines".into(),
                    Value::Array(lines.into_iter().map(Value::Str).collect()),
                ),
                ("progress".into(), Value::Float(j.progress.into())),
                (
                    "java".into(),
                    Value::Object(
                        horizon_prepare::browser_jvm::activities()
                            .into_iter()
                            .map(|(step, doing)| (step, Value::Str(doing)))
                            .collect(),
                    ),
                ),
                ("done".into(), Value::Bool(done)),
                ("error".into(), error),
                ("output".into(), output),
            ]),
            &COMPACT,
        )
    })
}

/// The key the prebuilt exporter classes at `/exporters` must have (their `manifest.json`): a
/// hash of the exporters' sources at `/mods/horizon/src/test/java` and the pins, so the page can
/// bring the matching bundle.
#[wasm_bindgen(js_name = prepareExportersKey)]
pub fn prepare_exporters_key() -> Result<String, JsError> {
    lock().map_err(|e| JsError::new(&e))?;
    horizon_prepare::exporters::key(Path::new(ROOT)).map_err(|e| JsError::new(&e))
}

/// Where preparation asks the page for Java runtimes (horizon_prepare::browser_jvm::MAILBOX), for
/// web/jvm/spawner.js.
#[wasm_bindgen(js_name = jvmMailboxAddr)]
pub fn jvm_mailbox_addr() -> usize {
    horizon_prepare::browser_jvm::mailbox_addr()
}

/// The SHA-1s of the asset-store objects step `kind` needs, from the asset index's bytes.
#[wasm_bindgen(js_name = prepareWanted)]
pub fn prepare_wanted(kind: &str, index: &[u8]) -> Result<Vec<String>, JsError> {
    horizon_prepare::wanted_objects(kind, index).map_err(|e| JsError::new(&e))
}

/// Runs `horizon-prepare <kind>` into the prepared files at `output`, from the asset store the
/// page copied to `/minecraft/assets`. Every object is checked against its SHA-1. Its log lines
/// go to the console. Returns the folder written.
#[wasm_bindgen(js_name = prepareAssets)]
pub fn prepare_assets(kind: &str, output: &str) -> Result<String, JsError> {
    lock().map_err(|e| JsError::new(&e))?;
    let log = |line: &str| println!("[horizon-prepare] {line}");
    horizon_prepare::add_assets(
        kind,
        Path::new(ROOT),
        Some(Path::new(output)),
        Some(Path::new(MINECRAFT)),
        &horizon_prepare::artifacts::Inputs::default(),
        &log,
    )
    .map(|p| p.to_string_lossy().into_owned())
    .map_err(|e| JsError::new(&e))
}
