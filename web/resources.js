// Horizon's prepared game files in the browser. Horizon ships none of Mojang's files: the player
// prepares them from their own Minecraft with `horizon-prepare --prepare` (it needs Java, so it
// runs natively), then picks the folder here. The files go into the game's in-memory filesystem
// under ROOT, and a copy into the page's private storage (OPFS), so the next visit loads them from
// there without asking again.

export const ROOT = "/resources";
const STORE = "horizon-resources";
const MANIFEST = "horizon-resources.json";

// Sounds are the bulk of the folder and the browser build plays none yet.
const skip = (path, withSounds) =>
  !withSounds && (path.startsWith("assets/minecraft/sounds/") || path.endsWith(".ogg"));

async function storeDir(create) {
  const root = await navigator.storage.getDirectory();
  return root.getDirectoryHandle(STORE, { create });
}

async function dirAt(base, parts, create) {
  let dir = base;
  for (const part of parts) dir = await dir.getDirectoryHandle(part, { create });
  return dir;
}

/** Whether a stored copy exists. */
export async function stored() {
  try {
    await (await storeDir(false)).getFileHandle(MANIFEST);
    return true;
  } catch {
    return false;
  }
}

/** Removes the stored copy. */
export async function forget() {
  const root = await navigator.storage.getDirectory();
  await root.removeEntry(STORE, { recursive: true }).catch(() => {});
}

/**
 * Imports the files of a picked folder (an <input webkitdirectory> FileList): the folder holding
 * horizon-resources.json, wherever it sits in what was picked. Writes each into the game's
 * filesystem and the stored copy. `progress(done, total)` as it goes.
 */
export async function importFiles(horizon, fileList, options = {}) {
  const files = [...fileList];
  const manifests = files
    .map((f) => f.webkitRelativePath || f.name)
    .filter((p) => p === MANIFEST || p.endsWith("/" + MANIFEST))
    .sort((a, b) => a.length - b.length);
  if (!manifests.length) {
    throw new Error(`no ${MANIFEST} in the picked folder: pick the folder horizon-prepare wrote (.local/horizon-client/<version>)`);
  }
  const prefix = manifests[0].slice(0, -MANIFEST.length);
  const entries = files
    .filter((file) => (file.webkitRelativePath || file.name).startsWith(prefix))
    .map((file) => ({ path: (file.webkitRelativePath || file.name).slice(prefix.length), bytes: async () => new Uint8Array(await file.arrayBuffer()) }));
  return importEntries(horizon, entries, options);
}

/**
 * Imports the folder `web/serve.py --resources` serves at `base` (its file list at
 * `base/index.json`): the page's `?fetch`, while developing.
 */
export async function importServed(horizon, base, options = {}) {
  const paths = await (await fetch(`${base}/index.json`)).json();
  if (!paths.includes(MANIFEST)) throw new Error(`no ${MANIFEST} at ${base}`);
  const entries = paths.map((path) => ({
    path,
    bytes: async () => {
      const response = await fetch(`${base}/${path.split("/").map(encodeURIComponent).join("/")}`);
      if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
      return new Uint8Array(await response.arrayBuffer());
    },
  }));
  return importEntries(horizon, entries, options);
}

async function importEntries(horizon, entries, { withSounds = false, progress = () => {} } = {}) {
  const chosen = entries.filter(({ path }) => !skip(path, withSounds));
  await forget();
  // Without this the browser may drop the stored copy when it runs short of space.
  await navigator.storage.persist?.().catch(() => false);
  const store = await storeDir(true);
  let done = 0;
  const one = async ({ path, bytes: read }) => {
    const bytes = await read();
    horizon.writeFile(`${ROOT}/${path}`, bytes);
    const parts = path.split("/");
    const name = parts.pop();
    const handle = await (await dirAt(store, parts, true)).getFileHandle(name, { create: true });
    const writable = await handle.createWritable();
    await writable.write(bytes);
    await writable.close();
    if (++done % 100 === 0 || done === chosen.length) progress(done, chosen.length);
  };
  // A few at a time: reading and storing overlap, and the order doesn't matter.
  let next = 0;
  await Promise.all(Array.from({ length: 16 }, async () => {
    while (next < chosen.length) await one(chosen[next++]);
  }));
  return chosen.length;
}

/** Loads the stored copy into the game's filesystem; the number of files. */
export async function load(horizon, { progress = () => {} } = {}) {
  const store = await storeDir(false);
  let done = 0;
  const walk = async (dir, path) => {
    for await (const [name, handle] of dir.entries()) {
      const at = path ? `${path}/${name}` : name;
      if (handle.kind === "directory") {
        await walk(handle, at);
      } else {
        const file = await handle.getFile();
        horizon.writeFile(`${ROOT}/${at}`, new Uint8Array(await file.arrayBuffer()));
        if (++done % 200 === 0) progress(done);
      }
    }
  };
  await walk(store, "");
  progress(done);
  return done;
}
