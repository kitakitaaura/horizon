// WASI preview 1 for the Java runtime (Ristretto's wasm32-wasip1 build), in a worker: the 28 calls
// it imports, over a tree of mounted filesystems. A mount is any object with the methods MemFs has
// (stat, list, read, write, truncate, create, mkdir, unlink, rmdir, rename); paths given to a
// mount are relative to it, without a leading slash.

const ERR = { SUCCESS: 0, BADF: 8, EXIST: 20, INVAL: 28, IO: 29, ISDIR: 31, NOENT: 44, NOSYS: 52, NOTDIR: 54, NOTEMPTY: 55 };
const TYPE = { UNKNOWN: 0, CHAR: 2, DIR: 3, FILE: 4 };
const OFLAGS = { CREAT: 1, DIRECTORY: 2, EXCL: 4, TRUNC: 8 };
const FDFLAGS_APPEND = 1;

export class FsError extends Error {
  constructor(errno, message = "") {
    super(message || `WASI errno ${errno}`);
    this.errno = errno;
  }
}

/** Thrown by proc_exit; `code` is the exit status. */
export class Exit extends Error {
  constructor(code) {
    super(`exit ${code}`);
    this.code = code;
  }
}

/** A writable in-memory filesystem. Nodes: `{ dir: Map }` or `{ bytes: Uint8Array, size, mtime }`. */
export class MemFs {
  constructor() {
    this.root = { dir: new Map(), mtime: Date.now() };
  }
  node(path, { parent = false } = {}) {
    const parts = path ? path.split("/") : [];
    const last = parent ? parts.pop() : undefined;
    let node = this.root;
    for (const part of parts) {
      if (!node.dir) throw new FsError(ERR.NOTDIR);
      node = node.dir.get(part) ?? this.missing(node, part, parts);
      if (!node) throw new FsError(ERR.NOENT);
    }
    return parent ? [node, last] : node;
  }
  /** A child the tree doesn't hold yet: subclasses load it on demand. */
  missing() {
    return undefined;
  }
  stat(path) {
    try {
      const node = this.node(path);
      return node.dir ? { type: "dir", size: 0, mtime: node.mtime } : { type: "file", size: node.size, mtime: node.mtime };
    } catch (e) {
      if (e.errno === ERR.NOENT || e.errno === ERR.NOTDIR) return null;
      throw e;
    }
  }
  list(path) {
    const node = this.node(path);
    if (!node.dir) throw new FsError(ERR.NOTDIR);
    return [...node.dir.keys()];
  }
  file(path) {
    const node = this.node(path);
    if (node.dir) throw new FsError(ERR.ISDIR);
    return node;
  }
  read(path, offset, length) {
    const node = this.file(path);
    const end = Math.min(node.size, offset + length);
    return offset >= end ? new Uint8Array(0) : node.bytes.subarray(offset, end);
  }
  write(path, offset, bytes) {
    const node = this.file(path);
    this.ensure(node, offset + bytes.length);
    node.bytes.set(bytes, offset);
    node.size = Math.max(node.size, offset + bytes.length);
    node.mtime = Date.now();
    return bytes.length;
  }
  ensure(node, size) {
    if (node.bytes.length >= size) return;
    const grown = new Uint8Array(Math.max(size, node.bytes.length * 2, 4096));
    grown.set(node.bytes.subarray(0, node.size));
    node.bytes = grown;
  }
  truncate(path, size) {
    const node = this.file(path);
    this.ensure(node, size);
    if (size < node.size) node.bytes.fill(0, size, node.size);
    node.size = size;
    node.mtime = Date.now();
  }
  create(path) {
    const [dir, name] = this.node(path, { parent: true });
    if (!dir.dir) throw new FsError(ERR.NOTDIR);
    if (!dir.dir.has(name)) dir.dir.set(name, { bytes: new Uint8Array(0), size: 0, mtime: Date.now() });
  }
  mkdir(path) {
    const [dir, name] = this.node(path, { parent: true });
    if (!dir.dir) throw new FsError(ERR.NOTDIR);
    if (dir.dir.has(name) || this.missing(dir, name)) throw new FsError(ERR.EXIST);
    dir.dir.set(name, { dir: new Map(), mtime: Date.now() });
  }
  unlink(path) {
    const [dir, name] = this.node(path, { parent: true });
    const node = dir.dir?.get(name);
    if (!node) throw new FsError(ERR.NOENT);
    if (node.dir) throw new FsError(ERR.ISDIR);
    dir.dir.delete(name);
  }
  rmdir(path) {
    const [dir, name] = this.node(path, { parent: true });
    const node = dir.dir?.get(name);
    if (!node) throw new FsError(ERR.NOENT);
    if (!node.dir) throw new FsError(ERR.NOTDIR);
    if (node.dir.size) throw new FsError(ERR.NOTEMPTY);
    dir.dir.delete(name);
  }
  rename(from, to) {
    const [fromDir, fromName] = this.node(from, { parent: true });
    const node = fromDir.dir?.get(fromName);
    if (!node) throw new FsError(ERR.NOENT);
    const [toDir, toName] = this.node(to, { parent: true });
    if (!toDir.dir) throw new FsError(ERR.NOTDIR);
    fromDir.dir.delete(fromName);
    toDir.dir.set(toName, node);
  }
}

