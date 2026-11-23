// A whole preparation in the page, from folders web/serve.py serves, while developing (`?prepare`):
// the Minecraft folder (`--minecraft`), Horizon's exporter sources (`--mount java-src=<repo>/mods/
// horizon/src/test/java`) and their prebuilt classes (`--mount exporters=<repo>/.local/exporters`,
// written by a native `horizon-prepare --prepare`), which go into the game's filesystem; then
// `horizon-prepare --prepare` runs there, its exporters on the page's Java runtimes. Each step's
// time goes to the console and to `say`.

import { servedFolder } from "./minecraft.js";
import * as resources from "./resources.js";

const SOURCES = "/mods/horizon/src/test/java";
const ASSET_KINDS = ["sounds", "panorama", "fonts", "packs", "languages"];
// horizon-prepare's record of the steps an unfinished preparation finished (lib.rs PROGRESS).
const PROGRESS = ".horizon-prepare-progress.json";
// The exporters' scratch folder, kept beside the stored resources while a preparation runs.
const LOCAL = "/.local";
const LOCAL_STORE = "horizon-prepare-local";

/**
 * Keeps the page's storage up to date with a preparation's files as they are written, so a tab
 * that is discarded or closed can continue where it was: `/resources` into the stored copy (where
 * it stays incomplete, with no manifest, until the end) and `/.local` beside it. Each call copies
 * what changed since the last; the progress record goes last, after the files it vouches for.
 */
function mirror(horizon) {
  const seen = new Map();
  let running = null;
  const changed = (root, skip) =>
    horizon.listFiles(root, skip).split("\n").filter(Boolean).map((l) => l.split("\t"))
      .filter(([path, size, time]) => seen.get(`${root}/${path}`) !== `${size}:${time}`)
      .map(([path, size, time]) => {
        seen.set(`${root}/${path}`, `${size}:${time}`);
        return path;
      });
  const sync = async () => {
    const local = changed(LOCAL, ["minecraft"]);
    await resources.storePaths(horizon, local, { from: LOCAL, store: LOCAL_STORE });
    const files = changed(resources.ROOT, []);
    await resources.storePaths(horizon, files.filter((p) => p !== PROGRESS));
    if (files.includes(PROGRESS)) await resources.storePaths(horizon, [PROGRESS]);
  };
  // One copy at a time; a call while one runs waits for it, then copies again.
  return () => (running = (running || Promise.resolve()).then(sync, sync));
}

async function forgetLocal() {
  const root = await navigator.storage.getDirectory();
  await root.removeEntry(LOCAL_STORE, { recursive: true }).catch(() => {});
}

async function index(base) {
  const response = await fetch(`${base}/.index.json`);
  if (!response.ok) throw new Error(`${base}/.index.json: HTTP ${response.status} (start web/serve.py with that --mount)`);
  return response.json();
}

