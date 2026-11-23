// A whole preparation in the page, from folders web/serve.py serves, while developing (`?prepare`):
// the Minecraft folder (`--minecraft`), Horizon's exporter sources (`--mount java-src=<repo>/mods/
// horizon/src/test/java`) and their prebuilt classes (`--mount exporters=<repo>/.local/exporters`,
// written by a native `horizon-prepare --prepare`), which go into the game's filesystem; then
// `horizon-prepare --prepare` runs there, its exporters on the page's Java runtimes. Each step's
// time goes to the console and to `say`.

import { servedFolder } from "./minecraft.js";

const SOURCES = "/mods/horizon/src/test/java";
const ASSET_KINDS = ["sounds", "panorama", "fonts", "packs", "languages"];

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
export async function prepareFromServed(horizon, output, { say = () => {} } = {}) {
  const started = performance.now();
  const at = () => `${((performance.now() - started) / 1000).toFixed(1)} s`;
  const step = (text) => {
    say(text);
    console.log(`[prepare ${at()}] ${text}`);
  };

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
  horizon.prepareStart(output);
  for (;;) {
    await new Promise((r) => setTimeout(r, 250));
    const news = JSON.parse(horizon.preparePoll());
    for (const line of news.lines) step(line);
    if (news.done) {
      if (news.error) throw new Error(news.error);
      step(`Prepared in ${at()}`);
      return news.output;
    }
  }
}
