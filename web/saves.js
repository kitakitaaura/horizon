// The game's own files in the browser: worlds (/saves), options and the rest of the game folder.
// The game writes them to its in-memory filesystem; this keeps a copy in the page's private
// storage (OPFS), restores it before the game starts, and mirrors changes back every few seconds
// and when the tab is hidden or closed. The prepared game files (resources.js) and logs stay out.

const STORE = "horizon-game";
// From the filesystem's root: the prepared files, scratch space and what's only for this visit.
const SKIP = ["resources", "tmp", "horizon/logs"];

async function storeDir(create = true) {
  const root = await navigator.storage.getDirectory();
  return root.getDirectoryHandle(STORE, { create });
}

async function dirAt(base, parts, create) {
  let dir = base;
  for (const part of parts) dir = await dir.getDirectoryHandle(part, { create });
  return dir;
}

// path -> "size:modified", as the filesystem lists it.
function listing(horizon) {
  const files = new Map();
  for (const line of horizon.listFiles("/", SKIP).split("\n")) {
    if (!line) continue;
    const [path, size, modified] = line.split("\t");
    files.set(path, `${size}:${modified}`);
  }
  return files;
}

/** Copies the stored game folder into the game's filesystem; the number of files. */
export async function restore(horizon) {
  const store = await storeDir();
  let count = 0;
  const walk = async (dir, path) => {
    for await (const [name, handle] of dir.entries()) {
      const at = path ? `${path}/${name}` : name;
      if (handle.kind === "directory") {
        await walk(handle, at);
      } else {
        const file = await handle.getFile();
        horizon.writeFile(`/${at}`, new Uint8Array(await file.arrayBuffer()));
        count++;
      }
    }
  };
  await walk(store, "");
  return count;
}

/**
 * Mirrors the game folder into storage from now on: what changed since the last pass is written,
 * what was removed is removed. Returns `flush()`, which runs a pass now.
 */
export function keep(horizon, { every = 5000, log = () => {} } = {}) {
  navigator.storage.persist?.().catch(() => false);
  // What the store holds, as of the filesystem right after `restore`.
  let stored = listing(horizon);
  let running = null;
  const pass = async () => {
    const now = listing(horizon);
    const store = await storeDir();
    let written = 0;
    let removed = 0;
    for (const [path, stamp] of now) {
      if (stored.get(path) === stamp) continue;
      const bytes = horizon.readFile(`/${path}`);
      if (!bytes) continue;
      const parts = path.split("/");
      const name = parts.pop();
      const handle = await (await dirAt(store, parts, true)).getFileHandle(name, { create: true });
      // createWritable writes to a scratch copy and swaps it in on close: a pass cut short
      // leaves the previous copy whole.
      const writable = await handle.createWritable();
      await writable.write(bytes);
      await writable.close();
      stored.set(path, stamp);
      written++;
    }
    for (const path of [...stored.keys()]) {
      if (now.has(path)) continue;
      const parts = path.split("/");
      const name = parts.pop();
      try {
        await (await dirAt(store, parts, false)).removeEntry(name);
      } catch {}
      stored.delete(path);
      removed++;
    }
    if (written || removed) log(`saved ${written} files, removed ${removed}`);
  };
  const flush = () => {
    // One pass at a time; a flush during one waits for it and runs another.
    running = (running || Promise.resolve()).then(pass).catch((e) => console.error("Horizon: saving the game folder failed", e));
    const mine = running;
    mine.finally(() => { if (running === mine) running = null; });
    return mine;
  };
  setInterval(flush, every);
  document.addEventListener("visibilitychange", () => { if (document.visibilityState === "hidden") flush(); });
  addEventListener("pagehide", flush);
  return flush;
}

/** Removes the stored game folder: every world and option kept in this browser. */
export async function forget() {
  const root = await navigator.storage.getDirectory();
  await root.removeEntry(STORE, { recursive: true }).catch(() => {});
}
