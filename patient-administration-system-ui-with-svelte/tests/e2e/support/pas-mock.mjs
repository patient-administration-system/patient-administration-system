// A tiny stand-in for the PAS REST API (port 4181). It records every request path
// at /__requests so a test can prove which patient id the BFF asked for.
import http from "node:http";

const PATIENT = "00000000-0000-0000-0000-000000000000";
const requests = [];
const envelope = (data) => JSON.stringify({ success: true, data });

http
  .createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    res.setHeader("content-type", "application/json");
    if (url.pathname === "/__health") return res.end("{}");
    if (url.pathname === "/__requests") return res.end(JSON.stringify(requests));
    requests.push(url.pathname + url.search);
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
    if (url.pathname === "/api/appointments") return res.end(envelope([]));
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
            created_at: "2026-10-01T00:00:00Z",
            updated_at: "2026-10-01T00:00:00Z",
          },
        ]),
      );
    }
    res.statusCode = 404;
    res.end(JSON.stringify({ success: false, error: { code: "NOT_FOUND", message: "not found" } }));
  })
  .listen(4181, "127.0.0.1", () => console.log("pas-mock on 4181"));
