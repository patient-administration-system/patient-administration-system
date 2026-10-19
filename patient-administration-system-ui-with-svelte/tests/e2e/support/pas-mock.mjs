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
// Reply links (CAP-13): any 64-hex token is a fresh, open link until it is acted on once. `0`s is unknown and
// `e`s is expired. Every action that really changes something is recorded at /__acts.
const links = new Map();
const offers = new Map();
const DEFAULT_TEMPLATE = {
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
};
let template = { ...DEFAULT_TEMPLATE };
let proofing = true;
const acts = [];
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
    if (url.pathname === "/__templates/reset") {
      template = { ...DEFAULT_TEMPLATE };
      return res.end("{}");
    }
    if (url.pathname === "/__proofing") {
      proofing = url.searchParams.get("on") !== "false";
      return res.end(JSON.stringify({ proofing }));
    }
    if (url.pathname === "/__acts") return res.end(JSON.stringify(acts));
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
    // PILOT-8: backfill offers. A token starting "eeee" is expired, "9999" lost the slot when accepted.
    const offer = /^\/api\/offer-links\/([^/]+)(?:\/([a-z]+))?$/.exec(url.pathname);
    if (offer) {
      const [, token, action] = offer;
      if (!/^[0-9a-f]{64}$/.test(token) || /^0+$/.test(token)) {
        res.statusCode = 404;
        return res.end(JSON.stringify({ success: false, error: { code: "NOT_FOUND", message: "this link is not valid" } }));
      }
      const state = offers.get(token) ? "used" : token.startsWith("eeee") ? "expired" : "open";
      if (req.method === "GET") {
        return res.end(envelope({ state, starts_at: state === "open" ? "2026-11-05T14:00:00Z" : null, time_zone: "Europe/London", expires_at: null, actions: state === "open" ? ["accept", "decline"] : [] }));
      }
      if (state === "used") {
        res.statusCode = 409;
        return res.end(JSON.stringify({ success: false, error: { code: "CONFLICT", message: "this offer has already been answered" } }));
      }
      if (state === "expired") {
        res.statusCode = 400;
        return res.end(JSON.stringify({ success: false, error: { code: "VALIDATION", message: "this offer has expired" } }));
      }
      offers.set(token, action);
      acts.push({ token, action: `offer-${action}` });
      if (action === "accept" && token.startsWith("9999")) return res.end(envelope({ status: "gone", starts_at: null, time_zone: "Europe/London" }));
      return res.end(
        envelope(action === "accept" ? { status: "accepted", starts_at: "2026-11-05T14:00:00Z", time_zone: "Europe/London" } : { status: "declined", starts_at: null, time_zone: "Europe/London" }),
      );
    }
    // DTAC-9: has staff proofed this account as this patient? On unless a test turns it off.
    const proof = /^\/api\/identity-proofings\/([^/]+)\/check$/.exec(url.pathname);
    if (proof) return res.end(envelope({ proofed: proofing }));
    const link = /^\/api\/reply-links\/([^/]+)(?:\/([a-z]+))?$/.exec(url.pathname);
    if (link) {
      const [, token, action] = link;
      const unknown = !/^[0-9a-f]{64}$/.test(token) || /^0+$/.test(token);
      if (unknown) {
        res.statusCode = 404;
        return res.end(JSON.stringify({ success: false, error: { code: "NOT_FOUND", message: "this link is not valid" } }));
      }
      const expired = /^e+$/.test(token);
      const state = links.get(token) ? "used" : expired ? "expired" : "open";
      const startsAt = "2026-11-03T09:30:00Z";
      if (req.method === "GET") {
        return res.end(envelope({ state, language: "en", when: "Tue 3 Nov 09:30", starts_at: startsAt, time_zone: "Europe/London", actions: state === "open" ? ["confirm", "cancel", "reschedule", "help"] : [] }));
      }
      const body = await new Promise((resolve) => {
        let b = "";
        req.on("data", (c) => (b += c));
        req.on("end", () => resolve(b ? JSON.parse(b) : {}));
      });
      const send = (code, error, message) => {
        res.statusCode = code;
        return res.end(JSON.stringify({ success: false, error: { code: error, message } }));
      };
      if (state === "used") return send(409, "CONFLICT", "this link has already been used");
      if (state === "expired") return send(400, "VALIDATION", "this link has expired");
      if (action === "help") return res.end(envelope({ status: "help" }));
      if (action === "cancel" && !body.confirmed) return res.end(envelope({ status: "needs_confirmation" }));
      // CAP-5: a token starting "aaaa" has two real times to offer; Reschedule only lists them, and the
      // link is used up by choosing one. A token starting "bbbb" lets the choice fail as if the slot had just gone.
      const offered = token.startsWith("aaaa") || token.startsWith("bbbb");
      if (action === "reschedule" && offered) {
        acts.push({ token, action: "reschedule-offered" });
        return res.end(
          envelope({
            status: "rebook_options",
            time_zone: "Europe/London",
            options: [
              { slot_id: "11111111-1111-4111-8111-111111111111", starts_at: "2026-11-10T14:00:00Z", when: "Tue 10 Nov 14:00" },
              { slot_id: "22222222-2222-4222-8222-222222222222", starts_at: "2026-11-11T09:00:00Z", when: "Wed 11 Nov 09:00" },
            ],
          }),
        );
      }
      if (action === "rebook") {
        if (token.startsWith("bbbb")) return send(409, "CONFLICT", "slot 1 is no longer free");
        links.set(token, "rebook");
        acts.push({ token, action: `rebook:${body.slot_id}` });
        return res.end(envelope({ status: "rebooked", starts_at: "2026-11-10T14:00:00Z", time_zone: "Europe/London" }));
      }
      // CAP-7: a token starting "dddd" is on an urgent pathway, so a confirmed cancel is held for a phone call.
      if (action === "cancel" && body.confirmed && token.startsWith("dddd")) {
        links.set(token, action);
        acts.push({ token, action: "cancel-held" });
        return res.end(envelope({ status: "held_for_call" }));
      }
      links.set(token, action);
      acts.push({ token, action: action === "cancel" && body.barrier_code ? `cancel:${body.barrier_code}` : action });
      const status = { confirm: "confirmed", cancel: "cancelled", reschedule: "reschedule_requested" }[action];
      // CAS-2: a reason given when cancelling is answered with help that fits it.
      const signpost = action === "cancel" && body.barrier_code ? `Help for ${body.barrier_code}. Cancelling will not affect your care.` : undefined;
      return res.end(envelope({ status, signpost }));
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
    // REM-5: the template editor saves, so the mock keeps the one template it serves and applies a PUT to it.
    if (url.pathname === "/api/letter-templates") return res.end(envelope([template]));
    const tpl = /^\/api\/letter-templates\/([0-9a-f-]{36})$/.exec(url.pathname);
    if (tpl && req.method === "PUT") {
      const body = await new Promise((resolve) => {
        let b = "";
        req.on("data", (c) => (b += c));
        req.on("end", () => resolve(b ? JSON.parse(b) : {}));
      });
      if (tpl[1] !== template.id) {
        res.statusCode = 404;
        return res.end(JSON.stringify({ success: false, error: { code: "NOT_FOUND", message: "no such template" } }));
      }
      for (const k of ["subject", "body_tera", "channels", "active"]) if (k in body) template[k] = body[k];
      template.updated_at = new Date().toISOString();
      return res.end(envelope(template));
    }
    res.statusCode = 404;
    res.end(JSON.stringify({ success: false, error: { code: "NOT_FOUND", message: "not found" } }));
  })
  .listen(MOCK_PORT, "127.0.0.1", () => console.log(`pas-mock on ${MOCK_PORT}`));
