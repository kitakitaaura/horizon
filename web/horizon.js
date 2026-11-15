// Loads Horizon's module on the page and starts the thread pool's spawner (spawner.js): std
// hands each new Rust thread to an idle worker at once, and the spawner starts more as std asks,
// from a thread no Rust code ever blocks.
import init, * as horizon from "./pkg/horizon_web.js";

// WGSL keeps texture samples and derivatives in uniform control flow, and the browser's compiler
// enforces it; wgpu's own (naga) doesn't, and Horizon's shaders sample in branches. The standard
// diagnostic filter turns the check off for every module the game makes, as natively.
if (self.GPUDevice && !GPUDevice.prototype.__horizonDiagnostics) {
  const create = GPUDevice.prototype.createShaderModule;
  GPUDevice.prototype.createShaderModule = function (desc) {
    if (desc && typeof desc.code === "string" && !desc.code.includes("derivative_uniformity")) {
      desc = { ...desc, code: "diagnostic(off, derivative_uniformity);\n" + desc.code };
    }
    return create.call(this, desc);
  };
  GPUDevice.prototype.__horizonDiagnostics = true;
}

// Each pool worker's stack (a Rust thread's): debug builds need far more than release ones.
const STACK_SIZE = 8 << 20;

export async function start({ reserve = 32 } = {}) {
  if (!crossOriginIsolated) {
    throw new Error("the page needs COOP and COEP headers for shared memory (web/serve.py sends them)");
  }
  // Compiled once here; every worker instantiates this same module (structured-cloned, not
  // compiled again).
  const module = await WebAssembly.compileStreaming(fetch(new URL("./pkg/horizon_web_bg.wasm", import.meta.url)));
  const wasm = await init({ module_or_path: module });
  horizon.poolSetReserve(reserve);
  // The spawner keeps the pool full from its own thread (spawner.js).
  const spawner = new Worker(new URL("./spawner.js", import.meta.url), { type: "module", name: "horizon spawner" });
  spawner.postMessage({ module, memory: wasm.memory, stackSize: STACK_SIZE, wantedAddr: horizon.poolWantedAddr() });
  // Wait for the reserve before any Rust code spawns.
  while (horizon.poolIdle() < reserve) {
    await new Promise((r) => setTimeout(r, 10));
  }
  return horizon;
}