async function fetchBytes(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

/** Copies a served mount into the filesystem at `to`; the number of files. */
async function copyMount(horizon, base, to) {
  const files = await index(base);
  let next = 0;
  await Promise.all(Array.from({ length: 16 }, async () => {
    while (next < files.length) {
      const [path] = files[next++];
      horizon.writeFile(`${to}/${path}`, await fetchBytes(`${base}/${path.split("/").map(encodeURIComponent).join("/")}`));
    }
  }));
  return files.length;
}

/** Runs the preparation into `output`; resolves with the folder, or rejects with its error. */
export async function prepareFromServed(horizon, output, options = {}) {
  // One preparation per browser profile: two tabs would share (and spoil) the stored progress.
  if (!navigator.locks) return prepareAlone(horizon, output, options);
  return navigator.locks.request("horizon-prepare", { ifAvailable: true }, (lock) => {
    if (!lock) throw new Error("Another tab is preparing Horizon's files: let it finish there");
    return prepareAlone(horizon, output, options);
  });
}

async function prepareAlone(horizon, output, { say = () => {} } = {}) {
  const started = performance.now();
  const at = () => `${((performance.now() - started) / 1000).toFixed(1)} s`;
  const step = (text) => {
    say(text);
    console.log(`[prepare ${at()}] ${text}`);
  };

  // An unfinished preparation's files, from the page's storage: its finished steps are skipped.
  if (await resources.storedFile(PROGRESS)) {
    step("Continuing the preparation that stopped…");
    await resources.load(horizon, { progress: resources.timeLeft(say, "Restoring prepared files") });
    await resources.load(horizon, { store: LOCAL_STORE, to: LOCAL }).catch(() => 0);
  } else {
    await resources.forget();
    await forgetLocal();
  }

  const pins = JSON.parse(horizon.preparePins());
  const minecraft = pins.minecraft;
  const folder = servedFolder();

  step("Copying Horizon's exporters…");
  await copyMount(horizon, "/mnt/java-src", SOURCES);
  const key = horizon.prepareExportersKey();
  const bundles = await index("/mnt/exporters");
  if (!bundles.some(([path]) => path === `${key}/manifest.json`)) {
    throw new Error(`no prebuilt exporters with key ${key}: run horizon-prepare --prepare natively once`);
  }
  const bundle = bundles.filter(([path]) => path.startsWith(`${key}/`));
  for (const [path] of bundle) horizon.writeFile(`/exporters/${path.slice(key.length + 1)}`, await fetchBytes(`/mnt/exporters/${path}`));

  step(`Reading Minecraft ${pins.version} from ${folder.name}…`);
  const response = await fetch(`https://piston-meta.mojang.com/v1/packages/${pins.manifest_sha1}/${pins.version}.json`);
  if (!response.ok) throw new Error(`Mojang's ${pins.version} version file: HTTP ${response.status}`);
  const manifestBytes = new Uint8Array(await response.arrayBuffer());
  const manifest = JSON.parse(new TextDecoder().decode(manifestBytes));
  horizon.writeFile(`${minecraft}/versions/${pins.version}/${pins.version}.json`, manifestBytes);
  const want = [`versions/${pins.version}/${pins.version}.jar`];
  for (const library of manifest.libraries || []) {
    const path = library.downloads?.artifact?.path;
    if (path) want.push(`libraries/${path}`);
  }
  const indexPath = `assets/indexes/${manifest.assetIndex.id}.json`;
  const assetIndex = await folder.read(indexPath);
  if (!assetIndex) throw new Error(`${folder.name} has no ${indexPath}`);
  horizon.writeFile(`${minecraft}/${indexPath}`, assetIndex);
  const objects = new Set();
  for (const kind of ASSET_KINDS) for (const hash of horizon.prepareWanted(kind, assetIndex)) objects.add(`assets/objects/${hash.slice(0, 2)}/${hash}`);
  want.push(...objects);
  let next = 0;
  let copied = 0;
  let bytes = 0;
  await Promise.all(Array.from({ length: 16 }, async () => {
    while (next < want.length) {
      const path = want[next++];
      const data = await folder.read(path);
      if (!data) continue;
      horizon.writeFile(`${minecraft}/${path}`, data);
      copied++;
      bytes += data.length;
      if (copied % 500 === 0) say(`Reading Minecraft… ${copied} / ${want.length}`);
    }
  }));
  step(`Read ${copied} files (${(bytes / 2 ** 20).toFixed(0)} MiB)`);

  step("Preparing…");
  const title = document.title;
  const keep = mirror(horizon);
  // Closing the tab loses the steps under way: ask first.
  const warn = (event) => event.preventDefault();
  addEventListener("beforeunload", warn);
  const running = new Map();
  const started0 = performance.now();
  horizon.prepareStart(output);
  try {
    for (;;) {
      await new Promise((r) => setTimeout(r, 250));
      const news = JSON.parse(horizon.preparePoll());
      for (const line of news.lines) {
        step(line);
        // The steps under way, from their "Exporting …" and "Exported … in" lines.
        const started = line.match(/^Exporting (.+?) \(/);
        const ended = line.match(/^Exported (.+?) in /);
        if (started) running.set(started[1], performance.now());
        if (ended) running.delete(ended[1]);
      }
      if (!news.lines.length && running.size) {
        const now = performance.now();
        const mmss = (ms) => `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
        const list = [...running].map(([what, since]) => `${what} ${mmss(now - since)}`).join(" · ");
        say(`Running ${running.size} Java step${running.size > 1 ? "s" : ""} (${mmss(now - started0)} in): ${list}`);
      }
      // The tab's title shows how far it got, also from another tab.
      document.title = `${Math.round(news.progress * 100)}% · Preparing Horizon`;
      if (news.lines.length) keep();
      if (news.done) {
        if (news.error) throw new Error(news.error);
        await keep();
        // Files a step wrote and then removed (a `.partial` it renamed) were stored on the way.
        const final = horizon.listFiles(resources.ROOT, []).split("\n").filter(Boolean).map((l) => l.split("\t")[0]);
        await resources.pruneStored(new Set(final));
        await resources.removeStored(PROGRESS);
        await forgetLocal();
        step(`Prepared in ${at()}`);
        notify(`Horizon is ready: prepared in ${at()}`);
        return news.output;
      }
    }
  } catch (e) {
    notify(`Horizon's preparation stopped: ${e.message || e}`);
    throw e;
  } finally {
    removeEventListener("beforeunload", warn);
    document.title = title;
  }
}

/** A system notification when the page is not in front and the player allowed them. */
function notify(text) {
  if (document.visibilityState === "visible" || !self.Notification || Notification.permission !== "granted") return;
  try {
    new Notification("Horizon", { body: text });
  } catch {}
}
