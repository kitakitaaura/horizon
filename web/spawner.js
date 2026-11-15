// The thread pool's spawner: a worker that runs no wasm, so its event loop is free even while
// every wasm thread (the page's main thread included) is blocked. It waits on the pool's "workers
// wanted" counter in shared memory (std's sys/web_host.rs) and starts that many pool workers
// (worker.js), each of which runs one Rust thread.

self.onmessage = ({ data }) => {
  const { module, memory, stackSize, wantedAddr } = data;
  let starting = 0;
  const spawn = () => {
    starting++;
    const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module", name: "horizon thread" });
    worker.onmessage = () => { starting--; check(); };
    worker.onerror = (e) => { starting--; console.error("Horizon worker failed", e.message || e); };
    worker.postMessage({ module, memory, stackSize });
  };
  // The counter's view: a new one each time, as the memory may have grown.
  const counter = () => new Int32Array(memory.buffer, wantedAddr, 1);
  const check = () => {
    for (let n = Atomics.load(counter(), 0) - starting; n > 0; n--) spawn();
  };
  (async () => {
    for (;;) {
      check();
      const seen = Atomics.load(counter(), 0);
      const wait = Atomics.waitAsync(counter(), 0, seen, 100);
      if (wait.async) await wait.value;
    }
  })();
};