/**
 * A folder served over HTTP (`web/serve.py --mount`), read on demand and writable in memory: its
 * file list comes first (`<base>/.index.json`: `[[path, size], ...]`), and a file's bytes the
 * first time it is opened, with a synchronous request (allowed in workers).
 */
export class HttpFs extends MemFs {
  constructor(base, index) {
    super();
    this.base = base;
    for (const [path, size] of index) {
      const parts = path.split("/");
      const name = parts.pop();
      let dir = this.root;
      for (const part of parts) {
        if (!dir.dir.has(part)) dir.dir.set(part, { dir: new Map(), mtime: 0 });
        dir = dir.dir.get(part);
      }
      dir.dir.set(name, { remote: path, size, mtime: 0, bytes: null });
    }
  }
  static load(base) {
    const request = new XMLHttpRequest();
    request.open("GET", `${base}/.index.json`, false);
    request.send();
    if (request.status !== 200) throw new Error(`${base}/.index.json: HTTP ${request.status}`);
    return new HttpFs(base, JSON.parse(request.responseText));
  }
  file(path) {
    const node = super.file(path);
    if (node.bytes === null) {
      const request = new XMLHttpRequest();
      request.open("GET", `${this.base}/${node.remote.split("/").map(encodeURIComponent).join("/")}`, false);
      request.responseType = "arraybuffer";
      request.send();
      if (request.status !== 200) throw new FsError(ERR.IO, `${node.remote}: HTTP ${request.status}`);
      node.bytes = new Uint8Array(request.response);
      node.size = node.bytes.length;
    }
    return node;
  }
}

const normalize = (path) => {
  const out = [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") out.pop();
    else out.push(part);
  }
  return out.join("/");
};

/**
 * The WASI imports. `mounts` maps an absolute folder ("/w", "/jdk", "/") to a filesystem; each
 * is a preopened directory. `stdout(bytes)` and `stderr(bytes)` take the program's output;
 * `stdin(max)` returns up to `max` bytes, an empty array at the end of input.
 */
export class Wasi {
  constructor({ args, env = {}, mounts, stdout, stderr, stdin = () => new Uint8Array(0) }) {
    this.args = args;
    this.env = Object.entries(env).map(([k, v]) => `${k}=${v}`);
    this.mounts = Object.entries(mounts)
      .map(([at, fs]) => ({ at: normalize(at), fs }))
      .sort((a, b) => b.at.length - a.at.length);
    this.stdio = [stdin, stdout, stderr];
    this.memory = null;
    // fd -> { kind: "dir" | "file", path (absolute, normalized), position, append, preopen }
    this.fds = new Map();
    let fd = 3;
    for (const { at } of [...this.mounts].sort((a, b) => a.at.length - b.at.length)) {
      this.fds.set(fd++, { kind: "dir", path: at, preopen: "/" + at });
    }
    this.sleeper = new Int32Array(new SharedArrayBuffer(4));
    this.imports = this.make();
  }

  view() {
    return new DataView(this.memory.buffer);
  }
  bytes(pointer, length) {
    return new Uint8Array(this.memory.buffer, pointer, length);
  }
  string(pointer, length) {
    return new TextDecoder().decode(this.bytes(pointer, length).slice());
  }

