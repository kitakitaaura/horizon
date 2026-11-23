// One Java runtime (Ristretto's wasm32-wasip1 build) in a dedicated worker, on wasi.js.
// The page posts `{ wasm, args, env, mounts }`: `wasm` a WebAssembly.Module or its URL, `mounts`
// `[{ at, http }]` (a folder web/serve.py serves) or `[{ at }]` (empty, in memory). The worker posts
// `{ out }` and `{ err }` as text arrives, then `{ exit, seconds, memory }` (memory: the wasm
// memory's size at the end, its peak: it never shrinks).
//
// For preparation (spawner.js), `{ wasm, args, env, jdk, memory, channel }` instead: `/` is the
// game's filesystem through the channel (rpc.js), `/jdk` the Java image served at `jdk`, and the
// standard streams and the exit go through the channel too.
import { Wasi, MemFs, HttpFs, Exit } from "./wasi.js";
import { Channel, RpcFs } from "./rpc.js";

addEventListener("message", ({ data }) => (data.channel ? prepareJvm(data) : checkJvm(data)));

/** The runtime compiler's host (ristretto_vm::wasm_jit) for `instance`, once it exists. */
function jitHost(getInstance) {
  return {
    // Compile a module on this one's memory, append its `run` export to the function table,
    // return the index (-1: failed).
    compile(pointer, length) {
      const instance = getInstance();
      try {
        const bytes = new Uint8Array(instance.exports.memory.buffer, pointer >>> 0, length >>> 0).slice();
        const env = { memory: instance.exports.memory };
        for (const [name, value] of Object.entries(instance.exports)) {
          if (name.startsWith("ristretto_jit_")) env[name] = value;
        }
        const compiled = new WebAssembly.Instance(new WebAssembly.Module(bytes), { env });
        const table = instance.exports.__indirect_function_table;
        const index = table.grow(1);
        table.set(index, compiled.exports.run);
        return index;
      } catch (error) {
        console.error("[jit] compile failed:", error.message);
        return -1;
      }
    },
  };
}

async function prepareJvm(data) {
  const channel = new Channel(data.memory, data.channel);
  let code = -1;
  try {
    const wasi = new Wasi({
      args: data.args,
      env: data.env,
      mounts: { "/": new RpcFs(channel), "/jdk": HttpFs.load(data.jdk) },
      stdout: (bytes) => channel.stdout(bytes),
      stderr: (bytes) => channel.stderr(bytes),
      stdin: (max) => channel.stdin(max),
    });
    let instance;
    instance = await WebAssembly.instantiate(data.wasm, { wasi_snapshot_preview1: wasi.imports, ristretto_jit: jitHost(() => instance) });
    wasi.memory = instance.exports.memory;
    try {
      instance.exports._start();
      code = 0;
    } catch (e) {
      if (!(e instanceof Exit)) throw e;
      code = e.code;
    }
  } catch (e) {
    // A trap (a stack overflow, memory exhausted): its message goes to the log.
    try {
      channel.stderr(new TextEncoder().encode(`\n[jvm worker] ${e.stack || e}\n`));
    } catch {}
  }
  channel.exit(code);
  postMessage({ exit: code });
}

async function checkJvm(data) {
  const started = performance.now();
  let instance;
  const finish = (exit, error) => {
    postMessage({
      exit,
      error: error && String(error.stack || error),
      seconds: (performance.now() - started) / 1000,
      memory: instance ? instance.exports.memory.buffer.byteLength : 0,
    });
  };
  try {
    const mounts = {};
    for (const mount of data.mounts) mounts[mount.at] = mount.http ? HttpFs.load(mount.http) : new MemFs();
    // Output goes up a line at a time, not a write at a time.
    const lines = (key) => {
      let pending = "";
      const decoder = new TextDecoder();
      return {
        write(bytes) {
          pending += decoder.decode(bytes, { stream: true });
          const end = pending.lastIndexOf("\n");
          if (end >= 0) {
            postMessage({ [key]: pending.slice(0, end) });
            pending = pending.slice(end + 1);
          }
        },
        flush() {
          if (pending) postMessage({ [key]: pending });
          pending = "";
        },
      };
    };
    const out = lines("out");
    const err = lines("err");
    const wasi = new Wasi({
      args: data.args,
      env: data.env,
      mounts,
      stdout: (bytes) => out.write(bytes),
      stderr: (bytes) => err.write(bytes),
    });
    const module = data.wasm instanceof WebAssembly.Module ? data.wasm : await WebAssembly.compileStreaming(fetch(data.wasm));
    instance = await WebAssembly.instantiate(module, { wasi_snapshot_preview1: wasi.imports, ristretto_jit: jitHost(() => instance) });
    wasi.memory = instance.exports.memory;
    let code = 0;
    try {
      instance.exports._start();
    } catch (e) {
      if (!(e instanceof Exit)) throw e;
      code = e.code;
    }
    out.flush();
    err.flush();
    // `collect`: a folder whose files the page wants back, `{ files: [[path, bytes], ...] }`.
    if (data.collect) {
      const { fs, rel } = wasi.resolve(data.collect.replace(/^\/+/, ""));
      const files = [];
      const walk = (dir) => {
        for (const name of fs.list(dir)) {
          const path = dir ? `${dir}/${name}` : name;
          if (fs.stat(path).type === "dir") walk(path);
          else files.push([path.slice(rel.length).replace(/^\//, ""), fs.read(path, 0, Infinity).slice()]);
        }
      };
      if (fs.stat(rel)) walk(rel);
      postMessage({ files }, files.map(([, bytes]) => bytes.buffer));
    }
    finish(code);
  } catch (e) {
    finish(-1, e);
  }
}
