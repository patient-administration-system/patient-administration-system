// @vitest-environment node
// The Redis protocol parser reads bytes off the network (QUAL-19). A property test: any sequence of replies,
// encoded and cut into chunks at random points (including inside a multi-byte character), parses back to
// the same replies. Seeded, so a failure repeats; no property-testing dependency.
import { describe, expect, it } from "vitest";
import { RespParser, encodeCommand, type Resp } from "#lib/server/resp.js";

/** Mulberry32: small, seeded, good enough to explore inputs. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const ALPHABET = ["a", "Z", "0", " ", "\r", "\n", "é", "ŵ", "日", "🙂", ":", "$", "*", "-", "+"];

function text(r: () => number, simple: boolean): string {
  const n = Math.floor(r() * 12);
  let s = "";
  for (let i = 0; i < n; i++) {
    const c = ALPHABET[Math.floor(r() * ALPHABET.length)]!;
    // Simple strings and errors are one line: no CR or LF inside.
    s += simple && (c === "\r" || c === "\n") ? "x" : c;
  }
  return s;
}

function value(r: () => number, depth = 0): Resp {
  const k = Math.floor(r() * (depth < 3 ? 6 : 5));
  switch (k) {
    case 0:
      return text(r, true);
    case 1:
      return new Error(text(r, true));
    case 2:
      return Math.floor(r() * 2_000_000) - 1_000_000;
    case 3:
      return null;
    case 4:
      return text(r, false);
    default:
      return Array.from({ length: Math.floor(r() * 4) }, () => value(r, depth + 1));
  }
}

/** Encode a reply the way a Redis server would. Strings alternate between simple and bulk. */
function encode(v: Resp, r: () => number): Buffer {
  if (v === null) return Buffer.from(r() < 0.5 ? "$-1\r\n" : "*-1\r\n");
  if (v instanceof Error) return Buffer.from(`-${v.message}\r\n`);
  if (typeof v === "number") return Buffer.from(`:${v}\r\n`);
  if (Array.isArray(v)) return Buffer.concat([Buffer.from(`*${v.length}\r\n`), ...v.map((x) => encode(x, r))]);
  if (!/[\r\n]/.test(v) && r() < 0.5) return Buffer.from(`+${v}\r\n`);
  const b = Buffer.from(v, "utf8");
  return Buffer.concat([Buffer.from(`$${b.length}\r\n`), b, Buffer.from("\r\n")]);
}

/** A null array decodes as null; an error compares by message. */
function normal(v: Resp): unknown {
  if (v instanceof Error) return { error: v.message };
  if (Array.isArray(v)) return v.map(normal);
  return v;
}

describe("RespParser", () => {
  it("parses any replies, however the bytes are split", () => {
    for (let seed = 1; seed <= 300; seed++) {
      const r = rng(seed);
      const replies = Array.from({ length: 1 + Math.floor(r() * 6) }, () => value(r));
      const bytes = Buffer.concat(replies.map((v) => encode(v, r)));
      const parser = new RespParser();
      const got: Resp[] = [];
      let at = 0;
      while (at < bytes.length) {
        const step = 1 + Math.floor(r() * 7);
        got.push(...parser.feed(bytes.subarray(at, at + step)));
        at += step;
      }
      expect(got.map(normal), `seed ${seed}`).toEqual(replies.map(normal));
    }
  });

  it("waits for a whole reply and keeps what follows it", () => {
    const p = new RespParser();
    expect(p.feed(Buffer.from("$5\r\nhel"))).toEqual([]);
    expect(p.feed(Buffer.from("lo\r\n+OK\r\n:4"))).toEqual(["hello", "OK"]);
    expect(p.feed(Buffer.from("2\r\n"))).toEqual([42]);
  });

  it("refuses lengths it will not buffer, and unknown types", () => {
    expect(() => new RespParser().feed(Buffer.from(`$${64 * 1024 * 1024 + 1}\r\n`))).toThrow(/bulk length/);
    expect(() => new RespParser().feed(Buffer.from("*1000001\r\n"))).toThrow(/array length/);
    expect(() => new RespParser().feed(Buffer.from("$abc\r\n"))).toThrow(/bulk length/);
    expect(() => new RespParser().feed(Buffer.from("?x\r\n"))).toThrow(/unexpected RESP type/);
  });
});

describe("encodeCommand", () => {
  it("writes an array of bulk strings, counting bytes not characters", () => {
    expect(encodeCommand(["SET", "k", "é", 5]).toString("utf8")).toBe("*4\r\n$3\r\nSET\r\n$1\r\nk\r\n$2\r\né\r\n$1\r\n5\r\n");
  });

  it("round-trips through the parser as an array of strings", () => {
    const args = ["SET", "pas:sess:abc", '{"a":"line\r\nbreak 🙂"}', "EX", "60"];
    expect(new RespParser().feed(encodeCommand(args))).toEqual([args]);
  });
});
