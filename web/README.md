# Horizon in the browser

Experimental: Horizon built for `wasm32-unknown-unknown` with Wasm 3.0 features
(atomics, SIMD and relaxed SIMD, bulk memory, tail calls, exception handling), drawing with
WebGPU and running its threads on Web Workers over shared memory.

## Build and run

```sh
web/toolchain.sh          # once: the `horizon-web` rustup toolchain (pinned nightly + web/std.patch)
web/build.sh              # web/pkg/horizon_web{.js,_bg.wasm}; --release for an optimized build
python3 web/serve.py      # http://localhost:8765/ (sends COOP/COEP, which shared memory needs)
```

## Game files

Horizon ships none of Mojang's files, and preparing them runs Java, which the browser can't. Prepare
them natively from your own Minecraft: Java Edition 26.3 (needs JDK 25, the 26.3 install and its
server jar; [docs/native-client.md](../docs/native-client.md)):

```sh
cargo run --release -p horizon-prepare -- --prepare --minecraft-dir /path/to/minecraft --server-jar /path/to/server.jar
```

Then open the page and pick the folder it wrote, `.local/horizon-client/26.3`. The page copies it
into the game's filesystem (`/resources`, passed as `--resources`) and into the browser's private
storage (OPFS), so later visits load it from there (`resources.js`). Sounds are left out (no audio
yet; `?sounds` keeps them); `?forget` clears the stored copy.

Pages: `index.html` (the game), `smoke.html` (checks of the patched std).
`index.html` takes `?reserve=N` (idle workers kept ready; 32 by default, 4 with a debug build,
whose ~400 MB module runs the tab out of memory with more), `?arg=...` (a command-line argument,
repeatable), `?pause` (load, then wait for `horizon.startGame([...])` from the console) and
`?probe` (memory and the log's last line to `serve.py`'s request log every second, which outlives
a crashed tab), `?forget-saves` (drop the stored worlds and options).

The game folder (worlds in `/saves`, `options.txt`, the game's caches; not logs or game files)
is copied to the browser's private storage every 5 seconds and when the tab is hidden or closed,
and restored before the game starts (`saves.js`). A file the game is writing during a copy may be
stored half-written; the next copy fixes it, unless the tab closes first.

## How it fits together

- **std** (`std.patch`): on `wasm32-unknown-unknown` with atomics, std has no clock, threads or
  filesystem. The patch adds them under `sys/web_host.rs` and `sys/{time,thread,fs,env,paths,args}/web.rs`:
  a clock and CPU count from hooks the program defines (`crates/horizon-web/src/host.rs`), threads
  on a pool of Web Workers, a filesystem in shared memory, environment variables and arguments in
  memory, stdout and stderr to the console. The page's main thread may not block, so its waits spin.
- **The thread pool**: a worker starts only once the thread that made it returns to the event
  loop, so idle workers wait ready and std hands each new thread to one at once. A spawner worker
  (`spawner.js`) that runs no wasm, and so is never blocked, starts more as std asks, through a
  counter in shared memory it waits on with `Atomics.waitAsync`: a blocked main thread can't
  starve the pool. When a thread ends, its worker sets up a fresh instance of the same compiled
  module and goes back to the pool (`worker.js`), since the game starts short threads constantly.
- **GPU**: WebGPU objects belong to the thread that made them. The renderer's helper threads run in
  place in the browser (`horizon-client-render/src/gpu_thread.rs`), and the device is opened before
  the game starts (`gfx::preopen_web`), since its request resolves only through the event loop.
- **Shaders**: the browser's WGSL compiler enforces uniform control flow for texture samples, which
  wgpu's doesn't; `horizon.js` adds `diagnostic(off, derivative_uniformity);` to every module.
- **HTTP**: `crates/horizon-ureq` is ureq natively and ureq's API over synchronous XMLHttpRequest
  in the browser (CORS applies).

## Not done yet

Preparing game files in the browser (needs the Java exporters ported to Rust, #1750), sound, controllers, multiplayer (no TCP: a WebSocket path is needed), F2
screenshots to a download, and shader packs.
