// A tiny stand-in for the PAS REST API (port MOCK_PORT, default 4181). It records every request path
// at /__requests so a test can prove which patient id the BFF asked for.
import crypto from "node:crypto";
import http from "node:http";

// Another project on the same machine may hold 4181; run with MOCK_PORT=4191 (or any free port).
const MOCK_PORT = Number(process.env.MOCK_PORT ?? 4181);
const PATIENT = "00000000-0000-0000-0000-000000000000";
const requests = [];
// Who each request claimed to act for, and whether its token was genuine (see `verify`).
const actors = [];
const ISSUER_URL = process.env.ISSUER_URL ?? null;
const ISSUER = "authentication-service";
const AUDIENCE = "patient-administration-system";

// Verifies a PASETO v4.public token the way the real API does (Ed25519 over the pre-authentication
// encoding), against the keys the real issuer serves. Returns the claims, or null for any defect.
let keyCache = { at: 0, keys: {} };
async function keys() {
  if (Date.now() - keyCache.at < 2000) return keyCache.keys;
  const doc = await (await fetch(`${ISSUER_URL}/keys.json`)).json();
  keyCache = { at: Date.now(), keys: Object.fromEntries(doc.keys.map((k) => [k.kid, k.x])) };
  return keyCache.keys;
}
const le64 = (n) => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
const pae = (...parts) => Buffer.concat([le64(parts.length), ...parts.flatMap((p) => [le64(p.length), p])]);
async function verify(header) {
  try {
    const token = (header ?? "").replace(/^Bearer /i, "").trim();
    const [version, purpose, body, footer] = token.split(".");
    if (version !== "v4" || purpose !== "public" || !body || !footer) return null;
    const kid = JSON.parse(Buffer.from(footer, "base64url").toString()).kid;
    const x = (await keys())[kid];
    if (!x) return null;
    const raw = Buffer.from(body, "base64url");
    const msg = raw.subarray(0, raw.length - 64);
    const sig = raw.subarray(raw.length - 64);
    const spki = Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(x, "base64url")]);
    const pub = crypto.createPublicKey({ key: spki, format: "der", type: "spki" });
    const ok = crypto.verify(null, pae(Buffer.from("v4.public."), msg, Buffer.from(footer, "base64url"), Buffer.alloc(0)), pub, sig);
    if (!ok) return null;
    const claims = JSON.parse(msg.toString());
    if (claims.iss !== ISSUER || claims.aud !== AUDIENCE || claims.exp <= Date.now() / 1000) return null;
    return claims;
  } catch {
    return null;
  }
}
const envelope = (data) => JSON.stringify({ success: true, data });

http
  .createServer(async (req, res) => {
    const url = new URL(req.url, "http://x");
    res.setHeader("content-type", "application/json");
    if (url.pathname === "/__health") return res.end("{}");
    if (url.pathname === "/__requests") return res.end(JSON.stringify(requests));
    if (url.pathname === "/__actors") return res.end(JSON.stringify(actors));
    requests.push(url.pathname + url.search);
    if (ISSUER_URL) {
      const claims = await verify(req.headers.authorization);
      const isService = claims?.attrs?.svc?.includes("true") ?? false;
      actors.push({ path: url.pathname, actor: req.headers["x-on-behalf-of"] ?? null, sub: claims?.sub ?? null, service: isService });
      if (!claims) {
        res.statusCode = 401;
        return res.end(JSON.stringify({ success: false, error: { code: "UNAUTHORIZED", message: "invalid or expired token" } }));
      }
    }
    if (url.pathname === `/api/patients/${PATIENT}/appointments`) {
      return res.end(
        envelope([
          {
            id: "11111111-1111-1111-1111-111111111111",
            person_id: PATIENT,
            start_datetime: "2026-11-03T09:30:00Z",
            end_datetime: "2026-11-03T10:00:00Z",
            status: "booked",
            reason: "Knee review",
          },
        ]),
      );
    }
    if (url.pathname === `/api/patients/${PATIENT}/letters`) {
      return res.end(
        envelope(
          url.searchParams.get("channel") === "portal"
            ? [
                {
                  id: "33333333-3333-3333-3333-333333333333",
                  rendered_subject: "Your appointment letter",
                  rendered_body: "Please bring your medication list.",
                  created_at: "2026-10-18T09:00:00Z",
                },
              ]
            : [],
        ),
      );
    }
    if (url.pathname === "/api/appointments") {
      // Two appointments in the coming days, so the calendar and the schedule chart have content.
      const at = (days, hour) => {
        const d = new Date(Date.now() + days * 86_400_000);
        d.setUTCHours(hour, 0, 0, 0);
        return d.toISOString();
      };
      return res.end(
        envelope([
          { id: "44444444-4444-4444-4444-444444444444", person_id: PATIENT, start_datetime: at(1, 9), end_datetime: at(1, 10), status: "booked", reason: "Knee review" },
          { id: "55555555-5555-5555-5555-555555555555", person_id: PATIENT, start_datetime: at(3, 14), end_datetime: at(3, 15), status: "booked", reason: null },
        ]),
      );
    }
    if (url.pathname === "/api/waitlist") {
      return res.end(
        envelope([
          { id: "66666666-6666-6666-6666-666666666666", person_id: PATIENT, target_service: "general", priority: "urgent", status: "waiting" },
          { id: "77777777-7777-7777-7777-777777777777", person_id: PATIENT, target_service: "general", priority: "routine", status: "booked" },
        ]),
      );
    }
    if (url.pathname === "/api/patients") {
      return res.end(envelope([{ id: PATIENT, name: { given: ["Ada"], family: "Lovelace" }, birth_date: "1815-12-10" }]));
    }
    if (url.pathname === "/api/letter-templates") {
      return res.end(
        envelope([
          {
            id: "22222222-2222-2222-2222-222222222222",
            name: "appointment_reminder",
            subject: "Appointment reminder",
            body_tera: "Hello {{ patient.name.given }}",
            required_variables: [],
            channels: ["whatsapp", "sms"],
            active: true,
            locale: "en-001",
            created_at: "2026-10-01T00:00:00Z",
            updated_at: "2026-10-01T00:00:00Z",
          },
        ]),
      );
    }
    res.statusCode = 404;
    res.end(JSON.stringify({ success: false, error: { code: "NOT_FOUND", message: "not found" } }));
  })
  .listen(MOCK_PORT, "127.0.0.1", () => console.log(`pas-mock on ${MOCK_PORT}`));
