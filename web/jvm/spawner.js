// Starts the Java runtimes preparation asks for (crates/horizon-prepare/src/browser_jvm.rs): a
// worker that runs no wasm, like ../spawner.js, so its event loop is free while the preparation
// threads block. It waits on the mailbox's counter, takes each channel address from its slots, and
// starts a JVM worker (worker.js) on that channel; it stops a JVM whose channel asks it to (KILL),
// and marks a stopped one's channel GONE.
//
// Posted once: `{ memory, mailbox, wasm, jdk, env }`: the game's shared memory, the mailbox's
// address (`horizon.jvmMailboxAddr()`), the runtime's .wasm URL, the Java image's base URL
// (served with an index, as web/serve.py --mount does) and variables every JVM gets
// (`RISTRETTO_JIT`...).

const STATE = 0, FLAGS = 7, SPEC = 8;
const KILL = 1, GONE = 2;
const SLOTS = 32;
const HEADER = 64;

self.onmessage = async ({ data }) => {
  const { memory, mailbox, jdk, env = {} } = data;
  const wasm = await WebAssembly.compileStreaming(fetch(data.wasm));
  const count = new Int32Array(memory.buffer, mailbox, 1);
  const slots = new Uint32Array(memory.buffer, mailbox + 4, SLOTS);
  /** channel address -> { worker, words } */
  const live = new Map();

  const gone = (address) => {
    const jvm = live.get(address);
    if (!jvm) return;
    jvm.worker.terminate();
    live.delete(address);
    Atomics.or(jvm.words, FLAGS, GONE);
    Atomics.notify(jvm.words, FLAGS);
    Atomics.notify(jvm.words, STATE);
  };

  const start = (address) => {
    const words = new Int32Array(memory.buffer, address, 16);
    const length = Atomics.load(words, SPEC);
    const spec = JSON.parse(new TextDecoder().decode(new Uint8Array(memory.buffer, address + HEADER, length).slice()));
    const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module", name: "java" });
    live.set(address, { worker, words });
    worker.onmessage = () => gone(address);
    worker.onerror = (e) => {
      console.error("[jvm spawner] a Java worker failed:", e.message || e);
      gone(address);
    };
    worker.postMessage({ wasm, memory, channel: address, args: spec.args, env: { ...spec.env, ...env }, jdk });
  };

  for (;;) {
    const seen = Atomics.load(count, 0);
    for (let i = 0; i < SLOTS; i++) {
      const address = Atomics.exchange(slots, i, 0);
      if (address) start(address);
    }
    for (const [address, jvm] of live) {
      if (Atomics.load(jvm.words, FLAGS) & KILL) gone(address);
    }
    const wait = Atomics.waitAsync(count, 0, seen, 1000);
    if (wait.async) await wait.value;
  }
};
