// Zip files in the page, with no library: reading a picked .zip (a File or any Blob) entry by entry,
// and writing one. Entries are inflated and deflated with the browser's own
// DecompressionStream/CompressionStream ("deflate-raw"), one at a time, so a zip of several hundred MB
// never sits in memory whole. ZIP64 is read and written, for archives past 4 GB or 65 535 entries.

const u16 = (v, o) => v.getUint16(o, true);
const u32 = (v, o) => v.getUint32(o, true);
const u64 = (v, o) => Number(v.getBigUint64(o, true));
const MAX32 = 0xffffffff;

async function bytesOf(blob) {
  return new Uint8Array(await blob.arrayBuffer());
}

async function piped(blob, transform) {
  return new Uint8Array(await new Response(blob.stream().pipeThrough(transform)).arrayBuffer());
}

/**
 * Opens `blob` as a zip: `{ name, paths(), read(path) }`, `read` giving an entry's bytes or null.
 * Folders and macOS's `__MACOSX/` and `._` resource-fork files are left out.
 */
export async function openZip(blob, name = blob.name || "") {
  const tailSize = Math.min(blob.size, 22 + 0xffff);
  const tail = await bytesOf(blob.slice(blob.size - tailSize));
  const tv = new DataView(tail.buffer);
  let end = -1;
  for (let i = tail.length - 22; i >= 0; i--) {
    if (u32(tv, i) === 0x06054b50) { end = i; break; }
  }
  if (end < 0) throw new Error(`${name || "the file"} is not a zip file`);
  let count = u16(tv, end + 10);
  let dirSize = u32(tv, end + 12);
  let dirOffset = u32(tv, end + 16);
  if (count === 0xffff || dirSize === MAX32 || dirOffset === MAX32) {
    // ZIP64: the locator sits just before the end record and points at the ZIP64 end record.
    const locator = end - 20;
    if (locator < 0 || u32(tv, locator) !== 0x07064b50) throw new Error(`${name}: damaged ZIP64 archive`);
    const at = u64(tv, locator + 8);
    const record = new DataView((await bytesOf(blob.slice(at, at + 56))).buffer);
    if (u32(record, 0) !== 0x06064b50) throw new Error(`${name}: damaged ZIP64 archive`);
    count = u64(record, 32);
    dirSize = u64(record, 40);
    dirOffset = u64(record, 48);
  }
  const dir = await bytesOf(blob.slice(dirOffset, dirOffset + dirSize));
  const dv = new DataView(dir.buffer);
  const utf8 = new TextDecoder();
  const entries = new Map();
  for (let p = 0, n = 0; n < count; n++) {
    if (u32(dv, p) !== 0x02014b50) throw new Error(`${name}: damaged zip directory`);
    const flags = u16(dv, p + 8);
    const method = u16(dv, p + 10);
    let compressed = u32(dv, p + 20);
    let size = u32(dv, p + 24);
    const nameLength = u16(dv, p + 28);
    const extraLength = u16(dv, p + 30);
    const commentLength = u16(dv, p + 32);
    let local = u32(dv, p + 42);
    const path = utf8.decode(dir.subarray(p + 46, p + 46 + nameLength));
    // ZIP64 extra field: the 64-bit values, in this order, of the fields that hold 0xFFFFFFFF.
    for (let e = p + 46 + nameLength; e < p + 46 + nameLength + extraLength; ) {
      const id = u16(dv, e);
      const length = u16(dv, e + 2);
      if (id === 0x0001) {
        let q = e + 4;
        if (size === MAX32) { size = u64(dv, q); q += 8; }
        if (compressed === MAX32) { compressed = u64(dv, q); q += 8; }
        if (local === MAX32) { local = u64(dv, q); q += 8; }
      }
      e += 4 + length;
    }
    p += 46 + nameLength + extraLength + commentLength;
    const file = path.split("/").pop();
    if (path.endsWith("/") || path.startsWith("__MACOSX/") || file.startsWith("._")) continue;
    entries.set(path, { flags, method, compressed, size, local });
  }
  return {
    name,
    paths: () => [...entries.keys()],
    async read(path) {
      const entry = entries.get(path);
      if (!entry) return null;
      if (entry.flags & 1) throw new Error(`${path} in ${name} is encrypted`);
      const header = new DataView((await bytesOf(blob.slice(entry.local, entry.local + 30))).buffer);
      if (u32(header, 0) !== 0x04034b50) throw new Error(`${name}: damaged entry ${path}`);
      const start = entry.local + 30 + u16(header, 26) + u16(header, 28);
      const data = blob.slice(start, start + entry.compressed);
      if (entry.method === 0) return bytesOf(data);
      if (entry.method === 8) return piped(data, new DecompressionStream("deflate-raw"));
      throw new Error(`${path} in ${name} uses compression method ${entry.method}; re-zip it with ordinary (deflate) compression`);
    },
  };
}

/**
 * A zip's files under the shallowest folder that holds `marker` (a path such as
 * `horizon-resources.json` or `assets/indexes`): a zip of a folder usually wraps it in one more
 * folder. `{ name, paths(), read(path) }` with paths relative to that folder, or null.
 */
export function within(zip, marker) {
  // The folder a path has `marker` in, as a whole path component: "a/b/" for "a/b/<marker>/c".
  const folderOf = (path) => {
    for (let i = path.indexOf(marker); i >= 0; i = path.indexOf(marker, i + 1)) {
      const after = path[i + marker.length];
      if ((i === 0 || path[i - 1] === "/") && (after === undefined || after === "/")) return path.slice(0, i);
    }
    return null;
  };
  const prefixes = zip.paths().map(folderOf).filter((p) => p !== null).sort((a, b) => a.length - b.length);
  if (!prefixes.length) return null;
  const prefix = prefixes[0];
  return {
    name: zip.name,
    paths: () => zip.paths().filter((p) => p.startsWith(prefix)).map((p) => p.slice(prefix.length)),
    read: (path) => zip.read(prefix + path),
  };
}

