// @vitest-environment node
// The Redis client's connection handling (QUAL-19), against a small RESP server on a local port:
// authentication and database selection, replies in order, an error reply, a command that times out, and
// a dropped connection that the next command reopens.
import net from "node:net";
import { afterEach, describe, expect, it } from "vitest";
import { RedisClient, RespParser, encodeCommand, type Resp } from "#lib/server/resp.js";

/** Encode a server reply. */
function reply(v: Resp): Buffer {
  if (v === null) return Buffer.from("$-1\r\n");
  if (v instanceof Error) return Buffer.from(`-${v.message}\r\n`);
  if (typeof v === "number") return Buffer.from(`:${v}\r\n`);
  if (Array.isArray(v)) return Buffer.concat([Buffer.from(`*${v.length}\r\n`), ...v.map(reply)]);
  return encodeCommand([v]).subarray(4); // "$n\r\n…\r\n" without the "*1\r\n"
}

interface Fake {
  url: string;
  seen: string[][];
  connections: number;
  close(): Promise<void>;
}

const servers: Fake[] = [];
afterEach(async () => {
  for (const s of servers.splice(0)) await s.close();
});

/** A server that answers with `answer(command)`: `undefined` never answers, "drop" closes the socket, and
 * `{ delayMs, value }` answers late. */
async function fakeRedis(
  answer: (cmd: string[]) => Resp | undefined | "drop" | { delayMs: number; value: Resp },
): Promise<Fake> {
  const seen: string[][] = [];
  const sockets = new Set<net.Socket>();
  let connections = 0;
  const server = net.createServer((socket) => {
    connections++;
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
    const parser = new RespParser();
    socket.on("data", (chunk) => {
      for (const c of parser.feed(chunk)) {
        const cmd = (c as string[]).map(String);
        seen.push(cmd);
        const a = answer(cmd);
        if (a === "drop") socket.destroy();
        else if (a !== null && typeof a === "object" && "delayMs" in a) {
          const { delayMs, value } = a;
          setTimeout(() => socket.writable && socket.write(reply(value)), delayMs);
        } else if (a !== undefined) socket.write(reply(a));
      }
    });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  const { port } = server.address() as net.AddressInfo;
  const fake: Fake = {
    url: `redis://127.0.0.1:${port}`,
    seen,
    get connections() {
      return connections;
    },
    close: () =>
      new Promise<void>((r) => {
        for (const s of sockets) s.destroy();
        server.close(() => r());
      }),
  };
  servers.push(fake);
  return fake;
}

const store = new Map<string, string>();
const kv = (cmd: string[]): Resp | undefined => {
  switch (cmd[0]) {
    case "AUTH":
    case "SELECT":
      return "OK";
    case "SET":
      store.set(cmd[1]!, cmd[2]!);
      return "OK";
    case "GET":
      return store.get(cmd[1]!) ?? null;
    case "BOOM":
      return new Error("ERR unknown command");
    default:
      return undefined;
  }
};

describe("RedisClient", () => {
  it("authenticates, selects the database, and answers commands in order", async () => {
    const r = await fakeRedis(kv);
    const u = new URL(r.url);
    const c = new RedisClient({ url: `redis://pas:p%40ss@${u.host}/3` });
    const [a, b, d] = await Promise.all([
      c.command("SET", "k", "v é"),
      c.command("GET", "k"),
      c.command("GET", "missing"),
    ]);
    expect([a, b, d]).toEqual(["OK", "v é", null]);
    expect(r.seen.slice(0, 2)).toEqual([
      ["AUTH", "pas", "p@ss"],
      ["SELECT", "3"],
    ]);
    c.close();
  });

  it("rejects an error reply and carries on", async () => {
    const r = await fakeRedis(kv);
    const c = new RedisClient({ url: r.url });
    await expect(c.command("BOOM")).rejects.toThrow("ERR unknown command");
    await expect(c.command("SET", "x", "1")).resolves.toBe("OK");
    c.close();
  });

  it("times out a command that is never answered, then reconnects for the next", async () => {
    const r = await fakeRedis(kv);
    const c = new RedisClient({ url: r.url, timeoutMs: 100 });
    await expect(c.command("HANG")).rejects.toThrow(/timed out/);
    await expect(c.command("SET", "y", "2")).resolves.toBe("OK");
    expect(r.connections).toBe(2);
    c.close();
  });

  it("ignores a late event from a connection it has already dropped", async () => {
    // Found by CI: after a timeout, the dropped socket's `close` arrived while the next command was waiting on
    // a new connection, and dropped that one too. Made certain here by emitting it during the wait.
    const r = await fakeRedis((cmd) => (cmd[0] === "SLOW" ? { delayMs: 80, value: "late" } : kv(cmd)));
    const c = new RedisClient({ url: r.url, timeoutMs: 100 });
    await c.command("SET", "a", "1");
    const dropped = (c as unknown as { socket: net.Socket }).socket;
    await expect(c.command("HANG")).rejects.toThrow(/timed out/);
    const waiting = c.command("SLOW");
    await new Promise((res) => setTimeout(res, 30)); // connected again, waiting for the reply
    dropped.emit("close");
    dropped.emit("error", new Error("ECONNRESET on the old socket"));
    await expect(waiting).resolves.toBe("late");
    c.close();
  });

  it("rejects a command whose connection drops, and reopens it for the next", async () => {
    let first = true;
    const r = await fakeRedis((cmd) => (cmd[0] === "GET" && first ? ((first = false), "drop") : kv(cmd)));
    const c = new RedisClient({ url: r.url });
    await expect(c.command("GET", "k")).rejects.toThrow(/closed/);
    await expect(c.command("SET", "z", "3")).resolves.toBe("OK");
    expect(r.connections).toBe(2);
    c.close();
  });

  it("fails a command when nothing listens", async () => {
    const r = await fakeRedis(kv);
    const url = r.url;
    await r.close();
    servers.length = 0;
    const c = new RedisClient({ url, timeoutMs: 500 });
    await expect(c.command("GET", "k")).rejects.toThrow();
  });

  it("refuses a URL that is not redis:// or rediss://", () => {
    expect(() => new RedisClient({ url: "http://127.0.0.1:6379" })).toThrow(/redis:\/\//);
  });
});
