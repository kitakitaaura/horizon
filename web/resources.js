// Horizon's prepared game files in the browser. Horizon ships none of Mojang's files: the player
// prepares them from their own Minecraft with `horizon-prepare --prepare` (it needs Java, so it
// runs natively), then picks the folder here, or a zip of it (one this page exported on another
// device too). The files go into the game's in-memory filesystem under ROOT, and a copy into the
// page's private storage (OPFS), so the next visit loads them from there without asking again.

import { within, writeZip } from "./zip.js";

export const ROOT = "/resources";
const STORE = "horizon-resources";
const MANIFEST = "horizon-resources.json";

// Sounds are the bulk of the folder and the browser build plays none yet.
const skip = (path, withSounds) =>
  !withSounds && (path.startsWith("assets/minecraft/sounds/") || path.endsWith(".ogg"));

async function storeDir(create, name = STORE) {
  const root = await navigator.storage.getDirectory();
  return root.getDirectoryHandle(name, { create });
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

/** Whether the stored copy holds `name` (a path under it). */
export async function storedFile(name) {
  try {
    const parts = name.split("/");
    const file = parts.pop();
    await (await dirAt(await storeDir(false), parts, false)).getFileHandle(file);
    return true;
  } catch {
    return false;
  }
}

/** Removes `name` (a path under the stored copy), if there. */
export async function removeStored(name) {
  try {
    const parts = name.split("/");
    const file = parts.pop();
    await (await dirAt(await storeDir(false), parts, false)).removeEntry(file);
  } catch {}
}

/** Removes the stored copy's files whose paths `keep` (a Set) doesn't hold; the number removed. */
export async function pruneStored(keep) {
  let removed = 0;
  const walk = async (dir, path) => {
    for await (const [name, handle] of dir.entries()) {
      const at = path ? `${path}/${name}` : name;
      if (handle.kind === "directory") await walk(handle, at);
      else if (!keep.has(at)) {
        await dir.removeEntry(name).catch(() => {});
        removed++;
      }
    }
  };
  await walk(await storeDir(false), "").catch(() => {});
  return removed;
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

/**
 * Imports a zip of prepared files (zip.js's `openZip`): the folder holding horizon-resources.json,
 * wherever it sits in the zip.
 */
export async function importZip(horizon, zip, options = {}) {
  const folder = within(zip, MANIFEST);
  if (!folder) {
    throw new Error(`no ${MANIFEST} in ${zip.name || "the zip"}: zip the folder horizon-prepare wrote (.local/horizon-client/<version>), or one Horizon exported`);
  }
  const entries = folder.paths().map((path) => ({ path, bytes: () => folder.read(path) }));
  return importEntries(horizon, entries, options);
}

/**
 * Writes the stored copy as a zip to `sink` (zip.js's `saveSink`), to import on another device.
 * `progress(done)` as it goes; returns the number of files.
 */
export async function exportZip(sink, { progress = () => {} } = {}) {
  const store = await storeDir(false);
  let done = 0;
  async function* walk(dir, path) {
    for await (const [name, handle] of dir.entries()) {
      const at = path ? `${path}/${name}` : name;
      if (handle.kind === "directory") {
        yield* walk(handle, at);
      } else {
        yield { path: at, bytes: new Uint8Array(await (await handle.getFile()).arrayBuffer()) };
        if (++done % 200 === 0) progress(done);
      }
    }
  }
  const n = await writeZip(walk(store, ""), sink);
  progress(n);
  return n;
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
  rememberCount(STORE, chosen.length);
  return chosen.length;
}

/**
 * Adds `paths` (under ROOT, as the game's filesystem has them now) to the stored copy; or, with
 * `from` and `store`, paths under `from` to another directory of the page's storage.
 */
export async function storePaths(horizon, paths, { from = ROOT, store: name = STORE } = {}) {
  const store = await storeDir(true, name);
  for (const path of paths) {
    const bytes = horizon.readFile(`${from}/${path}`);
    if (!bytes) continue;
    const parts = path.split("/");
    const name = parts.pop();
    const handle = await (await dirAt(store, parts, true)).getFileHandle(name, { create: true });
    const writable = await handle.createWritable();
    await writable.write(bytes);
    await writable.close();
  }
}

/**
 * Loads the stored copy into the game's filesystem; the number of files. With `store` and `to`,
 * another directory of the page's storage, into `to`.
 */
export async function load(horizon, { progress = () => {}, store: name = STORE, to = ROOT } = {}) {
  const store = await storeDir(false, name);
  // The walk can't count the files ahead of it: the last load's (or import's) count stands in.
  const total = knownCount(name);
  let done = 0;
  const walk = async (dir, path) => {
    for await (const [name, handle] of dir.entries()) {
      const at = path ? `${path}/${name}` : name;
      if (handle.kind === "directory") {
        await walk(handle, at);
      } else {
        const file = await handle.getFile();
        horizon.writeFile(`${to}/${at}`, new Uint8Array(await file.arrayBuffer()));
        if (++done % 200 === 0) progress(done, done < total ? total : 0);
      }
    }
  };
  await walk(store, "");
  rememberCount(name, done);
  progress(done, done);
  return done;
}

/** How many files the stored copy `name` had when last loaded or imported; 0 when unknown. */
function knownCount(name) {
  try {
    return Number(localStorage.getItem(`${name}.files`)) || 0;
  } catch {
    return 0;
  }
}

function rememberCount(name, n) {
  try {
    localStorage.setItem(`${name}.files`, String(n));
  } catch {
    // Without it the next load shows no time left.
  }
}

/**
 * A `progress(done, total)` that shows `label… done / total, about N s left` through `say`: the
 * time left at the rate so far. Without a total, only the count.
 */
export function timeLeft(say, label) {
  const started = performance.now();
  return (done, total) => {
    if (!total) return say(`${label}… ${done}`);
    let text = `${label}… ${done} / ${total}`;
    const elapsed = (performance.now() - started) / 1000;
    if (done > 0 && done < total && elapsed >= 1) {
      const left = ((total - done) * elapsed) / done;
      text += left < 60 ? `, about ${Math.ceil(left)} s left` : `, about ${Math.round(left / 60)} min left`;
    }
    say(text);
  };
}