const CRC = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes) {
  let c = MAX32;
  for (let i = 0; i < bytes.length; i++) c = CRC[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ MAX32) >>> 0;
}

// Formats whose bytes are compressed already: deflating them again costs time and saves nothing.
const STORED = /\.(png|ogg|jpe?g|zip|jar|gz|nbt|mca|webp)$/i;

/**
 * Writes a zip of `entries` (an async iterable of `{ path, bytes }`, bytes a Uint8Array) to `sink`
 * (`{ write(Uint8Array), close() }`, both may return promises). Returns the number of entries.
 */
export async function writeZip(entries, sink) {
  const utf8 = new TextEncoder();
  const central = [];
  let offset = 0;
  const now = new Date();
  const time = (now.getHours() << 11) | (now.getMinutes() << 5) | (now.getSeconds() >> 1);
  const date = ((now.getFullYear() - 1980) << 9) | ((now.getMonth() + 1) << 5) | now.getDate();
  const put = async (bytes) => {
    await sink.write(bytes);
    offset += bytes.length;
  };
  for await (const { path, bytes } of entries) {
    const name = utf8.encode(path);
    const crc = crc32(bytes);
    const store = STORED.test(path) || bytes.length < 64;
    const data = store ? bytes : await piped(new Blob([bytes]), new CompressionStream("deflate-raw"));
    const method = store ? 0 : 8;
    const zip64 = offset >= MAX32 || bytes.length >= MAX32;
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(4, zip64 ? 45 : 20, true);
    local.setUint16(6, 0x0800, true); // names are UTF-8
    local.setUint16(8, method, true);
    local.setUint16(10, time, true);
    local.setUint16(12, date, true);
    local.setUint32(14, crc, true);
    local.setUint32(18, Math.min(data.length, MAX32), true);
    local.setUint32(22, Math.min(bytes.length, MAX32), true);
    local.setUint16(26, name.length, true);
    central.push({ name, crc, method, compressed: data.length, size: bytes.length, offset });
    await put(new Uint8Array(local.buffer));
    await put(name);
    await put(data);
  }
  const dirStart = offset;
  for (const e of central) {
    const big = e.offset >= MAX32 || e.size >= MAX32 || e.compressed >= MAX32;
    const extra = big ? new DataView(new ArrayBuffer(28)) : null;
    if (extra) {
      extra.setUint16(0, 0x0001, true);
      extra.setUint16(2, 24, true);
      extra.setBigUint64(4, BigInt(e.size), true);
      extra.setBigUint64(12, BigInt(e.compressed), true);
      extra.setBigUint64(20, BigInt(e.offset), true);
    }
    const head = new DataView(new ArrayBuffer(46));
    head.setUint32(0, 0x02014b50, true);
    head.setUint16(4, big ? 45 : 20, true);
    head.setUint16(6, big ? 45 : 20, true);
    head.setUint16(8, 0x0800, true);
    head.setUint16(10, e.method, true);
    head.setUint16(12, time, true);
    head.setUint16(14, date, true);
    head.setUint32(16, e.crc, true);
    head.setUint32(20, big ? MAX32 : e.compressed, true);
    head.setUint32(24, big ? MAX32 : e.size, true);
    head.setUint16(28, e.name.length, true);
    head.setUint16(30, extra ? 28 : 0, true);
    head.setUint32(42, big ? MAX32 : e.offset, true);
    await put(new Uint8Array(head.buffer));
    await put(e.name);
    if (extra) await put(new Uint8Array(extra.buffer));
  }
  const dirSize = offset - dirStart;
  const zip64 = central.length >= 0xffff || dirStart >= MAX32 || dirSize >= MAX32;
  if (zip64) {
    const record = new DataView(new ArrayBuffer(56 + 20));
    record.setUint32(0, 0x06064b50, true);
    record.setBigUint64(4, 44n, true);
    record.setUint16(12, 45, true);
    record.setUint16(14, 45, true);
    record.setBigUint64(24, BigInt(central.length), true);
    record.setBigUint64(32, BigInt(central.length), true);
    record.setBigUint64(40, BigInt(dirSize), true);
    record.setBigUint64(48, BigInt(dirStart), true);
    record.setUint32(56, 0x07064b50, true);
    record.setBigUint64(64, BigInt(offset), true);
    record.setUint32(72, 1, true);
    await put(new Uint8Array(record.buffer));
  }
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, Math.min(central.length, 0xffff), true);
  end.setUint16(10, Math.min(central.length, 0xffff), true);
  end.setUint32(12, zip64 ? MAX32 : dirSize, true);
  end.setUint32(16, zip64 ? MAX32 : dirStart, true);
  await put(new Uint8Array(end.buffer));
  await sink.close();
  return central.length;
}

/**
 * Where a zip goes: a file the player names (File System Access API, written as it goes), else a
 * download built from the parts in memory.
 */
export async function saveSink(suggestedName) {
  if (self.showSaveFilePicker) {
    const handle = await showSaveFilePicker({
      suggestedName,
      types: [{ description: "Zip archive", accept: { "application/zip": [".zip"] } }],
    });
    return handle.createWritable();
  }
  const parts = [];
  return {
    write(bytes) { parts.push(bytes); },
    close() {
      const url = URL.createObjectURL(new Blob(parts, { type: "application/zip" }));
      const a = Object.assign(document.createElement("a"), { href: url, download: suggestedName });
      document.body.append(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
    },
  };
}