  /** The mount holding absolute path `path`, and the path inside it. */
  resolve(path) {
    for (const mount of this.mounts) {
      if (!mount.at) return { fs: mount.fs, rel: path };
      if (path === mount.at) return { fs: mount.fs, rel: "" };
      if (path.startsWith(mount.at + "/")) return { fs: mount.fs, rel: path.slice(mount.at.length + 1) };
    }
    throw new FsError(ERR.NOENT);
  }
  pathAt(dirfd, pointer, length) {
    const dir = this.fds.get(dirfd);
    if (!dir) throw new FsError(ERR.BADF);
    if (dir.kind !== "dir") throw new FsError(ERR.NOTDIR);
    const path = this.string(pointer, length);
    return normalize(path.startsWith("/") ? path : `${dir.path}/${path}`);
  }
  stat(path) {
    const { fs, rel } = this.resolve(path);
    return fs.stat(rel);
  }
  writeFilestat(pointer, stat) {
    const view = this.view();
    const ns = BigInt(Math.round((stat?.mtime ?? 0) * 1e6));
    view.setBigUint64(pointer, 0n, true);
    view.setBigUint64(pointer + 8, 0n, true);
    view.setUint8(pointer + 16, stat.type === "dir" ? TYPE.DIR : stat.type === "char" ? TYPE.CHAR : TYPE.FILE);
    view.setBigUint64(pointer + 24, 1n, true);
    view.setBigUint64(pointer + 32, BigInt(stat.size), true);
    view.setBigUint64(pointer + 40, ns, true);
    view.setBigUint64(pointer + 48, ns, true);
    view.setBigUint64(pointer + 56, ns, true);
  }

