// The JVM worker's side of a channel to the preparation thread that started it
// (crates/horizon-prepare/src/browser_jvm.rs, which documents the protocol): a filesystem for
// wasi.js whose every operation is a request answered from the game's filesystem, and the JVM's
// standard input, output and error. Calls block the worker until the answer comes (Atomics.wait).

import { FsError } from "./wasi.js";

// The channel's words, requests and states: the same numbers as browser_jvm.rs.
const STATE = 0, OP = 1, A = 2, B = 3, LEN = 4, OFFSET_LO = 5, OFFSET_HI = 6;
const IDLE = 0, REQUEST = 1, REPLY = 2;
const STAT = 1, LIST = 2, READ = 3, WRITE = 4, TRUNCATE = 5, CREATE = 6, MKDIR = 7, UNLINK = 8,
  RMDIR = 9, RENAME = 10, STDOUT = 11, STDERR = 12, STDIN = 13, EXIT = 14;
const ENOENT = 44;
const HEADER = 64;
export const CAPACITY = 1 << 20;

export class Channel {
  /** `memory`: the game module's shared memory; `address`: the channel's place in it. */
  constructor(memory, address) {
    // Views of the memory as it is now: the channel never moves, and a shared buffer's old views
    // stay valid when it grows.
    this.words = new Int32Array(memory.buffer, address, 16);
    this.data = new Uint8Array(memory.buffer, address + HEADER, CAPACITY);
    this.encoder = new TextEncoder();
  }

  /**
   * One request: `path` (relative to `/`) and `payload` go in `data`; returns `{ errno, count,
   * bytes }` (`bytes` a copy).
   */
  request(op, { path = "", payload, a = 0, b = 0, offset = 0 } = {}) {
    const words = this.words;
    const name = this.encoder.encode(path);
    new DataView(this.data.buffer, this.data.byteOffset, 4).setUint32(0, name.length, true);
    this.data.set(name, 4);
    let length = 4 + name.length;
    if (payload) {
      this.data.set(payload, length);
      length += payload.length;
    }
    Atomics.store(words, OP, op);
    Atomics.store(words, A, a);
    Atomics.store(words, B, b);
    Atomics.store(words, LEN, length);
    Atomics.store(words, OFFSET_LO, offset % 2 ** 32);
    Atomics.store(words, OFFSET_HI, Math.floor(offset / 2 ** 32));
    Atomics.store(words, STATE, REQUEST);
    Atomics.notify(words, STATE);
    while (Atomics.load(words, STATE) !== REPLY) Atomics.wait(words, STATE, REQUEST, 1000);
    const reply = {
      errno: Atomics.load(words, A),
      count: Atomics.load(words, B),
      bytes: this.data.slice(0, Atomics.load(words, LEN)),
    };
    Atomics.store(words, STATE, IDLE);
    return reply;
  }

  /** `request`, throwing the errno a failed one returns. */
  call(op, options) {
    const reply = this.request(op, options);
    if (reply.errno) throw new FsError(reply.errno);
    return reply;
  }

  stdout(bytes) {
    for (let at = 0; at < bytes.length; at += CAPACITY - 4) this.call(STDOUT, { payload: bytes.subarray(at, at + CAPACITY - 4) });
  }
  stderr(bytes) {
    for (let at = 0; at < bytes.length; at += CAPACITY - 4) this.call(STDERR, { payload: bytes.subarray(at, at + CAPACITY - 4) });
  }
  stdin(max) {
    return this.call(STDIN, { b: Math.min(max, CAPACITY) }).bytes;
  }
  exit(code) {
    this.request(EXIT, { a: code });
  }
}

/** The game's filesystem, through a channel: a mount for wasi.js. */
export class RpcFs {
  constructor(channel) {
    this.channel = channel;
  }
  stat(path) {
    const reply = this.channel.request(STAT, { path });
    if (reply.errno === ENOENT) return null;
    if (reply.errno) throw new FsError(reply.errno);
    const view = new DataView(reply.bytes.buffer);
    return { type: reply.bytes[0] === 1 ? "dir" : "file", size: view.getFloat64(8, true), mtime: view.getFloat64(16, true) };
  }
  list(path) {
    const text = new TextDecoder().decode(this.channel.call(LIST, { path }).bytes);
    return text ? text.split("\n") : [];
  }
  read(path, offset, length) {
    const parts = [];
    let total = 0;
    while (total < length) {
      const want = Math.min(length - total, CAPACITY);
      const { bytes } = this.channel.call(READ, { path, offset: offset + total, b: want });
      parts.push(bytes);
      total += bytes.length;
      if (bytes.length < want) break;
    }
    if (parts.length === 1) return parts[0];
    const out = new Uint8Array(total);
    let at = 0;
    for (const part of parts) {
      out.set(part, at);
      at += part.length;
    }
    return out;
  }
  write(path, offset, bytes) {
    const room = CAPACITY - 4 - this.channel.encoder.encode(path).length;
    for (let at = 0; at < bytes.length; at += room) {
      this.channel.call(WRITE, { path, offset: offset + at, payload: bytes.subarray(at, at + room) });
    }
    return bytes.length;
  }
  truncate(path, size) {
    this.channel.call(TRUNCATE, { path, offset: size });
  }
  create(path) {
    this.channel.call(CREATE, { path });
  }
  mkdir(path) {
    this.channel.call(MKDIR, { path });
  }
  unlink(path) {
    this.channel.call(UNLINK, { path });
  }
  rmdir(path) {
    this.channel.call(RMDIR, { path });
  }
  rename(from, to) {
    const target = this.channel.encoder.encode(to);
    const payload = new Uint8Array(4 + target.length);
    new DataView(payload.buffer).setUint32(0, target.length, true);
    payload.set(target, 4);
    this.channel.call(RENAME, { path: from, payload });
  }
}
