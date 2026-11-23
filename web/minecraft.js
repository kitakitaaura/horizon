// Preparation steps from the player's own Minecraft folder (the official launcher's, SKLauncher's,
// Prism's): horizon-prepare's Rust code runs in the page (crates/horizon-web/src/prepare.rs), and
// this brings it the files it asks for. Nothing is downloaded but Mojang's version JSON for the
// pinned release, which is metadata and is checked against its pinned SHA-1.

/** The folder `web/serve.py --minecraft` serves, read as a picked one (`?fetch-minecraft`). */
export function servedFolder(base = "/minecraft-folder") {
  return {
    name: base,
    async read(path) {
      const response = await fetch(`${base}/${path.split("/").map(encodeURIComponent).join("/")}`);
      return response.ok ? new Uint8Array(await response.arrayBuffer()) : null;
    },
  };
}

/** A picked folder: `read(path)` gives a file's bytes, or null. */
export async function pickFolder() {
  if (self.showDirectoryPicker) {
    const root = await showDirectoryPicker({ id: "minecraft", mode: "read" });
    return {
      name: root.name,
      async read(path) {
        try {
          const parts = path.split("/");
          const name = parts.pop();
          let dir = root;
          for (const part of parts) dir = await dir.getDirectoryHandle(part);
          const file = await (await dir.getFileHandle(name)).getFile();
          return new Uint8Array(await file.arrayBuffer());
        } catch {
          return null;
        }
      },
    };
  }
  // Browsers without the File System Access API: a folder input, which lists every file.
  const files = await new Promise((resolve, reject) => {
    const input = Object.assign(document.createElement("input"), { type: "file", webkitdirectory: true });
    input.onchange = () => resolve([...input.files]);
    input.oncancel = () => reject(new DOMException("No folder picked", "AbortError"));
    input.click();
  });
  const byPath = new Map(files.map((f) => [f.webkitRelativePath.split("/").slice(1).join("/"), f]));
  return {
    name: files[0]?.webkitRelativePath.split("/")[0] || "",
    async read(path) {
      const file = byPath.get(path);
      return file ? new Uint8Array(await file.arrayBuffer()) : null;
    },
  };
}

/**
 * Runs preparation's asset steps (`kinds`, e.g. ["sounds"]) into the prepared files at `output`,
 * from `folder` (pickFolder's). Returns the paths it added or changed under `output`.
 */
export async function addAssets(horizon, folder, kinds, output, { say = () => {} } = {}) {
  const pins = JSON.parse(horizon.preparePins());
  say("Fetching Mojang's version file for " + pins.version + "…");
  const response = await fetch(`https://piston-meta.mojang.com/v1/packages/${pins.manifest_sha1}/${pins.version}.json`);
  if (!response.ok) throw new Error(`Mojang's ${pins.version} version file: HTTP ${response.status}`);
  const manifestBytes = new Uint8Array(await response.arrayBuffer());
  horizon.writeFile(pins.manifest_path, manifestBytes);
  const manifest = JSON.parse(new TextDecoder().decode(manifestBytes));
  const indexPath = `assets/indexes/${manifest.assetIndex.id}.json`;
  const index = await folder.read(indexPath);
  if (!index) {
    throw new Error(`${folder.name} has no ${indexPath}: pick the Minecraft folder your launcher plays ${pins.version} from, after playing it once`);
  }
  horizon.writeFile(`${pins.minecraft}/${indexPath}`, index);
  const before = new Map(horizon.listFiles(output, []).split("\n").filter(Boolean).map((l) => l.split("\t")).map(([p, s, t]) => [p, s + ":" + t]));
  for (const kind of kinds) {
    const hashes = horizon.prepareWanted(kind, index);
    let done = 0;
    const copied = [];
    let next = 0;
    // A few reads at a time: the picked folder is on disk, the copies in memory.
    await Promise.all(Array.from({ length: 16 }, async () => {
      while (next < hashes.length) {
        const hash = hashes[next++];
        const path = `assets/objects/${hash.slice(0, 2)}/${hash}`;
        const bytes = await folder.read(path);
        if (bytes) {
          horizon.writeFile(`${pins.minecraft}/${path}`, bytes);
          copied.push(path);
        }
        if (++done % 100 === 0) say(`Reading your ${kind}… ${done} / ${hashes.length}`);
      }
    }));
    say(`Preparing ${kind}…`);
    try {
      horizon.prepareAssets(kind, output);
    } finally {
      // The asset store's copies: the step has its own now.
      for (const path of copied) horizon.removeFile(`${pins.minecraft}/${path}`);
    }
  }
  const after = horizon.listFiles(output, []).split("\n").filter(Boolean).map((l) => l.split("\t"));
  return after.filter(([p, s, t]) => before.get(p) !== s + ":" + t).map(([p]) => p);
}