  make() {
    // Each call: errors from a filesystem become their errno; anything else is a bug, reported once.
    const call = (body) => (...args) => {
      try {
        return body(...args) ?? ERR.SUCCESS;
      } catch (e) {
        if (e instanceof FsError) return e.errno;
        if (e instanceof Exit) throw e;
        console.error("[wasi]", body.name, e);
        return ERR.IO;
      }
    };
    const putList = (list, countPointer, sizePointer) => {
      const view = this.view();
      view.setUint32(countPointer, list.length, true);
      view.setUint32(sizePointer, list.reduce((n, s) => n + new TextEncoder().encode(s).length + 1, 0), true);
    };
    const getList = (list, pointers, buffer) => {
      const view = this.view();
      for (const item of list) {
        view.setUint32(pointers, buffer, true);
        pointers += 4;
        const bytes = new TextEncoder().encode(item);
        this.bytes(buffer, bytes.length).set(bytes);
        view.setUint8(buffer + bytes.length, 0);
        buffer += bytes.length + 1;
      }
    };
    const iovecs = (pointer, count) => {
      const view = this.view();
      return Array.from({ length: count }, (_, i) => [view.getUint32(pointer + i * 8, true), view.getUint32(pointer + i * 8 + 4, true)]);
    };
    const open = (fd) => {
      const entry = this.fds.get(fd);
      if (!entry) throw new FsError(ERR.BADF);
      return entry;
    };

    return {
      args_sizes_get: call((count, size) => putList(this.args, count, size)),
      args_get: call((pointers, buffer) => getList(this.args, pointers, buffer)),
      environ_sizes_get: call((count, size) => putList(this.env, count, size)),
      environ_get: call((pointers, buffer) => getList(this.env, pointers, buffer)),
      random_get: call((pointer, length) => {
        for (let at = 0; at < length; at += 65536) {
          crypto.getRandomValues(this.bytes(pointer + at, Math.min(65536, length - at)));
        }
      }),
      clock_time_get: call((id, _precision, pointer) => {
        const ms = id === 0 ? performance.timeOrigin + performance.now() : performance.now();
        this.view().setBigUint64(pointer, BigInt(Math.round(ms * 1e6)), true);
      }),
      sched_yield: call(() => {}),
      proc_exit: (code) => {
        throw new Exit(code);
      },
      poll_oneoff: call((input, output, count, eventsPointer) => {
        const view = this.view();
        let wait = Infinity;
        const ready = [];
        for (let i = 0; i < count; i++) {
          const at = input + i * 48;
          const userdata = view.getBigUint64(at, true);
          const tag = view.getUint8(at + 8);
          if (tag === 0) {
            const timeout = Number(view.getBigUint64(at + 24, true)) / 1e6;
            const absolute = view.getUint16(at + 40, true) & 1;
            const ms = absolute ? timeout - (performance.timeOrigin + performance.now()) : timeout;
            wait = Math.min(wait, Math.max(0, ms));
            ready.push({ userdata, type: 0, ms: Math.max(0, ms) });
          } else {
            ready.push({ userdata, type: tag, ms: 0 });
          }
        }
        const fdReady = ready.some((r) => r.type !== 0);
        if (!fdReady && wait > 0 && wait !== Infinity) Atomics.wait(this.sleeper, 0, 0, wait);
        const fired = fdReady ? ready.filter((r) => r.type !== 0) : ready.filter((r) => r.ms <= wait);
        fired.forEach((r, n) => {
          const at = output + n * 32;
          view.setBigUint64(at, r.userdata, true);
          view.setUint16(at + 8, 0, true);
          view.setUint8(at + 10, r.type);
          view.setBigUint64(at + 16, 1n, true);
          view.setUint16(at + 24, 0, true);
        });
        view.setUint32(eventsPointer, fired.length, true);
      }),

      fd_prestat_get: call((fd, pointer) => {
        const entry = this.fds.get(fd);
        if (!entry?.preopen) return ERR.BADF;
        const view = this.view();
        view.setUint8(pointer, 0);
        view.setUint32(pointer + 4, new TextEncoder().encode(entry.preopen).length, true);
      }),
      fd_prestat_dir_name: call((fd, pointer, length) => {
        const entry = this.fds.get(fd);
        if (!entry?.preopen) return ERR.BADF;
        this.bytes(pointer, length).set(new TextEncoder().encode(entry.preopen).subarray(0, length));
      }),
      fd_fdstat_get: call((fd, pointer) => {
        const type = fd < 3 ? TYPE.CHAR : open(fd).kind === "dir" ? TYPE.DIR : TYPE.FILE;
        const view = this.view();
        view.setUint8(pointer, type);
        view.setUint16(pointer + 2, fd < 3 ? 0 : open(fd).append ? FDFLAGS_APPEND : 0, true);
        view.setBigUint64(pointer + 8, 0xffffffffn, true);
        view.setBigUint64(pointer + 16, 0xffffffffn, true);
      }),
      fd_filestat_get: call((fd, pointer) => {
        if (fd < 3) return this.writeFilestat(pointer, { type: "char", size: 0 });
        const stat = this.stat(open(fd).path);
        if (!stat) return ERR.NOENT;
        this.writeFilestat(pointer, stat);
      }),
      fd_filestat_set_size: call((fd, size) => {
        const { fs, rel } = this.resolve(open(fd).path);
        fs.truncate(rel, Number(size));
      }),
      fd_close: call((fd) => {
        if (fd < 3) return;
        open(fd);
        this.fds.delete(fd);
      }),
      fd_sync: call(() => {}),
      fd_seek: call((fd, offset, whence, pointer) => {
        const entry = open(fd);
        if (entry.kind !== "file") return ERR.BADF;
        const base = whence === 0 ? 0 : whence === 1 ? entry.position : this.stat(entry.path).size;
        const position = base + Number(offset);
        if (position < 0) return ERR.INVAL;
        entry.position = position;
        this.view().setBigUint64(pointer, BigInt(position), true);
      }),
      fd_read: call((fd, iovs, count, pointer) => {
        let total = 0;
        if (fd === 0) {
          for (const [buffer, length] of iovecs(iovs, count)) {
            const bytes = this.stdio[0](length);
            this.bytes(buffer, bytes.length).set(bytes);
            total += bytes.length;
            if (bytes.length < length) break;
          }
        } else {
          const entry = open(fd);
          if (entry.kind !== "file") return ERR.ISDIR;
          const { fs, rel } = this.resolve(entry.path);
          for (const [buffer, length] of iovecs(iovs, count)) {
            const bytes = fs.read(rel, entry.position, length);
            this.bytes(buffer, bytes.length).set(bytes);
            entry.position += bytes.length;
            total += bytes.length;
            if (bytes.length < length) break;
          }
        }
        this.view().setUint32(pointer, total, true);
      }),
      fd_write: call((fd, iovs, count, pointer) => {
        let total = 0;
        const chunks = iovecs(iovs, count).map(([buffer, length]) => this.bytes(buffer, length).slice());
        if (fd === 1 || fd === 2) {
          for (const chunk of chunks) {
            this.stdio[fd](chunk);
            total += chunk.length;
          }
        } else {
          const entry = open(fd);
          if (entry.kind !== "file") return ERR.ISDIR;
          const { fs, rel } = this.resolve(entry.path);
          for (const chunk of chunks) {
            if (entry.append) entry.position = fs.stat(rel).size;
            fs.write(rel, entry.position, chunk);
            entry.position += chunk.length;
            total += chunk.length;
          }
        }
        this.view().setUint32(pointer, total, true);
      }),
      fd_readdir: call((fd, buffer, length, cookie, pointer) => {
        const entry = open(fd);
        if (entry.kind !== "dir") return ERR.NOTDIR;
        const { fs, rel } = this.resolve(entry.path);
        const names = fs.list(rel);
        const view = this.view();
        let used = 0;
        for (let i = Number(cookie); i < names.length && used < length; i++) {
          const name = new TextEncoder().encode(names[i]);
          const stat = fs.stat(rel ? `${rel}/${names[i]}` : names[i]);
          const header = new DataView(new ArrayBuffer(24));
          header.setBigUint64(0, BigInt(i + 1), true);
          header.setBigUint64(8, BigInt(i + 1), true);
          header.setUint32(16, name.length, true);
          header.setUint8(20, stat?.type === "dir" ? TYPE.DIR : TYPE.FILE);
          const record = new Uint8Array(24 + name.length);
          record.set(new Uint8Array(header.buffer));
          record.set(name, 24);
          // A record that doesn't fit is cut short: the caller asks again from its cookie.
          const n = Math.min(record.length, length - used);
          this.bytes(buffer + used, n).set(record.subarray(0, n));
          used += n;
        }
        view.setUint32(pointer, used, true);
      }),

      path_open: call((dirfd, _dirflags, pathPointer, pathLength, oflags, _base, _inheriting, fdflags, pointer) => {
        const path = this.pathAt(dirfd, pathPointer, pathLength);
        const { fs, rel } = this.resolve(path);
        let stat = fs.stat(rel);
        if (stat && oflags & OFLAGS.EXCL && oflags & OFLAGS.CREAT) return ERR.EXIST;
        if (!stat) {
          if (!(oflags & OFLAGS.CREAT)) return ERR.NOENT;
          fs.create(rel);
          stat = fs.stat(rel);
        }
        if (oflags & OFLAGS.DIRECTORY && stat.type !== "dir") return ERR.NOTDIR;
        if (stat.type === "file" && oflags & OFLAGS.TRUNC) fs.truncate(rel, 0);
        let fd = 3;
        while (this.fds.has(fd)) fd++;
        this.fds.set(fd, { kind: stat.type, path, position: 0, append: !!(fdflags & FDFLAGS_APPEND) });
        this.view().setUint32(pointer, fd, true);
      }),
      path_filestat_get: call((dirfd, _flags, pathPointer, pathLength, pointer) => {
        const stat = this.stat(this.pathAt(dirfd, pathPointer, pathLength));
        if (!stat) return ERR.NOENT;
        this.writeFilestat(pointer, stat);
      }),
      path_create_directory: call((dirfd, pathPointer, pathLength) => {
        const { fs, rel } = this.resolve(this.pathAt(dirfd, pathPointer, pathLength));
        fs.mkdir(rel);
      }),
      path_remove_directory: call((dirfd, pathPointer, pathLength) => {
        const { fs, rel } = this.resolve(this.pathAt(dirfd, pathPointer, pathLength));
        fs.rmdir(rel);
      }),
      path_unlink_file: call((dirfd, pathPointer, pathLength) => {
        const { fs, rel } = this.resolve(this.pathAt(dirfd, pathPointer, pathLength));
        fs.unlink(rel);
      }),
      path_rename: call((fromFd, fromPointer, fromLength, toFd, toPointer, toLength) => {
        const from = this.resolve(this.pathAt(fromFd, fromPointer, fromLength));
        const to = this.resolve(this.pathAt(toFd, toPointer, toLength));
        if (from.fs !== to.fs) return ERR.NOSYS;
        from.fs.rename(from.rel, to.rel);
      }),
      // No symbolic links anywhere: an existing path isn't one.
      path_readlink: call((dirfd, pathPointer, pathLength) => (this.stat(this.pathAt(dirfd, pathPointer, pathLength)) ? ERR.INVAL : ERR.NOENT)),
    };
  }
}
