//! The operations a worker runs, each one HTTP call (or a short dependent chain) against the server under
//! test, and the scenarios that mix them.

use crate::fixtures::Fixtures;
use crate::stats::{Recorder, Sample, Verdict};
use jiff::Timestamp;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use uuid::Uuid;

const HL7_A01: &str = include_str!("../../examples/hl7-adt-a01.txt");

/// What a run drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// Reads only: health, an appointment, a slot, a day or month of the calendar, the two capability statements, and
    /// an HL7 v2 parse (which writes nothing).
    Read,
    /// Each iteration books a free slot, reads it back for its `ETag`, and cancels it with `If-Match`.
    Cycle,
    /// Every worker books from the same few slots at once: a slot must go to exactly one of them.
    Contention,
    /// The IHE ITI Scheduling flow: `$find`, `$hold` one of the proposals, `$book` it.
    Ihe,
    /// 80 in 100 iterations a read, 15 a cycle, 5 the IHE flow.
    Mixed,
}

impl Scenario {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "read" => Self::Read,
            "cycle" => Self::Cycle,
            "contention" => Self::Contention,
            "ihe" => Self::Ihe,
            "mixed" => Self::Mixed,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Cycle => "cycle",
            Self::Contention => "contention",
            Self::Ihe => "ihe",
            Self::Mixed => "mixed",
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Read,
        Self::Cycle,
        Self::Contention,
        Self::Ihe,
        Self::Mixed,
    ];
}

/// Everything the workers share.
pub struct Ctx {
    pub client: Client,
    pub base: String,
    pub token: Option<String>,
    pub fx: Fixtures,
    next_cycle: AtomicUsize,
    /// The appointment ids each contended booking that succeeded got, by slot: read after the run to show
    /// that no slot went to two callers.
    pub contention_wins: Mutex<HashMap<Uuid, Vec<Uuid>>>,
    /// Every appointment booked over HTTP in this run, for the audit and outbox check.
    pub booked: Mutex<Vec<Uuid>>,
}

impl Ctx {
    pub fn new(client: Client, base: String, token: Option<String>, fx: Fixtures) -> Self {
        Self {
            client,
            base,
            token,
            fx,
            next_cycle: AtomicUsize::new(0),
            contention_wins: Mutex::new(HashMap::new()),
            booked: Mutex::new(Vec::new()),
        }
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("x-user-id", "load-test")
            .header("user-agent", "pas-load");
        if let Some(t) = &self.token {
            r = r.bearer_auth(t);
        }
        r
    }

    fn note_booked(&self, id: Uuid) {
        if let Ok(mut v) = self.booked.lock() {
            v.push(id);
        }
    }
}

/// A small, fast generator; the choice of operation needs no cryptographic quality.
pub struct Rng(u64);

impl Rng {
    pub fn seeded(worker: usize) -> Self {
        const MIX: u64 = 0xBF58_476D_1CE4_E5B9;
        let w = u64::try_from(worker).unwrap_or(0);
        // Rotating by the worker number (a bit operation, so nothing overflows) keeps every worker's stream apart.
        let stream = MIX.rotate_left(u32::try_from(w & 63).unwrap_or(0)) ^ w;
        Self(0x9E37_79B9_7F4A_7C15 ^ stream)
    }

    pub fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        // RCA-DECL-1: the divisor is at least 1, so the remainder always exists.
        self.0.checked_rem(n.max(1) as u64).unwrap_or(0) as usize
    }
}

