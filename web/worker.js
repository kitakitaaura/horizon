// A worker of the thread pool (spawner.js): it sets the module up on the shared memory and runs
// one Rust thread (workerMain waits for it), then frees that thread's stack and thread-locals and
// sets the module up again for the next one. Each thread gets a fresh instance, as on a new
// worker, without the cost of starting one: the game starts short threads all the time.
self.onmessage = async ({ data }) => {
  const { module, memory, stackSize } = data;
  for (let run = 0; ; run++) {
    // A new copy of the glue each time: wasm-bindgen's glue holds one instance.
    const glue = await import(run === 0 ? "./pkg/horizon_web.js" : `./pkg/horizon_web.js?run=${run}`);
    const wasm = glue.initSync({ module, memory, thread_stack_size: stackSize });
    if (run === 0) self.postMessage("ready");
    glue.workerMain();
    wasm.__wbindgen_thread_destroy?.();
  }
};
