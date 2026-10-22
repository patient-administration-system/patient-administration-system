// A stand-in for person-service, the separate identity service the PAS API proxies patient search to
// (QUAL-20). Everything else in the real-API browser tests is the real PAS API on a real database; this
// answers only what patient search asks, in person-service's envelope.
import http from "node:http";

const PORT = Number(process.env.PERSON_SERVICE_PORT ?? 4197);
const PEOPLE = [
  {
    id: "5eed0000-0000-4000-8000-000000000001",
    name: { given: ["Ada"], family: "Lovelace" },
    birth_date: "1815-12-10",
  },
  {
    id: "5eed0000-0000-4000-8000-000000000002",
    name: { given: ["Grace"], family: "Hopper" },
    birth_date: "1906-12-09",
  },
];
const envelope = (data) => JSON.stringify({ success: true, data, error: null });

http
  .createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");
    res.setHeader("content-type", "application/json");
    if (url.pathname === "/api/health") return res.end(envelope({ status: "ok" }));
    if (url.pathname === "/api/persons/search") {
      const q = (url.searchParams.get("q") ?? "").toLowerCase();
      return res.end(envelope(PEOPLE.filter((p) => JSON.stringify(p.name).toLowerCase().includes(q))));
    }
    res.statusCode = 404;
    res.end(JSON.stringify({ success: false, data: null, error: { code: "NOT_FOUND", message: "not found" } }));
  })
  .listen(PORT, "127.0.0.1", () => console.log(`person-service stand-in on ${PORT}`));
