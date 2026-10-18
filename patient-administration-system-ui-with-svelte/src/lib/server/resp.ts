// A small Redis client (RG-12): just enough of RESP2 to keep OpenID Connect sessions in a shared
// store, so several front-end replicas need no sticky sessions. No dependency, on purpose: a
// session store is security-sensitive and this is ~150 lines the project can read in full.
//
// Supports `redis://` and `rediss://` (TLS), `AUTH` (password, or user and password) and `SELECT`.
// One connection, commands answered in order. A command that fails or times out rejects; the
// connection is dropped and the next command opens a new one. No cluster, no pub/sub, no
// pipelining across callers.

import net from "node:net";
import tls from "node:tls";

export type Resp = string | number | null | Resp[] | Error;

/** A reply claiming a bulk string or array bigger than this is refused, not buffered. */
const MAX_BULK = 64 * 1024 * 1024;
const MAX_ARRAY = 1_000_000;

/** An incremental RESP2 parser: feed bytes, take complete replies. */
export class RespParser {
  private buf = Buffer.alloc(0);

  feed(chunk: Buffer): Resp[] {
    this.buf = Buffer.concat([this.buf, chunk]);
    const out: Resp[] = [];
    for (;;) {
      const parsed = this.parseOne(0);
      if (!parsed) break;
      out.push(parsed.value);
      this.buf = this.buf.subarray(parsed.end);
    }
    return out;
  }

  private line(at: number): { text: string; end: number } | null {
    const i = this.buf.indexOf("\r\n", at);
    return i < 0 ? null : { text: this.buf.toString("utf8", at, i), end: i + 2 };
  }

  private parseOne(at: number): { value: Resp; end: number } | null {
    if (at >= this.buf.length) return null;
    const type = String.fromCharCode(this.buf[at]!);
    const l = this.line(at + 1);
    if (!l) return null;
    switch (type) {
      case "+":
        return { value: l.text, end: l.end };
      case "-":
        return { value: new Error(l.text), end: l.end };
      case ":":
        return { value: Number(l.text), end: l.end };
      case "$": {
        const n = Number(l.text);
        if (!Number.isInteger(n) || n > MAX_BULK) throw new Error("unreasonable bulk length");
        if (n < 0) return { value: null, end: l.end };
        if (this.buf.length < l.end + n + 2) return null;
        return { value: this.buf.toString("utf8", l.end, l.end + n), end: l.end + n + 2 };
      }
      case "*": {
        const n = Number(l.text);
        if (!Number.isInteger(n) || n > MAX_ARRAY) throw new Error("unreasonable array length");
        if (n < 0) return { value: null, end: l.end };
        const items: Resp[] = [];
        let p = l.end;
        for (let i = 0; i < n; i++) {
          const item = this.parseOne(p);
          if (!item) return null;
          items.push(item.value);
          p = item.end;
        }
        return { value: items, end: p };
      }
      default:
        throw new Error(`unexpected RESP type ${JSON.stringify(type)}`);
    }
  }
}

/** Encode a command as a RESP array of bulk strings. */
export function encodeCommand(args: (string | number)[]): Buffer {
  const parts: Buffer[] = [Buffer.from(`*${args.length}\r\n`)];
  for (const a of args) {
    const b = Buffer.from(String(a), "utf8");
    parts.push(Buffer.from(`$${b.length}\r\n`), b, Buffer.from("\r\n"));
  }
  return Buffer.concat(parts);
}

export interface RedisOptions {
  /** `redis://[user:password@]host[:port][/db]` or `rediss://…`. */
  url: string;
  /** Milliseconds a command may take (default 3000). */
  timeoutMs?: number;
}

export class RedisClient {
  private socket: net.Socket | tls.TLSSocket | null = null;
  private parser = new RespParser();
  private waiting: { resolve: (v: Resp) => void; reject: (e: Error) => void }[] = [];
  private chain: Promise<unknown> = Promise.resolve();
  private readonly cfg: { host: string; port: number; tls: boolean; user?: string; password?: string; db?: number };
  private readonly timeoutMs: number;

  constructor(opts: RedisOptions) {
    const u = new URL(opts.url);
    if (u.protocol !== "redis:" && u.protocol !== "rediss:") {
      throw new Error("the Redis URL must start with redis:// or rediss://");
    }
    this.cfg = {
      host: u.hostname,
      port: u.port ? Number(u.port) : 6379,
      tls: u.protocol === "rediss:",
      user: u.username ? decodeURIComponent(u.username) : undefined,
      password: u.password ? decodeURIComponent(u.password) : undefined,
      db: u.pathname.length > 1 ? Number(u.pathname.slice(1)) : undefined,
    };
    this.timeoutMs = opts.timeoutMs ?? 3000;
  }

  /** Run one command; commands are sent one at a time, in order. */
  command(...args: (string | number)[]): Promise<Resp> {
    const run = () => this.send(args);
    const next = this.chain.then(run, run);
    this.chain = next.catch(() => undefined);
    return next;
  }

  close(): void {
    this.drop(new Error("closed"));
  }

  private drop(why: Error): void {
    const s = this.socket;
    this.socket = null;
    this.parser = new RespParser();
    s?.destroy();
    for (const w of this.waiting.splice(0)) w.reject(why);
  }

  private async ensure(): Promise<void> {
    if (this.socket) return;
    const { host, port } = this.cfg;
    const socket = await new Promise<net.Socket | tls.TLSSocket>((resolve, reject) => {
      const s = this.cfg.tls
        ? tls.connect({ host, port, servername: host }, () => resolve(s))
        : net.connect({ host, port }, () => resolve(s));
      s.once("error", reject);
      s.setTimeout(this.timeoutMs, () => reject(new Error("Redis connect timed out")));
    });
    socket.setTimeout(0);
    socket.removeAllListeners("error");
    socket.on("error", (e) => this.drop(e));
    socket.on("close", () => this.drop(new Error("Redis connection closed")));
    socket.on("data", (chunk) => {
      let replies: Resp[];
      try {
        replies = this.parser.feed(chunk);
      } catch (e) {
        this.drop(e as Error);
        return;
      }
      for (const r of replies) {
        const w = this.waiting.shift();
        if (!w) continue;
        if (r instanceof Error) w.reject(r);
        else w.resolve(r);
      }
    });
    this.socket = socket;
    if (this.cfg.password) {
      await this.raw(this.cfg.user ? ["AUTH", this.cfg.user, this.cfg.password] : ["AUTH", this.cfg.password]);
    }
    if (this.cfg.db !== undefined) await this.raw(["SELECT", this.cfg.db]);
  }

  private raw(args: (string | number)[]): Promise<Resp> {
    return new Promise<Resp>((resolve, reject) => {
      const socket = this.socket;
      if (!socket) return reject(new Error("not connected"));
      const timer = setTimeout(() => this.drop(new Error("Redis command timed out")), this.timeoutMs);
      this.waiting.push({
        resolve: (v) => (clearTimeout(timer), resolve(v)),
        reject: (e) => (clearTimeout(timer), reject(e)),
      });
      socket.write(encodeCommand(args));
    });
  }

  private async send(args: (string | number)[]): Promise<Resp> {
    await this.ensure();
    return this.raw(args);
  }
}