fn iso(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// One call: send, read the whole body (the time to last byte is the latency), classify.
async fn call(
    rec: &mut Recorder,
    op: &'static str,
    req: reqwest::RequestBuilder,
    contended: &[StatusCode],
) -> Option<(StatusCode, reqwest::header::HeaderMap, Vec<u8>)> {
    let started = Instant::now();
    let result = async {
        let res = req.send().await?;
        let status = res.status();
        let headers = res.headers().clone();
        let body = res.bytes().await?.to_vec();
        Ok::<_, reqwest::Error>((status, headers, body))
    }
    .await;
    let elapsed = started.elapsed();
    match result {
        Ok((status, headers, body)) => {
            let verdict = if status.is_success() {
                Verdict::Ok
            } else if contended.contains(&status) {
                Verdict::Contended
            } else {
                Verdict::Error
            };
            rec.record(Sample {
                op,
                status: Some(status.as_u16()),
                verdict,
                elapsed,
            });
            Some((status, headers, body))
        }
        Err(_) => {
            rec.record(Sample {
                op,
                status: None,
                verdict: Verdict::Error,
                elapsed,
            });
            None
        }
    }
}

fn appointment_id(body: &[u8]) -> Option<Uuid> {
    let v: Value = serde_json::from_slice(body).ok()?;
    v["data"]["id"].as_str()?.parse().ok()
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "RCA-EXC-2: `read_day` is a fixture date a little over 600 days ahead, so adding 1 or 31 days stays far inside the calendar"
)]
pub async fn read_once(ctx: &Ctx, rng: &mut Rng, rec: &mut Recorder) {
    let fx = &ctx.fx;
    let pick = rng.below(100);
    let (op, req) = match pick {
        0..5 => ("get_health", ctx.request(Method::GET, "/api/health")),
        5..35 => {
            let id = fx.read_appointments[rng.below(fx.read_appointments.len())];
            (
                "get_appointment",
                ctx.request(Method::GET, &format!("/api/appointments/{id}")),
            )
        }
        35..55 => {
            let id = fx.read_slots[rng.below(fx.read_slots.len())];
            (
                "get_slot",
                ctx.request(Method::GET, &format!("/api/slots/{id}")),
            )
        }
        55..70 => {
            let from = fx.read_day;
            let path = format!(
                "/api/appointments?from={}&to={}",
                iso(from),
                iso(from + patient_administration_system::time::days(1))
            );
            ("list_appointments_day", ctx.request(Method::GET, &path))
        }
        70..75 => {
            let from = fx.read_day;
            let path = format!(
                "/api/appointments?from={}&to={}&limit=500",
                iso(from),
                iso(from + patient_administration_system::time::days(31))
            );
            ("list_appointments_month", ctx.request(Method::GET, &path))
        }
        75..80 => (
            "fhir_r5_metadata",
            ctx.request(Method::GET, "/fhir/metadata"),
        ),
        80..85 => (
            "fhir_r4_metadata",
            ctx.request(Method::GET, "/fhir/r4/metadata"),
        ),
        _ => (
            "hl7v2_parse",
            ctx.request(Method::POST, "/api/hl7/v2/parse")
                .header("content-type", "text/plain")
                .body(HL7_A01),
        ),
    };
    call(rec, op, req, &[]).await;
}

/// Book, read back for the version, cancel with it. Two workers that reach the same slot (the pool wraps
/// round) make one of them see 409, which is counted as contention, not an error.
pub async fn cycle_once(ctx: &Ctx, rec: &mut Recorder) {
    let slots = &ctx.fx.cycle_slots;
    let Some(slot) = ctx
        .next_cycle
        .fetch_add(1, Ordering::Relaxed)
        .checked_rem(slots.len())
        .and_then(|i| slots.get(i))
        .copied()
    else {
        return;
    };
    let book = ctx
        .request(Method::POST, &format!("/api/slots/{slot}/book"))
        .json(&json!({"person_id": Uuid::new_v4()}));
    let Some((StatusCode::OK, _, body)) =
        call(rec, "book_slot", book, &[StatusCode::CONFLICT]).await
    else {
        return;
    };
    let Some(id) = appointment_id(&body) else {
        return;
    };
    ctx.note_booked(id);
    read_and_cancel(ctx, id, rec).await;
}

/// Every worker books a random one of the few hot slots. The first wins; the rest must get 409.
pub async fn contend_once(ctx: &Ctx, rng: &mut Rng, rec: &mut Recorder) {
    let slots = &ctx.fx.hot_slots;
    let slot = slots[rng.below(slots.len())];
    let book = ctx
        .request(Method::POST, &format!("/api/slots/{slot}/book"))
        .json(&json!({"person_id": Uuid::new_v4()}));
    if let Some((StatusCode::OK, _, body)) =
        call(rec, "book_contended", book, &[StatusCode::CONFLICT]).await
        && let Some(id) = appointment_id(&body)
    {
        ctx.note_booked(id);
        if let Ok(mut wins) = ctx.contention_wins.lock() {
            wins.entry(slot).or_default().push(id);
        }
    }
}

fn reference(id: &str) -> Value {
    json!({"resourceType": "Parameters", "parameter": [
        {"name": "appointment-reference", "valueReference": {"reference": format!("Appointment/{id}")}},
    ]})
}

/// `$find` five proposals, `$hold` a random one, `$book` it, then cancel it as the patient would (their
/// list, the appointment's `ETag`, the cancel), so the slot is free again and the pool does not run dry. A
/// proposal another worker held or booked first answers 404 ("no longer available", as the IHE profile
/// asks) or 409, counted as contention. A `$find` that proposes nothing is an error: the pool ran out, and
/// the run would be measuring empty searches.
pub async fn ihe_once(ctx: &Ctx, rng: &mut Rng, rec: &mut Recorder) {
    let fx = &ctx.fx;
    let patient = Uuid::new_v4();
    let find = json!({"resourceType": "Parameters", "parameter": [
        {"name": "start", "valueDateTime": iso(fx.ihe_window.0)},
        {"name": "end", "valueDateTime": iso(fx.ihe_window.1)},
        {"name": "practitioner", "valueReference": {"reference": format!("Practitioner/{}", fx.ihe_practitioner)}},
        {"name": "patient-reference", "valueReference": {"reference": format!("Patient/{patient}")}},
    ]});
    let req = ctx
        .request(Method::POST, "/fhir/r4/Appointment/$find?_count=5")
        .header("content-type", "application/fhir+json")
        .json(&find);
    let started = Instant::now();
    let Some((StatusCode::OK, _, body)) = call(rec, "ihe_find", req, &[]).await else {
        return;
    };
    let ids = proposal_ids(&body);
    if ids.is_empty() {
        rec.record(Sample {
            op: "ihe_find_empty",
            status: Some(200),
            verdict: Verdict::Error,
            elapsed: started.elapsed(),
        });
        return;
    }
    let id = ids[rng.below(ids.len())].clone();
    for (op, path) in [
        ("ihe_hold", "/fhir/r4/Appointment/$hold"),
        ("ihe_book", "/fhir/r4/Appointment/$book"),
    ] {
        let req = ctx
            .request(Method::POST, path)
            .header("content-type", "application/fhir+json")
            .json(&reference(&id));
        let contended = [StatusCode::CONFLICT, StatusCode::NOT_FOUND];
        match call(rec, op, req, &contended).await {
            Some((StatusCode::OK, _, _)) => {}
            _ => return,
        }
    }
    patient_cancels(ctx, patient, rec).await;
}

fn proposal_ids(body: &[u8]) -> Vec<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|b| {
            b["entry"].as_array().map(|e| {
                e.iter()
                    .filter_map(|x| x["resource"]["id"].as_str().map(String::from))
                    .collect()
            })
        })
        // RCA-DECL-1: a body that does not list proposals has none; the scenario then books nothing.
        .unwrap_or(Vec::new())
}

/// The appointment's `ETag`, then the cancel that sends it back in `If-Match` (NFR-27).
async fn read_and_cancel(ctx: &Ctx, id: Uuid, rec: &mut Recorder) {
    let get = ctx.request(Method::GET, &format!("/api/appointments/{id}"));
    let Some((StatusCode::OK, headers, _)) = call(rec, "get_appointment_etag", get, &[]).await
    else {
        return;
    };
    let Some(etag) = headers.get("etag").cloned() else {
        return;
    };
    let cancel = ctx
        .request(Method::POST, &format!("/api/appointments/{id}/cancel"))
        .header("if-match", etag)
        .json(&json!({"cancelled_by": "patient"}));
    call(rec, "cancel_appointment", cancel, &[]).await;
}

/// The patient's own list, their one appointment, and the cancel.
async fn patient_cancels(ctx: &Ctx, patient: Uuid, rec: &mut Recorder) {
    let list = ctx.request(
        Method::GET,
        &format!("/api/patients/{patient}/appointments"),
    );
    let Some((StatusCode::OK, _, body)) = call(rec, "list_patient_appointments", list, &[]).await
    else {
        return;
    };
    let Some(id) = serde_json::from_slice::<Value>(&body).ok().and_then(|v| {
        v["data"][0]["id"]
            .as_str()
            .and_then(|s| s.parse::<Uuid>().ok())
    }) else {
        return;
    };
    ctx.note_booked(id);
    read_and_cancel(ctx, id, rec).await;
}

/// One iteration of a scenario.
pub async fn once(scenario: Scenario, ctx: &Ctx, rng: &mut Rng, rec: &mut Recorder) {
    match scenario {
        Scenario::Read => read_once(ctx, rng, rec).await,
        Scenario::Cycle => cycle_once(ctx, rec).await,
        Scenario::Contention => contend_once(ctx, rng, rec).await,
        Scenario::Ihe => ihe_once(ctx, rng, rec).await,
        Scenario::Mixed => match rng.below(100) {
            0..80 => read_once(ctx, rng, rec).await,
            80..95 => cycle_once(ctx, rec).await,
            _ => ihe_once(ctx, rng, rec).await,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn scenario_server(
        appointment: Uuid,
        empty_find: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind scenario server");
        let address = listener.local_addr().expect("scenario server address");
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut request = [0_u8; 8192];
                    let read = socket.read(&mut request).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&request[..read]);
                    let line = head.lines().next().unwrap_or_default();
                    let path = line.split_whitespace().nth(1).unwrap_or_default();
                    let (status, etag, body) = if path == "/conflict" {
                        ("409 Conflict", "", "{}".to_string())
                    } else if path == "/error" {
                        ("500 Internal Server Error", "", "{}".to_string())
                    } else if path.starts_with("/fhir/r4/Appointment/$find") {
                        (
                            "200 OK",
                            "",
                            if empty_find {
                                r#"{"entry":[]}"#.to_string()
                            } else {
                                r#"{"entry":[{"resource":{"id":"proposal-1"}}]}"#.to_string()
                            },
                        )
                    } else if path.starts_with("/api/patients/") {
                        (
                            "200 OK",
                            "",
                            format!(r#"{{"data":[{{"id":"{appointment}"}}]}}"#),
                        )
                    } else if line.starts_with("POST /api/slots/") {
                        (
                            "200 OK",
                            "",
                            format!(r#"{{"data":{{"id":"{appointment}"}}}}"#),
                        )
                    } else if line.starts_with("GET /api/appointments/") {
                        ("200 OK", "etag: \"fixture-v1\"\r\n", "{}".to_string())
                    } else {
                        ("200 OK", "", "{}".to_string())
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\n{etag}connection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _written = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        (format!("http://{address}"), server)
    }

    fn scenario_context(base: String) -> Ctx {
        let at: Timestamp = "2026-11-20T12:00:00Z".parse().expect("fixed timestamp");
        Ctx::new(
            Client::new(),
            base,
            None,
            Fixtures {
                read_slots: vec![Uuid::new_v4()],
                read_appointments: vec![Uuid::new_v4()],
                read_day: at,
                cycle_slots: vec![Uuid::new_v4()],
                hot_slots: vec![Uuid::new_v4()],
                ihe_practitioner: Uuid::new_v4(),
                ihe_window: (at, at + patient_administration_system::time::days(1)),
            },
        )
    }

    #[test]
    fn every_scenario_name_reads_back() {
        for s in Scenario::ALL {
            assert_eq!(Scenario::parse(s.name()), Some(s));
        }
        assert_eq!(Scenario::parse("bogus"), None);
    }

    #[test]
    fn the_generator_stays_in_range_and_spreads() {
        let mut r = Rng::seeded(3);
        let mut seen = [0usize; 10];
        for _ in 0..10_000 {
            seen[r.below(10)] += 1;
        }
        assert!(seen.iter().all(|&n| n > 800), "{seen:?}");
        assert_eq!(Rng::seeded(1).below(0), 0);
    }

    #[test]
    fn an_appointment_id_is_read_from_the_envelope() {
        let id = Uuid::new_v4();
        let body = serde_json::to_vec(&json!({"data": {"id": id}})).unwrap_or_default();
        assert_eq!(appointment_id(&body), Some(id));
        assert_eq!(appointment_id(b"not json"), None);
        assert_eq!(appointment_id(br#"{"data":{"id":"not-a-uuid"}}"#), None);
    }

    #[test]
    fn request_context_and_ihe_helpers_build_the_expected_wire_values() {
        let _provider = rustls::crypto::ring::default_provider().install_default();
        let at: Timestamp = "2026-11-19T08:00:00Z".parse().unwrap();
        let fx = Fixtures {
            read_slots: Vec::new(),
            read_appointments: Vec::new(),
            read_day: at,
            cycle_slots: Vec::new(),
            hot_slots: Vec::new(),
            ihe_practitioner: Uuid::new_v4(),
            ihe_window: (at, at),
        };
        let ctx = Ctx::new(
            Client::new(),
            "http://example.test".into(),
            Some("token".into()),
            fx,
        );
        let req = ctx.request(Method::GET, "/api/health").build().unwrap();
        assert_eq!(req.url().as_str(), "http://example.test/api/health");
        assert_eq!(req.headers()["x-user-id"], "load-test");
        assert_eq!(req.headers()["authorization"], "Bearer token");

        let booked = Uuid::new_v4();
        ctx.note_booked(booked);
        assert_eq!(*ctx.booked.lock().unwrap(), [booked]);
        assert_eq!(iso(at), "2026-11-19T08:00:00Z");

        let reference = reference("abc");
        assert_eq!(
            reference["parameter"][0]["valueReference"]["reference"],
            "Appointment/abc"
        );
        let proposals = json!({
            "entry": [
                {"resource": {"id": "one"}},
                {"resource": {"id": 2}},
                {"resource": {"id": "three"}}
            ]
        });
        assert_eq!(
            proposal_ids(&serde_json::to_vec(&proposals).unwrap()),
            ["one", "three"]
        );
        assert!(proposal_ids(b"bad json").is_empty());
    }

    #[tokio::test]
    async fn scenarios_drive_their_complete_http_flows_and_classify_failures() {
        let _provider = rustls::crypto::ring::default_provider().install_default();
        let appointment = Uuid::new_v4();
        let (base, server) = scenario_server(appointment, false).await;
        let ctx = scenario_context(base.clone());
        let mut recorder = Recorder::default();

        let expected_reads = [
            "get_health",
            "get_appointment",
            "get_slot",
            "list_appointments_day",
            "list_appointments_month",
            "fhir_r5_metadata",
            "fhir_r4_metadata",
            "hl7v2_parse",
        ];
        for worker in 0..512 {
            once(
                Scenario::Read,
                &ctx,
                &mut Rng::seeded(worker),
                &mut recorder,
            )
            .await;
            let summaries = recorder.summarise(Duration::from_secs(1));
            if expected_reads.iter().all(|op| summaries.contains_key(*op)) {
                break;
            }
        }
        let summaries = recorder.summarise(Duration::from_secs(1));
        assert!(
            expected_reads.iter().all(|op| summaries.contains_key(*op)),
            "{summaries:?}"
        );

        once(Scenario::Cycle, &ctx, &mut Rng::seeded(1), &mut recorder).await;
        once(
            Scenario::Contention,
            &ctx,
            &mut Rng::seeded(2),
            &mut recorder,
        )
        .await;
        once(Scenario::Ihe, &ctx, &mut Rng::seeded(3), &mut recorder).await;
        for worker in 0..128 {
            once(
                Scenario::Mixed,
                &ctx,
                &mut Rng::seeded(worker),
                &mut recorder,
            )
            .await;
        }

        let summaries = recorder.summarise(Duration::from_secs(1));
        for op in [
            "book_slot",
            "get_appointment_etag",
            "cancel_appointment",
            "book_contended",
            "ihe_find",
            "ihe_hold",
            "ihe_book",
            "list_patient_appointments",
        ] {
            assert!(summaries.contains_key(op), "missing {op}: {summaries:?}");
        }
        assert!(summaries.values().all(|summary| summary.errors == 0));
        assert!(!ctx.booked.lock().expect("booked lock").is_empty());
        assert!(
            !ctx.contention_wins
                .lock()
                .expect("contention lock")
                .is_empty()
        );

        assert!(
            call(
                &mut recorder,
                "expected_conflict",
                ctx.request(Method::GET, "/conflict"),
                &[StatusCode::CONFLICT],
            )
            .await
            .is_some()
        );
        assert!(
            call(
                &mut recorder,
                "server_error",
                ctx.request(Method::GET, "/error"),
                &[],
            )
            .await
            .is_some()
        );
        server.abort();
        assert!(
            call(
                &mut recorder,
                "transport_error",
                Client::new().get(format!("{base}/after-stop")),
                &[],
            )
            .await
            .is_none()
        );

        let final_summaries = recorder.summarise(Duration::from_secs(1));
        assert_eq!(final_summaries["expected_conflict"].contended, 1);
        assert_eq!(final_summaries["server_error"].errors, 1);
        assert_eq!(final_summaries["transport_error"].errors, 1);

        let (empty_base, empty_server) = scenario_server(appointment, true).await;
        let empty_ctx = scenario_context(empty_base);
        let mut empty_recorder = Recorder::default();
        ihe_once(&empty_ctx, &mut Rng::seeded(4), &mut empty_recorder).await;
        let empty = empty_recorder.summarise(Duration::from_secs(1));
        assert_eq!(empty["ihe_find_empty"].errors, 1);
        assert!(!empty.contains_key("ihe_hold"));
        empty_server.abort();
    }
}
